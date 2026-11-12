//! Measure the production roster-refresh path using an explicitly supplied synthetic config.
use sidenote::core::{
    Config, Teams, UiPrefs,
    people::{Affiliations, Side},
};
fn main() {
    let path = std::env::args().nth(1).expect("synthetic config path");
    let config: Config = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let cold = Affiliations::load(&config);
    assert_eq!(
        cold.side("Avery Reed", &Teams::default(), &UiPrefs::default()),
        Some(Side::OurTeam)
    );
    let reads = Affiliations::cache_reads();
    let mut timings = Vec::new();
    for _ in 0..1000 {
        let start = std::time::Instant::now();
        std::hint::black_box(Affiliations::load(&config));
        timings.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    timings.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({"samples":1000,"median_ms":timings[500],"p95_ms":timings[950],"warm_git_reads":Affiliations::cache_reads()-reads})
    );
}
