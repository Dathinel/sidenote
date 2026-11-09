use crate::core::{file_reads, store_io};
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
    let original = folder.join("hex-original/transcript.ndjson");
    let source = if original.is_file() {
        original
    } else {
        folder.join("transcript.ndjson")
    };
    let bytes = match store_io::read_bounded(&source, file_reads::STORE_MAX) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    Ok(render(&String::from_utf8_lossy(&bytes)))
}
pub(super) fn write(folder: &Path) -> io::Result<String> {
    let text = read(folder)?;
    store_io::atomic_write(&folder.join("english-transcript.md"), text.as_bytes())?;
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
}
