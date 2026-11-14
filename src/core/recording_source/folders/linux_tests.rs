use super::*;
use crate::core::recording_source::Session;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};

fn source(meetings: &std::path::Path) -> Discovery {
    Discovery::hex_support(meetings.parent().unwrap().into()).unwrap()
}

fn collect(session: &mut Session<Discovery>, queued: &mut Vec<String>) {
    session
        .drain(|m| {
            queued.push(m.id().to_owned());
            Ok(())
        })
        .unwrap();
}

#[test]
fn completion_restart_and_missing_tracks_preserve_originals() {
    let (_root, meetings, cursor) = super::super::tests::fixture();
    let folder = meetings.join("99-1");
    let manifest = folder.join("manifest.json");
    fs::write(&manifest, br#"{"status":"transcribing"}"#).unwrap();
    let originals = ["system.wav", "microphone.wav", "notes.md"].map(|name| {
        let path = folder.join(name);
        let bytes = format!("synthetic {name}").into_bytes();
        fs::write(&path, &bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        let meta = fs::metadata(&path).unwrap();
        (path, bytes, meta)
    });
    let mut queued = Vec::new();
    let mut first = Session::open(source(&meetings), cursor.clone()).unwrap();
    collect(&mut first, &mut queued);
    drop(first);
    assert_eq!(queued, ["100-1"]); // A complete transcript alone is insufficient.
    fs::write(manifest, br#"{"status":"complete"}"#).unwrap();
    let mut restarted = Session::open(source(&meetings), cursor.clone()).unwrap();
    for _ in 0..2 {
        collect(&mut restarted, &mut queued);
    }
    drop(restarted);
    Session::open(source(&meetings), cursor)
        .unwrap()
        .drain(|_| panic!("duplicate"))
        .unwrap();
    assert_eq!(queued, ["100-1", "99-1"]); // 100-1 has no audio; expired tracks are optional.
    for (path, bytes, before) in originals {
        let after = fs::metadata(&path).unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(
            (
                before.ino(),
                before.len(),
                before.mode(),
                before.mtime(),
                before.mtime_nsec(),
                before.ctime(),
                before.ctime_nsec()
            ),
            (
                after.ino(),
                after.len(),
                after.mode(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec()
            )
        );
    }
    fs::remove_file(folder.join("system.wav")).unwrap();
    fs::remove_file(folder.join("microphone.wav")).unwrap();
    assert_eq!(source(&meetings).poll().unwrap().len(), 2);
}

#[test]
fn missing_deleted_and_partial_inputs_do_not_starve_other_folders() {
    for name in ["manifest.json", "transcript.ndjson"] {
        let (_root, meetings, cursor) = super::super::tests::fixture();
        let path = meetings.join("99-1").join(name);
        let original = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        let mut first = Session::open(source(&meetings), cursor.clone()).unwrap();
        let mut queued = Vec::new();
        collect(&mut first, &mut queued);
        drop(first);
        fs::write(&path, b"{").unwrap();
        let mut restarted = Session::open(source(&meetings), cursor.clone()).unwrap();
        restarted.drain(|_| panic!("partial input")).unwrap();
        fs::write(&path, original).unwrap();
        collect(&mut restarted, &mut queued);
        fs::remove_dir_all(meetings.join("99-1")).unwrap();
        drop(restarted);
        Session::open(source(&meetings), cursor)
            .unwrap()
            .drain(|_| panic!("deleted or duplicate"))
            .unwrap();
        assert_eq!(queued, ["100-1", "99-1"]);
    }
}

#[test]
fn explicit_paths_reject_traversal_links_and_special_files() {
    for path in ["", "relative", "/tmp/../outside"] {
        assert!(matches!(
            Discovery::new(path.into()),
            Err(Error::InvalidInput)
        ));
        assert!(matches!(
            Discovery::hex_support(path.into()),
            Err(Error::InvalidInput)
        ));
    }
    for role in [
        "root",
        "ancestor",
        "meeting",
        "manifest.json",
        "transcript.ndjson",
        "system.wav",
        "microphone.wav",
    ] {
        let (root, meetings, _cursor) = super::super::tests::fixture();
        let outside = root.path().join("outside");
        let target = match role {
            "root" => meetings.clone(),
            "ancestor" => {
                let alias = root.path().join("alias");
                symlink(root.path(), &alias).unwrap();
                assert!(matches!(
                    Discovery::new(alias.join("meetings")).unwrap().poll(),
                    Err(Error::Input(_))
                ));
                continue;
            }
            "meeting" => meetings.join("99-1"),
            name => meetings.join("99-1").join(name),
        };
        if target.exists() {
            fs::rename(&target, &outside).unwrap();
        } else {
            fs::write(&outside, b"synthetic").unwrap();
        }
        symlink(&outside, &target).unwrap();
        let result = source(&meetings).poll();
        if role == "root" {
            assert!(matches!(result, Err(Error::Input(_))));
        } else {
            assert_eq!(
                result
                    .unwrap()
                    .iter()
                    .map(CompletedMeeting::id)
                    .collect::<Vec<_>>(),
                ["100-1"]
            );
        }
    }
    let (_root, meetings, _cursor) = super::super::tests::fixture();
    let audio = meetings.join("99-1/system.wav");
    let name = std::ffi::CString::new(audio.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert_eq!(source(&meetings).poll().unwrap().len(), 1);
}
