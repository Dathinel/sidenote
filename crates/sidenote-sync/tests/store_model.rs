use anyhow::Result;
use automerge::{ActorId, Automerge};
use sidenote_sync::{
    model,
    store::{Store, valid_domain},
};

fn fork(doc: &Automerge) -> Automerge {
    let mut other = doc.fork();
    other.set_actor(ActorId::from(b"B".as_slice()));
    other
}

#[test]
fn merge_policies_and_recovery() -> Result<()> {
    let mut a = Automerge::new();
    model::create(&mut a, "task-1", "Task")?;
    model::check(&mut a, "task-1")?;
    let mut b = fork(&a);
    model::put(&mut a, "task-1", "owner", "Alice")?;
    model::put(&mut b, "task-1", "due", "2030-01-02")?;
    a.merge(&mut b)?;
    b.merge(&mut a)?;
    assert_eq!(model::values(&a, "task-1", "owner")?, ["Alice"]);
    assert_eq!(model::values(&b, "task-1", "due")?, ["2030-01-02"]);
    model::put(&mut a, "task-1", "owner", "concurrent-A")?;
    model::put(&mut b, "task-1", "owner", "concurrent-B")?;
    model::check(&mut a, "task-1")?;
    model::uncheck(&mut b, "task-1")?;
    model::splice(&mut a, "task-1", 2, 0, "alpha")?;
    model::splice(&mut b, "task-1", 2, 0, "beta")?;
    a.merge(&mut b)?;
    b.merge(&mut a)?;
    assert_eq!(a.get_heads(), b.get_heads());
    assert_eq!(model::values(&a, "task-1", "owner")?.len(), 2);
    assert!(model::checked(&a, "task-1")?);
    assert!(model::text(&a, "task-1")?.contains("alpha"));
    assert!(model::text(&a, "task-1")?.contains("beta"));
    model::uncheck(&mut a, "task-1")?;
    assert!(!model::checked(&a, "task-1")?);
    model::resolve(&mut a, "task-1", "owner", "concurrent-A")?;
    assert_eq!(model::values(&a, "task-1", "owner")?, ["concurrent-A"]);
    let recovery = model::recovery(&a, "task-1")?;
    assert_eq!(recovery.len(), 1);
    assert!(recovery[0].values.contains(&"concurrent-B".to_owned()));
    b.merge(&mut a)?;
    model::delete(&mut a, "task-1")?;
    model::put(&mut b, "task-1", "owner", "recoverable concurrent edit")?;
    a.merge(&mut b)?;
    assert!(model::deleted(&a, "task-1")?);
    assert_eq!(
        model::values(&a, "task-1", "owner")?,
        ["recoverable concurrent edit"]
    );
    model::restore(&mut a, "task-1")?;
    assert!(!model::deleted(&a, "task-1")?);
    Ok(())
}

#[test]
fn writer_lock_stale_save_and_disk_write_failure() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("durable.sqlite");
    let mut store = Store::open(&path, b"A")?;
    assert!(Store::open(&path, b"A").is_err());
    store.edit("tasks", |d| model::create(d, "one", "before"))?;
    let stale = store.load("tasks")?;
    store.edit("tasks", |d| model::put(d, "one", "owner", "retained"))?;
    assert!(store.save("tasks", &stale).is_err());
    let heads = store.heads("tasks")?;
    let conn = rusqlite::Connection::open(&path)?;
    conn.execute_batch("CREATE TRIGGER fail_write BEFORE INSERT ON doc_chunks BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;")?;
    assert!(
        store
            .edit("tasks", |d| model::put(d, "one", "owner", "not-durable"))
            .is_err()
    );
    assert_eq!(store.heads("tasks")?, heads);
    conn.execute_batch("DROP TRIGGER fail_write")?;
    store.edit("tasks", |d| model::put(d, "one", "owner", "retry"))?;
    assert_eq!(
        model::values(&store.load("tasks")?, "one", "owner")?,
        ["retry"]
    );
    drop(store);
    assert!(Store::open(&path, b"A").is_ok());
    Ok(())
}

#[test]
fn durable_domains_export_import_and_failed_transaction() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("a.sqlite");
    let domains = ["tasks", "memory", "projects", "diary", "meeting:m-1"];
    let mut a = Store::open(&path, b"A")?;
    for domain in domains {
        a.edit(domain, |d| model::create(d, "one", "retained"))?;
    }
    let before = a.heads("tasks")?;
    assert!(
        a.edit("tasks", |d| -> Result<()> {
            model::put(d, "one", "owner", "must not commit")?;
            anyhow::bail!("synthetic failure")
        })
        .is_err()
    );
    assert_eq!(before, a.heads("tasks")?);
    drop(a);
    let a = Store::open(&path, b"A")?;
    let mut b = Store::open(&dir.path().join("b.sqlite"), b"B")?;
    for domain in domains {
        b.import(domain, a.export(domain, &[])?)?;
        assert_eq!(a.heads(domain)?, b.heads(domain)?);
        b.import(domain, a.export(domain, &[])?)?;
        assert_eq!(model::text(&b.load(domain)?, "one")?, "retained");
    }
    assert_eq!(a.inventory()?.len(), 5);
    for bad in ["audio", "../tasks", "meeting:", "meeting:../secret"] {
        assert!(!valid_domain(bad));
    }
    Ok(())
}
