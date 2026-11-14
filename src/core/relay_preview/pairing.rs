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
impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Operation {}

fn failure(mut stage: &'static str, error: &anyhow::Error) -> Update {
    let category = if error.is::<tokio::time::error::Elapsed>() {
        stage = "timeout";
        "deadline".into()
    } else if let Some(error) = error.downcast_ref::<sidenote_sync::worker::WorkerLaunch>() {
        error.to_string()
    } else if let Some(error) = error.chain().find_map(|e| e.downcast_ref::<io::Error>()) {
        store_io::os_code(error).map_or_else(
            || format!("io:{:?}", error.kind()),
            |code| format!("os:{code}"),
        )
    } else {
        "protocol".into()
    };
    let category = match error.downcast_ref::<Operation>() {
        Some(operation) => format!("{category}:{}", operation.0),
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
        let io = io::Error::new(io::ErrorKind::PermissionDenied, "private path key");
        let launch = sidenote_sync::worker::WorkerLaunch(Some(5));
        let wrapped = io::Error::other(store_io::RelayFailure("private path", Some(32)));
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
}

fn token(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone)]
pub struct PairedDevice {
    pub token: String,
    pub name: String,
    pub local: bool,
    pub unconfirmed: bool,
}

pub fn devices(root: &Path) -> io::Result<Vec<PairedDevice>> {
    let Some(preview) = get(root)? else {
        return Ok(Vec::new());
    };
    let _mode = store_io::lock(&root.join("local/relay/mode.lock"))?;
    let settings = Settings::load(root)?;
    let group = preview
        .device
        .store(|s| Group::load(s, &settings.room))
        .map_err(failed)?;
    let local_name = super::super::DeviceSettings::read_in(root)?.name;
    let document = preview.device.load("tasks").map_err(failed)?;
    Ok(group
        .members()
        .into_iter()
        .map(|key| {
            let local = key == preview.device.identity.public();
            let token = token(&key);
            let name = if local {
                local_name.clone()
            } else {
                settings
                    .peers
                    .get(&token)
                    .cloned()
                    .or_else(|| {
                        let key = serde_json::to_string(&["$device", token.as_str()]).ok()?;
                        let value = document.get(ROOT, key).ok()??.0;
                        let name: String = serde_json::from_str(value.as_str()?).ok()?;
                        (!name.trim().is_empty()
                            && name.chars().count() <= 80
                            && !name.chars().any(char::is_control))
                        .then_some(name)
                    })
                    .unwrap_or_else(|| "Paired device".into())
            };
            let unconfirmed = settings.unconfirmed.contains(&token);
            PairedDevice {
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
            .map_err(failed)?;
        let mut settings =
            Settings::load(&self.root).context(Operation("read relay settings.json"))?;
        if !settings.enabled {
            return Err(store_io::invalid("Relay preview was turned off").into());
        }
        self.device
            .store(|s| s.check_group_active(&group.room()))
            .map_err(failed)?;
        self.device.merge_group(group).map_err(failed)?;
        settings.room = group.room();
        self.device
            .store(|s| {
                s.configure_capabilities(&settings.room, super::operations::supported(&self.root))
            })
            .map_err(failed)?;
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
                    let mut tx = doc.transaction();
                    for (key, name) in [
                        (self.device.identity.public(), local_name.as_str()),
                        (peer.peer, peer.peer_name.as_str()),
                    ] {
                        tx.put(
                            ROOT,
                            serde_json::to_string(&["$device", token(&key).as_str()])?,
                            serde_json::to_string(name)?,
                        )?;
                    }
                    tx.commit();
                    Ok(())
                })
            })
            .map_err(failed)?;
        settings.save_authority(&self.root, &self.device)?;
        self.online(true).context(Operation("start sync session"))
    }
    pub fn remove_device(&self, _target: &str) -> io::Result<()> {
        Err(store_io::invalid(
            "Removing a device is not available in this preview",
        ))
    }
}
