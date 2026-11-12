//! Private per-client bearer tokens, stored only as digests by the server.
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::Path,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    client: String,
    revision: String,
}
type Grants = BTreeMap<String, String>;
fn denied() -> io::Error {
    io::Error::other("MCP_ACCESS_DENIED")
}
fn valid(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 64
        && label
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn read(root: &Path) -> io::Result<Grants> {
    let directory = root.join("local/mcp");
    sidenote_sync::validate_private_directory(&directory).map_err(|_| denied())?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join("clients.json"))?;
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
        || meta.nlink() != 1
    {
        return Err(denied());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(65537)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(denied());
    }
    serde_json::from_slice(&bytes).map_err(|_| denied())
}
pub(crate) fn current(root: &Path, client: &str) -> io::Result<Grant> {
    if !valid(client) {
        return Err(denied());
    }
    let revision = read(root)?.remove(client).ok_or_else(denied)?;
    Ok(Grant {
        client: client.into(),
        revision,
    })
}
pub(crate) fn check(root: &Path, grant: &Grant) -> io::Result<()> {
    if current(root, &grant.client)?.revision != grant.revision {
        return Err(denied());
    }
    Ok(())
}
pub fn authenticate(root: &Path, bearer: &str) -> io::Result<Grant> {
    let (client, token) = bearer.split_once('.').ok_or_else(denied)?;
    if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(denied());
    }
    let grant = current(root, client)?;
    let hash = blake3::hash(bearer.as_bytes()).to_hex().to_string();
    let difference = hash
        .bytes()
        .zip(grant.revision.bytes())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b));
    if grant.revision.len() != hash.len() || difference != 0 {
        return Err(denied());
    }
    Ok(grant)
}
fn update(root: &Path, action: &str, client: &str, output: Option<&Path>) -> io::Result<()> {
    if !valid(client) || !matches!(action, "grant" | "revoke") {
        return Err(denied());
    }
    let _lock = crate::core::store_io::lock(&root.join("local/mcp/access.lock"))?;
    sidenote_sync::validate_private_directory(&root.join("local/mcp")).map_err(|_| denied())?;
    let mut grants = match read(root) {
        Ok(grants) => grants,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Grants::new(),
        Err(error) => return Err(error),
    };
    if action == "grant" {
        if grants.len() >= 128 && !grants.contains_key(client) {
            return Err(denied());
        }
        let output = output.filter(|p| p.is_absolute()).ok_or_else(denied)?;
        sidenote_sync::validate_private_directory(output.parent().ok_or_else(denied)?)
            .map_err(|_| denied())?;
        let random = zeroize::Zeroizing::new(sidenote_sync::identity::random32());
        let encoded = zeroize::Zeroizing::new(hex::encode(random.as_slice()));
        let token = zeroize::Zeroizing::new(format!("{client}.{}", encoded.as_str()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(output)?;
        if file
            .write_all(token.as_bytes())
            .and_then(|_| file.sync_all())
            .is_err()
        {
            let _ = fs::remove_file(output);
            return Err(denied());
        }
        grants.insert(
            client.into(),
            blake3::hash(token.as_bytes()).to_hex().to_string(),
        );
    } else {
        grants.remove(client);
    }
    let result = crate::core::store_io::write(&root.join("local/mcp/clients.json"), &grants);
    if result.is_err()
        && action == "grant"
        && let Some(output) = output
    {
        let _ = fs::remove_file(output);
    }
    result
}
pub fn cli(args: &[std::ffi::OsString]) -> i32 {
    let fields = args
        .iter()
        .skip_while(|a| *a != "--mcp-access")
        .skip(1)
        .map(|a| a.to_str())
        .collect::<Vec<_>>();
    let result = match fields.as_slice() {
        [Some("grant"), Some(client), Some(output)] => update(
            &crate::core::state_dir(),
            "grant",
            client,
            Some(Path::new(output)),
        ),
        [Some("revoke"), Some(client)] => update(&crate::core::state_dir(), "revoke", client, None),
        _ => Err(denied()),
    };
    if result.is_ok() {
        println!("MCP_ACCESS_UPDATED");
        0
    } else {
        eprintln!("MCP_ACCESS_DENIED");
        2
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deny_revoke_regrant_and_unsafe_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("token");
        assert!(current(root.path(), "phone").is_err());
        for client in ["../other", "Phone", "", "a/b"] {
            assert!(update(root.path(), "grant", client, Some(&output)).is_err());
        }
        update(root.path(), "grant", "phone", Some(&output)).unwrap();
        let token = zeroize::Zeroizing::new(fs::read_to_string(&output).unwrap());
        assert!(authenticate(root.path(), &token).is_ok());
        assert!(authenticate(root.path(), &format!("phone.{}", "0".repeat(64))).is_err());
        assert!(
            !fs::read_to_string(root.path().join("local/mcp/clients.json"))
                .unwrap()
                .contains(token.as_str())
        );
        assert!(update(root.path(), "grant", "phone", Some(&output)).is_err());
        let old = current(root.path(), "phone").unwrap();
        check(root.path(), &old).unwrap();
        update(root.path(), "revoke", "phone", None).unwrap();
        assert!(check(root.path(), &old).is_err());
        fs::remove_file(&output).unwrap();
        update(root.path(), "grant", "phone", Some(&output)).unwrap();
        assert!(check(root.path(), &old).is_err());
        let path = root.path().join("local/mcp/clients.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(current(root.path(), "phone").is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, "{synthetic-canary").unwrap();
        assert!(current(root.path(), "phone").is_err());
    }
}
