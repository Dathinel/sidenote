//! Join immutable per-part language readings only after an explicit notes request.
use super::*;
use crate::core::{
    self, file_reads,
    meeting_router::fallback::{Reason, RoutingFallback},
    store_io,
};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};
pub(super) type Chosen = Option<(String, String, Vec<RoutingFallback>)>;
#[derive(Serialize, Deserialize)]
struct Reading {
    text: String,
    reasons: Vec<RoutingFallback>,
    parts: Vec<(String, String)>,
}
fn waiting() -> io::Error {
    io::Error::new(
        io::ErrorKind::WouldBlock,
        store_io::UserMessage("routing_pending".into()),
    )
}
fn publish(root: &Path, id: &str, folder: &Path, reading: Reading) -> io::Result<Chosen> {
    let bytes = serde_json::to_vec(&reading)?;
    let revision = blake3::hash(&bytes).to_hex().to_string();
    store_io::atomic_write(
        &root
            .join("local/joined-transcripts")
            .join(id)
            .join(format!("{revision}.json")),
        &bytes,
    )?;
    store_io::atomic_write(
        &folder.join("english-transcript.md"),
        reading.text.as_bytes(),
    )?;
    Ok(Some((revision, reading.text, reading.reasons)))
}
pub(super) fn chosen(
    root: &Path,
    hex: &Path,
    id: &str,
    folder: &Path,
    chosen: Option<&str>,
    ended_ms: i64,
    now_ms: i64,
) -> io::Result<Chosen> {
    if !core::meeting_router::jobs::notes_enabled(root)? {
        return Ok(None);
    }
    let manifest: serde_json::Value = serde_json::from_slice(&file_reads::read(
        &folder.join("manifest.json"),
        1024 * 1024,
    )?)?;
    let requested = manifest["notes_requested_at_ms"]
        .as_i64()
        .unwrap_or(ended_ms);
    if let Some(pin) = chosen {
        if pin.len() != 64 || !pin.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(store_io::invalid("Invalid joined reading"));
        }
        let path = root
            .join("local/joined-transcripts")
            .join(id)
            .join(format!("{pin}.json"));
        if let Ok(bytes) = file_reads::read(&path, file_reads::STORE_MAX)
            && blake3::hash(&bytes).to_hex().as_str() == pin
        {
            let reading: Reading = serde_json::from_slice(&bytes)?;
            store_io::atomic_write(
                &folder.join("english-transcript.md"),
                reading.text.as_bytes(),
            )?;
            return Ok(Some((pin.into(), reading.text, reading.reasons)));
        }
        let config = core::Config::load_in(root)?;
        let deadline = requested.saturating_add(
            i64::from(config.engine_settings.stages.routing_deadline_seconds) * 1000,
        );
        if now_ms < deadline {
            return Err(waiting());
        }
        return publish(
            root,
            id,
            folder,
            Reading {
                text: transcript::read(folder)?,
                reasons: vec![RoutingFallback::new(Reason::PinnedRevisionUnavailable)],
                parts: Vec::new(),
            },
        );
    }
    let parts: Vec<core::meeting_parts::Part> = serde_json::from_slice(&file_reads::read(
        &folder.join("sidenote-parts.json"),
        file_reads::SUMMARY_MAX,
    )?)?;
    let mut reading = Reading {
        text: String::new(),
        reasons: Vec::new(),
        parts: Vec::new(),
    };
    let mut offset = 0u64;
    for part in parts {
        if !queue::valid_id(&part.id) {
            return Err(store_io::invalid("Invalid meeting part"));
        }
        let Some((revision, text, reasons)) =
            transcript::chosen_part(root, &part.id, &hex.join(&part.id), None, requested, now_ms)?
        else {
            return Ok(None);
        };
        reading
            .text
            .push_str(&core::notes::shift_transcript(&text, offset)?);
        reading.parts.push((part.id, revision));
        reading.reasons.extend(reasons);
        offset = offset.saturating_add(part.duration_ms);
        if reading.text.len() as u64 > file_reads::STORE_MAX {
            return Err(store_io::invalid("Meeting transcript is too large"));
        }
    }
    publish(root, id, folder, reading)
}
