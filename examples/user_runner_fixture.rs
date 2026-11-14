fn main() {
    #[cfg(target_os = "linux")]
    {
        sidenote::platform::supervisor_entry();
        let mut runner = sidenote::platform::UserRunner::spawn(
            std::process::Command::new("/bin/sh").args(["-c", "printf synthetic"]),
        )
        .unwrap();
        use std::io::Read;
        let mut text = String::new();
        runner
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(runner.wait().unwrap().success());
        assert_eq!(text, "synthetic");
    }
}
