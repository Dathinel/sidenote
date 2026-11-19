use super::*;
use crate::core::{meeting_router::inputs::Snapshot, store_io};
use sha2::{Digest, Sha256};
use std::io;
const SELECTED: &str = "transcript.segments.v1.json";
const MAX: u64 = 16 << 20;
#[derive(Serialize, Deserialize)]
struct Selected {
    revision: String,
    #[serde(flatten)]
    data: Revision,
}
fn digest(data: &Revision) -> io::Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(data)?)))
}
fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn validate(data: &Revision) -> io::Result<()> {
    let t = &data.transcript;
    if t.schema != "transcript.segments.v1"
        || t.input_checksum.is_empty()
        || t.baseline.is_empty()
        || t.baseline.iter().any(|b| {
            !b.start.is_finite()
                || !b.end.is_finite()
                || b.start < 0.0
                || b.start >= b.end
                || b.english.trim().is_empty()
        })
        || t.segments.iter().any(|s| {
            !s.start.is_finite()
                || !s.end.is_finite()
                || s.start < 0.0
                || s.start >= s.end
                || s.text_original.trim().is_empty()
                || s.lang.is_empty()
                || (!crate::core::lid::LANGUAGES.contains(&s.lang.as_str()) && s.lang != "mixed")
                || s.route != s.lang
                || s.sources.is_empty()
                || s.sources.iter().any(|o| {
                    o.chunk.track != s.track || o.chunk.start < s.start || o.chunk.end > s.end
                })
                || s.sources.iter().any(|o| {
                    !o.chunk.start.is_finite()
                        || !o.chunk.end.is_finite()
                        || o.chunk.start < 0.0
                        || o.chunk.start >= o.chunk.end
                })
        })
    {
        return Err(store_io::invalid("Invalid transcript revision"));
    }
    let mut check = t.clone();
    let coverage = data
        .provenance
        .get("assembly_coverage")
        .map(|value| serde_json::from_value::<super::reconciliation::Coverage>(value.clone()))
        .transpose()?;
    if let Some(coverage) = &coverage {
        super::reconciliation::validate_records(data, coverage)
            .map_err(|_| store_io::invalid("Invalid recorded fallback overlap evidence"))?;
        if data.alternatives.iter().any(|s| s.sources.is_empty()) {
            return Err(store_io::invalid("Invalid routed alternatives"));
        }
    }
    for (i, a) in t.segments.iter().enumerate() {
        if t.segments[i + 1..].iter().any(|b| {
            if coverage.as_ref().is_some_and(|c| {
                super::reconciliation::receipt(a, b, &t.baseline, c)
                    .is_some_and(|receipt| c.overlaps.contains(&receipt))
            }) {
                return false;
            }
            if a.route == "en"
                && b.route == "en"
                && ![a.status.as_str(), b.status.as_str()]
                    .iter()
                    .any(|s| matches!(*s, "segment-level" | "validated-samples"))
            {
                return false;
            }
            a.sources.iter().any(|x| {
                b.sources.iter().any(|y| {
                    x.chunk.track == y.chunk.track
                        && x.chunk.start < y.chunk.end
                        && y.chunk.start < x.chunk.end
                })
            })
        }) {
            return Err(store_io::invalid("Competing selected readings"));
        }
    }
    if let Some(mut coverage) = coverage {
        let recorded = serde_json::to_value(&coverage)?;
        let mut expected = data.clone();
        expected.transcript.segments = data.alternatives.clone();
        super::reconciliation::reconcile(&mut expected.transcript, &t.baseline, &mut coverage);
        super::super::fallback::attach(&mut expected, &super::super::fallback::read(data)?)?;
        if serde_json::to_value(&coverage)? != recorded {
            return Err(store_io::invalid("Invalid overlap receipt"));
        }
        super::gaps::validate(data, &mut expected, &coverage)
            .map_err(|_| store_io::invalid("Invalid marked gap evidence"))?;
        check = expected.transcript;
    } else {
        reconcile(&mut check, &t.baseline);
    }
    if serde_json::to_vec(&check.segments)? != serde_json::to_vec(&t.segments)? {
        return Err(store_io::invalid("Incomplete transcript coverage"));
    }
    Ok(())
}
/// The queue calls this under its per-meeting lock before publishing the request.
pub fn begin(dir: &Path, generation: u128) -> io::Result<()> {
    let _lock = store_io::lock(&dir.join("revision.lock"))?;
    store_io::write(&dir.join("generation.json"), &generation)
}
/// Publish immutable bytes first, then atomically select. A failure never replaces success.
pub fn commit(dir: &Path, data: &Revision) -> io::Result<String> {
    write_revision(dir, data, true)
}
/// Preserve an unchosen fallback without reading or replacing a damaged selection.
pub fn retain(dir: &Path, data: &Revision) -> io::Result<String> {
    write_revision(dir, data, false)
}
fn write_revision(dir: &Path, data: &Revision, select: bool) -> io::Result<String> {
    let _lock = store_io::lock(&dir.join("revision.lock"))?;
    let current: u128 =
        serde_json::from_slice(&Snapshot::read_file(&dir.join("generation.json"), 128)?)?;
    if current.to_string() != data.generation {
        return Err(store_io::invalid("Stale routing generation"));
    }
    validate(data)?;
    let revision = digest(data)?;
    let bytes = serde_json::to_vec(data)?;
    if bytes.len() as u64 > MAX {
        return Err(store_io::invalid("Transcript revision exceeds limit"));
    }
    let path = dir.join("revisions").join(format!("{revision}.json"));
    match store_io::publish(&path, &bytes) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if Snapshot::read_file(&path, MAX)? != bytes {
                return Err(store_io::invalid("Revision collision"));
            }
        }
        Err(e) => return Err(e),
    }
    if !select {
        return Ok(revision);
    }
    let legacy = match Snapshot::read_file(&dir.join(SELECTED), MAX) {
        Ok(bytes)
            if serde_json::from_slice::<Value>(&bytes)
                .map_or(true, |v| v["revision"].is_null())
                || load(dir, None).is_err() =>
        {
            Some(bytes)
        }
        Ok(_) => None,
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if let Some(bytes) = &legacy {
        let name = dir.join("revisions").join(format!(
            "legacy-{}.json",
            hex::encode(Sha256::digest(bytes))
        ));
        match store_io::publish(&name, bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    if !data.transcript.degraded() || (legacy.is_none() && load(dir, None)?.is_none()) {
        store_io::write(
            &dir.join(SELECTED),
            &Selected {
                revision: revision.clone(),
                data: serde_json::from_slice(&bytes)?,
            },
        )?;
    }
    Ok(revision)
}
/// Pinned reads never consult the latest selection and never open audio or invoke ASR.
pub fn load(dir: &Path, chosen: Option<&str>) -> io::Result<Option<(String, Revision)>> {
    let path = if let Some(id) = chosen {
        if !valid_id(id) {
            return Err(store_io::invalid("Invalid revision id"));
        }
        dir.join("revisions").join(format!("{id}.json"))
    } else {
        dir.join(SELECTED)
    };
    let bytes = match Snapshot::read_file(&path, MAX) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        other => other?,
    };
    let (id, data) = match chosen {
        Some(id) => (id.to_owned(), serde_json::from_slice::<Revision>(&bytes)?),
        None => {
            if serde_json::from_slice::<Value>(&bytes)?["revision"].is_null() {
                return Ok(None);
            }
            let s: Selected = serde_json::from_slice(&bytes)?;
            (s.revision, s.data)
        }
    };
    if digest(&data)? != id {
        return Err(store_io::invalid("Revision checksum mismatch"));
    }
    validate(&data)?;
    Ok(Some((id, data)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_revisions_reject_stale_partial_and_preserve_last_success() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let b = Chunk {
            id: "b".into(),
            start: 0.0,
            end: 1.0,
            track: "system".into(),
            speaker: None,
            turn: None,
            english: "Baseline".into(),
            audio_start: None,
            words: Vec::new(),
        };
        let mut data = Revision {
            generation: "1".into(),
            plan_revision: "plan".into(),
            transcript: english_fallback(&[b], &Default::default(), "input".into(), "failed"),
            alternatives: vec![],
            evidence: BTreeMap::new(),
            provenance: Value::Null,
        };
        begin(dir, 1).unwrap();
        let fallback_status = data.transcript.segments[0].status.clone();
        let fallback = commit(dir, &data).unwrap();
        data.transcript.segments[0].status = "checked".into();
        let good = commit(dir, &data).unwrap();
        assert_eq!(load(dir, None).unwrap().unwrap().0, good);
        data.transcript.segments[0].status = fallback_status;
        assert_eq!(commit(dir, &data).unwrap(), fallback);
        assert_eq!(load(dir, None).unwrap().unwrap().0, good);
        assert_eq!(load(dir, Some(&fallback)).unwrap().unwrap().0, fallback);
        let complete = std::mem::take(&mut data.transcript.segments);
        assert!(commit(dir, &data).is_err());
        data.transcript.segments = complete;
        begin(dir, 2).unwrap();
        assert!(commit(dir, &data).is_err());
        assert_eq!(load(dir, None).unwrap().unwrap().0, good);
        assert!(load(dir, Some("../escape")).is_err());
        // A pre-revision routed file also survives a failed first attempt.
        let legacy = serde_json::to_vec(&data.transcript).unwrap();
        std::fs::write(dir.join(SELECTED), &legacy).unwrap();
        data.transcript = english_fallback(
            &data.transcript.baseline,
            &Default::default(),
            "input".into(),
            "failed",
        );
        data.generation = "2".into();
        commit(dir, &data).unwrap();
        assert_eq!(std::fs::read(dir.join(SELECTED)).unwrap(), legacy);
    }
}
