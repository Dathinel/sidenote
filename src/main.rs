#![cfg_attr(all(windows, feature = "desktop"), windows_subsystem = "windows")]
#[cfg(windows)]
mod hex_probe;
#[cfg(all(test, windows, feature = "desktop"))]
#[path = "../tests/common/mod.rs"]
mod test_paths;
#[cfg(all(test, windows, feature = "desktop"))]
#[test]
fn default_test_paths_are_isolated_from_owner_appdata() {
    test_paths::isolate();
}
#[cfg(all(windows, feature = "desktop"))]
mod ui;
#[cfg(all(windows, feature = "desktop"))]
use sidenote::platform;
fn main() {
    #[cfg(all(windows, feature = "desktop"))]
    let process_started = std::time::Instant::now();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "--language-detect") {
        std::process::exit(sidenote::core::lid::run_cli(&args));
    }
    #[cfg(all(windows, feature = "desktop"))]
    if args.iter().any(|a| a == "--render-meeting-indicator") {
        if sidenote::core::configure_paths(&args).is_err()
            || sidenote::meeting_icon::render_synthetic().is_err()
        {
            std::process::exit(2);
        }
        return;
    }
    #[cfg(windows)]
    {
        velopack::VelopackApp::build()
            .set_auto_apply_on_startup(false)
            .on_after_install_fast_callback(|_| sidenote::updates::listener_installed())
            .on_first_run(|_| sidenote::updates::listener_installed())
            .on_after_update_fast_callback(|_| {
                sidenote::updates::after_update();
                sidenote::updates::listener_installed();
            })
            .on_restarted(|_| sidenote::updates::restarted())
            .run();
    }
    #[cfg(all(windows, feature = "desktop"))]
    if args.iter().any(|arg| arg == "--response-times") {
        sidenote::response_time::startup(process_started);
    }
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
    if let Err(code) = sidenote::service::configure(&args) {
        eprintln!("{code}");
        std::process::exit(2);
    }
    if args.iter().any(|a| a == "--sync-control") {
        #[cfg(target_os = "linux")]
        std::process::exit(sidenote::sync_owner::cli());
        #[cfg(not(target_os = "linux"))]
        {
            eprintln!("SYNC_CONTROL_UNAVAILABLE_ON_PLATFORM");
            std::process::exit(2);
        }
    }
    #[cfg(target_os = "linux")]
    if args.iter().any(|a| a == "--mcp-access") {
        std::process::exit(sidenote::mcp::access::cli(&args));
    }
    #[cfg(target_os = "linux")]
    if args.iter().any(|a| a == "--ops") {
        std::process::exit(sidenote::operations::cli(&args));
    }
    #[cfg(target_os = "linux")]
    if args.iter().any(|a| a == "--cpu-models") {
        std::process::exit(sidenote::server_models::cli(&args));
    }
    if !args.iter().any(|a| a == "--mcp") && sidenote::service::requested(&args) {
        std::process::exit(sidenote::service::run());
    }
    if args.iter().any(|a| a == "--mcp") {
        std::process::exit(sidenote::mcp::run());
    }
    if let Some(variant) = args.windows(2).find(|a| a[0] == "--notebook-benchmark") {
        let Some(output) = args
            .windows(2)
            .find(|a| a[0] == "--annotation-proof-output")
        else {
            std::process::exit(2)
        };
        if let Err(e) = sidenote::core::annotations::notebook::benchmark(
            std::path::Path::new(&output[1]),
            &variant[1].to_string_lossy(),
        ) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(output) = args.windows(2).find(|a| a[0] == "--notebook-live-proof") {
        if let Err(e) =
            sidenote::core::annotations::notebook::live_proof(std::path::Path::new(&output[1]))
        {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(variant) = args.windows(2).find(|a| a[0] == "--annotation-benchmark") {
        let Some(output) = args
            .windows(2)
            .find(|a| a[0] == "--annotation-proof-output")
        else {
            std::process::exit(2);
        };
        if let Err(e) = sidenote::core::annotations::generation::benchmark(
            &variant[1].to_string_lossy(),
            std::path::Path::new(&output[1]),
        ) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(model) = args.windows(2).find(|a| a[0] == "--annotation-live-proof") {
        let output = args
            .windows(2)
            .find(|a| a[0] == "--annotation-proof-output")
            .map(|a| std::path::PathBuf::from(&a[1]));
        let Some(output) = output else {
            eprintln!("Provide --annotation-proof-output for the synthetic proof.");
            std::process::exit(2);
        };
        if let Err(error) = sidenote::core::annotations::generation::live_proof(
            &model[1].to_string_lossy(),
            &output,
        ) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if args.iter().any(|a| a == "--relay-preview-headless") {
        #[cfg(any(debug_assertions, feature = "relay-qualification"))]
        std::process::exit(i32::from(
            sidenote::core::relay_preview::headless(&args).is_err(),
        ));
        #[cfg(not(any(debug_assertions, feature = "relay-qualification")))]
        std::process::exit(2);
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
            Some(
                "--transcribe-file"
                    | "--route-one"
                    | "--route-pending"
                    | "--notes-pending"
                    | "--daily-check"
            )
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
    if args.iter().any(|a| a == "--notes-pending") {
        let result = sidenote::core::notes::queue::run_pending();
        std::process::exit(i32::from(result.is_err()));
    }
    if std::env::args().any(|a| a == "--hex-engine-status") {
        std::process::exit(sidenote::hex_engine::run_status());
    }
    #[cfg(windows)]
    if std::env::args().any(|a| a == "--probe-hex") {
        std::process::exit(hex_probe::run());
    }
    if std::env::args().any(|a| a == "--daily-check") {
        std::process::exit(sidenote::core::daily_check::run_cli());
    }
    #[cfg(all(windows, feature = "desktop"))]
    desktop();
    #[cfg(not(all(windows, feature = "desktop")))]
    {
        eprintln!("UNAVAILABLE_ON_PLATFORM");
        std::process::exit(2);
    }
}

#[cfg(all(windows, feature = "desktop"))]
fn desktop() {
    if sidenote::service::role() != sidenote::service::DeviceRole::Full {
        eprintln!("UNAVAILABLE_ON_PLATFORM");
        return;
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
            Err(_) => {
                platform::show_error(
                    "Sidenote could not check whether it is already open. Reopen Sidenote to try again. If this continues, restart Windows.",
                );
                return;
            }
        }
    };
    let _update_window = if headless || offscreen_bench {
        None
    } else {
        match sidenote::updates::window_lock() {
            Ok(guard) => Some(guard),
            Err(sidenote::updates::WindowLockError::AlreadyOpen) => return,
            Err(sidenote::updates::WindowLockError::Io(error)) => {
                let message = match error.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        "Sidenote cannot access its update folder. Check your Windows account's folder permissions, then reopen Sidenote."
                    }
                    _ => {
                        "Sidenote could not open its update lock. Check that your local drive is available and has free space, then reopen Sidenote."
                    }
                };
                platform::show_error(message);
                return;
            }
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
