use super::*;
use sidenote_sync::pairing::{self as protocol, Confirmation, PairHost, Receipt, SecretCode};
use std::time::{Duration, Instant};

pub enum Update {
    Code {
        code: SecretCode,
        expires: Instant,
    },
    Confirm {
        local_name: String,
        peer_name: String,
        phrase: String,
    },
    Ready,
    Failed,
}

pub struct Pairing {
    confirm: tokio::sync::mpsc::UnboundedSender<bool>,
    cancel: tokio::sync::watch::Sender<bool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Pairing {
    pub fn confirm(&self, accept: bool) {
        let _ = self.confirm.send(accept);
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
    let thread = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build();
        let Ok(runtime) = runtime else {
            let _ = events.send(Update::Failed);
            return;
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
                async move {
                    let shown = Update::Confirm {
                        local_name: local,
                        peer_name: info.peer_name.clone(),
                        phrase: phrase(&info.phrase),
                    };
                    if messages.send(shown).is_err() {
                        return false;
                    }
                    let accepted = tokio::time::timeout(Duration::from_secs(90), async {
                        confirmations.lock().await.recv().await
                    })
                    .await
                    .ok()
                    .flatten()
                        == Some(true);
                    if accepted && let Ok(mut saved) = peer.lock() {
                        *saved = Some(info);
                    }
                    accepted
                }
            };
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
                    anyhow::ensure!(
                        grant.receipt == Receipt::Confirmed,
                        "pairing receipt missing"
                    );
                    grant.group
                };
                let peer = confirmed_peer
                    .lock()
                    .map_err(|_| anyhow::anyhow!("confirmation unavailable"))?
                    .take()
                    .ok_or_else(|| anyhow::anyhow!("confirmation missing"))?;
                preview.adopt(&group, &peer).map_err(anyhow::Error::from)?;
                Ok::<_, anyhow::Error>(())
            };
            let result = tokio::select! {_=cancelled.changed()=>return,result=operation=>result};
            let _ = events.send(if result.is_ok() {
                Update::Ready
            } else {
                Update::Failed
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

fn token(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Clone)]
pub struct PairedDevice {
    pub token: String,
    pub name: String,
    pub local: bool,
}

pub fn devices(root: &Path) -> io::Result<Vec<PairedDevice>> {
    let Some(preview) = get(root)? else {
        return Ok(Vec::new());
    };
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
            PairedDevice { token, name, local }
        })
        .collect())
}

impl Preview {
    fn adopt(&self, group: &Group, peer: &Confirmation) -> io::Result<()> {
        self.online(false)?;
        self.device
            .store(|s| s.check_compatible())
            .map_err(failed)?;
        let mut settings = Settings::load(&self.root)?;
        if !settings.enabled {
            return Err(store_io::invalid("Relay preview was turned off"));
        }
        self.device
            .store(|s| s.check_group_active(&group.room()))
            .map_err(failed)?;
        self.device.merge_group(group).map_err(failed)?;
        if settings.room != group.room() {
            self.device
                .store(|s| s.retire_group(&settings.room))
                .map_err(failed)?;
        }
        settings.room = group.room();
        self.device
            .store(|s| {
                s.configure_capabilities(&settings.room, super::operations::supported(&self.root))
            })
            .map_err(failed)?;
        settings
            .peers
            .insert(token(&peer.peer), peer.peer_name.clone());
        let local_name = super::super::DeviceSettings::read_in(&self.root)?.name;
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
        settings.save_authority(&self.root)?;
        self.online(true)
    }
    pub fn remove_device(&self, _target: &str) -> io::Result<()> {
        Err(store_io::invalid(
            "Removing a device is not available in this preview",
        ))
    }
}
