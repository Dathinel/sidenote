#![windows_subsystem = "windows"]
mod hex_probe;
mod ui;
use sidenote::{listener, platform};
fn main() {
    velopack::VelopackApp::build()
        .set_auto_apply_on_startup(false)
        .on_after_update_fast_callback(|_| sidenote::updates::after_update())
        .on_restarted(|_| sidenote::updates::restarted())
        .run();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "--listener-isolated")
        && let Some(pair) = args
            .windows(2)
            .find(|pair| pair[0] == "--listener-localappdata")
    {
        if !std::path::Path::new(&pair[1]).is_absolute() {
            std::process::exit(2);
        }
        // The isolated task configures its own environment before any app worker starts.
        unsafe {
            std::env::set_var("LOCALAPPDATA", &pair[1]);
        }
    }
    if let Err(error) = sidenote::core::configure_paths(&args) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    if args.iter().any(|a| a == "--mcp") {
        std::process::exit(sidenote::mcp::run());
    }
    if args.iter().any(|a| a == "--version") {
        println!("Sidenote {}", sidenote::updates::VERSION);
        return;
    }
    if args.iter().any(|a| {
        matches!(
            a.to_str(),
            Some(
                "--update-check"
                    | "--update-apply"
                    | "--update-status"
                    | "--update-window-start"
                    | "--update-probe"
            )
        )
    }) {
        std::process::exit(sidenote::updates::cli(&args));
    }
    let helper = args.iter().any(|a| {
        matches!(
            a.to_str(),
            Some("--transcribe-file" | "--route-one" | "--route-pending" | "--daily-check")
        )
    });
    let _activity = if helper {
        match sidenote::updates::activity() {
            Ok(guard) => Some(guard),
            Err(_) => std::process::exit(2),
        }
    } else {
        None
    };
    if args.iter().any(|a| a == "--transcribe-file") {
        std::process::exit(sidenote::core::meeting_router::run_cli(&args));
    }
    if args.iter().any(|a| a == "--route-one") {
        let result = sidenote::core::meeting_router::jobs::run_one();
        if let Err(error) = &result
            && let Some(path) = args
                .windows(2)
                .find(|a| a[0] == "--router-request")
                .map(|a| std::path::PathBuf::from(&a[1]))
            && let Some(parent) = path.parent()
        {
            let _ = std::fs::write(
                parent.join("failure.json"),
                serde_json::json!({"error": error.to_string()}).to_string(),
            );
        }
        std::process::exit(i32::from(result.is_err()));
    }
    if let Some(id) = args.windows(2).find(|a| a[0] == "--enqueue-meeting") {
        let result = sidenote::core::meeting_router::jobs::enqueue(&id[1].to_string_lossy());
        std::process::exit(i32::from(result.is_err()));
    }
    if args.iter().any(|a| a == "--route-pending") {
        let result = sidenote::core::meeting_router::jobs::run_pending();
        std::process::exit(i32::from(result.is_err()));
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
    let _update_window = if headless || offscreen_bench {
        None
    } else {
        match sidenote::updates::window_lock() {
            Ok(guard) => Some(guard),
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
