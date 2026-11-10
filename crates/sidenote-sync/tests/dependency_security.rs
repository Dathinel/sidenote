#[test]
fn locked_worker_graph_excludes_unmaintained_atomic_polyfill() {
    assert!(
        !include_str!("../../../Cargo.lock")
            .lines()
            .any(|line| line == "name = \"atomic-polyfill\""),
        "RUSTSEC-2023-0089: atomic-polyfill must stay outside the worker dependency graph"
    );
}
