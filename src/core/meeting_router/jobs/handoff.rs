use super::super::{batches::Plan, inputs::Snapshot, pieces::PieceManifest, revision::Revision};
use super::*;

/// 8c writes this beside the immutable plan/results in router-work/<id>/batches.
#[derive(Serialize, Deserialize)]
pub struct BatchOutput {
    pub generation: String,
    pub source_revision: String,
    pub plan: Plan,
    pub manifest: PieceManifest,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<super::super::fallback::RoutingFallback>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub english_transport: serde_json::Value,
}
#[cfg(all(windows, feature = "asr-runtime"))]
impl BatchOutput {
    pub(crate) fn publish(&self, path: &Path) -> io::Result<()> {
        store_io::write(path, self)
    }
}
#[derive(Serialize, Deserialize)]
pub(super) struct Released {
    pub generation: String,
    pub revision: String,
    pub source_revision: String,
    pub completed_ms: i64,
}
pub(super) fn work(root: &Path, id: &str) -> PathBuf {
    root.join("local/router-work").join(id)
}
pub(super) fn completed(root: &Path, id: &str, source: &str) -> bool {
    let check = || -> Result<bool> {
        let released: Released = serde_json::from_slice(&Snapshot::read_file(
            &work(root, id).join("released.json"),
            4096,
        )?)?;
        let Some((chosen, data)) = revision::load(destination(root, id).parent().unwrap(), None)?
        else {
            return Ok(false);
        };
        Ok(chosen == released.revision
            && data.generation == released.generation
            && released.source_revision == source
            && data.transcript.input_checksum == source
            && !data.transcript.degraded())
    };
    check().unwrap_or(false)
}
pub(super) fn assembled(
    root: &Path,
    id: &str,
    generation: u128,
    source: &str,
    baseline: &[Chunk],
) -> Result<Revision> {
    let batches = work(root, id).join("batches");
    let output: BatchOutput = serde_json::from_slice(&Snapshot::read_file(
        &batches.join("output.json"),
        16 << 20,
    )?)?;
    if output.generation != generation.to_string() || output.source_revision != source {
        return Err("Stale batch output".into());
    }
    let mut data = revision::assemble(
        &batches,
        &output.plan,
        &output.manifest,
        baseline,
        generation,
    )?;
    if !output.english_transport.is_null() && data.provenance.get("speech_coverage").is_none() {
        return Err("Missing speech coverage record".into());
    }
    data.transcript.input_checksum = source.into();
    data.provenance["pieces"] = serde_json::to_value(&output.manifest)?;
    data.provenance["source_revision"] = source.into();
    data.provenance["english_transport"] = output.english_transport;
    super::super::fallback::attach(&mut data, &output.fallbacks)?;
    Ok(data)
}
/// The app parent uses this after its owned worker and source-stability checks.
pub fn commit_batches(
    root: &Path,
    id: &str,
    generation: u128,
    source: &str,
    baseline: &[Chunk],
) -> Result<String> {
    if !valid_id(id) {
        return Err("Invalid meeting id".into());
    }
    let _lock = pending_lock(root, id)?;
    let pending: Option<Pending> = store_io::read(&queue(root).join(format!("{id}.json")))?;
    if !pending.is_some_and(|p| p.id == id && p.generation == generation) {
        return Err("Stale routing generation".into());
    }
    let data = assembled(root, id, generation, source, baseline)?;
    let chosen = revision::commit(destination(root, id).parent().unwrap(), &data)?;
    store_io::write(
        &work(root, id).join("released.json"),
        &Released {
            generation: generation.to_string(),
            revision: chosen.clone(),
            source_revision: source.into(),
            completed_ms: chrono::Utc::now().timestamp_millis(),
        },
    )?;
    Ok(chosen)
}

#[cfg(all(test, windows))]
pub(crate) mod tests;
