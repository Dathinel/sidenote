fn wired(source: &str) -> bool {
    let Some(builder) = source.split("velopack::VelopackApp::build()").nth(1) else {
        return false;
    };
    let Some((builder, _)) = builder.split_once(".run();") else {
        return false;
    };
    let compact: String = builder.chars().filter(|c| !c.is_whitespace()).collect();
    [
        ".on_after_install_fast_callback(|_|{ifsidenote::updates::after_update().is_err(){std::process::exit(1);}})",
        ".on_after_update_fast_callback(|_|{ifsidenote::updates::after_update().is_err(){std::process::exit(1);}})",
        ".on_first_run(|_|{let_=sidenote::updates::after_update();})",
        ".on_restarted(|_|sidenote::updates::restarted())",
    ].iter().all(|route| compact.contains(route))
}

#[test]
fn installed_callbacks_reconcile_and_fast_hooks_propagate_failure() {
    assert!(wired(include_str!("main.rs")));
}

#[test]
fn reverting_any_install_route_is_detected() {
    let source = include_str!("main.rs");
    for name in [
        "on_after_install_fast_callback",
        "on_first_run",
        "on_after_update_fast_callback",
        "on_restarted",
    ] {
        assert!(
            !wired(&source.replacen(name, "old_registration_only_hook", 1)),
            "{name}"
        );
    }
    assert!(!wired(&source.replacen(
        "std::process::exit(1);",
        "return;",
        1
    )));
}
