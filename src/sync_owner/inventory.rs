use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn observe(prior: &[Ingested]) -> (Vec<Ingested>, usize) {
    // Absence is not a deletion protocol. Keep last-good state until a source recovers.
    let mut rows: BTreeMap<_, _> = prior
        .iter()
        .cloned()
        .map(|mut row| {
            row.pending = true;
            ((row.device.clone(), row.id.clone()), row)
        })
        .collect();
    let mut roots = vec![(None, core::meetings_dir(), true)];
    let mut errors = 0;
    match core::file_reads::entries(&core::peers_dir()) {
        Ok(entries) => {
            for entry in entries {
                if core::file_reads::directory(&entry.path()) {
                    roots.push((
                        Some(entry.file_name().to_string_lossy().into_owned()),
                        entry.path(),
                        false,
                    ));
                }
            }
        }
        Err(_) => errors += 1,
    }
    for (device, path, pending) in roots {
        let (meetings, warnings) = core::scan_ingestion_source(&path, pending);
        errors += warnings.len();
        for meeting in meetings {
            let row = Ingested {
                revision: meeting_revision(&meeting),
                device: device.clone(),
                id: meeting.id,
                pending: false,
                conflict: false,
            };
            rows.insert((row.device.clone(), row.id.clone()), row);
        }
    }
    let mut variants: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows.values() {
        variants
            .entry(row.id.clone())
            .or_default()
            .insert(row.revision.clone());
    }
    for row in rows.values_mut() {
        row.conflict = variants[&row.id].len() > 1;
        errors += usize::from(row.pending);
    }
    (rows.into_values().collect(), errors)
}
