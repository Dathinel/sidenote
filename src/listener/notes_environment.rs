//! Refresh the scheduled listener's cached environment for each notes script job.
use std::{
    ffi::OsString,
    io,
    os::windows::{
        ffi::OsStringExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    process::Command,
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    Security::{TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY},
    System::{
        Environment::ExpandEnvironmentStringsForUserW,
        Registry::{
            HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ,
            RRF_RT_REG_SZ, RegGetValueW,
        },
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

const NAMES: [&str; 8] = [
    "HEX_MEETING_PROJECT",
    "HEX_MEETING_PROJECT_PATTERN",
    "HEX_MEETING_PROJECT_CONTEXT_PATTERN",
    "HEX_MEETING_KB_REPO",
    "HEX_MEETING_OWNER_NAME",
    "HEX_MEETING_COMPANY_NAME",
    "HEX_MEETING_PRODUCT_NAME",
    "HEX_TE_ASR_MODEL",
];

pub(super) fn read_user(name: &str) -> io::Result<Option<OsString>> {
    let subkey: Vec<u16> = "Environment\0".encode_utf16().collect();
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // A Windows environment value is at most 32,767 characters plus its terminator.
    let mut buffer = vec![0u16; 32768];
    let mut bytes = (buffer.len() * size_of::<u16>()) as u32;
    let mut kind = 0;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            name.as_ptr(),
            // Expand below using the user's environment, not the listener's cached block.
            RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND,
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    match status {
        ERROR_SUCCESS => decode_registry_value(kind, &buffer, bytes, expand_for_user).map(Some),
        ERROR_FILE_NOT_FOUND => Ok(None),
        code => Err(io::Error::from_raw_os_error(code as i32)),
    }
}

fn decode_registry_value(
    kind: u32,
    buffer: &[u16],
    bytes: u32,
    expand: impl FnOnce(&[u16]) -> io::Result<OsString>,
) -> io::Result<OsString> {
    if !bytes.is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid registry string",
        ));
    }
    let value = buffer
        .get(..bytes as usize / 2)
        .filter(|value| value.last() == Some(&0) && !value[..value.len() - 1].contains(&0))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid registry string"))?;
    match kind {
        REG_SZ => Ok(OsString::from_wide(&value[..value.len() - 1])),
        REG_EXPAND_SZ => expand(value),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Unsupported registry string type",
        )),
    }
}

fn expand_for_user(value: &[u16]) -> io::Result<OsString> {
    let mut token = std::ptr::null_mut();
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE,
            &mut token,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut expanded = vec![0u16; 32768];
    // Userenv builds the user's environment from registry settings for this expansion.
    if unsafe {
        ExpandEnvironmentStringsForUserW(
            token.as_raw_handle(),
            value.as_ptr(),
            expanded.as_mut_ptr(),
            expanded.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let length = expanded.iter().position(|&c| c == 0).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid expanded environment string",
        )
    })?;
    Ok(OsString::from_wide(&expanded[..length]))
}

pub(super) fn apply(
    command: &mut Command,
    mut registry: impl FnMut(&str) -> io::Result<Option<OsString>>,
    mut process: impl FnMut(&str) -> Option<OsString>,
    mut report: impl FnMut(&str),
) -> io::Result<()> {
    for name in NAMES {
        match registry(name)?.or_else(|| process(name)) {
            Some(value) => {
                command.env(name, value);
            }
            None => {
                command.env_remove(name);
                report(&format!("notes environment missing={name}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_user_expansion_handles_a_synthetic_expandable_value() {
        let raw: Vec<u16> = "%SystemRoot%\\sidenote-synthetic\0"
            .encode_utf16()
            .collect();
        let expanded = decode_registry_value(
            REG_EXPAND_SZ,
            &raw,
            (raw.len() * size_of::<u16>()) as u32,
            expand_for_user,
        )
        .unwrap();
        // Check only shape so no real environment values appear in assertion diagnostics.
        assert!(!expanded.to_string_lossy().contains('%'));
        assert!(std::path::Path::new(&expanded).is_absolute());
    }

    #[test]
    fn expandable_registry_value_is_decoded_and_expanded_before_child_assignment() {
        let raw: Vec<u16> = "%USERPROFILE%\\kb\0".encode_utf16().collect();
        let bytes = (raw.len() * size_of::<u16>()) as u32;
        for kind in [REG_SZ, REG_EXPAND_SZ] {
            let mut command = Command::new("unused");
            let mut expansions = 0;
            apply(
                &mut command,
                |name| {
                    if name != "HEX_MEETING_KB_REPO" {
                        return Ok(Some("synthetic".into()));
                    }
                    // Inject the native reader's type and UTF-16 payload at its conversion boundary.
                    decode_registry_value(kind, &raw, bytes, |value| {
                        expansions += 1;
                        assert_eq!(value, raw);
                        Ok(r"C:\synthetic-user\kb".into())
                    })
                    .map(Some)
                },
                |_| panic!("registry values must win"),
                |_| panic!("nothing missing"),
            )
            .unwrap();
            let expected = if kind == REG_EXPAND_SZ {
                r"C:\synthetic-user\kb"
            } else {
                r"%USERPROFILE%\kb"
            };
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == "HEX_MEETING_KB_REPO"
                        && value == Some(std::ffi::OsStr::new(expected)))
            );
            assert_eq!(expansions, usize::from(kind == REG_EXPAND_SZ));
        }
    }

    #[test]
    fn registry_conversion_rejects_partial_strings_and_propagates_expansion_errors() {
        let raw: Vec<u16> = "%SYNTHETIC%\0".encode_utf16().collect();
        let bytes = (raw.len() * size_of::<u16>()) as u32;
        for bad_bytes in [0, 1, bytes - 2, bytes + 2] {
            let error = decode_registry_value(REG_EXPAND_SZ, &raw, bad_bytes, |_| {
                panic!("invalid string must not expand")
            })
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        let error = decode_registry_value(REG_EXPAND_SZ, &raw, bytes, |_| {
            Err(io::Error::from_raw_os_error(5))
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(5));
    }

    #[test]
    fn fresh_registry_wins_for_all_eight_names_and_each_job() {
        for fresh in ["synthetic first", "synthetic second"] {
            let mut command = Command::new("unused");
            let mut queried = Vec::new();
            apply(
                &mut command,
                |name| {
                    queried.push(name.to_owned());
                    Ok(Some(fresh.into()))
                },
                |_| panic!("process fallback must not override registry"),
                |_| panic!("nothing missing"),
            )
            .unwrap();
            assert_eq!(queried, NAMES);
            let env: Vec<_> = command.get_envs().collect();
            assert_eq!(env.len(), 8);
            for name in NAMES {
                assert!(env.iter().any(
                    |(key, value)| key == &name && *value == Some(std::ffi::OsStr::new(fresh))
                ));
            }
        }
    }

    #[test]
    fn only_missing_registry_values_use_process_and_logs_contain_names_only() {
        let mut command = Command::new("unused");
        let mut logs = Vec::new();
        apply(
            &mut command,
            |name| Ok((name == NAMES[0]).then(OsString::new)),
            |name| (name != NAMES[0] && name != NAMES[7]).then(|| "synthetic fallback".into()),
            |line| logs.push(line.to_owned()),
        )
        .unwrap();
        let env: Vec<_> = command.get_envs().collect();
        assert_eq!(env.len(), 8);
        for name in NAMES {
            let expected = if name == NAMES[0] {
                Some(std::ffi::OsStr::new(""))
            } else if name == NAMES[7] {
                None
            } else {
                Some(std::ffi::OsStr::new("synthetic fallback"))
            };
            assert!(
                env.iter()
                    .any(|(key, value)| key == &name && *value == expected)
            );
        }
        assert_eq!(logs, ["notes environment missing=HEX_TE_ASR_MODEL"]);
    }

    #[test]
    fn registry_errors_do_not_silently_use_stale_process_values() {
        let mut command = Command::new("unused");
        let error = apply(
            &mut command,
            |_| Err(io::Error::from_raw_os_error(5)),
            |_| panic!("only missing registry values may fall back"),
            |_| panic!("not missing"),
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(5));
    }
}
