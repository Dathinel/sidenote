#![windows_subsystem = "windows"]
mod hex_probe;
mod ui;
use sidenote::{listener, platform};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Err(error) = sidenote::core::configure_paths(&args) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    if args.iter().any(|a| a == "--mcp") {
        std::process::exit(sidenote::mcp::run());
    }
    if std::env::args().any(|a| a == "--hex-engine-status") {
        std::process::exit(sidenote::hex_engine::run_status());
    }
    if std::env::args().any(|a| a == "--probe-hex") {
        std::process::exit(hex_probe::run());
    }
    if std::env::args().any(|a| a == "--daily-check") {
        std::process::exit(sidenote::core::daily_check::run_cli());
    }
    if std::env::args().any(|a| a == "--listen") {
        listener::run()
    }
    let headless = std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--snap"
                | "--tree"
                | "--script"
                | "--frames"
                | "--video"
                | "--strip"
                | "--list-scenes"
                | "--help"
        )
    });
    let offscreen_bench = sidenote::env_var("SIDENOTE_BENCH_OFFSCREEN").is_some();
    let _instance = if headless || offscreen_bench {
        None
    } else {
        match platform::single_instance() {
            Ok(Some(instance)) => Some(instance),
            Ok(None) => return,
            Err(_) => return,
        }
    };
    if !headless && (!offscreen_bench || sidenote::env_var("SIDENOTE_BENCH_NO_WARM").is_none()) {
        rune::warm_gpu(Default::default());
    }
    ui::run();
    // Closing the window ends the process. A running Ask is stopped first so its CLI, that CLI's
    // children and its temporary prompt file go with it. The question is already saved.
    sidenote::core::ask::cancel_all(std::time::Duration::from_secs(3));
    std::process::exit(0);
}
