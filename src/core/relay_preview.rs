//! Opt-in manual-task collection. Folder journals never receive its projections.
use super::{ManualTask, store_io};
use automerge::{Automerge, ROOT, ReadDoc, transaction::Transactable};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sidenote_sync::{
    client::{Device, SyncClient},
    identity::Group,
    transport::RelayUrl,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    thread,
};
mod migration;
pub mod operations;
pub mod pairing;
pub(crate) mod shared;
#[cfg(any(debug_assertions, feature = "relay-qualification"))]
mod test_driver;
mod versions;
pub use versions::{Choice, Conflict, Resolution, current as conflicts};

#[derive(Clone)]
pub struct SharedProfile {
    pub your_name: String,
    pub people_sides: BTreeMap<String, super::people::Side>,
    pub relay: bool,
}
pub fn profile(root: &Path) -> io::Result<SharedProfile> {
    let mut name = shared::local_profile(root)?;
    let mut people = shared::local_people(root)?;
    let preview = get(root)?;
    if let Some(p) = &preview {
        name = p.shared_value("profile.json", name)?;
        people = p.shared_value("people.json", people)?;
    }
    Ok(SharedProfile {
        your_name: name["your_name"].as_str().unwrap_or("").into(),
        people_sides: serde_json::from_value(people["sides"].clone())?,
        relay: preview.is_some(),
    })
}
/// A document revision, not a delivery acknowledgement. No keys or content leave this API.
pub fn revision(root: &Path) -> io::Result<Option<String>> {
    let Some(preview) = get(root)? else {
        return Ok(None);
    };
    let heads = preview
        .device
        .store(|s| {
            s.inventory()?
                .into_iter()
                .map(|d| Ok((d.clone(), s.heads(&d)?)))
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .map_err(failed)?;
    Ok(Some(
        blake3::hash(&serde_json::to_vec(&heads)?)
            .to_hex()
            .to_string(),
    ))
}

/// Builds a trusted synthetic conflict for the offscreen renderer, never a live data root.
#[doc(hidden)]
pub fn seed_render_conflict(root: &Path) -> io::Result<()> {
    let resolved = root.canonicalize()?;
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()?
        .join("target");
    if !resolved.starts_with(target) && !super::is_proof_path(root, "sidenote-relay-preview") {
        return Err(store_io::invalid(
            "Conflict render requires an isolated proof folder",
        ));
    }
    let mut device = super::DeviceSettings::load_in(root)?;
    device.name = "DEMO-PC".into();
    store_io::write(&root.join("device.json"), &device)?;
    let mut settings = Settings::load(root)?;
    settings.enabled = true;
    settings.address.clear();
    settings.save(root)?;
    let preview = get(root)?.ok_or_else(|| failed("preview off"))?;
    preview
        .device
        .store(|s| {
            s.edit("tasks", |doc| {
                let initial = ManualTask {
                    id: "synthetic-relay-choice".into(),
                    task: "Review the field guide".into(),
                    owner: Some("Initial owner".into()),
                    ..Default::default()
                };
                versions::apply(doc, &[], std::slice::from_ref(&initial), "DEMO-PC", "edit")?;
                let mut other = doc.fork();
                other.set_actor(automerge::ActorId::from(&[77; 32][..]));
                let mut mine = initial.clone();
                mine.owner = Some("Taylor".into());
                let mut theirs = initial.clone();
                theirs.owner = Some("Morgan".into());
                versions::apply(
                    doc,
                    std::slice::from_ref(&initial),
                    &[mine],
                    "DEMO-PC",
                    "edit",
                )?;
                versions::apply(&mut other, &[initial], &[theirs], "Travel laptop", "edit")?;
                doc.merge(&mut other)?;
                Ok(())
            })
        })
        .map_err(failed)
}

#[doc(hidden)]
pub fn seed_shared_render_conflict(root: &Path) -> io::Result<()> {
    let resolved = root.canonicalize()?;
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()?
        .join("target");
    if !resolved.starts_with(target) && !super::is_proof_path(root, "sidenote-relay-preview") {
        return Err(store_io::invalid(
            "Shared render requires an isolated proof folder",
        ));
    }
    let mut device = super::DeviceSettings::load_in(root)?;
    device.name = "DEMO-PC".into();
    store_io::write(&root.join("device.json"), &device)?;
    let mut settings = Settings::load(root)?;
    settings.enabled = true;
    settings.address.clear();
    settings.save(root)?;
    let preview = get(root)?.ok_or_else(|| failed("preview off"))?;
    preview.device.store(|s|s.edit("memory",|doc| {
        let base=serde_json::json!({"schema_version":1,"records":[{"id":"synthetic-render-memory","type":"story","text":"Initial field guide","project":null,"people":[],"sources":[],"first_seen":null,"last_confirmed":null,"status":"active","basis":"owner_confirmed","supersedes":null,"updated_at":"2030-01-01T00:00:00Z","source_fingerprint":"synthetic","source_fingerprints":[]}],"proposals":[],"events":[],"migration":null});
        shared::apply(doc,"memory/records.json",&Value::Null,&base,"DEMO-PC","render",true)?;
        let mut other=doc.fork();other.set_actor(automerge::ActorId::from(&[78;32][..]));
        let mut a=base.clone();a["records"][0]["text"]=Value::String("Ship the revised field guide".into());
        let mut b=base.clone();b["records"][0]["text"]=Value::String("Review the field guide with the team".into());
        shared::apply(doc,"memory/records.json",&base,&a,"DEMO-PC","render-a",false)?;
        shared::apply(&mut other,"memory/records.json",&base,&b,"Travel laptop","render-b",false)?;
        doc.merge(&mut other)?;Ok(())
    })).map_err(failed)
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub address: String,
    pub room: String,
    pub backup: String,
    pub shared_backup: String,
    pub peers: BTreeMap<String, String>,
}

impl Settings {
    pub fn load(root: &Path) -> io::Result<Self> {
        match store_io::read(&root.join("local/relay/settings.json")) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            result => result,
        }
    }
    pub fn save(&self, root: &Path) -> io::Result<()> {
        if !self.address.is_empty() {
            endpoint(&self.address)?;
        }
        let _mode = store_io::lock(&root.join("local/relay/mode.lock"))?;
        let prior = Self::load(root)?;
        let mut next = self.clone();
        if !prior.room.is_empty() {
            next.room = prior.room.clone();
            next.peers = prior.peers.clone();
            next.backup = prior.backup.clone();
            next.shared_backup = prior.shared_backup.clone();
        }
        if next.enabled && (!prior.enabled || prior.backup.is_empty()) {
            ensure_replica_excluded(root)?;
            migration::enable(root, &mut next)?;
        } else if !next.enabled && prior.enabled {
            migration::verify(root, &prior.backup)?;
        }
        if next.enabled && (!prior.enabled || prior.shared_backup.is_empty()) {
            migration::enable_shared(root, &mut next)?;
        } else if !next.enabled && prior.enabled {
            migration::verify_shared(root, &prior.shared_backup)?;
        }
        store_io::write(&root.join("local/relay/settings.json"), &next)?;
        if !self.enabled {
            disconnect(root)?;
        }
        Ok(())
    }
    fn save_authority(&self, root: &Path, device: &Device) -> io::Result<()> {
        let _mode = store_io::lock(&root.join("local/relay/mode.lock"))?;
        let mut current = Self::load(root)?;
        if !current.enabled {
            return Err(store_io::invalid("Relay preview was turned off"));
        }
        let previous_room = current.room.clone();
        current.room = self.room.clone();
        current.peers = self.peers.clone();
        // Startup retires inactive groups if a crash interrupts this durable handoff.
        store_io::write(&root.join("local/relay/settings.json"), &current)?;
        if previous_room != current.room {
            device
                .store(|s| s.retire_group(&previous_room))
                .map_err(failed)?;
        }
        Ok(())
    }
}

fn ensure_replica_excluded(root: &Path) -> io::Result<()> {
    let path = root.join(".stignore");
    let _guard = store_io::lock(&root.join("local/relay/ignore.lock"))?;
    let prior = match store_io::read_bounded(&path, super::file_reads::JOURNAL_MAX) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let content = prior.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&prior);
    if content.starts_with(b"/local\n") || content.starts_with(b"/local\r\n") {
        return Ok(());
    }
    let mut next = prior[..prior.len() - content.len()].to_vec();
    next.extend_from_slice(b"/local\n");
    next.extend_from_slice(content);
    store_io::atomic_write(&path, &next)
}

pub fn endpoint(address: &str) -> io::Result<RelayUrl> {
    let result = if address.starts_with("ws:") {
        RelayUrl::local(address)
    } else {
        RelayUrl::new(address)
    };
    result.map_err(|_| store_io::invalid("Use a WSS origin on port 443 or ws://127.0.0.1:port"))
}

#[cfg(not(any(debug_assertions, feature = "relay-qualification")))]
fn qualification_point(_root: &Path, _point: &str) -> io::Result<()> {
    Ok(())
}

#[cfg(any(debug_assertions, feature = "relay-qualification"))]
fn qualification_point(root: &Path, point: &str) -> io::Result<()> {
    if !super::is_proof_path(root, "sidenote-relay-preview")
        || !std::env::args_os().any(|a| a == "--relay-preview-headless")
    {
        return Ok(());
    }
    let request: Value = store_io::read(&root.join("local/relay/qualification-control.json"))?;
    if request["pause"] == point {
        store_io::write(
            &root.join("local/relay/qualification-state.json"),
            &serde_json::json!({"stage":point}),
        )?;
        loop {
            std::thread::park();
        }
    }
    Ok(())
}

fn failed(_: impl std::fmt::Display) -> io::Error {
    io::Error::other("Relay preview could not read or save its local replica")
}

struct Connection {
    client: Arc<SyncClient>,
    stop: tokio::sync::oneshot::Sender<()>,
    thread: thread::JoinHandle<()>,
}

pub struct Preview {
    device: Arc<Device>,
    root: PathBuf,
    connection: Mutex<Option<Connection>>,
}

impl Preview {
    fn open(root: &Path, settings: Settings) -> io::Result<Self> {
        let mut settings = settings;
        if settings.backup.is_empty() {
            migration::enable(root, &mut settings)?;
        }
        if settings.shared_backup.is_empty() {
            migration::enable_shared(root, &mut settings)?;
        }
        std::fs::create_dir_all(root.join("local/relay"))?;
        let device = open_device(root, &mut settings)?;
        store_io::write(&root.join("local/relay/settings.json"), &settings)?;
        Ok(Self {
            device,
            root: root.into(),
            connection: Mutex::new(None),
        })
    }

    pub fn online(&self, online: bool) -> io::Result<()> {
        let mut connection = self.connection.lock().map_err(failed)?;
        if let Some(old) = connection.take() {
            let _ = old.stop.send(());
            old.thread.join().map_err(|_| failed("session stopped"))?;
        }
        if !online {
            return Ok(());
        }
        let device = self.device.clone();
        let settings = Settings::load(&self.root)?;
        let group = device
            .store(|s| Group::load(s, &settings.room))
            .map_err(failed)?;
        if settings.address.is_empty() || group.members().len() < 2 {
            return Ok(());
        }
        if device.store(|s| s.check_compatible()).is_err() {
            return Ok(());
        }
        let relay = endpoint(&settings.address)?;
        let (ready, receiver) = mpsc::sync_channel(1);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let thread = thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(_) => {
                    let _ = ready.send(Err(failed("runtime")));
                    return;
                }
            };
            runtime.block_on(async move {
                match SyncClient::start(device, group, relay) {
                    Ok(running) => {
                        if ready.send(Ok(running.client.clone())).is_ok() {
                            let _ = stopped.await;
                        }
                        let _ = running.stop().await;
                    }
                    Err(_) => {
                        let _ = ready.send(Err(failed("worker")));
                    }
                }
            });
        });
        match receiver.recv().map_err(failed)? {
            Ok(client) => {
                *connection = Some(Connection {
                    client,
                    stop,
                    thread,
                });
                Ok(())
            }
            Err(error) => {
                let _ = thread.join();
                Err(error)
            }
        }
    }

    pub fn tasks(&self) -> io::Result<Vec<ManualTask>> {
        project(&self.device.load("tasks").map_err(failed)?).map_err(failed)
    }

    pub fn edit(&self, before: &[ManualTask], after: &[ManualTask]) -> io::Result<()> {
        let name = super::DeviceSettings::read_in(&self.root)?.name;
        let connection = self.connection.lock().map_err(failed)?;
        if let Some(connection) = connection.as_ref() {
            connection
                .client
                .edit("tasks", |doc| {
                    versions::apply(doc, before, after, &name, "edit")
                })
                .map_err(failed)
        } else {
            self.device
                .store(|store| {
                    store.edit("tasks", |doc| {
                        versions::apply(doc, before, after, &name, "edit")
                    })
                })
                .map_err(failed)
        }
    }
}

fn open_device(root: &Path, settings: &mut Settings) -> io::Result<Arc<Device>> {
    ensure_replica_excluded(root)?;
    store_io::reject_reparse(&root.join("local/relay/replica.sqlite"))?;
    let device = Arc::new(Device::open(&root.join("local/relay/replica.sqlite")).map_err(failed)?);
    if settings.room.is_empty() {
        let group = Group::new(&device.identity);
        device.merge_group(&group).map_err(failed)?;
        settings.room = group.room();
    } else {
        device
            .store(|s| Group::load(s, &settings.room))
            .map_err(failed)?;
    }
    device
        .store(|s| s.retire_inactive_groups(&settings.room))
        .map_err(failed)?;
    device
        .store(|s| s.configure_capabilities(&settings.room, operations::supported(root)))
        .map_err(failed)?;
    Ok(device)
}

impl Drop for Preview {
    fn drop(&mut self) {
        let _ = self.online(false);
    }
}

type Registry = Mutex<BTreeMap<PathBuf, Arc<Preview>>>;
static REPLICAS: OnceLock<Registry> = OnceLock::new();
fn registry() -> &'static Registry {
    REPLICAS.get_or_init(Mutex::default)
}

pub fn get(root: &Path) -> io::Result<Option<Arc<Preview>>> {
    let settings = Settings::load(root)?;
    if !settings.enabled {
        return Ok(None);
    }
    let mut replicas = registry().lock().map_err(failed)?;
    if let Some(preview) = replicas.get(root) {
        return Ok(Some(preview.clone()));
    }
    let preview = Arc::new(Preview::open(root, settings)?);
    preview.online(true)?;
    replicas.insert(root.to_owned(), preview.clone());
    Ok(Some(preview))
}

pub fn disconnect(root: &Path) -> io::Result<()> {
    if let Some(preview) = registry().lock().map_err(failed)?.remove(root) {
        preview.online(false)?;
    }
    Ok(())
}

fn fields(tasks: &[ManualTask]) -> anyhow::Result<BTreeMap<String, Map<String, Value>>> {
    let mut result = BTreeMap::new();
    for task in tasks {
        anyhow::ensure!(
            !task.id.is_empty() && !task.id.starts_with('$') && task.id.len() <= 256,
            "invalid task ID"
        );
        let Value::Object(fields) = serde_json::to_value(task)? else {
            anyhow::bail!("invalid task");
        };
        anyhow::ensure!(
            result.insert(task.id.clone(), fields).is_none(),
            "duplicate task"
        );
    }
    Ok(result)
}

#[cfg(test)]
fn apply(doc: &mut Automerge, before: &[ManualTask], after: &[ManualTask]) -> anyhow::Result<()> {
    versions::apply(doc, before, after, "Synthetic device", "edit")
}
fn project(doc: &Automerge) -> anyhow::Result<Vec<ManualTask>> {
    let mut rows: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut present = BTreeSet::new();
    for key in doc.keys(ROOT) {
        let [id, field]: [String; 2] = serde_json::from_str(&key)?;
        if id.starts_with('$') {
            continue;
        }
        let (value, _) = doc
            .get(ROOT, &key)?
            .ok_or_else(|| anyhow::anyhow!("missing field"))?;
        let value = versions::decode(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid field"))?,
        )?
        .value;
        if field == "$present" {
            if versions::present(doc, &key)? {
                present.insert(id);
            }
        } else if !value.is_null() {
            rows.entry(id).or_default().insert(field, value);
        }
    }
    present
        .into_iter()
        .map(|id| {
            let mut row = rows
                .remove(&id)
                .ok_or_else(|| anyhow::anyhow!("missing task"))?;
            shared::manual_fields(doc, &id, &mut row)?;
            let task: ManualTask = serde_json::from_value(Value::Object(row))?;
            anyhow::ensure!(task.id == id, "task ID mismatch");
            Ok(task)
        })
        .collect()
}

/// Synthetic two-process test driver. Runs before singleton, updates or window creation.
#[doc(hidden)]
#[cfg(any(debug_assertions, feature = "relay-qualification"))]
pub fn headless(args: &[std::ffi::OsString]) -> io::Result<()> {
    use std::io::{BufRead, Write};
    let mut roots = Vec::new();
    for flag in ["--data-dir", "--meetings-dir", "--peers-dir"] {
        let path = args
            .windows(2)
            .find(|p| p[0] == flag)
            .map(|p| PathBuf::from(&p[1]))
            .ok_or_else(|| store_io::invalid("Explicit isolated paths required"))?;
        if !super::is_proof_path(&path, "sidenote-relay-preview") {
            return Err(store_io::invalid(
                "Synthetic relay paths must be in the test temp directory",
            ));
        }
        std::fs::create_dir_all(&path)?;
        roots.push(path);
    }
    let root = &roots[0];
    let store = super::TaskStore { root: root.clone() };
    let _ = get(root)?.ok_or_else(|| store_io::invalid("Preview is disabled"))?;
    let mut driver = test_driver::Driver::new(root);
    let mut output = io::stdout().lock();
    writeln!(output, "{{\"ready\":true}}")?;
    output.flush()?;
    for line in io::stdin().lock().lines() {
        let command: Value = serde_json::from_str(&line?)?;
        let result: io::Result<Value> = (|| match command["op"].as_str().unwrap_or_default() {
            "list" => Ok(serde_json::to_value(store.load_overrides()?.manual)?),
            "add" => {
                let task: ManualTask = serde_json::from_value(command["task"].clone())?;
                let (_, id) = store.add_manual(task)?;
                Ok(Value::String(id))
            }
            "edit" => {
                let id = command["id"]
                    .as_str()
                    .ok_or_else(|| store_io::invalid("Missing task ID"))?;
                let current = store
                    .load_overrides()?
                    .manual
                    .into_iter()
                    .find(|t| t.id == id)
                    .ok_or_else(|| store_io::invalid("Missing task"))?;
                let base = serde_json::to_value(current)?;
                let mut value = base.clone();
                let patch = command["fields"]
                    .as_object()
                    .ok_or_else(|| store_io::invalid("Missing fields"))?;
                for (name, field) in patch {
                    if name == "id" {
                        return Err(store_io::invalid("Task IDs are immutable"));
                    }
                    value[name] = field.clone();
                }
                store.merge_task(&format!("manual/{id}"), 0, &base, &value, None)?;
                Ok(Value::Null)
            }
            "delete" => {
                store.delete_manual(
                    command["id"]
                        .as_str()
                        .ok_or_else(|| store_io::invalid("Missing task ID"))?,
                )?;
                Ok(Value::Null)
            }
            "offline" => {
                get(root)?
                    .ok_or_else(|| failed("preview off"))?
                    .online(false)?;
                Ok(Value::Null)
            }
            "online" => {
                get(root)?
                    .ok_or_else(|| failed("preview off"))?
                    .online(true)?;
                Ok(Value::Null)
            }
            "quit" => {
                get(root)?
                    .ok_or_else(|| failed("preview off"))?
                    .online(false)?;
                Ok(Value::Bool(true))
            }
            _ => driver.process(&command),
        })();
        let response = match result {
            Ok(value) => serde_json::json!({"ok":true,"value":value}),
            Err(_) => serde_json::json!({"ok":false}),
        };
        serde_json::to_writer(&mut output, &response)?;
        writeln!(output)?;
        output.flush()?;
        if command["op"] == "quit" {
            break;
        }
    }
    disconnect(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_endpoint_boundaries() {
        assert!(!Settings::default().enabled);
        for address in [
            "ws://localhost:8787",
            "ws://192.0.2.1",
            "wss://u:p@example.test",
            "wss://example.test/path",
            "wss://example.test?q=x",
            "wss://example.test#x",
            "wss://example.test:8443",
        ] {
            assert!(endpoint(address).is_err());
        }
        assert!(endpoint("ws://127.0.0.1:8787").is_ok());
        assert!(endpoint("wss://relay.example.test").is_ok());
    }
    #[test]
    fn independent_fields_and_delete_survive_without_a_shared_map_bootstrap() -> anyhow::Result<()>
    {
        let task = ManualTask {
            id: "synthetic".into(),
            task: "Synthetic task".into(),
            ..Default::default()
        };
        let mut a = Automerge::new();
        apply(&mut a, &[], std::slice::from_ref(&task))?;
        let mut b = a.fork();
        let mut mine = task.clone();
        mine.owner = Some("Synthetic owner".into());
        let mut theirs = task.clone();
        theirs.due = Some("2030-01-02".into());
        apply(&mut a, std::slice::from_ref(&task), &[mine])?;
        apply(&mut b, &[task], &[theirs])?;
        a.merge(&mut b)?;
        let tasks = project(&a)?;
        assert_eq!(tasks[0].owner.as_deref(), Some("Synthetic owner"));
        assert_eq!(tasks[0].due.as_deref(), Some("2030-01-02"));
        apply(&mut a, &tasks, &[])?;
        assert!(project(&a)?.is_empty());
        Ok(())
    }
    #[test]
    fn concurrent_first_creates_preserve_both_tasks() -> anyhow::Result<()> {
        let mut a = Automerge::new();
        let mut b = Automerge::new();
        for (doc, id) in [(&mut a, "synthetic-a"), (&mut b, "synthetic-b")] {
            apply(
                doc,
                &[],
                &[ManualTask {
                    id: id.into(),
                    task: "Synthetic task".into(),
                    ..Default::default()
                }],
            )?;
        }
        a.merge(&mut b)?;
        b.merge(&mut a)?;
        assert_eq!(project(&a)?.len(), 2);
        assert_eq!(a.get_heads(), b.get_heads());
        Ok(())
    }
    #[test]
    fn manual_and_meeting_edits_commit_together_without_folder_writes() -> anyhow::Result<()> {
        let fixture = tempfile::tempdir()?;
        let root = fixture.path();
        std::fs::create_dir_all(root.join("local/relay"))?;
        let device = Device::open(&root.join("local/relay/replica.sqlite"))?;
        let group = Group::new(&device.identity);
        device.merge_group(&group)?;
        drop(device);
        Settings {
            enabled: true,
            address: "ws://127.0.0.1:9".into(),
            room: group.room(),
            ..Default::default()
        }
        .save(root)?;
        let store = super::super::TaskStore { root: root.into() };
        let result = store.update_overrides(|overrides| {
            overrides.manual.push(ManualTask {
                id: "synthetic".into(),
                task: "Synthetic preview".into(),
                ..Default::default()
            });
            overrides.tasks.entry("100-1".into()).or_default().insert(
                "0".into(),
                super::super::TaskOverride {
                    title: Some("Synthetic folder edit".into()),
                    ..Default::default()
                },
            );
        });
        assert!(result.is_ok());
        let preview = get(root)?.unwrap();
        assert_eq!(preview.tasks()?.len(), 1);
        assert_eq!(
            store.load_overrides()?.tasks["100-1"]["0"].title.as_deref(),
            Some("Synthetic folder edit")
        );
        let heads = preview.device.load("tasks")?.get_heads();
        let invalid = store.update_overrides(|o| {
            o.manual.push(o.manual[0].clone());
            o.tasks
                .get_mut("100-1")
                .unwrap()
                .values_mut()
                .next()
                .unwrap()
                .title = Some("Must not commit".into());
        });
        assert!(invalid.is_err());
        assert_eq!(preview.device.load("tasks")?.get_heads(), heads);
        disconnect(root)?;
        drop(preview);
        Settings::default().save(root)?;
        let overrides = store.load_overrides()?;
        assert!(overrides.manual.is_empty() && overrides.tasks.is_empty());
        Ok(())
    }
    #[test]
    fn legacy_identity_backup_drift_and_resume_keep_both_collections() -> anyhow::Result<()> {
        let fixture = tempfile::tempdir()?;
        let root = fixture.path();
        store_io::write(
            &root.join("device.json"),
            &super::super::DeviceSettings {
                name: "Synthetic PC".into(),
                id: "c".repeat(32),
                daily_check: false,
            },
        )?;
        let task = ManualTask {
            id: "l".repeat(137),
            task: "Synthetic legacy task".into(),
            ..Default::default()
        };
        store_io::write(
            &root.join("task-overrides.json"),
            &super::super::Overrides {
                manual: vec![task.clone()],
                ..Default::default()
            },
        )?;
        let original = super::super::sync_state::manual_journal_files(root)?;
        Settings {
            enabled: true,
            ..Default::default()
        }
        .save(root)?;
        let preview = get(root)?.unwrap();
        let mut changed = task.clone();
        changed.owner = Some("Relay owner".into());
        preview.edit(std::slice::from_ref(&task), &[changed])?;
        assert_eq!(preview.tasks()?.len(), 1);
        drop(preview);
        let mut drift = original["task-overrides.json"].clone();
        drift.push(b'\n');
        std::fs::write(root.join("task-overrides.json"), drift)?;
        let mut settings = Settings::load(root)?;
        settings.enabled = false;
        assert!(settings.save(root).is_err());
        assert!(Settings::load(root)?.enabled);
        std::fs::write(
            root.join("task-overrides.json"),
            &original["task-overrides.json"],
        )?;
        settings.save(root)?;
        assert_eq!(
            super::super::sync_state::manual_journal_files(root)?,
            original
        );
        let folder = super::super::TaskStore { root: root.into() };
        assert!(folder.load_overrides()?.manual[0].owner.is_none());
        settings.enabled = true;
        settings.save(root)?;
        let preview = get(root)?.unwrap();
        assert_eq!(preview.tasks()?[0].owner.as_deref(), Some("Relay owner"));
        disconnect(root)?;
        drop(preview);
        Ok(())
    }
    #[test]
    fn a_stale_address_save_cannot_restore_an_old_pairing_group() -> anyhow::Result<()> {
        let fixture = tempfile::tempdir()?;
        let root = fixture.path();
        Settings {
            enabled: true,
            ..Default::default()
        }
        .save(root)?;
        let preview = get(root)?.unwrap();
        let mut stale = Settings::load(root)?;
        let fresh = Group::new(&preview.device.identity);
        preview.device.merge_group(&fresh)?;
        let mut authority = Settings::load(root)?;
        authority.room = fresh.room();
        authority.save_authority(root, &preview.device)?;
        stale.address = "ws://127.0.0.1:9".into();
        stale.save(root)?;
        assert!(Settings::load(root)?.room == fresh.room());
        disconnect(root)?;
        drop(preview);
        Ok(())
    }
    #[test]
    fn an_independent_old_import_cannot_resurrect_a_deleted_task() -> anyhow::Result<()> {
        let task = ManualTask {
            id: "synthetic".into(),
            task: "Synthetic task".into(),
            ..Default::default()
        };
        let mut a = Automerge::new();
        let mut stale = Automerge::new();
        apply(&mut a, &[], std::slice::from_ref(&task))?;
        apply(&mut a, std::slice::from_ref(&task), &[])?;
        apply(&mut stale, &[], &[task])?;
        a.merge(&mut stale)?;
        assert!(project(&a)?.is_empty());
        Ok(())
    }
}
