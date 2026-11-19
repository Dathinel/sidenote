#![cfg(target_os = "linux")]
use sidenote_sync::tree_worker::{command, owner_unit, speech_command};
use std::{ffi::OsStr, fs, path::Path};

#[test]
fn packaged_speech_service_admits_only_speech() {
    let speech = include_str!("../../../packaging/systemd/sidenote-speech@.service");
    let ordinary = include_str!("../../../packaging/systemd/sidenote-server@.service");
    let property = |unit: &str, name: &str| -> String {
        let prefix = format!("--property={name}=");
        unit.split_whitespace()
            .find_map(|word| word.strip_prefix(&prefix))
            .unwrap()
            .to_owned()
    };
    assert_eq!(property(speech, "MemoryMax"), "1G");
    assert_eq!(property(speech, "TasksMax"), "64");
    assert_eq!(property(speech, "CPUQuota"), "100%%");
    // All hardening, mounts, environment removal and stop policy match the ordinary unit.
    let normalized = speech
        .replace("sidenote-speech", "sidenote-server")
        .replace("MemoryMax=1G --property", "MemoryMax=512M --property")
        .replace("TasksMax=64 --property", "TasksMax=32 --property")
        .replace(" --speech-worker", "");
    assert_eq!(
        normalized,
        ordinary.replace("unattended server", "speech server")
    );
    let (unit, group) = owner_unit().unwrap();
    assert_eq!(unit, "sidenote-speech-worker.service");
    let directory = Path::new("/sys/fs/cgroup").join(group.strip_prefix("/").unwrap());
    for (file, value) in [("memory.max", "1073741824"), ("pids.max", "64")] {
        assert_eq!(
            fs::read_to_string(directory.join(file)).unwrap().trim(),
            value
        );
    }
    let cpu = fs::read_to_string(directory.join("cpu.max")).unwrap();
    let values: Vec<u64> = cpu.split_whitespace().map(|v| v.parse().unwrap()).collect();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0], values[1]);
    let speech = speech_command(OsStr::new("/bin/true")).unwrap();
    assert_eq!(speech.get_args().next(), Some(OsStr::new("--speech")));
    assert!(command(OsStr::new("/bin/true")).is_err());
}
