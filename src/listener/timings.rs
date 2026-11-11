use regex::Regex;
use std::{collections::BTreeMap, io::Read, path::PathBuf, sync::LazyLock};

static LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"^pipeline-timing ([0-9-]{1,100}) (Audio preparation|Prepare[^:\r\n]{0,60}|te transcription|hi transcription|Translation and notes|Total): ([0-9]+(?:\.[0-9]+)?)s$"
).unwrap()
});

fn parse(line: &[u8]) -> Option<(&str, &'static str, f64)> {
    let line = std::str::from_utf8(line).ok()?.trim_end_matches('\r');
    let captures = LINE.captures(line)?;
    let id = captures.get(1)?.as_str();
    if !id.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    let key = match captures.get(2)?.as_str() {
        "Audio preparation" => "prepare",
        "te transcription" => "asr_te",
        "hi transcription" => "asr_hi",
        "Translation and notes" => "notes",
        "Total" => "total",
        label if label.starts_with("Prepare") => "prepare",
        _ => return None,
    };
    let seconds: f64 = captures.get(3)?.as_str().parse().ok()?;
    seconds.is_finite().then_some((id, key, seconds))
}

pub(super) fn capture(mut reader: impl Read, folders: &[PathBuf], mut report: impl FnMut(String)) {
    let mut values = BTreeMap::<String, BTreeMap<&str, f64>>::new();
    let mut line = Vec::with_capacity(512);
    let mut oversized = false;
    let mut buffer = [0; 4096];
    let mut consume = |line: &[u8]| {
        if let Some((id, key, seconds)) = parse(line)
            && folders
                .iter()
                .any(|p| p.file_name().is_some_and(|n| n == id))
        {
            values
                .entry(id.to_owned())
                .or_default()
                .insert(key, seconds);
        }
    };
    while let Ok(count) = reader.read(&mut buffer) {
        if count == 0 {
            break;
        }
        for &byte in &buffer[..count] {
            if byte == b'\n' {
                if !oversized {
                    consume(&line);
                }
                line.clear();
                oversized = false;
            } else if line.len() < 512 && !oversized {
                line.push(byte);
            } else {
                oversized = true;
                line.clear();
            }
        }
    }
    if !oversized {
        consume(&line);
    }
    for (id, timings) in values {
        let mut message = format!("pipeline {id}");
        for key in ["prepare", "asr_te", "asr_hi", "notes", "total"] {
            if let Some(seconds) = timings.get(key) {
                message.push_str(&format!(" {key}={seconds:.1}s"));
            }
        }
        report(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_numeric_timings_from_requested_meetings_reach_the_log() {
        let raw = format!(
            "private transcript\nte 1/12 private-id: 10s\npipeline-timing 100-1 Prepare audio: 2.5s\r\npipeline-timing 100-1 te transcription: 30s\npipeline-timing 100-1 hi transcription: 40.1s\npipeline-timing 100-1 Translation and notes: 20s\npipeline-timing 100-1 Total: 100s\npipeline-timing 200-1 Total: 100s\npipeline-timing 100-1 Total: 1s private\npipeline-timing 100-1 Total: NaNs\n{}pipeline-timing 100-1 Total: 999s\npipeline-timing 100-1 Private: 1s\n",
            "private".repeat(2000)
        );
        let mut lines = Vec::new();
        capture(raw.as_bytes(), &[PathBuf::from("meetings/100-1")], |s| {
            lines.push(s)
        });
        assert_eq!(
            lines,
            ["pipeline 100-1 prepare=2.5s asr_te=30.0s asr_hi=40.1s notes=20.0s total=100.0s"]
        );
    }
    #[test]
    fn handles_multiple_meetings_and_a_final_line_without_newline() {
        let mut lines = Vec::new();
        capture(
            &b"pipeline-timing 100-1 Audio preparation: 1s\npipeline-timing 200-1 Total: 2s"[..],
            &[PathBuf::from("100-1"), PathBuf::from("200-1")],
            |s| lines.push(s),
        );
        assert_eq!(
            lines,
            ["pipeline 100-1 prepare=1.0s", "pipeline 200-1 total=2.0s"]
        );
    }
}
