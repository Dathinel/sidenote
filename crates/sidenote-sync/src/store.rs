use crate::{
    identity::hash,
    preflight::{MAX_DOC, split_chunks},
    status::{Failure, Reason},
};
use anyhow::{Context, Result, ensure};
use automerge::{ActorId, Automerge, Change, ChangeHash, ReadDoc};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub const MAX_STORE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_PEER_DOCUMENTS: u64 = 50_000;
pub const INVENTORY_PAGE: usize = 128;
pub(crate) const COMPACT_AT: usize = 1024 * 1024;
pub(crate) const ORPHAN_CAP: usize = 256 * 1024;
pub(crate) struct Commit {
    pub creator: Option<[u8; 32]>,
    pub chunks: Vec<Vec<u8>>,
    pub heads: Vec<ChangeHash>,
    pub bytes: usize,
    pub compact: Option<Vec<u8>>,
    pub orphans: usize,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub document_bytes: usize,
    pub store_bytes: u64,
    pub peer_documents: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            document_bytes: MAX_DOC,
            store_bytes: MAX_STORE_BYTES,
            peer_documents: MAX_PEER_DOCUMENTS,
        }
    }
}

pub fn valid_domain(id: &str) -> bool {
    ["tasks", "memory", "projects", "diary", "people", "settings"].contains(&id)
        || id.strip_prefix("meeting:").is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
}
fn local<T>(r: rusqlite::Result<T>) -> Result<T> {
    r.map_err(|error| {
        Failure(
            if error.sqlite_error_code() == Some(rusqlite::ErrorCode::DiskFull) {
                Reason::StoreFull
            } else {
                Reason::LocalStore
            },
        )
        .into()
    })
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Snapshot {
    pub version: i64,
    pub chunks: Vec<Vec<u8>>,
    pub reset: bool,
    pub heads: Vec<ChangeHash>,
}

pub struct Store {
    pub(crate) capabilities: Option<crate::capabilities::Policy>,
    db: Connection,
    edit_cache: Option<(String, i64, Automerge)>,
    actor: ActorId,
    path: PathBuf,
    limits: Limits,
    _lock: File,
}
impl Store {
    pub fn open(path: &Path, actor: &[u8]) -> Result<Self> {
        Self::open_with_limits(path, actor, Limits::default())
    }
    pub fn open_with_limits(path: &Path, actor: &[u8], limits: Limits) -> Result<Self> {
        ensure!(
            limits.document_bytes <= MAX_DOC
                && limits.store_bytes <= MAX_STORE_BYTES
                && limits.peer_documents <= MAX_PEER_DOCUMENTS,
            "limits cannot exceed hard bounds"
        );
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".sync-lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(Path::new(&lock_path))?;
        lock.try_lock()
            .context("sync store already open; one writer per install is required")?;
        let db = local(Connection::open(path))?;
        local(db.busy_timeout(std::time::Duration::from_secs(5)))?;
        local(db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA wal_autocheckpoint=256;
            CREATE TABLE IF NOT EXISTS documents(id TEXT PRIMARY KEY,data BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS protected(id TEXT PRIMARY KEY,data BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS doc_meta(id TEXT PRIMARY KEY,version INTEGER NOT NULL,heads BLOB NOT NULL,bytes INTEGER NOT NULL,creator BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS doc_chunks(id TEXT NOT NULL,version INTEGER NOT NULL,hash BLOB NOT NULL,data BLOB NOT NULL,PRIMARY KEY(id,hash));
            CREATE INDEX IF NOT EXISTS chunk_version ON doc_chunks(id,version);
            CREATE TABLE IF NOT EXISTS doc_baselines(id TEXT PRIMARY KEY,version INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS doc_orphans(id TEXT NOT NULL,sender BLOB NOT NULL,bytes INTEGER NOT NULL,PRIMARY KEY(id,sender));
            CREATE TABLE IF NOT EXISTS doc_senders(id TEXT PRIMARY KEY,sender BLOB NOT NULL);
            CREATE INDEX IF NOT EXISTS doc_sender ON doc_senders(sender,id);
            CREATE INDEX IF NOT EXISTS doc_creator ON doc_meta(creator,id);"))?;
        let page_size: i64 = local(db.pragma_query_value(None, "page_size", |row| row.get(0)))?;
        ensure!(page_size > 0, Failure(Reason::CorruptDocument));
        local(db.pragma_update(
            None,
            "max_page_count",
            (limits.store_bytes / page_size as u64) as i64,
        ))?;
        Ok(Self {
            db,
            capabilities: None,
            edit_cache: None,
            actor: ActorId::from(actor),
            path: path.to_owned(),
            limits,
            _lock: lock,
        })
    }
    pub fn set_actor(&mut self, actor: &[u8]) {
        self.actor = ActorId::from(actor);
    }
    pub fn inventory_page(&self, after: Option<&str>) -> Result<Vec<String>> {
        let mut stmt = local(
            self.db
                .prepare("SELECT id FROM documents WHERE id>?1 ORDER BY id LIMIT ?2"),
        )?;
        let rows = local(
            stmt.query_map(params![after.unwrap_or(""), INVENTORY_PAGE as i64], |r| {
                r.get(0)
            }),
        )?;
        local(rows.collect())
    }
    pub fn inventory(&self) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let mut cursor = None;
        loop {
            let page = self.inventory_page(cursor.as_deref())?;
            if page.is_empty() {
                break;
            }
            cursor = page.last().cloned();
            result.extend(page);
        }
        Ok(result)
    }
    pub(crate) fn version(&self, id: &str) -> Result<i64> {
        Ok(local(
            self.db
                .query_row("SELECT version FROM doc_meta WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional(),
        )?
        .unwrap_or(0))
    }
    pub(crate) fn snapshot(&self, id: &str, loaded: Option<i64>) -> Result<Snapshot> {
        ensure!(valid_domain(id), "unauthorized document domain");
        let version = self.version(id)?;
        let baseline: i64 = local(self.db.query_row(
            "SELECT coalesce((SELECT version FROM doc_baselines WHERE id=?1),0)",
            [id],
            |r| r.get(0),
        ))?;
        let reset = loaded.is_none_or(|v| v < baseline);
        let loaded = if reset { None } else { loaded };
        let mut chunks = Vec::new();
        if reset
            && let Some(data) = local(
                self.db
                    .query_row("SELECT data FROM documents WHERE id=?1", [id], |r| {
                        r.get::<_, Vec<u8>>(0)
                    })
                    .optional(),
            )?
            && !data.is_empty()
        {
            chunks.push(data);
        }
        let mut stmt = local(self.db.prepare(
            "SELECT data FROM doc_chunks WHERE id=?1 AND version>?2 ORDER BY version,rowid",
        ))?;
        let rows =
            local(stmt.query_map(params![id, loaded.unwrap_or(0)], |r| r.get::<_, Vec<u8>>(0)))?;
        for row in rows {
            chunks.push(local(row)?);
        }
        ensure!(
            chunks.iter().map(Vec::len).sum::<usize>() <= self.limits.document_bytes,
            Failure(Reason::CorruptDocument)
        );
        Ok(Snapshot {
            version,
            chunks,
            reset,
            heads: self.heads(id)?,
        })
    }
    /// Local projections decode only bounded, already-validated durable history.
    /// Network decode/receive/generate runs exclusively in the child worker.
    pub fn load(&self, id: &str) -> Result<Automerge> {
        let snapshot = self.snapshot(id, None)?;
        let mut doc = Automerge::new();
        for bytes in snapshot.chunks {
            doc.load_incremental(&bytes)
                .map_err(|_| Failure(Reason::CorruptDocument))?;
        }
        doc.set_actor(self.actor.clone());
        Ok(doc)
    }
    pub fn heads(&self, id: &str) -> Result<Vec<ChangeHash>> {
        let bytes: Option<Vec<u8>> = local(
            self.db
                .query_row("SELECT heads FROM doc_meta WHERE id=?1", [id], |r| r.get(0))
                .optional(),
        )?;
        match bytes {
            Some(b) => {
                postcard::from_bytes(&b).map_err(|_| Failure(Reason::CorruptDocument).into())
            }
            None => {
                let data: Option<Vec<u8>> = local(
                    self.db
                        .query_row("SELECT data FROM documents WHERE id=?1", [id], |r| r.get(0))
                        .optional(),
                )?;
                match data {
                    Some(data) if !data.is_empty() => Ok(Automerge::load(&data)
                        .map_err(|_| Failure(Reason::CorruptDocument))?
                        .get_heads()),
                    _ => Ok(Vec::new()),
                }
            }
        }
    }
    pub fn edit<T>(&mut self, id: &str, f: impl FnOnce(&mut Automerge) -> Result<T>) -> Result<T> {
        self.check_compatible()?;
        let version = self.version(id)?;
        let mut doc = match self.edit_cache.take() {
            Some((cached, loaded, doc)) if cached == id && loaded == version => doc,
            _ => self.load(id)?,
        };
        let result = f(&mut doc)?;
        self.save(id, &doc)?;
        self.edit_cache = Some((id.to_owned(), self.version(id)?, doc));
        Ok(result)
    }
    pub fn save(&mut self, id: &str, doc: &Automerge) -> Result<()> {
        self.check_compatible()?;
        ensure!(
            self.heads(id)?
                .iter()
                .all(|h| doc.get_change_by_hash(h).is_some()),
            "stale save would discard committed history"
        );
        let size = doc.save_nocompress().len();
        if size > self.limits.document_bytes {
            return Err(Failure(Reason::DocumentTooLarge).into());
        }
        let delta = doc.save_after(&self.heads(id)?);
        let chunks = split_chunks(&delta)?
            .into_iter()
            .map(<[u8]>::to_vec)
            .collect();
        let creator = self.actor.to_bytes().to_vec();
        self.append(
            id,
            self.version(id)?,
            Commit {
                creator: doc
                    .get_changes(&[])
                    .into_iter()
                    .filter(|change| change.deps().is_empty())
                    .min_by_key(|change| change.hash())
                    .and_then(|change| change.actor_id().to_bytes().try_into().ok()),
                chunks,
                heads: doc.get_heads(),
                bytes: size,
                compact: Some(doc.save()),
                orphans: 0,
            },
            &creator,
        )?;
        Ok(())
    }
    pub(crate) fn append(
        &mut self,
        id: &str,
        expected: i64,
        commit: Commit,
        sender: &[u8],
    ) -> Result<i64> {
        self.check_compatible()?;
        let Commit {
            creator,
            chunks,
            heads,
            bytes: canonical_size,
            compact,
            orphans,
        } = commit;
        let peer = sender != self.actor.to_bytes();
        let creator = creator.as_ref().map_or(&[][..], |key| key.as_slice());
        let document_cap = if peer {
            self.limits.document_bytes * 9 / 10
        } else {
            self.limits.document_bytes
        };
        ensure!(valid_domain(id), "unauthorized domain");
        if self.version(id)? != expected {
            return Err(Failure(Reason::ConcurrentEdit).into());
        }
        if canonical_size > document_cap || heads.len() > 1024 {
            return Err(Failure(Reason::DocumentTooLarge).into());
        }
        let exists: bool = local(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM documents WHERE id=?1)",
            [id],
            |r| r.get(0),
        ))?;
        if !exists && peer {
            let count: i64 = local(self.db.query_row(
                "SELECT count(*) FROM doc_meta d LEFT JOIN doc_senders s ON d.id=s.id WHERE coalesce(s.sender,d.creator)=?1",
                [sender],
                |r| r.get(0),
            ))?;
            // A relay can introduce documents from all 16 roster members.
            if count >= (self.limits.peer_documents * 16) as i64 {
                return Err(Failure(Reason::DocumentQuota).into());
            }
        }
        let prior_creator: Option<Vec<u8>> = local(
            self.db
                .query_row("SELECT creator FROM doc_meta WHERE id=?1", [id], |r| {
                    r.get(0)
                })
                .optional(),
        )?;
        if peer && !creator.is_empty() && prior_creator.as_ref().is_none_or(Vec::is_empty) {
            let count: i64 = local(self.db.query_row(
                "SELECT count(*) FROM doc_meta WHERE creator=?1",
                [creator],
                |r| r.get(0),
            ))?;
            ensure!(
                count < self.limits.peer_documents as i64,
                Failure(Reason::DocumentQuota)
            );
        }
        let mut unique = Vec::new();
        let mut added = 0usize;
        for data in chunks {
            let key = hash(&data);
            let present: bool = local(self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM doc_chunks WHERE id=?1 AND hash=?2)",
                params![id, key.as_slice()],
                |r| r.get(0),
            ))?;
            if !present && !unique.iter().any(|(h, _)| h == &key) {
                added = added
                    .checked_add(data.len())
                    .ok_or(Failure(Reason::DocumentTooLarge))?;
                unique.push((key, data));
            }
        }
        let old:i64=local(self.db.query_row("SELECT coalesce((SELECT bytes FROM doc_meta WHERE id=?1),length((SELECT data FROM documents WHERE id=?1)),0)",[id],|r|r.get(0)))?;
        let compact = compact.filter(|_| old as usize + added > COMPACT_AT.min(document_cap / 2));
        let stored = compact.as_ref().map_or(old as usize + added, Vec::len);
        if stored > document_cap {
            return Err(Failure(Reason::DocumentTooLarge).into());
        }
        let other_orphans: i64 = local(self.db.query_row(
            "SELECT coalesce(sum(bytes),0) FROM doc_orphans WHERE sender=?1 AND id<>?2",
            params![sender, id],
            |r| r.get(0),
        ))?;
        ensure!(
            !peer || (other_orphans as usize).saturating_add(orphans) <= ORPHAN_CAP,
            Failure(Reason::PeerBadData)
        );
        self.reserve(
            added as u64 + compact.as_ref().map_or(0, |b| b.len() as u64),
            unique.len() as u64,
            peer,
        )?;
        let version = expected
            .checked_add(1)
            .ok_or(Failure(Reason::CorruptDocument))?;
        let encoded = postcard::to_allocvec(&heads)?;
        let tx = local(self.db.transaction())?;
        local(tx.execute("INSERT OR IGNORE INTO documents VALUES(?1,x'')", [id]))?;
        if let Some(data) = compact {
            local(tx.execute(
                "UPDATE documents SET data=?2 WHERE id=?1",
                params![id, data],
            ))?;
            local(tx.execute("DELETE FROM doc_chunks WHERE id=?1", [id]))?;
            local(tx.execute("INSERT INTO doc_baselines VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET version=excluded.version",params![id,version]))?;
        } else {
            for (key, data) in unique {
                local(tx.execute(
                    "INSERT INTO doc_chunks VALUES(?1,?2,?3,?4)",
                    params![id, version, key.as_slice(), data],
                ))?;
            }
        }
        if orphans == 0 {
            local(tx.execute("DELETE FROM doc_orphans WHERE id=?1", [id]))?;
        } else {
            local(tx.execute("INSERT INTO doc_orphans VALUES(?1,?2,?3) ON CONFLICT(id,sender) DO UPDATE SET bytes=excluded.bytes",params![id,sender,orphans as i64]))?;
        }
        local(tx.execute(
            "INSERT INTO doc_senders VALUES(?1,?2) ON CONFLICT(id) DO NOTHING",
            params![id, sender],
        ))?;
        local(tx.execute("INSERT INTO doc_meta VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET version=excluded.version,heads=excluded.heads,bytes=excluded.bytes,creator=CASE WHEN length(doc_meta.creator)=0 THEN excluded.creator ELSE doc_meta.creator END",params![id,version,encoded,stored as i64,creator]))?;
        local(tx.commit())?;
        Ok(version)
    }
    fn reserve(&self, added: u64, records: u64, peer: bool) -> Result<()> {
        let logical: i64 = local(self.db.query_row(
            "SELECT coalesce(sum(bytes+length(id)+512),0) FROM doc_meta",
            [],
            |r| r.get(0),
        ))?;
        let mut physical = 0u64;
        for suffix in ["", "-wal", "-shm"] {
            let path = PathBuf::from(format!("{}{suffix}", self.path.display()));
            let size = match std::fs::metadata(path) {
                Ok(meta) => meta.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(_) => return Err(Failure(Reason::LocalStore).into()),
            };
            physical = physical.saturating_add(size);
        }
        let reserve = added
            .saturating_mul(2)
            .saturating_add(records.saturating_mul(16 * 1024))
            .saturating_add(64 * 1024);
        let cap = if peer {
            self.limits.store_bytes * 9 / 10
        } else {
            self.limits.store_bytes
        };
        if (logical as u64).saturating_add(added) > cap || physical.saturating_add(reserve) > cap {
            return Err(Failure(Reason::StoreFull).into());
        }
        Ok(())
    }
    pub fn export(&self, id: &str, heads: &[ChangeHash]) -> Result<Vec<Vec<u8>>> {
        Ok(self
            .load(id)?
            .get_changes(heads)
            .iter()
            .map(|c| c.raw_bytes().to_vec())
            .collect())
    }
    /// Trusted local archive import; peer bytes must use the isolated sync worker.
    pub fn import(&mut self, id: &str, changes: Vec<Vec<u8>>) -> Result<()> {
        let changes = changes
            .into_iter()
            .map(Change::from_bytes)
            .collect::<Result<Vec<_>, _>>()?;
        self.edit(id, |doc| {
            doc.apply_changes(changes)?;
            Ok(())
        })
    }
    pub(crate) fn merge_membership(
        &self,
        candidate: &crate::identity::Group,
        local_key: [u8; 32],
    ) -> Result<crate::identity::Group> {
        use crate::identity::{Group, protect};
        use zeroize::Zeroizing;
        let root = candidate
            .enrollments
            .first()
            .context("missing genesis")?
            .member;
        candidate.validate(root)?;
        let slot = format!("group:{}", candidate.room());
        let exists = self.protected(&slot)?.is_some();
        let mut changed = !exists;
        let mut merged = if exists {
            let stored = Group::load(self, &candidate.room())?;
            ensure!(
                stored.id == candidate.id
                    && stored.same_key(candidate)
                    && stored.enrollments.first().is_some_and(|e| e.member == root),
                "group identity changed"
            );
            stored
        } else {
            candidate.clone()
        };
        for proof in &candidate.enrollments {
            if !merged.members().contains(&proof.member) {
                merged.enrollments.push(proof.clone());
                changed = true;
            }
        }
        merged.validate(local_key)?;
        if changed {
            let bytes =
                Zeroizing::new(serde_json::to_vec(&merged).map_err(|_| Failure(Reason::Protocol))?);
            self.put_protected(&slot, &protect(&bytes)?)?;
        }
        Ok(merged)
    }
    pub(crate) fn protected(&self, id: &str) -> Result<Option<Vec<u8>>> {
        local(
            self.db
                .query_row("SELECT data FROM protected WHERE id=?1", [id], |r| r.get(0))
                .optional(),
        )
    }
    pub(crate) fn put_protected(&self, id: &str, bytes: &[u8]) -> Result<()> {
        self.reserve(bytes.len() as u64, 1, false)?;
        local(self.db.execute(
            "INSERT INTO protected VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            params![id, bytes],
        ))?;
        Ok(())
    }
    pub(crate) fn insert_protected(&self, id: &str, bytes: &[u8]) -> Result<()> {
        self.reserve(bytes.len() as u64, 1, false)?;
        local(self.db.execute(
            "INSERT OR IGNORE INTO protected VALUES(?1,?2)",
            params![id, bytes],
        ))?;
        Ok(())
    }
}

#[cfg(test)]
mod round2_tests {
    use super::*;
    use crate::model;
    #[test]
    fn creator_quota_follows_author_across_relays_without_losing_sender_caps() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut store = Store::open_with_limits(
            &dir.path().join("creators.sqlite"),
            &[1; 32],
            Limits {
                peer_documents: 2,
                ..Limits::default()
            },
        )?;
        let commit = |author| Commit {
            creator: Some(author),
            chunks: Vec::new(),
            heads: Vec::new(),
            bytes: 0,
            compact: None,
            orphans: 0,
        };
        let author = [2; 32];
        let relay = [3; 32];
        store.append("tasks", 0, commit(author), &relay)?;
        store.append("memory", 0, commit(author), &[4; 32])?;
        let stored: Vec<u8> = local(store.db.query_row(
            "SELECT creator FROM doc_meta WHERE id='tasks'",
            [],
            |r| r.get(0),
        ))?;
        assert_eq!(stored, author);
        let error = store
            .append("projects", 0, commit(author), &[5; 32])
            .expect_err("author quota spans relays");
        assert_eq!(
            error.downcast_ref::<Failure>().map(|f| f.0),
            Some(Reason::DocumentQuota)
        );
        store.append("diary", 0, commit(relay), &relay)?;
        store.append("meeting:extra", 0, commit([6; 32]), &relay)?;
        for n in 0..29u8 {
            store.append(&format!("meeting:relay{n}"), 0, commit([7 + n; 32]), &relay)?;
        }
        let error = store
            .append("meeting:overflow", 0, commit([42; 32]), &relay)
            .expect_err("invented actors cannot bypass sender quota");
        assert_eq!(
            error.downcast_ref::<Failure>().map(|f| f.0),
            Some(Reason::DocumentQuota)
        );
        store.edit("meeting:local", |d| {
            model::create(d, "one", "owner still edits")
        })?;
        Ok(())
    }
    #[test]
    fn f5_local_edit_reuses_only_the_current_cached_version() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut store = Store::open(&dir.path().join("cache.sqlite"), b"owner")?;
        store.edit("tasks", |d| model::create(d, "one", "cached"))?;
        local(
            store
                .db
                .execute("UPDATE documents SET data=x'00' WHERE id='tasks'", []),
        )?;
        store.edit("tasks", |d| model::put(d, "one", "owner", "uses cache"))?;
        // A peer commit invalidates the version; the old cache must never hide corruption.
        local(
            store
                .db
                .execute("UPDATE doc_meta SET version=version+1 WHERE id='tasks'", []),
        )?;
        let error = store
            .edit("tasks", |_| Ok(()))
            .expect_err("must reload newer version");
        assert_eq!(
            error.downcast_ref::<Failure>().map(|f| f.0),
            Some(Reason::CorruptDocument)
        );
        Ok(())
    }
    #[test]
    fn f1_peer_reserve_orphans_and_compaction_preserve_owner_edits() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut store = Store::open_with_limits(
            &dir.path().join("quota.sqlite"),
            b"owner",
            Limits {
                document_bytes: 65536,
                store_bytes: 4 * 1024 * 1024,
                peer_documents: 1,
            },
        )?;
        let mut doc = Automerge::new();
        model::create(&mut doc, "one", "small")?;
        let commit = |bytes, orphans| Commit {
            creator: Some([2; 32]),
            chunks: vec![doc.save()],
            heads: doc.get_heads(),
            bytes,
            compact: Some(doc.save()),
            orphans,
        };
        let error = store
            .append("tasks", 0, commit(60000, 0), b"peer")
            .expect_err("peer uses owner's reserve");
        assert_eq!(
            error.downcast_ref::<Failure>().map(|f| f.0),
            Some(Reason::DocumentTooLarge)
        );
        let error = store
            .append("tasks", 0, commit(100, ORPHAN_CAP + 1), b"peer")
            .expect_err("orphan sender cap");
        assert_eq!(
            error.downcast_ref::<Failure>().map(|f| f.0),
            Some(Reason::PeerBadData)
        );
        store.append("tasks", 0, commit(100, 0), b"peer")?;
        store.edit("memory", |d| {
            model::create(d, "owner", "owner bypasses peer document quota")
        })?;
        for i in 0..250 {
            store.edit("tasks", |d| {
                model::put(d, "one", "owner", &format!("{i}{}", "x".repeat(30)))
            })?;
        }
        let snapshot = store.snapshot("tasks", Some(0))?;
        assert!(snapshot.reset);
        assert_eq!(store.load("tasks")?.get_heads(), snapshot.heads);
        let chunks: i64 = local(store.db.query_row(
            "SELECT count(*) FROM doc_chunks WHERE id='tasks'",
            [],
            |r| r.get(0),
        ))?;
        assert!(chunks < 250);
        Ok(())
    }
    #[test]
    fn f1_sqlite_full_is_store_full() {
        let error = local::<()>(Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            None,
        )))
        .expect_err("disk full");
        assert_eq!(
            error.downcast_ref::<Failure>().map(|f| f.0),
            Some(Reason::StoreFull)
        );
    }
}
