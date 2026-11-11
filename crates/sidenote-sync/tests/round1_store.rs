use anyhow::Result;
use sidenote_sync::{
    client::Device,
    identity::Group,
    model,
    store::{Limits, MAX_PEER_DOCUMENTS, MAX_STORE_BYTES, Store},
};
use std::sync::Arc;

#[test]
fn c4_inventory_pages_exceed_1024_and_writes_are_incremental() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("pages.sqlite");
    let mut store = Store::open(&path, b"pages")?;
    for index in 0..1025 {
        store.edit(&format!("meeting:m{index:04}"), |d| {
            model::create(d, "one", "small")
        })?;
    }
    let mut cursor = None;
    let mut ids = Vec::new();
    loop {
        let page = store.inventory_page(cursor.as_deref())?;
        assert!(page.len() <= 128);
        if page.is_empty() {
            break;
        }
        cursor = page.last().cloned();
        ids.extend(page);
    }
    assert_eq!(ids.len(), 1025);
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    store.edit("tasks", |d| model::create(d, "one", "unchanged snapshot"))?;
    let db = rusqlite::Connection::open(&path)?;
    let original: Vec<u8> =
        db.query_row("SELECT data FROM documents WHERE id='tasks'", [], |r| {
            r.get(0)
        })?;
    store.edit("tasks", |d| model::put(d, "one", "owner", "new change"))?;
    let current: Vec<u8> =
        db.query_row("SELECT data FROM documents WHERE id='tasks'", [], |r| {
            r.get(0)
        })?;
    assert_eq!(original, current);
    let count: i64 = db.query_row(
        "SELECT count(*) FROM doc_chunks WHERE id='tasks'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(count, 2);
    Ok(())
}
#[test]
fn c3_c4_commit_limits_leave_prior_heads_intact() -> Result<()> {
    assert_eq!(MAX_PEER_DOCUMENTS, 50_000);
    assert_eq!(MAX_STORE_BYTES, 2 * 1024 * 1024 * 1024);
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("limits.sqlite");
    let mut store = Store::open_with_limits(
        &path,
        b"limits",
        Limits {
            document_bytes: 2048,
            store_bytes: 1024 * 1024,
            peer_documents: 2,
        },
    )?;
    store.edit("tasks", |d| model::create(d, "one", "retained"))?;
    let heads = store.heads("tasks")?;
    assert!(
        store
            .edit("tasks", |d| model::put(
                d,
                "one",
                "owner",
                &"x".repeat(4096)
            ))
            .is_err()
    );
    assert_eq!(store.heads("tasks")?, heads);
    store.edit("memory", |d| model::create(d, "two", "second"))?;
    store.edit("projects", |d| model::create(d, "three", "third"))?;
    assert_eq!(store.inventory()?.len(), 3);
    Ok(())
}
#[test]
fn c4_total_store_budget_refuses_writes_with_a_reason() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut store = Store::open_with_limits(
        &dir.path().join("full.sqlite"),
        b"small",
        Limits {
            store_bytes: 64 * 1024,
            ..Limits::default()
        },
    )?;
    let error = store
        .edit("tasks", |d| model::create(d, "one", "refused"))
        .expect_err("physical reserve exceeds budget");
    assert_eq!(
        error
            .downcast_ref::<sidenote_sync::status::Failure>()
            .map(|f| f.0),
        Some(sidenote_sync::status::Reason::StoreFull)
    );
    assert!(store.inventory()?.is_empty());
    Ok(())
}
#[test]
fn c7_stale_startup_and_concurrent_pairings_merge_monotonically() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let device = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
    let b = Device::open(&dir.path().join("b.sqlite"))?;
    let c = Device::open(&dir.path().join("c.sqlite"))?;
    let original = Group::new(&device.identity);
    device.merge_group(&original)?;
    let mut left = original.clone();
    left.enroll(&device.identity, b.identity.public());
    let mut right = original.clone();
    right.enroll(&device.identity, c.identity.public());
    let a = device.clone();
    let task = std::thread::spawn(move || a.merge_group(&left));
    device.merge_group(&right)?;
    task.join().expect("merge thread")?;
    let merged = device.merge_group(&original)?;
    assert_eq!(merged.members().len(), 3);
    assert!(merged.members().contains(&b.identity.public()));
    assert!(merged.members().contains(&c.identity.public()));
    // An unchanged/stale bundle must not rewrite the protected row.
    let db = rusqlite::Connection::open(dir.path().join("owner.sqlite"))?;
    db.execute_batch("CREATE TRIGGER forbid_unchanged_roster BEFORE UPDATE ON protected WHEN new.id LIKE 'group:%' BEGIN SELECT RAISE(ABORT,'unexpected roster rewrite'); END;")?;
    assert_eq!(device.merge_group(&original)?.members().len(), 3);
    drop(device);
    let reopened = Device::open(&dir.path().join("owner.sqlite"))?;
    assert_eq!(
        reopened
            .store(|s| Group::load(s, &original.room()))?
            .members()
            .len(),
        3
    );
    Ok(())
}
