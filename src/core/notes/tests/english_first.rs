//! Notes start from English past the wait limit, through enqueue and drain. Not Windows-only.
use super::passes::{made_by, release};
use super::*;
use crate::core::meeting_router::{
    fallback::{Reason, RoutingFallback},
    revision,
};

const MIN: f64 = 60_000.0;

/// A meeting of `duration_ms` that ended `ago_ms` ago, with Labs routing on.
fn setup(duration_ms: f64, ago_ms: i64) -> (Temp, PathBuf, NotesSettings) {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(settings.clone());
    config.pipeline.profile = "experimental".into();
    config.labs.after_stop_language_routing = true;
    fs::write(
        root.0.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    let path = folder.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    m["duration_ms"] = duration_ms.into();
    m["ended_at_ms"] = (chrono::Utc::now().timestamp_millis() - ago_ms).into();
    fs::write(path, m.to_string()).unwrap();
    (root, folder, settings)
}
fn pending(root: &Path) {
    let queue = root.join("local/router-queue");
    fs::create_dir_all(&queue).unwrap();
    fs::write(
        queue.join(format!("{ID}.json")),
        format!(r#"{{"id":"{ID}","generation":7}}"#),
    )
    .unwrap();
    revision::begin(&root.join("local/transcripts").join(ID), 7).unwrap();
}
fn notes(root: &Path, s: &NotesSettings) -> (Fake, Vec<RoutingFallback>) {
    enqueue(root, s, None);
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(root, s, &mut fake);
    let job = queue::read(root, ID).unwrap().unwrap();
    (fake, job.routing_fallbacks)
}
fn reasons(reason: Reason) -> Vec<RoutingFallback> {
    vec![RoutingFallback::new(reason)]
}

#[test]
fn long_meeting_starts_from_english_one_second_after_the_end() {
    let (root, folder, s) = setup(30.0 * MIN, 1_000);
    pending(&root.0);
    let (fake, got) = notes(&root.0, &s);
    assert!(fake.calls[0].0.contains(PRIVATE) && fake.calls[0].0.contains("(English)"));
    assert_eq!(got, reasons(Reason::EnglishFirst));
    let by = made_by(&folder);
    assert_eq!(by["language_passes"], "english_only");
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    let pin = job.transcript_revision.unwrap();
    assert_eq!(by["transcript_revision"], pin.as_str());
}
#[test]
fn short_meeting_uses_a_release_that_lands_inside_the_wait() {
    let (root, folder, s) = setup(4.0 * MIN, 20_000);
    let pin = release(&root.0, &folder, "te", false);
    let (fake, got) = notes(&root.0, &s);
    assert!(fake.calls[0].0.contains("తెలుగు selected revision"));
    assert!(got.is_empty());
    let by = made_by(&folder);
    assert_eq!(by["language_passes"], "routed");
    assert_eq!(by["transcript_revision"], pin.as_str());
}
#[test]
fn short_meeting_without_a_release_falls_back_after_the_wait() {
    let (root, folder, s) = setup(4.0 * MIN, 41_000);
    pending(&root.0);
    let (fake, got) = notes(&root.0, &s);
    assert!(folder.join("english-summary.json").exists());
    assert!(fake.calls[0].0.contains(PRIVATE));
    assert_eq!(got, reasons(Reason::DeadlineExpired));
}
#[test]
fn zero_minutes_means_never_wait() {
    let (root, folder, s) = setup(4.0 * MIN, 1_000);
    let path = root.0.join("config.json");
    let mut c: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    c["engine_settings"]["stages"]["notes_wait_max_minutes"] = 0.into();
    fs::write(path, c.to_string()).unwrap();
    pending(&root.0);
    let (fake, got) = notes(&root.0, &s);
    assert!(folder.join("english-summary.json").exists());
    assert!(fake.calls[0].0.contains(PRIVATE));
    assert_eq!(got, reasons(Reason::EnglishFirst));
}
#[test]
fn long_meeting_still_uses_a_release_that_is_already_there() {
    let (root, folder, s) = setup(30.0 * MIN, 400_000);
    release(&root.0, &folder, "hi", false);
    let (fake, got) = notes(&root.0, &s);
    assert!(fake.calls[0].0.contains("हिंदी selected revision"));
    assert!(got.is_empty());
}
#[test]
fn a_release_published_while_notes_wait_for_the_lock_is_used() {
    let (root, folder, _) = setup(30.0 * MIN, 400_000);
    let pin = release(&root.0, &folder, "hi", false);
    let released = fs::read(
        root.0
            .join("local/router-work")
            .join(ID)
            .join("released.json"),
    );
    let done =
        serde_json::from_slice::<serde_json::Value>(&released.unwrap()).unwrap()["completed_ms"]
            .as_i64()
            .unwrap();
    // The clock reads older than the release until the notes job holds the router's lock.
    let lock = root.0.join("local/router-queue").join(format!("{ID}.lock"));
    let clock = || {
        if store_io::lock(&lock).is_err() {
            done
        } else {
            done - 1
        }
    };
    let got = transcript::chosen_sized(&root.0, ID, &folder, None, 0, &clock, Some(30.0 * MIN));
    assert_eq!(got.unwrap().unwrap().revision, pin);
}
#[test]
fn zero_wait_ignores_an_end_time_ahead_of_the_clock() {
    let (root, _, s) = setup(30.0 * MIN, -60_000);
    pending(&root.0);
    let (fake, got) = notes(&root.0, &s);
    assert!(fake.calls[0].0.contains(PRIVATE));
    assert_eq!(got, reasons(Reason::EnglishFirst));
}
#[test]
fn long_meeting_without_a_router_record_is_still_english_first() {
    let (root, _, s) = setup(30.0 * MIN, 1_000);
    let (_, got) = notes(&root.0, &s);
    assert_eq!(got, reasons(Reason::EnglishFirst));
}
#[test]
fn the_five_minute_limit_is_compared_at_full_precision() {
    let (root, folder, s) = setup(5.0 * MIN, 1_000);
    pending(&root.0);
    assert!(notes(&root.0, &s).0.calls.is_empty());
    release(&root.0, &folder, "te", false);
    // The waiting job is retried after one second.
    std::thread::sleep(Duration::from_millis(1100));
    let mut fake = Fake::new(vec![Ok(summary()), Ok(sorted())]);
    drain(&root.0, &s, &mut fake);
    assert!(fake.calls[0].0.contains("తెలుగు selected revision"));
    let (root, _, s) = setup(5.0 * MIN + 0.5, 1_000);
    pending(&root.0);
    assert_eq!(notes(&root.0, &s).1, reasons(Reason::EnglishFirst));
}
#[test]
fn wait_settings_outside_their_range_fall_back_to_the_defaults() {
    // A short meeting at 41 s, and a long one at 1 s.
    for (field, value, long) in [
        ("notes_wait_seconds", 600, false),
        ("notes_wait_max_minutes", 241, true),
    ] {
        let (minutes, ago) = [(4.0, 41_000), (30.0, 1_000)][long as usize];
        let reason = [Reason::DeadlineExpired, Reason::EnglishFirst][long as usize];
        let (root, _, s) = setup(minutes * MIN, ago);
        let path = root.0.join("config.json");
        let mut c: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        c["engine_settings"]["stages"][field] = value.into();
        fs::write(path, c.to_string()).unwrap();
        pending(&root.0);
        assert_eq!(notes(&root.0, &s).1, reasons(reason));
    }
}
