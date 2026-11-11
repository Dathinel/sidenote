use super::media::{ApprovedCapture, PreparedFrame};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

pub const SCHEMA: &str = include_str!("prompts/note.schema.json");
pub fn cli_schema() -> String {
    let mut schema: Value = serde_json::from_str(SCHEMA).expect("embedded annotation schema");
    // Claude's CLI lacks the 2020-12 meta-schema; all constraints are shared with its default draft.
    schema.as_object_mut().unwrap().remove("$schema");
    schema.to_string()
}
pub fn frame_prompt(frame: &PreparedFrame) -> String {
    let shape = json!({"frame_id":format!("frame-{}",frame.index),"summary":"A short factual frame summary","callouts":frame.metadata["marks"].as_array().unwrap().iter().map(|m|json!({"mark_number":m["mark_number"],"explanation":"Only what the image or words support","uncertainty":null})).collect::<Vec<_>>(),"open_questions":[]});
    format!(
        "{}\n\nApproved frame metadata:\n{}\nImage order: redacted original, numbered marked image, then crops in mark-number order.\nReturn JSON only in this shape: {}. Replace explanatory placeholders with grounded prose. Include precisely the supplied mark numbers. Never invent names or numbers.",
        include_str!("prompts/capture.md"),
        frame.metadata,
        shape
    )
}
pub fn notebook_prompt(approved: &ApprovedCapture, drafts: &[Value]) -> String {
    let records: Vec<_> = approved
        .approval
        .frames
        .iter()
        .map(|f| f.metadata.clone())
        .collect();
    format!(
        "{}\n\nApproved frame manifest:\n{}\nCompleted frame drafts:\n{}\nRequired JSON Schema:\n{}\nCopy figure_id, capture_id, app, window_title, keyframe_index, keyframe_reason and every mark_id, mark_number, boxNorm and description exactly from the manifest. Use mark_id or figure_id for evidence_refs. Include each approved frame and mark exactly once. Return a single JSON object, no markdown fences. Never invent names or numbers.",
        include_str!("prompts/capture-note.md"),
        json!(records),
        json!(drafts),
        SCHEMA
    )
}
pub fn parse_json(text: &str) -> Result<Value, String> {
    let trimmed = text.trim();
    let trimmed = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim().strip_suffix("```").unwrap_or(s).trim())
        .unwrap_or(trimmed);
    serde_json::from_str(trimmed)
        .map_err(|_| "The agent returned invalid JSON. Retry the note.".into())
}
pub fn validate_draft(value: &Value, frame: &PreparedFrame) -> Result<(), String> {
    if value["frame_id"] != format!("frame-{}", frame.index) || !value["summary"].is_string() {
        return Err("The frame draft does not match the approved frame.".into());
    }
    let expected: BTreeSet<_> = frame.metadata["marks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["mark_number"].as_u64().unwrap())
        .collect();
    let callouts = value["callouts"]
        .as_array()
        .ok_or("The frame draft has no callouts.")?;
    let found: Vec<_> = callouts.iter().map(|c| c["mark_number"].as_u64()).collect();
    if found.len() != expected.len()
        || found.iter().any(|n| n.is_none())
        || found.into_iter().flatten().collect::<BTreeSet<_>>() != expected
        || callouts.iter().any(|c| !c["explanation"].is_string())
    {
        return Err("The frame draft does not cover every approved mark exactly once.".into());
    }
    Ok(())
}
pub fn validate(value: &Value, approved: &ApprovedCapture) -> Result<(), String> {
    let schema: Value =
        serde_json::from_str(SCHEMA).map_err(|_| "Could not read the note schema.")?;
    let validator =
        jsonschema::validator_for(&schema).map_err(|_| "Could not compile the note schema.")?;
    if !validator.is_valid(value) {
        return Err("The note does not match the required schema. It remains a draft.".into());
    }
    let figures = value["figures"].as_array().unwrap();
    if figures.len() != approved.approval.frames.len() {
        return Err("The note must include every approved frame exactly once.".into());
    }
    let mut figure_ids = BTreeSet::new();
    let mut evidence = BTreeSet::new();
    for frame in &approved.approval.frames {
        evidence.insert(format!("frame-{}", frame.index));
        for mark in frame.metadata["marks"].as_array().unwrap() {
            evidence.insert(mark["mark_id"].as_str().unwrap().to_owned());
        }
    }
    for figure in figures {
        let id = figure["figure_id"].as_str().unwrap();
        if !figure_ids.insert(id) {
            return Err("A frame appears more than once in the note.".into());
        }
        let frame = approved
            .approval
            .frames
            .iter()
            .find(|f| f.metadata["figure_id"] == id)
            .ok_or("The note refers to an unapproved frame.")?;
        for key in [
            "capture_id",
            "app",
            "window_title",
            "keyframe_index",
            "keyframe_reason",
        ] {
            if figure[key] != frame.metadata[key] {
                return Err(format!("The note changed approved {key} metadata."));
            }
        }
        let callouts = figure["callouts"].as_array().unwrap();
        let marks = frame.metadata["marks"].as_array().unwrap();
        if callouts.len() != marks.len() {
            return Err("The note is missing approved marks.".into());
        }
        let mut seen = BTreeSet::new();
        for c in callouts {
            let n = c["mark_number"]
                .as_u64()
                .ok_or("Callout numbers must be stored as whole unsigned integers.")?;
            if !seen.insert(n) {
                return Err("A mark is repeated in the note.".into());
            }
            let mark = marks
                .iter()
                .find(|m| m["mark_number"] == n)
                .ok_or("The note invented a mark number.")?;
            for key in ["mark_id", "boxNorm", "description"] {
                if c[key] != mark[key] {
                    return Err(format!("The note changed approved mark {key}."));
                }
            }
            check_refs(&c["evidence_refs"], &evidence)?;
        }
    }
    let mut shown = BTreeSet::new();
    for section in value["sections"].as_array().unwrap() {
        for reference in section["figure_refs"].as_array().unwrap() {
            if !shown.insert(reference.as_str().unwrap()) {
                return Err("Each approved frame must appear in exactly one section.".into());
            }
        }
        if section["figure_refs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| !figure_ids.contains(r.as_str().unwrap()))
        {
            return Err("A section refers to an unknown figure.".into());
        }
    }
    if shown != figure_ids {
        return Err("Every approved frame must be shown in a section.".into());
    }
    for todo in value["todos"].as_array().unwrap() {
        check_refs(&todo["evidence_refs"], &evidence)?;
    }
    Ok(())
}
fn check_refs(refs: &Value, known: &BTreeSet<String>) -> Result<(), String> {
    if refs
        .as_array()
        .unwrap()
        .iter()
        .any(|r| !known.contains(r.as_str().unwrap()))
    {
        Err("The note contains an unknown evidence reference.".into())
    } else {
        Ok(())
    }
}
pub fn save(folder: &Path, value: &Value) -> std::io::Result<()> {
    crate::core::store_io::atomic_write(
        &folder.join("note.json"),
        &serde_json::to_vec_pretty(value)?,
    )
}
pub fn load(folder: &Path) -> std::io::Result<Option<Value>> {
    match crate::core::file_reads::read(&folder.join("note.json"), 16 * 1024 * 1024) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
/// A deterministic stand-in for offscreen rendering and schema tests, never a model result.
pub fn synthetic_note(approved: &ApprovedCapture) -> Value {
    json!({"title":"Workshop plan","summary":"The speaker reviews workshop navigation, paper materials and the next review.","sections":approved.approval.frames.iter().map(|f|json!({"heading":f.metadata["keyframe_reason"],"paragraphs":["The visible board provides context for the speaker's explanation."],"figure_refs":[format!("frame-{}",f.index)]})).collect::<Vec<_>>(),"figures":approved.approval.frames.iter().map(|f| {let mut v=f.metadata.clone(); let o=v.as_object_mut().unwrap();o.remove("marks");o.remove("keyframe_time");o.insert("caption".into(),json!(f.metadata["keyframe_reason"]));o.insert("callouts".into(),json!(f.metadata["marks"].as_array().unwrap().iter().map(|m|json!({"mark_id":m["mark_id"],"mark_number":m["mark_number"],"boxNorm":m["boxNorm"],"description":m["description"],"label":m["description"],"explanation":m["words"].as_array().unwrap().iter().filter_map(|w|w["text"].as_str()).collect::<Vec<_>>().join(" "),"evidence_refs":[m["mark_id"]],"uncertainty":null})).collect::<Vec<_>>()));v}).collect::<Vec<_>>(),"open_questions":["What should be ready for the next review?"],"todos":[]})
}
