use super::{
    notebook::{self, Input},
    share, *,
};
use std::{fs, path::PathBuf};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sidenote-notebook-test-{}-{}-{nonce}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        assert!(self.0.starts_with(std::env::temp_dir()));
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("sidenote-notebook-test-")
        );
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn notebook_order_survives_storage_and_changes_keep_the_previous_note() {
    let temp = Temp::new();
    let (mut b, inputs, captures) = notebook::synthetic(&temp.0, 3).unwrap();
    let original = fs::read(captures[0].folder.join("scrawl.json")).unwrap();
    b.note = Some(notebook::synthetic_document(&b, &inputs).unwrap());
    b.note_revision = b.revision;
    assert!(b.fresh());
    let first = b.captures[0].key.clone();
    assert!(b.reorder(0, 2));
    assert_eq!(b.captures[2].key, first);
    assert!(b.fresh());
    assert_eq!(
        b.note.as_ref().unwrap().value["sections"][2]["capture_key"],
        first
    );
    assert!(b.note.is_some());
    assert!(!b.add(&captures[0]));
    assert!(b.remove(1));
    notebook::save(&temp.0, &b).unwrap();
    let stored = notebook::list(&temp.0).unwrap();
    assert_eq!(stored[0].captures[1].key, first);
    assert_eq!(
        fs::read(captures[0].folder.join("scrawl.json")).unwrap(),
        original
    );
    b.id = "../outside".into();
    assert!(notebook::save(&temp.0, &b).is_err());
}
#[test]
fn notebook_synthesis_preserves_capture_order_and_deduplicates_questions_and_todos() {
    let temp = Temp::new();
    let (b, mut inputs, _) = notebook::synthetic(&temp.0, 3).unwrap();
    for i in &mut inputs {
        i.note["open_questions"] = serde_json::json!(["What is next?", "what is NEXT"]);
        i.note["todos"] = serde_json::json!([{"action":"Bring two versions","owner":null,"due_date":null,"evidence_refs":[i.note["figures"][0]["figure_id"]]}]);
    }
    let doc = notebook::synthetic_document(&b, &inputs).unwrap();
    notebook::validate(&doc).unwrap();
    assert_eq!(doc.value["open_questions"].as_array().unwrap().len(), 1);
    assert_eq!(doc.value["todos"].as_array().unwrap().len(), 1);
    for (section, input) in doc.value["sections"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&inputs)
    {
        assert_eq!(section["capture_key"], input.key);
        assert_eq!(section["figures"], input.note["figures"]);
    }
    let mut bad = doc.clone();
    bad.value["sections"].as_array_mut().unwrap().swap(0, 1);
    assert!(notebook::validate(&bad).is_err());
    let mut bad = doc.clone();
    bad.value["sections"][0]["figures"][0]["callouts"][0]["mark_number"] = serde_json::json!(999);
    assert!(notebook::validate(&bad).is_err());
    let prompt = notebook::prompt(&b, &inputs);
    assert!(prompt.contains("Whole-notebook"));
    assert!(prompt.contains("Never invent names"));
}
#[test]
fn notebook_rejects_unapproved_assets_and_stale_jobs_before_starting_a_model() {
    let temp = Temp::new();
    let (b, inputs, _) = notebook::synthetic(&temp.0, 2).unwrap();
    let mut unapproved: Vec<Input> = inputs.clone();
    unapproved[0].approved.approval.approved_at_ms = 0;
    assert!(notebook::synthetic_document(&b, &unapproved).is_err());
    let epochs = generation::Epoch::default();
    let old = epochs.begin();
    epochs.invalidate();
    assert!(old.check().is_err());
}
#[test]
fn shared_html_is_static_pathless_and_contains_exactly_the_approved_marked_pngs() {
    let temp = Temp::new();
    let (b, inputs, _) = notebook::synthetic(&temp.0, 3).unwrap();
    let mut doc = notebook::synthetic_document(&b, &inputs).unwrap();
    doc.value["summary"] = serde_json::json!(
        "Use <script>alert(1)</script>, https://example.invalid and C:\\private\\capture.png"
    );
    let html = share::notebook(&doc).unwrap();
    assert!(!html.to_lowercase().contains("<script"));
    assert!(
        !regex::Regex::new(r"(?i)https?://|file://|[a-z]:[\\/]|\\\\")
            .unwrap()
            .is_match(&html)
    );
    assert!(!html.contains(&temp.0.to_string_lossy().to_string()));
    assert!(html.contains("&lt;script&gt;"));
    assert!(html.contains("prefers-color-scheme:dark"));
    assert!(html.contains("@media print"));
    assert!(html.contains("Made with Sidenote"));
    let mut expected = std::collections::BTreeMap::new();
    for input in &inputs {
        for frame in &input.approved.approval.frames {
            expected.insert(
                input.approved.approval.file_hashes[&frame.marked].clone(),
                1usize,
            );
        }
    }
    assert_eq!(share::image_hashes(&html).unwrap(), expected);
    let capture = share::capture(&inputs[0].note, &inputs[0].approved).unwrap();
    assert_eq!(
        share::image_hashes(&capture)
            .unwrap()
            .values()
            .sum::<usize>(),
        2
    );
    let f = &inputs[0].approved.approval.frames[0];
    fs::write(inputs[0].approved.folder.join(&f.marked), b"changed").unwrap();
    assert!(share::notebook(&doc).is_err());
}
#[test]
fn share_names_are_portable_and_cannot_choose_a_path() {
    assert_eq!(share::default_name("CON"), "annotation-note.html");
    assert_eq!(
        share::default_name("Plan: paper / Friday?"),
        "Plan paper  Friday.html"
    );
    assert_eq!(share::default_name(""), "annotation-note.html");
    assert!(!share::default_name("../bad\\file").contains('/'));
}

#[test]
fn export_never_writes_inside_a_bundle_or_overwrites_other_file_formats() {
    let temp = Temp::new();
    let (_, _, captures) = notebook::synthetic(&temp.0, 1).unwrap();
    let target = captures[0].folder.join("share.html");
    assert!(share::save(&target, "test").is_err());
    assert!(!target.exists());
    assert!(share::save(&temp.0.join("scrawl.json"), "test").is_err());
    assert!(!temp.0.join("scrawl.json").exists());
}

#[test]
fn notebook_stream_shows_sections_before_title_without_exposing_reference_keys() {
    let partial = r#"{"sections":[{"capture_key":"private-key","heading":"Workshop setup","paragraphs":["Use paper ca"#;
    assert!(generation::stream_preview(partial).is_empty());
    let visible = generation::notebook_stream_preview(partial);
    assert!(visible.contains("Workshop setup"));
    assert!(visible.contains("Use paper ca"));
    assert!(!visible.contains("private-key"));
    assert!(!visible.contains("capture_key"));
    let complete = r#"{"sections":[{"capture_key":"opaque","heading":"First","paragraphs":["One.","Two."]},{"capture_key":"opaque2","heading":"Second","paragraphs":["Three."]}],"summary":"Overall point.","title":"Notebook"}"#;
    let visible = generation::notebook_stream_preview(complete);
    assert!(visible.starts_with("Notebook\n\nOverall point."));
    assert!(visible.contains("One.\n\nTwo.\n\nSecond\n\nThree."));
    assert!(generation::notebook_stream_preview(r#"{"title":"Paper \u12"#).contains("Paper"));
}

#[test]
fn conflicting_questions_remain_distinct_and_legacy_uncertainty_moves_to_the_end() {
    let temp = Temp::new();
    let (b, mut inputs, _) = notebook::synthetic(&temp.0, 1).unwrap();
    inputs[0].note["open_questions"] = serde_json::json!(["Is x < 1?", "Is x > 1?"]);
    inputs[0].note["figures"][0]["callouts"][0]["uncertainty"] =
        serde_json::json!("Which overview is intended?");
    let doc = notebook::synthetic_document(&b, &inputs).unwrap();
    assert_eq!(doc.value["open_questions"].as_array().unwrap().len(), 3);
    assert!(doc.value["sections"][0]["figures"][0]["callouts"][0]["uncertainty"].is_null());
    let html = share::capture(&inputs[0].note, &inputs[0].approved).unwrap();
    assert!(html.contains("Which overview is intended?"));
}

#[test]
fn shared_frames_appear_only_in_the_first_capture_with_unique_notebook_marks() {
    let temp = Temp::new();
    let (b, inputs, _) = notebook::synthetic(&temp.0, 2).unwrap();
    let doc = notebook::synthetic_document(&b, &inputs).unwrap();
    let view = notebook::reading_value(&doc);
    assert_eq!(
        view["sections"][0]["figures"][0]["callouts"][0]["mark_number"],
        1
    );
    assert_eq!(
        view["sections"][0]["figures"][1]["callouts"][0]["mark_number"],
        3
    );
    assert_eq!(
        view["sections"][1]["figures"][0]["same_screen"],
        "Same screen as Workshop setup, marks 1, 2."
    );
    assert_eq!(
        view["sections"][1]["figures"][1]["same_screen"],
        "Same screen as Workshop setup, mark 3."
    );
    let html = share::notebook(&doc).unwrap();
    assert_eq!(
        share::image_hashes(&html).unwrap().values().sum::<usize>(),
        2
    );
    assert_eq!(html.matches("class=\"number\"").count(), 3);
    assert_eq!(
        doc.value["sections"][1]["figures"][0]["callouts"][0]["mark_number"],
        1
    );
}

#[test]
fn distinct_capture_frames_get_global_numbers_without_changing_approved_bytes() {
    let temp = Temp::new();
    let (b, mut inputs, captures) = notebook::synthetic(&temp.0, 2).unwrap();
    let capture = &captures[1];
    let path = capture.frame_path(&capture.data.keyframes[1]).unwrap();
    let mut image = image::open(&path).unwrap().to_rgba8();
    image.put_pixel(400, 100, image::Rgba([12, 34, 56, 255]));
    image.save(&path).unwrap();
    let review = inputs[1].approved.approval.review.clone();
    inputs[1].approved = media::approve(capture, &review, &temp.0).unwrap();
    let doc = notebook::synthetic_document(&b, &inputs).unwrap();
    let view = notebook::reading_value(&doc);
    assert!(view["sections"][1]["figures"][0]["same_screen"].is_string());
    let last = &view["sections"][1]["figures"][1]["callouts"][0];
    assert_eq!(last["mark_number"], 4);
    assert_eq!(last["original_mark_number"], 3);
    let before = fs::read(
        inputs[1]
            .approved
            .folder
            .join(&inputs[1].approved.approval.frames[1].marked),
    )
    .unwrap();
    let html = share::notebook(&doc).unwrap();
    assert!(html.contains("<svg viewBox="));
    assert!(html.contains(">4</text>"));
    let hashes = share::image_hashes(&html).unwrap();
    assert_eq!(hashes.len(), 3);
    assert!(hashes.values().all(|n| *n == 1));
    assert!(hashes.contains_key(blake3::hash(&before).to_hex().as_str()));
    assert_eq!(
        before,
        fs::read(
            inputs[1]
                .approved
                .folder
                .join(&inputs[1].approved.approval.frames[1].marked)
        )
        .unwrap()
    );
}

#[test]
fn shared_screen_references_follow_reordering_and_keep_edited_explanations() {
    let temp = Temp::new();
    let (mut book, inputs, _) = notebook::synthetic(&temp.0, 2).unwrap();
    let mut doc = notebook::synthetic_document(&book, &inputs).unwrap();
    doc.value["sections"][1]["figures"][0]["callouts"][1]["explanation"] =
        serde_json::json!("Bring the paper prototype to the review.");
    book.note = Some(doc);
    book.note_revision = book.revision;
    let html = share::notebook(book.note.as_ref().unwrap()).unwrap();
    assert!(
        html.contains(
            "Mark 2: Paper prototype materials. Bring the paper prototype to the review."
        )
    );
    assert_eq!(
        share::image_hashes(&html).unwrap().values().sum::<usize>(),
        2
    );
    assert!(book.reorder(1, 0));
    let doc = book.note.as_ref().unwrap();
    let html = share::notebook(doc).unwrap();
    assert!(html.contains("Same screen as Materials for the prototype, marks 1, 2."));
    assert!(!html.contains("Same screen as Workshop setup"));
    assert_eq!(
        share::image_hashes(&html).unwrap().values().sum::<usize>(),
        2
    );
    assert_eq!(
        doc.value["sections"][0]["figures"][0]["callouts"][1]["explanation"],
        "Bring the paper prototype to the review."
    );
}
