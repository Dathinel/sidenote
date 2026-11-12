fn main() {
    #[cfg(target_os = "linux")]
    {
        let result = sidenote_sync::tree_worker::run(std::env::args_os().skip(1).collect());
        match result {
            Ok(code) => std::process::exit(code),
            Err(_) => {
                eprintln!("{}", sidenote_sync::tree_worker::UNAVAILABLE);
                std::process::exit(2);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    std::process::exit(2);
}
