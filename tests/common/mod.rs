use std::{path::PathBuf, sync::OnceLock};

pub fn isolate() {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let parent = project.join("target/test-storage");
        std::fs::create_dir_all(&parent).expect("create isolated test storage");
        let root = tempfile::tempdir_in(parent).expect("create test home");
        let mut args = Vec::new();
        for (flag, name, folder) in [
            ("--data-dir", "SIDENOTE_DATA_DIR", "state"),
            ("--meetings-dir", "SIDENOTE_HEX_DIR", "meetings"),
            ("--peers-dir", "SIDENOTE_PEERS_DIR", "peers"),
        ] {
            // Existing UI child fixtures already supply paths inside this worktree.
            let path = sidenote::env_var(name)
                .map(PathBuf::from)
                .filter(|path| path.starts_with(&project))
                .unwrap_or_else(|| root.path().join(folder));
            args.extend([flag.into(), path.into_os_string()]);
        }
        sidenote::core::configure_paths(&args).expect("isolate test paths");
        root
    });
    for path in [
        sidenote::core::log_path(),
        sidenote::core::state_dir().join("config.json"),
        sidenote::core::meetings_dir(),
        sidenote::core::peers_dir(),
    ] {
        assert!(
            path.starts_with(env!("CARGO_MANIFEST_DIR")),
            "test storage escaped the worktree"
        );
        for name in ["APPDATA", "LOCALAPPDATA"] {
            if let Some(owner) = std::env::var_os(name).filter(|value| !value.is_empty()) {
                assert!(
                    !path.starts_with(std::path::Path::new(&owner)),
                    "test path resolved into owner app data"
                );
            }
        }
    }
}
