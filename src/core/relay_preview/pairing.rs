use super::super::DeviceSettings;
use super::*;
use anyhow::Context;
use sidenote_sync::pairing::{self as protocol, Confirmation, PairHost, Receipt, SecretCode};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

pub const TIMED_OUT: &str = "Pairing timed out. Start again on both devices.";
pub const UNCONFIRMED: &str = "The other device may have joined. Check it. If you don't trust it, turn off relay sync on all devices and pair again.";
#[derive(Debug)]
pub(super) struct Operation(pub &'static str);
impl Operation {
    pub(super) fn io(self, error: io::Error) -> io::Error {
        io::Error::new(
            error.kind(),
            store_io::RelayFailure(
                store_io::relay_reason(&error).unwrap_or("relay_io"),
                store_io::os_code(&error),
                Some(self.0),
            ),
        )
    }
}
impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Operation {}

fn failure(mut stage: &'static str, error: &anyhow::Error) -> Update {
    let io = error.chain().find_map(|e| e.downcast_ref::<io::Error>());
    let category = if error.is::<tokio::time::error::Elapsed>() {
        stage = "timeout";
        "deadline".into()
    } else if let Some(error) = error.downcast_ref::<sidenote_sync::worker::WorkerLaunch>() {
        error.to_string()
    } else if let Some(error) = io {
        store_io::os_code(error).map_or_else(
            || format!("io:{:?}", error.kind()),
            |code| format!("os:{code}"),
        )
    } else {
        "protocol".into()
    };
    let operation = io
        .and_then(|e| e.get_ref()?.downcast_ref::<store_io::RelayFailure>()?.2)
        .or_else(|| {
            error
                .downcast_ref::<Operation>()
                .map(|operation| operation.0)
        });
    let category = match operation {
        Some(operation) => format!("{category}:{operation}"),
        None => category,
    };
    if stage != "timeout"
        && let Some(marker) = error.downcast_ref::<protocol::FailureStage>()
    {
        stage = marker.0;
    }
    Update::Failed { stage, category }
}

static ATTEMPT: AtomicU64 = AtomicU64::new(1);
struct Reply {
    attempt: u64,
    accept: bool,
}
async fn confirmation_reply(
    attempt: u64,
    replies: &mut tokio::sync::mpsc::UnboundedReceiver<Reply>,
) -> Result<bool, tokio::time::error::Elapsed> {
    tokio::time::timeout(Duration::from_secs(90), async {
        while let Some(reply) = replies.recv().await {
            if reply.attempt == attempt {
                return reply.accept;
            }
        }
        false
    })
    .await
}
async fn await_confirmation(
    attempt: u64,
    replies: &mut tokio::sync::mpsc::UnboundedReceiver<Reply>,
    events: &mpsc::Sender<Update>,
    cancel: &tokio::sync::watch::Sender<bool>,
) -> bool {
    match confirmation_reply(attempt, replies).await {
        Ok(accepted) => accepted,
        Err(_) => {
            let _ = events.send(Update::TimedOut);
            let _ = cancel.send(true);
            false
        }
    }
}

pub enum Update {
    Code {
        code: SecretCode,
        expires: Instant,
    },
    Confirm {
        attempt: u64,
        local_name: String,
        peer_name: String,
        phrase: String,
    },
    Ready,
    TimedOut,
    Unconfirmed,
    Failed {
        stage: &'static str,
        category: String,
    },
}

pub struct Pairing {
    confirm: tokio::sync::mpsc::UnboundedSender<Reply>,
    cancel: tokio::sync::watch::Sender<bool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Pairing {
    pub fn confirm(&self, attempt: u64, accept: bool) {
        let _ = self.confirm.send(Reply { attempt, accept });
    }
    pub fn cancel(&mut self) {
        let _ = self.cancel.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Pairing {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub fn phrase(sas: &str) -> String {
    const WORDS: [&str; 16] = [
        "amber", "birch", "cedar", "dawn", "elm", "fern", "glen", "harbor", "iris", "jade", "kite",
        "lake", "moss", "north", "oak", "pine",
    ];
    sas.chars()
        .filter_map(|c| c.to_digit(16).map(|n| WORDS[n as usize]))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn start(
    root: &Path,
    join: Option<SecretCode>,
) -> io::Result<(Pairing, mpsc::Receiver<Update>)> {
    let preview = get(root)?.ok_or_else(|| store_io::invalid("Turn on relay preview first"))?;
    preview
        .device
        .store(|s| s.check_compatible())
        .map_err(failed)?;
    let settings = Settings::load(root)?;
    let relay = endpoint(&settings.address)?;
    let name = super::super::DeviceSettings::read_in(root)?.name;
    let (events, receiver) = mpsc::channel();
    let (confirm, confirmation) = tokio::sync::mpsc::unbounded_channel();
    let (cancel, mut cancelled) = tokio::sync::watch::channel(false);
    let timeout_cancel = cancel.clone();
    let thread = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build();
        let runtime = match runtime {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = events.send(failure("exchange", &error.into()));
                return;
            }
        };
        runtime.block_on(async move {
            let confirmed_peer = Arc::new(Mutex::new(None::<Confirmation>));
            let confirmations = Arc::new(tokio::sync::Mutex::new(confirmation));
            let peer = confirmed_peer.clone();
            let messages = events.clone();
            let local = name.clone();
            let confirm = move |info: Confirmation| {
                let messages = messages.clone();
                let local = local.clone();
                let confirmations = confirmations.clone();
                let peer = peer.clone();
                let timeout_cancel = timeout_cancel.clone();
                async move {
                    let attempt = ATTEMPT.fetch_add(1, Ordering::Relaxed);
                    let shown = Update::Confirm {
                        attempt,
                        local_name: local,
                        peer_name: info.peer_name.clone(),
                        phrase: phrase(&info.phrase),
                    };
                    if messages.send(shown).is_err() {
                        return false;
                    }
                    let accepted = await_confirmation(
                        attempt,
                        &mut *confirmations.lock().await,
                        &messages,
                        &timeout_cancel,
                    )
                    .await;
                    if accepted && let Ok(mut saved) = peer.lock() {
                        *saved = Some(info);
                    }
                    accepted
                }
            };
            let mut receipt = Receipt::Confirmed;
            let mut stage = "exchange";
            let operation = async {
                let group = if let Some(code) = join {
                    protocol::join_named(&relay, &code, &preview.device, &name, confirm).await?
                } else {
                    let host = PairHost::open(&relay, &preview.device.identity).await?;
                    let expires = Instant::now() + host.remaining();
                    events
                        .send(Update::Code {
                            code: SecretCode::new(host.code().into()),
                            expires,
                        })
                        .map_err(|_| anyhow::anyhow!("pairing closed"))?;
                    let existing = preview.device.store(|s| Group::load(s, &settings.room))?;
                    let grant = host
                        .accept_named(&preview.device, Some(existing), &name, confirm)
                        .await?;
                    stage = "receipt";
                    receipt = grant.receipt;
                    anyhow::ensure!(receipt != Receipt::NotSent, "pairing receipt missing");
                    grant.group
                };
                stage = "confirm";
                let peer = confirmed_peer
                    .lock()
                    .map_err(|_| anyhow::anyhow!("confirmation unavailable"))?
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("confirmation missing"))?;
                stage = "adopt";
                preview.adopt(&group, &peer, receipt)?;
                Ok::<_, anyhow::Error>(())
            };
            let result = tokio::select! {_=cancelled.changed()=>return,result=operation=>result};
            if *cancelled.borrow() {
                return;
            }
            let _ = events.send(if receipt == Receipt::Unknown {
                Update::Unconfirmed
            } else if let Err(error) = result {
                failure(stage, &error)
            } else {
                Update::Ready
            });
        });
    });
    Ok((
        Pairing {
            confirm,
            cancel,
            thread: Some(thread),
        },
        receiver,
    ))
}

#[cfg(test)]
mod security_tests {
    use super::*;
    #[test]
    fn failure_categories_suppress_private_error_text() {
        use sidenote_sync::status::{Failure, Reason};
        let io = io::Error::new(io::ErrorKind::PermissionDenied, "private path key");
        let launch = sidenote_sync::worker::WorkerLaunch(Some(5));
        let wrapped = io::Error::other(store_io::RelayFailure("private path", Some(32), None));
        for (error, expected) in [
            (anyhow::anyhow!("private code"), "protocol"),
            (io.into(), "io:PermissionDenied"),
            (wrapped.into(), "os:32"),
            (launch.into(), "worker_launch:os:5"),
        ] {
            assert!(
                matches!(failure("adopt", &error), Update::Failed { stage: "adopt", category } if category == expected)
            );
        }
        let error = anyhow::Error::new(io::Error::from_raw_os_error(32))
            .context("private path key")
            .context(Operation("open mode.lock"));
        assert!(
            matches!(failure("adopt", &error), Update::Failed { stage: "adopt", category }
            if category == "os:32:open mode.lock")
        );
        let settings = Operation("read session relay settings.json").io(io::Error::other(
            store_io::RelayFailure("relay_io", Some(32), None),
        ));
        assert_eq!(settings.kind(), io::ErrorKind::Other);
        assert_eq!(store_io::relay_reason(&settings), Some("relay_io"));
        assert_eq!(store_io::os_code(&settings), Some(32));
        assert_eq!(settings.to_string(), "relay_io");
        let error = anyhow::Error::new(settings)
            .context("private path key")
            .context(Operation("start sync session"));
        assert!(
            matches!(failure("adopt", &error), Update::Failed { stage: "adopt", category }
            if category == "os:32:read session relay settings.json")
        );
        for label in [
            "check replica database compatibility",
            "read active group replica database",
            "merge membership replica database",
            "publish capabilities replica database",
            "persist peer names replica database",
            "retire old group replica database",
            "read session group replica database",
        ] {
            for code in [None, Some(32)] {
                let source = match code {
                    Some(code) => {
                        io::Error::other(store_io::RelayFailure("relay_database", Some(code), None))
                    }
                    None => failed(Failure(Reason::LocalStore)),
                };
                let error = if label == "read session group replica database" {
                    let source = Operation(label).io(source);
                    assert_eq!(store_io::relay_reason(&source), Some("relay_database"));
                    assert_eq!(store_io::os_code(&source), code);
                    assert_eq!(source.to_string(), "relay_database");
                    anyhow::Error::new(source).context(Operation("start sync session"))
                } else {
                    anyhow::Error::new(source).context(Operation(label))
                }
                .context("private path key");
                let category = code.map_or_else(|| "io:Other".into(), |code| format!("os:{code}"));
                let expected = format!("{category}:{label}");
                assert!(
                    matches!(failure("adopt", &error), Update::Failed { stage: "adopt", category }
                    if category == expected)
                );
            }
        }
    }
    #[tokio::test(start_paused = true)]
    async fn confirmation_timeout_ends_job_and_expired_replies_cannot_approve_next_prompt() {
        let (send, mut replies) = tokio::sync::mpsc::unbounded_channel();
        let (events, updates) = mpsc::channel();
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        assert!(!await_confirmation(1, &mut replies, &events, &cancel).await);
        assert!(*cancelled.borrow());
        assert!(matches!(updates.try_recv(), Ok(Update::TimedOut)));
        send.send(Reply {
            attempt: 1,
            accept: true,
        })
        .unwrap();
        send.send(Reply {
            attempt: 2,
            accept: false,
        })
        .unwrap();
        assert!(!confirmation_reply(2, &mut replies).await.unwrap());
    }
    #[tokio::test(start_paused = true)]
    async fn a_reply_for_another_attempt_never_confirms_the_current_prompt() {
        let (send, mut replies) = tokio::sync::mpsc::unbounded_channel();
        send.send(Reply {
            attempt: 41,
            accept: true,
        })
        .unwrap();
        send.send(Reply {
            attempt: 42,
            accept: false,
        })
        .unwrap();
        assert!(!confirmation_reply(42, &mut replies).await.unwrap());
    }
    #[test]
    #[cfg(windows)]
    fn devices_releases_mode_lock_while_loading_tasks() -> anyhow::Result<()> {
        use std::os::windows::fs::OpenOptionsExt;

        let fixture = tempfile::tempdir()?;
        let root = fixture.path();
        DeviceSettings::set_name_in(root, "Synthetic device")?;
        Settings {
            enabled: true,
            address: String::new(),
            ..Default::default()
        }
        .save(root)?;
        let _preview = get(root)?.unwrap();
        let (entered, waiting) = mpsc::channel();
        let (resume, released) = mpsc::channel();
        let path = root.to_owned();
        let worker = thread::spawn(move || {
            DEVICES_TASKS_LOAD.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    entered.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(2)).unwrap();
                }));
            });
            devices(&path)
        });
        // This hook runs with Store held after the mode-protected snapshot.
        let mode = waiting.recv_timeout(Duration::from_secs(2)).map(|()| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(root.join("local/relay/mode.lock"))
        });
        let _ = resume.send(());
        let devices = worker.join();
        disconnect(root)?;
        let _mode = mode??;
        assert!(
            devices
                .expect("devices thread panicked")?
                .iter()
                .any(|device| device.local && device.name == "Synthetic device")
        );
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn disabled_authority_save_reports_the_settings_operation() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let device = Device::open(&root.path().join("replica.sqlite"))?;
        let error = Settings::default()
            .save_authority(root.path(), &device)
            .unwrap_err();
        assert!(matches!(
            failure("adopt", &error),
            Update::Failed { stage: "adopt", category }
                if category == "io:InvalidData:read authority settings.json"
        ));
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn a_renamed_device_shows_its_new_name_and_a_bad_name_changes_nothing() -> anyhow::Result<()> {
        let d = tempfile::tempdir()?;
        let root = d.path();
        Settings {
            enabled: true,
            ..Default::default()
        }
        .save(root)?;
        let preview = get(root)?.unwrap();
        let peer = Device::open(&d.path().join("peer.sqlite"))?;
        let room = Settings::load(root)?.room;
        let mut group = preview.device.store(|s| Group::load(s, &room))?;
        group.enroll(&preview.device.identity, peer.identity.public());
        let confirmation = Confirmation {
            peer: peer.identity.public(),
            peer_name: "Old name".into(),
            phrase: String::new(),
        };
        preview.adopt(&group, &confirmation, Receipt::Confirmed)?;
        let shown = |root: &Path| -> io::Result<Vec<String>> {
            Ok(devices(root)?.into_iter().map(|p| p.name).collect())
        };
        assert!(shown(root)?.contains(&"Old name".to_owned()));
        // The peer renames itself; its entry arrives through the shared document.
        preview.device.store(|s| {
            s.edit("tasks", |doc| {
                put_names(doc, &[(peer.identity.public(), "New name")])
            })
        })?;
        let names = shown(root)?;
        assert!(names.contains(&"New name".to_owned()) && !names.contains(&"Old name".to_owned()));
        rename_device(root, " Studio PC ")?;
        let local = devices(root)?.into_iter().find(|p| p.local).unwrap();
        assert_eq!(local.name, "Studio PC");
        assert!(rename_device(root, "").is_err());
        assert!(rename_device(root, &"x".repeat(81)).is_err());
        assert_eq!(
            devices(root)?.into_iter().find(|p| p.local).unwrap().name,
            "Studio PC"
        );
        disconnect(root)?;
        Ok(())
    }
    #[test]
    #[cfg(windows)]
    fn uncertain_pairing_status_and_peer_name_survive_restart() -> anyhow::Result<()> {
        let d = tempfile::tempdir()?;
        let root = d.path();
        Settings {
            enabled: true,
            ..Default::default()
        }
        .save(root)?;
        let preview = get(root)?.unwrap();
        let peer = Device::open(&d.path().join("peer.sqlite"))?;
        let room = Settings::load(root)?.room;
        let mut group = preview.device.store(|s| Group::load(s, &room))?;
        group.enroll(&preview.device.identity, peer.identity.public());
        let confirmation = Confirmation {
            peer: peer.identity.public(),
            peer_name: "Synthetic laptop".into(),
            phrase: String::new(),
        };
        preview.adopt(&group, &confirmation, Receipt::Unknown)?;
        disconnect(root)?;
        drop(preview);
        let peers = devices(root)?;
        assert!(
            peers
                .iter()
                .any(|p| !p.local && p.unconfirmed && p.name == "Synthetic laptop")
        );
        disconnect(root)?;
        Ok(())
    }
    #[cfg(windows)]
    fn enable(root: &Path) -> anyhow::Result<Arc<Preview>> {
        Settings {
            enabled: true,
            ..Settings::load(root)?
        }
        .save(root)?;
        Ok(get(root)?.expect("relay sync is on"))
    }
    /// Two devices that adopted each other and have exchanged their shared documents.
    #[cfg(windows)]
    fn paired(a_root: &Path, b_root: &Path) -> anyhow::Result<(Arc<Preview>, Arc<Preview>)> {
        let (a, b) = (enable(a_root)?, enable(b_root)?);
        for (me, other, other_name) in [(&a, &b, "Peer B"), (&b, &a, "Peer A")] {
            let room = Settings::load(&me.root)?.room;
            let mut group = me.device.store(|s| Group::load(s, &room))?;
            group.enroll(&me.device.identity, other.device.identity.public());
            let confirmation = Confirmation {
                peer: other.device.identity.public(),
                peer_name: other_name.into(),
                phrase: String::new(),
            };
            me.adopt(&group, &confirmation, Receipt::Confirmed)?;
        }
        sync_documents(&a, &b)?;
        sync_documents(&b, &a)?;
        Ok((a, b))
    }
    /// What the relay does for the shared document: the receiver merges the sender's changes.
    #[cfg(windows)]
    fn sync_documents(from: &Preview, to: &Preview) -> anyhow::Result<()> {
        let mut theirs = from.device.load("tasks").map_err(failed)?;
        to.device
            .store(|s| {
                s.edit("tasks", |doc| {
                    doc.merge(&mut theirs)?;
                    Ok(())
                })
            })
            .map_err(failed)?;
        Ok(())
    }
    #[cfg(windows)]
    fn other_names(root: &Path) -> io::Result<Vec<String>> {
        Ok(devices(root)?
            .into_iter()
            .filter(|p| !p.local)
            .map(|p| p.name)
            .collect())
    }
    #[test]
    #[cfg(windows)]
    fn a_rename_made_with_relay_on_reaches_the_other_device() -> anyhow::Result<()> {
        let (da, db) = (tempfile::tempdir()?, tempfile::tempdir()?);
        let (a, b) = paired(da.path(), db.path())?;
        assert!(!other_names(db.path())?.contains(&"Studio PC".to_owned()));
        let tags: Vec<String> = devices(db.path())?.into_iter().map(|p| p.tag).collect();
        assert!(tags.len() == 2 && tags[0] != tags[1] && tags.iter().all(|t| t.len() == 4));
        rename_device(da.path(), "Studio PC")?;
        sync_documents(&a, &b)?;
        assert_eq!(other_names(db.path())?, ["Studio PC"]);
        disconnect(da.path())?;
        disconnect(db.path())?;
        Ok(())
    }
    #[test]
    #[cfg(windows)]
    fn a_rename_made_with_relay_off_is_published_when_relay_is_turned_on() -> anyhow::Result<()> {
        let (da, db) = (tempfile::tempdir()?, tempfile::tempdir()?);
        let (a, b) = paired(da.path(), db.path())?;
        Settings {
            enabled: false,
            ..Settings::load(da.path())?
        }
        .save(da.path())?;
        drop(a);
        rename_device(da.path(), "Studio PC")?;
        assert_eq!(DeviceSettings::read_in(da.path())?.name, "Studio PC");
        let a = enable(da.path())?;
        sync_documents(&a, &b)?;
        assert_eq!(other_names(db.path())?, ["Studio PC"]);
        disconnect(da.path())?;
        disconnect(db.path())?;
        Ok(())
    }
}

pub(super) fn token(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

fn put_names(doc: &mut Automerge, names: &[([u8; 32], &str)]) -> anyhow::Result<()> {
    let mut tx = doc.transaction();
    for (key, name) in names {
        tx.put(
            ROOT,
            serde_json::to_string(&["$device", token(key).as_str()])?,
            serde_json::to_string(name)?,
        )?;
    }
    tx.commit();
    Ok(())
}

/// The name a device published about itself in the shared document, if it is a valid one.
fn shared_name(document: &Automerge, token: &str) -> Option<String> {
    let key = serde_json::to_string(&["$device", token]).ok()?;
    let value = document.get(ROOT, key).ok()??.0;
    let name: String = serde_json::from_str(value.as_str()?).ok()?;
    DeviceSettings::valid_name(&name).filter(|valid| *valid == name)
}

/// Renames this device: device.json first, then the shared document when relay sync is on.
pub fn rename_device(root: &Path, name: &str) -> io::Result<()> {
    let name = DeviceSettings::valid_name(name)
        .ok_or_else(|| store_io::invalid("Use 1 to 80 characters, with no control characters"))?;
    DeviceSettings::set_name_in(root, &name)?;
    match get(root)? {
        Some(preview) => preview.publish_name(&name),
        None => Ok(()),
    }
}

#[derive(Clone)]
pub struct PairedDevice {
    pub token: String,
    /// Last 4 characters of the token: tells apart devices that share a name.
    pub tag: String,
    pub name: String,
    pub local: bool,
    pub unconfirmed: bool,
    pub presence: Presence,
}

#[cfg(all(test, windows))]
thread_local! {
    static DEVICES_TASKS_LOAD: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

pub fn devices(root: &Path) -> io::Result<Vec<PairedDevice>> {
    let Some(preview) = get(root)? else {
        return Ok(Vec::new());
    };
    let (settings, group) = {
        let _mode = store_io::lock(&root.join("local/relay/mode.lock"))?;
        let settings = Settings::load(root)?;
        let group = preview
            .device
            .store(|s| Group::load(s, &settings.room))
            .map_err(failed)?;
        (settings, group)
    };
    let local_name = super::super::DeviceSettings::read_in(root)?.name;
    let document = preview
        .device
        .store(|s| {
            #[cfg(all(test, windows))]
            if let Some(hook) = DEVICES_TASKS_LOAD.with(|hook| hook.borrow_mut().take()) {
                hook();
            }
            s.load("tasks")
        })
        .map_err(failed)?;
    let presence = preview.presence();
    Ok(group
        .members()
        .into_iter()
        .map(|key| {
            let local = key == preview.device.identity.public();
            let token = token(&key);
            // The device's own entry in the shared document is newest; the pairing-time name is the fallback.
            let name = if local {
                local_name.clone()
            } else {
                shared_name(&document, &token)
                    .or_else(|| settings.peers.get(&token).cloned())
                    .unwrap_or_else(|| "Paired device".into())
            };
            let unconfirmed = settings.unconfirmed.contains(&token);
            PairedDevice {
                tag: token[token.len().saturating_sub(4)..].to_owned(),
                presence: if local {
                    Presence::Online
                } else {
                    presence.get(&token).copied().unwrap_or(Presence::Unknown)
                },
                token,
                name,
                local,
                unconfirmed,
            }
        })
        .collect())
}

impl Preview {
    fn adopt(&self, group: &Group, peer: &Confirmation, receipt: Receipt) -> anyhow::Result<()> {
        self.online(false).context(Operation("stop sync session"))?;
        self.device
            .store(|s| s.check_compatible())
            .map_err(failed)
            .context(Operation("check replica database compatibility"))?;
        let mut settings =
            Settings::load(&self.root).context(Operation("read relay settings.json"))?;
        if !settings.enabled {
            return Err(store_io::invalid("Relay preview was turned off"))
                .context(Operation("read relay settings.json"));
        }
        self.device
            .store(|s| s.check_group_active(&group.room()))
            .map_err(failed)
            .context(Operation("read active group replica database"))?;
        self.device
            .merge_group(group)
            .map_err(failed)
            .context(Operation("merge membership replica database"))?;
        settings.room = group.room();
        self.device
            .store(|s| {
                s.configure_capabilities(&settings.room, super::operations::supported(&self.root))
            })
            .map_err(failed)
            .context(Operation("publish capabilities replica database"))?;
        settings
            .peers
            .insert(token(&peer.peer), peer.peer_name.clone());
        if receipt == Receipt::Unknown {
            settings.unconfirmed.insert(token(&peer.peer));
        } else {
            settings.unconfirmed.remove(&token(&peer.peer));
        }
        let local_name = super::super::DeviceSettings::read_in(&self.root)
            .context(Operation("read device.json"))?
            .name;
        self.device
            .store(|s| {
                s.edit("tasks", |doc| {
                    put_names(
                        doc,
                        &[
                            (self.device.identity.public(), local_name.as_str()),
                            (peer.peer, peer.peer_name.as_str()),
                        ],
                    )
                })
            })
            .map_err(failed)
            .context(Operation("persist peer names replica database"))?;
        settings.save_authority(&self.root, &self.device)?;
        self.online(true).context(Operation("start sync session"))
    }
    /// A rename made while relay sync was off lives only in device.json. When sync starts again,
    /// publish it so the paired devices stop showing the old name. Best effort: the next start retries.
    pub(super) fn reconcile_name(&self) {
        let Ok(local) = DeviceSettings::read_in(&self.root) else {
            return;
        };
        let Ok(document) = self.device.load("tasks") else {
            return;
        };
        let published = shared_name(&document, &token(&self.device.identity.public()));
        if published.is_some_and(|name| name != local.name) {
            let _ = self.publish_name(&local.name);
        }
    }
    /// Publishes this device's name in the shared document; paired devices learn it through sync.
    pub fn publish_name(&self, name: &str) -> io::Result<()> {
        let names = [(self.device.identity.public(), name)];
        let connection = self.connection.lock().map_err(|_| failed("mutex"))?;
        if let Some(connection) = connection.as_ref() {
            connection
                .client
                .edit("tasks", |doc| put_names(doc, &names))
                .map_err(failed)
        } else {
            self.device
                .store(|s| s.edit("tasks", |doc| put_names(doc, &names)))
                .map_err(failed)
        }
    }
    pub fn remove_device(&self, _target: &str) -> io::Result<()> {
        Err(store_io::invalid(
            "Removing a device is not available in this preview",
        ))
    }
}
