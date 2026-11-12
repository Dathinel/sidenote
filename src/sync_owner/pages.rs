use super::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const PAGE_BYTES: usize = 16 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Cursor {
    revision: String,
    offset: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConflictRef {
    pub domain: String,
    pub task_id: String,
    pub field: String,
    pub revision: String,
}
struct Digest(blake3::Hasher);
impl io::Write for Digest {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn status(state: &Snapshot) -> Value {
    let meetings: BTreeSet<_> = state.meetings.iter().map(|m| &m.id).collect();
    let conflicts: BTreeSet<_> = state
        .meetings
        .iter()
        .filter(|m| m.conflict)
        .map(|m| &m.id)
        .collect();
    json!({"generation":state.generation,"source_revision":state.source_revision,
        "meetings":meetings.len(),"meeting_sources":state.meetings.len(),"meeting_conflicts":conflicts.len(),
        "pending_sources":state.meetings.iter().filter(|m|m.pending).count(),
        "tasks":state.tasks,"errors":state.errors,"relay_enabled":state.relay_enabled,
        "relay_status":state.relay_status,"peers":state.peers,"identity":state.identity})
}
pub(super) fn revision<T: Serialize + ?Sized>(items: &T) -> io::Result<String> {
    let mut digest = Digest(blake3::Hasher::new());
    serde_json::to_writer(&mut digest, items)?;
    Ok(digest.0.finalize().to_hex().to_string())
}
pub(super) fn conflict_ref(c: &relay::Conflict) -> io::Result<ConflictRef> {
    Ok(ConflictRef {
        domain: c.domain.clone(),
        task_id: c.task_id.clone(),
        field: c.field.clone(),
        revision: revision(c)?,
    })
}
pub(super) fn page<T: Serialize>(items: &[T], cursor: Option<Cursor>) -> io::Result<Value> {
    // Hash by streaming, without allocating a serialized copy of the full inventory.
    let revision = revision(items)?;
    let start = match cursor {
        Some(cursor) if cursor.revision == revision && cursor.offset <= items.len() => {
            cursor.offset
        }
        Some(_) => return Err(io::Error::other("SYNC_CONTROL_CURSOR_STALE")),
        None => 0,
    };
    let mut end = start;
    let mut size = 0;
    while end < items.len() && end - start < 32 {
        let bytes = serde_json::to_vec(&items[end])?;
        if bytes.len() > PAGE_BYTES {
            return Err(io::Error::other("SYNC_CONTROL_ITEM_TOO_LARGE"));
        }
        if size + bytes.len() > PAGE_BYTES {
            break;
        }
        size += bytes.len() + 1;
        end += 1;
    }
    let next = (end < items.len()).then(|| Cursor {
        revision: revision.clone(),
        offset: end,
    });
    Ok(json!({"revision":revision,"items":&items[start..end],"total":items.len(),"next":next}))
}
