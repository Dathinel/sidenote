use super::super::translation::Snapshot;
use super::*;
use crate::core::meeting_router::{
    Chunk, RoutingConfig, english_fallback,
    revision::{self, Revision},
};

type Lines = Vec<serde_json::Value>;
type Damage = Option<fn(&mut Lines)>;
/// Stands in for the translation helper: no network, no process. `damage` spoils its reply.
struct Translator {
    notes: Fake,
    damage: Damage,
    calls: Vec<Vec<String>>,
    budgets: Vec<Duration>,
    pause: Duration,
    time_out_call: Option<usize>,
}
impl Translator {
    fn new(damage: Damage) -> Self {
        let notes = Fake::new(vec![Ok(summary()), Ok(sorted())]);
        let (calls, budgets) = (Vec::new(), Vec::new());
        Self {
            notes,
            damage,
            calls,
            budgets,
            pause: Duration::ZERO,
            time_out_call: None,
        }
    }
}
impl Run for Translator {
    fn installed(&self, helper: Helper) -> bool {
        self.notes.installed(helper)
    }
    fn run(&mut self, h: Helper, m: &str, p: String, t: Duration) -> Result<String, Failure> {
        self.notes.run(h, m, p, t)
    }
    fn translate(
        &mut self,
        _: &Snapshot,
        prompt: String,
        end: std::time::Instant,
    ) -> Result<String, &'static str> {
        self.budgets
            .push(end.saturating_duration_since(std::time::Instant::now()));
        std::thread::sleep(self.pause);
        if self.time_out_call == Some(self.budgets.len() - 1) {
            return Err("translation_timed_out");
        }
        let data = prompt.split("Selected transcript data:\n").nth(1).unwrap();
        let items: Lines = serde_json::from_str(data).unwrap();
        let texts = items.iter().map(|i| i["text"].as_str().unwrap().into());
        self.calls.push(texts.collect());
        let mut lines: Lines = items
            .iter()
            .map(|i| {
                let ordinal = i["id"].as_str().unwrap().rsplit(':').next().unwrap();
                serde_json::json!({"id": i["id"], "text_en": format!("English line {ordinal}")})
            })
            .collect();
        self.damage.iter().for_each(|damage| damage(&mut lines));
        Ok(serde_json::json!({"segments": lines}).to_string())
    }
}
/// Six Hindi lines of 6 characters each; a 12 character batch limit makes batches of 2.
fn tuned(langs: &[&str], batch_chars: u32, deadline: u32) -> (Temp, PathBuf, NotesSettings) {
    let (root, folder, settings) = meeting_with(langs);
    let mut config = crate::core::Config::load_in(&root.0).unwrap();
    config.engine_settings.stages.translation_batch_chars = batch_chars;
    config.engine_settings.stages.translation_deadline_seconds = deadline;
    store_io::write(&root.0.join("config.json"), &config).unwrap();
    (root, folder, settings)
}
/// A released revision pinned by the job; the queue reads it as it reads a production one.
fn meeting_with(langs: &[&str]) -> (Temp, PathBuf, NotesSettings) {
    let root = Temp::new();
    let folder = meeting(&root.0, PRIVATE);
    let settings = settings_fixture();
    let mut config = crate::core::Config::default();
    config.notes = Some(settings.clone());
    config.pipeline.profile = "experimental".into();
    config.labs.after_stop_language_routing = true;
    config.engine_settings.steps.translation = true;
    config.engine_settings.stages.translation_enabled_at_ms = Some(1);
    config.engine_settings.stages.translation_model = "translation-fixture".into();
    store_io::write(&root.0.join("config.json"), &config).unwrap();
    let chunk = |i: usize| serde_json::json!({"id":format!("english-{i}"),"start":i,"end":i+1,"track":"system","english":format!("baseline {i}")});
    let baseline: Vec<Chunk> = (0..langs.len())
        .map(|i| serde_json::from_value(chunk(i)).unwrap())
        .collect();
    let mut t = english_fallback(
        &baseline,
        &RoutingConfig::default(),
        "fixture".into(),
        "fixture",
    );
    for (s, lang) in t.segments.iter_mut().zip(langs) {
        if *lang == "en" {
            continue;
        }
        let script = if *lang == "te" { "\u{c15}" } else { "\u{915}" };
        s.text_original = format!("{} {}", script.repeat(3), s.start as u32 + 10);
        (s.lang, s.route, s.text_en, s.translate) =
            (lang.to_string(), lang.to_string(), None, true);
    }
    let dir = root.0.join("local/transcripts").join(ID);
    fs::create_dir_all(&dir).unwrap();
    revision::begin(&dir, 7).unwrap();
    let data = Revision {
        generation: "7".into(),
        plan_revision: "fixture".into(),
        transcript: t,
        alternatives: Vec::new(),
        evidence: Default::default(),
        provenance: serde_json::json!({}),
    };
    enqueue(&root.0, &settings, None);
    let mut job = queue::read(&root.0, ID).unwrap().unwrap();
    job.transcript_revision = Some(revision::commit(&dir, &data).unwrap());
    let path = root.0.join("local/notes-jobs").join(format!("{ID}.json"));
    store_io::write(&path, &job).unwrap();
    (root, folder, settings)
}
fn drain_with(root: &Path, s: &NotesSettings, run: &mut Translator) {
    queue::drain(root, &root.join("meetings"), run, || Ok(s.clone())).unwrap();
}
fn run_notes(root: &Path, s: &NotesSettings, damage: Damage) -> (Translator, String) {
    let mut run = Translator::new(damage);
    drain_with(root, s, &mut run);
    let job = queue::read(root, ID).unwrap().unwrap();
    assert!(matches!(job.state, JobState::Done { .. }));
    (run, job.translation.unwrap().reason)
}

#[test]
fn translated_text_reaches_notes() {
    let (root, _, settings) = meeting_with(&["en", "hi", "te"]);
    let (run, reason) = run_notes(&root.0, &settings, None);
    assert_eq!(reason, "translated");
    assert_eq!(
        run.calls,
        [["\u{915}\u{915}\u{915} 11", "\u{c15}\u{c15}\u{c15} 12"]]
    );
    let notes = &run.notes.calls[0].0;
    assert!(notes.contains("(English): baseline 0"));
    assert!(notes.contains("(Hindi, translated): English line 1"));
    assert!(notes.contains("(Telugu, translated): English line 2"));
    assert!(!notes.chars().any(|c| ('\u{900}'..='\u{c7f}').contains(&c)));
}

#[test]
fn any_reply_problem_keeps_the_original_text() {
    let damage: [fn(&mut Lines); 4] = [
        |l| drop(l.pop()),
        |l| l[1] = l[0].clone(),
        |l| l[1]["id"] = "other".into(),
        |l| l[1]["text_en"] = " ".into(),
    ];
    for damage in damage {
        let (root, folder, settings) = meeting_with(&["en", "hi", "te"]);
        let (run, reason) = run_notes(&root.0, &settings, Some(damage));
        assert_eq!(reason, "translation_reply_invalid");
        let pin = queue::read(&root.0, ID)
            .unwrap()
            .unwrap()
            .transcript_revision;
        let original = transcript::chosen(&root.0, ID, &folder, pin.as_deref(), 0, 1);
        assert!(
            run.notes.calls[0]
                .0
                .contains(&original.unwrap().unwrap().render())
        );
        assert!(!run.notes.calls[0].0.contains("English line"));
    }
}

#[test]
fn long_meeting_translates_in_batches() {
    let (root, _, settings) = tuned(&["hi"; 6], 12, 240);
    let (run, reason) = run_notes(&root.0, &settings, None);
    assert_eq!(reason, "translated");
    assert_eq!(
        run.calls.iter().map(Vec::len).collect::<Vec<_>>(),
        [2, 2, 2]
    );
    for i in 0..6 {
        let line = format!("(Hindi, translated): English line {i}");
        assert!(run.notes.calls[0].0.contains(&line));
    }
}

#[test]
fn stalled_batch_falls_back_whole_inside_one_budget() {
    let (root, folder, settings) = tuned(&["hi"; 6], 12, 2);
    let mut run = Translator::new(None);
    (run.pause, run.time_out_call) = (Duration::from_millis(1100), Some(1));
    drain_with(&root.0, &settings, &mut run);
    let job = queue::read(&root.0, ID).unwrap().unwrap();
    assert_eq!(job.translation.unwrap().reason, "translation_timed_out");
    assert_eq!(run.budgets.len(), 2);
    assert!(run.budgets[0] <= Duration::from_secs(2));
    assert!(run.budgets[1] > Duration::ZERO && run.budgets[1] <= Duration::from_millis(900));
    let original = transcript::chosen(
        &root.0,
        ID,
        &folder,
        job.transcript_revision.as_deref(),
        0,
        1,
    );
    assert!(
        run.notes.calls[0]
            .0
            .contains(&original.unwrap().unwrap().render())
    );
    assert!(!run.notes.calls[0].0.contains("English line"));
}

#[test]
fn more_than_sixteen_batches_are_refused_before_any_call() {
    let (root, _, settings) = tuned(&["hi"; 17], 6, 240);
    let (run, reason) = run_notes(&root.0, &settings, None);
    assert_eq!(reason, "translation_input_limit");
    assert!(run.calls.is_empty());
}

#[cfg(target_os = "linux")]
mod claude;
