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

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub address: String,
    pub room: String,
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
        store_io::write(&root.join("local/relay/settings.json"), self)?;
        if !self.enabled {
            disconnect(root)?;
        }
        Ok(())
    }
}

pub fn endpoint(address: &str) -> io::Result<RelayUrl> {
    let result = if address.starts_with("ws:") {
        RelayUrl::local(address)
    } else {
        RelayUrl::new(address)
    };
    result.map_err(|_| store_io::invalid("Use a WSS origin on port 443 or ws://127.0.0.1:port"))
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
    settings: Settings,
    connection: Mutex<Option<Connection>>,
}

impl Preview {
    fn open(root: &Path, settings: Settings) -> io::Result<Self> {
        endpoint(&settings.address)?;
        if settings.room.is_empty() {
            return Err(store_io::invalid(
                "Relay preview needs paired test devices. Pairing UI is not available yet.",
            ));
        }
        std::fs::create_dir_all(root.join("local/relay"))?;
        let device =
            Arc::new(Device::open(&root.join("local/relay/replica.sqlite")).map_err(failed)?);
        device
            .store(|s| Group::load(s, &settings.room))
            .map_err(failed)?;
        Ok(Self {
            device,
            settings,
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
        let group = device
            .store(|s| Group::load(s, &self.settings.room))
            .map_err(failed)?;
        let relay = endpoint(&self.settings.address)?;
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
        let connection = self.connection.lock().map_err(failed)?;
        if let Some(connection) = connection.as_ref() {
            connection
                .client
                .edit("tasks", |doc| apply(doc, before, after))
                .map_err(failed)
        } else {
            self.device
                .store(|store| store.edit("tasks", |doc| apply(doc, before, after)))
                .map_err(failed)
        }
    }
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
            !task.id.is_empty() && task.id.len() <= 128,
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

fn apply(doc: &mut Automerge, before: &[ManualTask], after: &[ManualTask]) -> anyhow::Result<()> {
    let (before, after) = (fields(before)?, fields(after)?);
    let mut tx = doc.transaction();
    for id in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        let (old, new) = (before.get(id), after.get(id));
        if old.is_none() || new.is_none() {
            tx.put(
                ROOT,
                serde_json::to_string(&[id.as_str(), "$present"])?,
                if new.is_some() { "true" } else { "false" },
            )?;
        }
        let names: BTreeSet<_> = old
            .into_iter()
            .flat_map(|f| f.keys())
            .chain(new.into_iter().flat_map(|f| f.keys()))
            .collect();
        for name in names {
            anyhow::ensure!(name.len() <= 128 && name != "$present", "invalid field");
            let (a, b) = (old.and_then(|f| f.get(name)), new.and_then(|f| f.get(name)));
            if a != b && new.is_some() {
                tx.put(
                    ROOT,
                    serde_json::to_string(&[id, name])?,
                    serde_json::to_string(&b)?,
                )?;
            }
        }
    }
    tx.commit();
    Ok(())
}

fn project(doc: &Automerge) -> anyhow::Result<Vec<ManualTask>> {
    let mut rows: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut present = BTreeSet::new();
    for key in doc.keys(ROOT) {
        let [id, field]: [String; 2] = serde_json::from_str(&key)?;
        let (value, _) = doc
            .get(ROOT, &key)?
            .ok_or_else(|| anyhow::anyhow!("missing field"))?;
        let value: Value = serde_json::from_str(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid field"))?,
        )?;
        if field == "$present" {
            if value == Value::Bool(true) {
                present.insert(id);
            }
        } else if !value.is_null() {
            rows.entry(id).or_default().insert(field, value);
        }
    }
    present
        .into_iter()
        .map(|id| {
            let row = rows
                .remove(&id)
                .ok_or_else(|| anyhow::anyhow!("missing task"))?;
            let task: ManualTask = serde_json::from_value(Value::Object(row))?;
            anyhow::ensure!(task.id == id, "task ID mismatch");
            Ok(task)
        })
        .collect()
}

/// Synthetic two-process test driver. Runs before singleton, updates or window creation.
#[doc(hidden)]
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
    let preview = get(root)?.ok_or_else(|| store_io::invalid("Preview is disabled"))?;
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
                preview.online(false)?;
                Ok(Value::Null)
            }
            "online" => {
                preview.online(true)?;
                Ok(Value::Null)
            }
            "quit" => {
                preview.online(false)?;
                Ok(Value::Bool(true))
            }
            _ => Err(store_io::invalid("Unknown test operation")),
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
    fn mixed_folder_and_preview_edit_is_rejected_before_either_store_changes() -> anyhow::Result<()>
    {
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
        assert!(result.is_err());
        let preview = get(root)?.unwrap();
        assert!(preview.tasks()?.is_empty());
        disconnect(root)?;
        drop(preview);
        Settings::default().save(root)?;
        let overrides = store.load_overrides()?;
        assert!(overrides.manual.is_empty() && overrides.tasks.is_empty());
        Ok(())
    }
}
