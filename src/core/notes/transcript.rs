use crate::core::{file_reads, meeting_router::inputs::Snapshot, store_io};
use serde::Deserialize;
use std::{io, path::Path};

#[derive(Deserialize)]
struct Row {
    start_ms: f64,
    text: String,
    #[serde(default)]
    source: String,
    speaker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Source {
    Compatibility,
    HexBaseline,
    ReleasedRevision,
}
#[derive(Debug)]
pub(super) struct Selection {
    pub revision: String,
    pub transcript: Option<crate::core::meeting_router::Transcript>,
    pub reasons: Vec<crate::core::meeting_router::fallback::RoutingFallback>,
    rendered: String,
}
impl Selection {
    pub fn segment_key(&self, ordinal: usize) -> String {
        format!("{}:{ordinal}", self.revision)
    }
    pub fn source(&self) -> Source {
        match &self.transcript {
            None => Source::Compatibility,
            Some(transcript) if transcript.input_checksum == "compatibility" => Source::HexBaseline,
            Some(_) => Source::ReleasedRevision,
        }
    }
    pub fn render(&self) -> String {
        self.rendered.clone()
    }
}
pub(super) fn render(raw: &str) -> String {
    let mut rows: Vec<Row> = raw
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|r: &Row| r.start_ms.is_finite() && r.start_ms >= 0.0 && !r.text.trim().is_empty())
        .collect();
    rows.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    let mut out = String::new();
    for row in rows {
        let ms = row.start_ms.min(u64::MAX as f64) as u64;
        let speaker = row
            .speaker
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if row.source == "microphone" {
                    "You"
                } else {
                    "Them"
                }
                .into()
            });
        // The existing transcript reader uses a single token for a speaker label.
        let speaker = speaker.split_whitespace().collect::<Vec<_>>().join("_");
        let text = row.text.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push_str(&format!(
            "[{:02}:{:02}:{:02}.{:03}] {speaker} (English): {text}\n",
            ms / 3_600_000,
            ms / 60_000 % 60,
            ms / 1000 % 60,
            ms % 1000
        ));
    }
    out
}

pub(super) fn read(folder: &Path) -> io::Result<String> {
    Ok(render(&String::from_utf8_lossy(&captured(folder)?)))
}
fn captured(folder: &Path) -> io::Result<Vec<u8>> {
    let original = folder.join("hex-original/transcript.ndjson");
    let captured = match Snapshot::read_file(&original, file_reads::STORE_MAX) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Snapshot::read_file(&folder.join("transcript.ndjson"), file_reads::STORE_MAX)
        }
        result => result,
    };
    let bytes = match captured {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    Ok(bytes)
}
#[cfg(test)]
pub(super) fn chosen(
    root: &Path,
    id: &str,
    folder: &Path,
    chosen: Option<&str>,
    ended_ms: i64,
    now_ms: i64,
) -> io::Result<Option<Selection>> {
    chosen_sized(root, id, folder, chosen, ended_ms, now_ms, None)
}
/// `duration_ms` is the meeting's length, so a long one starts from English at once.
pub(super) fn chosen_sized(
    root: &Path,
    id: &str,
    folder: &Path,
    chosen: Option<&str>,
    ended_ms: i64,
    now_ms: i64,
    duration_ms: Option<u64>,
) -> io::Result<Option<Selection>> {
    if crate::core::meeting_parts::read(root, id)?.is_some() {
        return super::joined::chosen(
            root,
            &crate::core::meetings_dir(),
            id,
            folder,
            chosen,
            ended_ms,
            now_ms,
        )
        .map(|reading| {
            reading.map(|(revision, rendered, reasons)| Selection {
                revision,
                transcript: None,
                reasons,
                rendered,
            })
        });
    }
    chosen_part_sized(root, id, folder, chosen, ended_ms, now_ms, duration_ms)
}
pub(super) fn chosen_part(
    root: &Path,
    id: &str,
    folder: &Path,
    chosen: Option<&str>,
    ended_ms: i64,
    now_ms: i64,
) -> io::Result<Option<Selection>> {
    chosen_part_sized(root, id, folder, chosen, ended_ms, now_ms, None)
}
fn chosen_part_sized(
    root: &Path,
    id: &str,
    folder: &Path,
    chosen: Option<&str>,
    ended_ms: i64,
    now_ms: i64,
    duration_ms: Option<u64>,
) -> io::Result<Option<Selection>> {
    use crate::core::meeting_router::{self as router, Chunk};
    // With Labs off, retain the compatibility reader and its existing-file behavior.
    if !router::jobs::notes_enabled(root)? {
        return Ok(None);
    }
    // Healthy pins never need originals; failed pins can request Hex at the deadline.
    let baseline = || -> io::Result<Vec<Chunk>> {
        Ok(String::from_utf8_lossy(&captured(folder)?)
            .lines()
            .enumerate()
            .filter_map(|(i, line)| {
                let r: Row = serde_json::from_str(line).ok()?;
                let start = r.start_ms / 1000.0;
                let row: serde_json::Value = serde_json::from_str(line).ok()?;
                let end = row["end_ms"]
                    .as_f64()
                    .map(|v| v / 1000.0)
                    .filter(|v| v.is_finite() && *v > start)
                    .unwrap_or(start + 0.1);
                (start.is_finite()
                    && end.is_finite()
                    && start >= 0.0
                    && end > start
                    && !r.text.trim().is_empty())
                .then_some(Chunk {
                    id: format!("english-{i}"),
                    start,
                    end,
                    track: if r.source.is_empty() {
                        "system".into()
                    } else {
                        r.source
                    },
                    speaker: r.speaker,
                    turn: None,
                    audio_start: None,
                    words: router::capture_words(&row, start, end, &r.text),
                    english: r.text,
                })
            })
            .collect())
    };
    router::jobs::notes_input_sized(root, id, chosen, baseline, ended_ms, now_ms, duration_ms)?
        .map(|(id, t, reasons)| {
            let mut text = String::new();
            for s in &t.segments {
                let lang = match s.lang.as_str() {
                    "en" => "English",
                    "hi" => "Hindi",
                    "te" => "Telugu",
                    other => other,
                };
                let speaker = s
                    .sources
                    .first()
                    .and_then(|o| o.chunk.speaker.as_deref())
                    .unwrap_or(if s.track == "microphone" {
                        "You"
                    } else {
                        "Them"
                    });
                let ms = (s.start * 1000.0).round() as u64;
                text.push_str(&format!(
                    "[{:02}:{:02}:{:02}.{:03}] {} ({lang}): {}\n",
                    ms / 3_600_000,
                    ms / 60_000 % 60,
                    ms / 1000 % 60,
                    ms % 1000,
                    speaker.split_whitespace().collect::<Vec<_>>().join("_"),
                    s.text_original
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
            Ok(Selection {
                revision: id,
                transcript: Some(t),
                reasons,
                rendered: text,
            })
        })
        .transpose()
}
pub(super) fn write(folder: &Path) -> io::Result<String> {
    let path = folder.join("english-transcript.md");
    let existing = || -> io::Result<String> {
        String::from_utf8(Snapshot::read_file(&path, file_reads::STORE_MAX)?)
            .map_err(|_| store_io::invalid("Invalid text encoding"))
    };
    match existing() {
        Ok(text) => return Ok(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let text = read(folder)?;
    match store_io::publish(&path, text.as_bytes()) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return existing(),
        Err(e) => return Err(e),
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sorts_labels_skips_invalid_and_round_trips() {
        let raw = r#"{"start_ms":2300,"source":"system","text":"Second"}
garbage

{"start_ms":1000,"source":"microphone","text":"First"}
{"start_ms":3000,"speaker":"Guest","text":"Third"}
{"start_ms":0,"text":"   "}
{"start_ms":-3,"text":"bad"}"#;
        let rendered = render(raw);
        let lines = crate::core::parse_transcript(&rendered);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines.iter().map(|l| l.who.as_str()).collect::<Vec<_>>(),
            ["You", "Them", "Guest"]
        );
        assert!(rendered.starts_with("[00:00:01.000]"));
        assert_eq!(lines[1].text, "Second");
    }
    #[test]
    fn blank_is_empty() {
        assert!(render("\n{}\ninvalid").is_empty());
    }
    #[test]
    fn flag_off_keeps_legacy_end_shapes_and_existing_file_preference() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("meeting");
        std::fs::create_dir_all(&folder).unwrap();
        let raw = r#"{"start_ms":1000,"end_ms":"2000","text":"String end","source":"system"}
{"start_ms":2000,"end_ms":{"old":true},"text":"Object end","source":"microphone"}
{"start_ms":3000,"end_ms":null,"text":"Null end","speaker":"Guest"}
{"start_ms":4000,"text":"Missing end"}
{"start_ms":5000,"end_ms":-1,"text":"Negative end"}"#;
        let expected = "[00:00:01.000] Them (English): String end\n[00:00:02.000] You (English): Object end\n[00:00:03.000] Guest (English): Null end\n[00:00:04.000] Them (English): Missing end\n[00:00:05.000] Them (English): Negative end\n";
        std::fs::write(folder.join("transcript.ndjson"), raw).unwrap();
        assert!(
            chosen(
                root.path(),
                "123-4",
                &folder,
                Some("invalid stale pin"),
                0,
                0
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(write(&folder).unwrap(), expected);
        std::fs::write(folder.join("english-transcript.md"), "Saved legacy text").unwrap();
        std::fs::write(root.path().join("config.json"), b"invalid config").unwrap();
        assert!(
            chosen(root.path(), "123-4", &folder, None, 0, 0)
                .unwrap()
                .is_none()
        );
        assert_eq!(write(&folder).unwrap(), "Saved legacy text");
    }
}
