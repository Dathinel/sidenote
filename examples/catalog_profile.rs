fn main() {
    for _ in 0..7 {
        let start = std::time::Instant::now();
        let (meetings, errors) = sidenote::core::scan();
        assert!(errors.is_empty(), "catalog scan errors");
        println!(
            "{} meetings, {} transcript bytes, {} index bytes, {:?}",
            meetings.len(),
            meetings
                .iter()
                .map(|m| m.transcript_raw.len())
                .sum::<usize>(),
            meetings.iter().map(|m| m.search.len()).sum::<usize>(),
            start.elapsed()
        );
    }
}
