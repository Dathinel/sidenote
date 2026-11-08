//! Headless migration inspection, restricted to an explicitly redirected temporary state copy.
use hex_meetings::core::{self, memory::MemoryStore};
use serde_json::json;
use std::{fs, io, path::PathBuf};

fn counts(preview: &hex_meetings::core::memory::MigrationPreview) -> serde_json::Value {
    let mut counts = std::collections::BTreeMap::new();
    for kind in [
        "person",
        "project",
        "decision",
        "commitment",
        "question",
        "story",
    ] {
        counts.insert(kind.to_string(), 0usize);
    }
    for candidate in &preview.candidates {
        let kind = serde_json::to_value(candidate.record.record_type).unwrap();
        *counts.get_mut(kind.as_str().unwrap()).unwrap() += 1;
    }
    json!(counts)
}

fn main() -> io::Result<()> {
    let root = PathBuf::from(
        std::env::var_os("HEX_MEETINGS_STATE_DIR")
            .ok_or_else(|| io::Error::other("Set HEX_MEETINGS_STATE_DIR to a temp copy"))?,
    );
    let root = root.canonicalize()?;
    if !root.starts_with(std::env::temp_dir().canonicalize()?)
        || !root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .starts_with("hex-memory2-")
    {
        return Err(io::Error::other(
            "Refusing a state directory outside a hex-memory2- temp copy",
        ));
    }
    let store = MemoryStore::new(root.clone());
    let original = fs::read(root.join("memory/company.md"))?;
    let before = store.preview_migration()?;
    let preview = if std::env::args().any(|s| s == "--model") {
        store.prepare_migration(true)?
    } else {
        before.clone()
    };
    println!(
        "{}",
        json!({"before":counts(&before), "after":counts(&preview), "preview": {"imported":preview.imported,"uncited":preview.uncited,"ambiguous":preview.ambiguous, "unplaced":preview.unplaced,"fallbacks":preview.fallbacks,"runner":preview.runner,"runtime_ms":preview.runtime_ms}})
    );
    fs::write(
        root.join("migration-preview.json"),
        serde_json::to_vec_pretty(&preview)?,
    )?;
    if fs::read(root.join("memory/company.md"))? != original || store.load()?.migration.is_some() {
        return Err(io::Error::other(
            "Preview changed the legacy memory or accepted the migration",
        ));
    }
    let reread = store.preview_migration()?;
    if serde_json::to_value(&preview)? != serde_json::to_value(&reread)? {
        return Err(io::Error::other(
            "Cached preview did not reproduce every candidate",
        ));
    }
    if std::env::args().any(|s| s == "--preview-only") {
        println!("Preview only; legacy memory unchanged");
        return Ok(());
    }
    store.accept_migration(&preview.source_fingerprint)?;
    let file = store.load()?;
    let backup = fs::read(root.join("local/memory/company.legacy.md"))?;
    let preserved = preview.candidates.iter().all(|c| {
        let Some(r) = file.records.iter().find(|r| r.id == c.record.id) else {
            return false;
        };
        let mut expected = c.record.clone();
        expected.status = r.status;
        expected.updated_at = r.updated_at.clone();
        r.status == core::memory::RecordStatus::Active
            && serde_json::to_value(r).ok() == serde_json::to_value(expected).ok()
    });
    if original != backup || file.records.len() != preview.imported || !preserved {
        return Err(io::Error::other(
            "Migration did not preserve all candidate text, citations and backup bytes",
        ));
    }
    // Explicit temporary inspection output for the UI fixture builder, never authoritative state.
    fs::write(
        root.join("local/memory/records.json"),
        serde_json::to_vec_pretty(&file)?,
    )?;
    for date in core::list_checks()? {
        let report = core::load_check(date)?;
        println!(
            "{}",
            json!({"report":date,"schema_version":report.schema_version,"findings":report.content.findings.len(),"legacy_marks":core::check_state::load(date)?.len()})
        );
    }
    println!(
        "{}",
        json!({"accepted":true,"records_refetched":file.records.len(),"all_candidate_fields_preserved":preserved,"backup_bytes_preserved":original == backup,"unknown_first_seen":file.records.iter().filter(|r| r.first_seen.is_none()).count(),"unknown_last_confirmed":file.records.iter().filter(|r| r.last_confirmed.is_none()).count()})
    );
    Ok(())
}
