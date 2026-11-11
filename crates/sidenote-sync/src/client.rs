use crate::{
    crypto::{self, Cipher},
    identity::{Enrollment, Group, Identity, random32},
    status::{Backoff, Failure, PeerStatus, Reason, Status},
    store::{Commit, INVENTORY_PAGE, MAX_PEER_DOCUMENTS, Store},
    transport::{Frame, RelayUrl, Transport, WebSocketTransport, authenticate},
    wire::Inner,
    worker::{Pool, Request as WorkRequest, Response as WorkResponse, Worker},
};
use anyhow::{Context, Result, ensure};
use automerge::{Automerge, ChangeHash};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::{
    sync::{Notify, watch},
    time::Instant,
};
const BUFFER_BUDGET: usize = 128 * 1024 * 1024;
const PEER_OPERATIONS: usize = 256;
const TOTAL_OPERATIONS: usize = 1024;
const OPERATION_COST: usize = 256;
const HEARTBEAT: Duration = Duration::from_secs(20);

pub struct Device {
    store: Mutex<Store>,
    pub identity: Identity,
}
impl Device {
    pub fn open(path: &Path) -> Result<Self> {
        let mut store = Store::open(path, b"uninitialized")?;
        let identity = Identity::load_or_create(&store)?;
        store.set_actor(&identity.public());
        Ok(Self {
            store: Mutex::new(store),
            identity,
        })
    }
    pub fn store<T>(&self, f: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
        let mut guard = self
            .store
            .lock()
            .map_err(|_| Failure(Reason::LockPoisoned))?;
        f(&mut guard)
    }
    pub fn merge_group(&self, candidate: &Group) -> Result<Group> {
        self.store(|s| s.merge_membership(candidate, self.identity.public()))
    }
    pub fn load(&self, id: &str) -> Result<Automerge> {
        self.store(|s| s.load(id))
    }
}
#[derive(Clone)]
pub struct Applied {
    pub revision: u64,
    pub domain: String,
    pub heads: Vec<ChangeHash>,
    pub at: std::time::Instant,
}

pub struct SyncClient {
    pub device: Arc<Device>,
    group: Mutex<Group>,
    relay: RelayUrl,
    pool: Pool,
    notify: Notify,
    dirty: Mutex<BTreeSet<String>>,
    fatal: Mutex<Option<Reason>>,
    documents: Mutex<BTreeMap<String, Reason>>,
    healthy_session: AtomicBool,
    #[cfg(test)]
    retry_ceilings: Mutex<Vec<Duration>>,
    pub connections: AtomicU64,
    pub applied: AtomicU64,
    pub rejected: AtomicU64,
    updates: watch::Sender<Option<Applied>>,
    status: watch::Sender<Status>,
    peers: Mutex<BTreeMap<[u8; 32], PeerStatus>>,
}
pub struct Running {
    pub client: Arc<SyncClient>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Running {
    pub async fn stop(self) -> Result<()> {
        let _ = self.stop.send(true);
        self.task.await?
    }
}
fn reason(error: &anyhow::Error) -> Reason {
    error
        .downcast_ref::<Failure>()
        .map(|f| f.0)
        .unwrap_or(Reason::Protocol)
}
fn local_failure(reason: Reason) -> bool {
    matches!(
        reason,
        Reason::LocalStore
            | Reason::CorruptDocument
            | Reason::StoreFull
            | Reason::LockPoisoned
            | Reason::Incompatible
    )
}

impl SyncClient {
    pub fn start(device: Arc<Device>, group: Group, relay: RelayUrl) -> Result<Running> {
        Self::start_with_worker(device, group, relay, Pool::adjacent()?)
    }
    pub fn start_with_worker(
        device: Arc<Device>,
        group: Group,
        relay: RelayUrl,
        pool: Pool,
    ) -> Result<Running> {
        group.validate(device.identity.public())?;
        let group = device.merge_group(&group)?;
        let (updates, _) = watch::channel(None);
        let (status, _) = watch::channel(Status::Offline);
        let client = Arc::new(Self {
            device,
            group: Mutex::new(group),
            relay,
            pool,
            notify: Notify::new(),
            dirty: Mutex::new(BTreeSet::new()),
            fatal: Mutex::new(None),
            documents: Mutex::new(BTreeMap::new()),
            healthy_session: AtomicBool::new(false),
            #[cfg(test)]
            retry_ceilings: Mutex::new(Vec::new()),
            connections: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            updates,
            status,
            peers: Mutex::new(BTreeMap::new()),
        });
        let (stop, receiver) = watch::channel(false);
        let task = tokio::spawn(client.clone().run(receiver));
        Ok(Running { client, stop, task })
    }
    pub fn edit<T>(&self, id: &str, f: impl FnOnce(&mut Automerge) -> Result<T>) -> Result<T> {
        let result = self.device.store(|s| s.edit(id, f));
        if let Err(error) = &result {
            let code = reason(error);
            if local_failure(code) {
                self.fail(code)?;
            } else if matches!(code, Reason::DocumentTooLarge | Reason::DocumentQuota) {
                self.documents
                    .lock()
                    .map_err(|_| Failure(Reason::LockPoisoned))?
                    .insert(id.to_owned(), code);
            }
        }
        if result.is_ok() {
            self.documents
                .lock()
                .map_err(|_| Failure(Reason::LockPoisoned))?
                .remove(id);
            self.dirty
                .lock()
                .map_err(|_| Failure(Reason::LockPoisoned))?
                .insert(id.to_owned());
            self.notify.notify_one();
        }
        result
    }
    pub fn document_status(&self, id: &str) -> Result<Option<Reason>> {
        Ok(self
            .documents
            .lock()
            .map_err(|_| Failure(Reason::LockPoisoned))?
            .get(id)
            .copied())
    }
    pub fn subscribe(&self) -> watch::Receiver<Option<Applied>> {
        self.updates.subscribe()
    }
    pub fn subscribe_status(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }
    pub fn peer_statuses(&self) -> Result<Vec<PeerStatus>> {
        Ok(self
            .peers
            .lock()
            .map_err(|_| Failure(Reason::LockPoisoned))?
            .values()
            .cloned()
            .collect())
    }
    pub fn group(&self) -> Result<Group> {
        Ok(self
            .group
            .lock()
            .map_err(|_| Failure(Reason::LockPoisoned))?
            .clone())
    }
    fn fail(&self, code: Reason) -> Result<()> {
        *self
            .fatal
            .lock()
            .map_err(|_| Failure(Reason::LockPoisoned))? = Some(code);
        self.status.send_replace(Status::Failed(code));
        self.notify.notify_one();
        Ok(())
    }
    fn fatal(&self) -> Result<Option<Reason>> {
        Ok(*self
            .fatal
            .lock()
            .map_err(|_| Failure(Reason::LockPoisoned))?)
    }
    pub fn update_membership(&self, enrollments: Vec<Enrollment>) -> Result<bool> {
        let mut candidate = self.group()?;
        candidate.enrollments = enrollments;
        self.device.store(|store| {
            let merged = store.merge_membership(&candidate, self.device.identity.public())?;
            let mut current = self
                .group
                .lock()
                .map_err(|_| Failure(Reason::LockPoisoned))?;
            let changed = merged.members() != current.members();
            *current = merged;
            if changed {
                self.notify.notify_one();
            }
            Ok(changed)
        })
    }
    async fn run(self: Arc<Self>, mut stop: watch::Receiver<bool>) -> Result<()> {
        let mut backoff = Backoff::default();
        let mut bans = BTreeMap::new();
        loop {
            if *stop.borrow() {
                self.status.send_replace(Status::Offline);
                return Ok(());
            }
            if let Some(code) = self.fatal()? {
                self.status.send_replace(Status::Failed(code));
                return Err(Failure(code).into());
            }
            self.status.send_replace(Status::Connecting);
            self.healthy_session.store(false, Ordering::Relaxed);
            let connection = async {
                let group = self.group()?;
                let mut ws = WebSocketTransport::connect(&self.relay.endpoint(
                    "sync",
                    &group.room(),
                    "device",
                ))
                .await?;
                let ready = authenticate(
                    &mut ws,
                    &group.room(),
                    "device",
                    &self.device.identity,
                    Some(&group),
                )
                .await?;
                self.connections.fetch_add(1, Ordering::Relaxed);
                self.session(&mut ws, ready, &group, &mut bans).await
            };
            let result = tokio::select! {_=stop.changed()=>{self.status.send_replace(Status::Offline);return Ok(());},result=connection=>result};
            if let Err(error) = result {
                let code = reason(&error);
                if local_failure(code) {
                    self.fail(code)?;
                    return Err(Failure(code).into());
                }
            }
            if let Some(code) = self.fatal()? {
                self.status.send_replace(Status::Failed(code));
                return Err(Failure(code).into());
            }
            self.status.send_replace(Status::Offline);
            let healthy = if self.healthy_session.load(Ordering::Relaxed) {
                Duration::from_secs(60)
            } else {
                Duration::ZERO
            };
            let delay = backoff.next_delay(healthy);
            #[cfg(test)]
            self.retry_ceilings
                .lock()
                .expect("test trace")
                .push(backoff.ceiling());
            tokio::select! {_=stop.changed()=>return Ok(()),_=tokio::time::sleep(delay)=>{}}
        }
    }
    fn reject(
        &self,
        s: &mut Session,
        id: [u8; 32],
        code: Reason,
        bans: &mut BTreeMap<[u8; 32], Ban>,
    ) -> Result<()> {
        s.peers.remove(&id);
        s.probes.remove(&id);
        s.challenges.remove(&id);
        s.handshakes.remove(&id);
        let ban = bans.entry(id).or_default();
        ban.strike(Instant::now());
        self.rejected.fetch_add(1, Ordering::Relaxed);
        let mut statuses = self
            .peers
            .lock()
            .map_err(|_| Failure(Reason::LockPoisoned))?;
        let record = statuses.entry(id).or_insert(PeerStatus {
            device: id,
            sending_bad_data: false,
            reason: None,
            last_synced: None,
        });
        record.reason = Some(code);
        record.sending_bad_data = !matches!(code, Reason::Transport);
        Ok(())
    }
    async fn probe(
        &self,
        ws: &mut impl Transport,
        s: &mut Session,
        id: [u8; 32],
        group: &Group,
    ) -> Result<()> {
        if self.device.identity.public() >= id
            || !group.members().contains(&id)
            || s.probes.contains_key(&id)
            || s.handshakes.contains_key(&id)
        {
            return Ok(());
        }
        let nonce = random32();
        s.probes.insert(id, (nonce, Instant::now()));
        ws.send(Frame::Binary(crypto::seal(
            &self.device.identity,
            group,
            id,
            4,
            [0; 32],
            &nonce,
        )))
        .await
    }
    fn activate(&self, s: &mut Session, id: [u8; 32], state: snow::HandshakeState) -> Result<()> {
        let worker = self.pool.spawn()?;
        s.peers.insert(
            id,
            Peer {
                negotiated: self.device.store(|s| Ok(s.requirements().is_none()))?,
                cipher: Cipher::from_handshake(state)?,
                worker: Some(worker),
                busy: false,
                loaded: None,
                ack_previous: false,
                domains: BTreeSet::new(),
                advertised: BTreeSet::new(),
                synced: BTreeSet::new(),
                pending: VecDeque::new(),
                queued: BTreeSet::new(),
                outbound: VecDeque::new(),
                inventory_cursor: None,
                inventory_sent: false,
                inventory_received: false,
                ping: None,
                last_pong: None,
                last_pong_system: None,
                ping_at: Instant::now(),
                born: Instant::now(),
            },
        );
        if let Some(caps) = self.device.store(|s| Ok(s.requirements()))? {
            s.peers
                .get_mut(&id)
                .ok_or(Failure(Reason::Protocol))?
                .queue(Inner::Capabilities(caps))?;
        }
        s.peers
            .get_mut(&id)
            .ok_or(Failure(Reason::Protocol))?
            .queue(Inner::Membership(self.group()?.enrollments.clone()))?;
        Ok(())
    }
    async fn envelope(
        &self,
        ws: &mut impl Transport,
        s: &mut Session,
        frame: Vec<u8>,
        group: &Group,
        bans: &mut BTreeMap<[u8; 32], Ban>,
    ) -> Result<()> {
        let id: [u8; 32] = match frame.get(2..34).and_then(|b| b.try_into().ok()) {
            Some(id) => id,
            None => return Ok(()),
        };
        if !group.members().contains(&id) || bans.get(&id).is_some_and(|b| Instant::now() < b.until)
        {
            return Ok(());
        }
        // An unauthenticated relay forgery cannot attribute bad-data status to a device.
        let Ok(opened) = crypto::open(&self.device.identity, group, &frame) else {
            return Ok(());
        };
        let action = async {
            match opened.kind {
                4 => {
                    ensure!(
                        id < self.device.identity.public() && opened.payload.len() == 32,
                        Failure(Reason::Protocol)
                    );
                    let probe = opened.payload.try_into()?;
                    let nonce = random32();
                    s.challenges.insert(
                        id,
                        Challenge {
                            probe,
                            nonce,
                            at: Instant::now(),
                        },
                    );
                    let mut reply = probe.to_vec();
                    reply.extend_from_slice(&nonce);
                    ws.send(Frame::Binary(crypto::seal(
                        &self.device.identity,
                        group,
                        id,
                        5,
                        [0; 32],
                        &reply,
                    )))
                    .await?;
                }
                5 => {
                    ensure!(opened.payload.len() == 64, Failure(Reason::Protocol));
                    let Some((probe, at)) = s.probes.remove(&id) else {
                        return Ok(());
                    };
                    if at.elapsed() > Duration::from_secs(10) || opened.payload[..32] != probe {
                        return Ok(());
                    }
                    let nonce: [u8; 32] = opened.payload[32..].try_into()?;
                    let mut fresh = [0; 64];
                    fresh[..32].copy_from_slice(&probe);
                    fresh[32..].copy_from_slice(&nonce);
                    let mut state =
                        crypto::handshake(group, self.device.identity.public(), id, true, &fresh)?;
                    let mut out = vec![0; 1024];
                    let len = state.write_message(&[], &mut out)?;
                    let mut body = nonce.to_vec();
                    body.extend_from_slice(&out[..len]);
                    s.handshakes.insert(id, (state, nonce, Instant::now()));
                    ws.send(Frame::Binary(crypto::seal(
                        &self.device.identity,
                        group,
                        id,
                        1,
                        [0; 32],
                        &body,
                    )))
                    .await?;
                }
                1 => {
                    ensure!(
                        id < self.device.identity.public() && opened.payload.len() > 32,
                        Failure(Reason::Protocol)
                    );
                    let Some(challenge) = s.challenges.get(&id) else {
                        return Ok(());
                    };
                    // Invalid/replayed initiations do not consume a fresh challenge or replace a live cipher.
                    if challenge.at.elapsed() > Duration::from_secs(10)
                        || opened.payload[..32] != challenge.nonce
                    {
                        return Ok(());
                    }
                    let mut fresh = [0; 64];
                    fresh[..32].copy_from_slice(&challenge.probe);
                    fresh[32..].copy_from_slice(&challenge.nonce);
                    let mut state =
                        crypto::handshake(group, self.device.identity.public(), id, false, &fresh)?;
                    let mut out = vec![0; 65535];
                    state
                        .read_message(&opened.payload[32..], &mut out)
                        .map_err(|_| Failure(Reason::Transport))?;
                    let len = state.write_message(&[], &mut out)?;
                    let mut body = challenge.nonce.to_vec();
                    body.extend_from_slice(&out[..len]);
                    s.challenges.remove(&id);
                    ws.send(Frame::Binary(crypto::seal(
                        &self.device.identity,
                        group,
                        id,
                        2,
                        [0; 32],
                        &body,
                    )))
                    .await?;
                    self.activate(s, id, state)?;
                }
                2 => {
                    ensure!(opened.payload.len() > 32, Failure(Reason::Protocol));
                    let Some((mut state, nonce, at)) = s.handshakes.remove(&id) else {
                        return Ok(());
                    };
                    if at.elapsed() > Duration::from_secs(10) || opened.payload[..32] != nonce {
                        return Ok(());
                    }
                    state
                        .read_message(&opened.payload[32..], &mut vec![0; 65535])
                        .map_err(|_| Failure(Reason::Transport))?;
                    self.activate(s, id, state)?;
                }
                3 => {
                    let Some(peer) = s.peers.get_mut(&id) else {
                        return Ok(());
                    };
                    if peer.cipher.session != opened.session {
                        return Ok(());
                    }
                    let bytes = peer.cipher.decrypt(opened.payload)?;
                    // Only fresh Noise data can consume memory; shared pressure is not peer misconduct.
                    ensure!(
                        s.memory()
                            .saturating_add(bytes.as_ref().map_or(0, Vec::len))
                            <= BUFFER_BUDGET,
                        Failure(Reason::Transport)
                    );
                    let peer = s.peers.get_mut(&id).ok_or(Failure(Reason::Protocol))?;
                    if let Some(bytes) = bytes {
                        match Inner::decode(bytes)? {
                            Inner::Capabilities(caps) => {
                                self.device.store(|s| s.observe_capabilities(caps))?;
                                peer.negotiated = true;
                            }
                            Inner::Membership(proofs) => {
                                self.update_membership(proofs)?;
                            }
                            Inner::Inventory { done, domains } => {
                                ensure!(peer.negotiated, Failure(Reason::Protocol));
                                self.device.store(|s| s.check_compatible())?;
                                peer.inventory_received |= done;
                                for (domain, heads) in domains {
                                    peer.advertised.insert(domain.clone());
                                    ensure!(
                                        peer.advertised.len() <= MAX_PEER_DOCUMENTS as usize,
                                        Failure(Reason::DocumentQuota)
                                    );
                                    peer.domains.insert(domain.clone());
                                    if self.device.store(|store| store.heads(&domain))? == heads {
                                        peer.synced.insert(domain.clone());
                                        peer.queued.remove(&domain);
                                        peer.pending.retain(|op| {
                                            op.domain != domain || !op.incoming.is_empty()
                                        });
                                    } else {
                                        peer.synced.remove(&domain);
                                        peer.generate(domain)?;
                                    }
                                }
                            }
                            Inner::Sync { domain, bytes } => {
                                ensure!(peer.negotiated, Failure(Reason::Protocol));
                                self.device.store(|s| s.check_compatible())?;
                                ensure!(peer.domains.contains(&domain), Failure(Reason::Protocol));
                                s.enqueue(
                                    id,
                                    Operation {
                                        domain,
                                        incoming: bytes,
                                    },
                                )?;
                            }
                            Inner::Ping(nonce) => {
                                let out =
                                    peer.cipher.encrypt_control(&Inner::Pong(nonce).encode()?)?;
                                ws.send(Frame::Binary(crypto::seal(
                                    &self.device.identity,
                                    group,
                                    id,
                                    3,
                                    peer.cipher.session,
                                    &out,
                                )))
                                .await?;
                            }
                            Inner::Pong(nonce) => {
                                if peer.ping.as_ref().is_some_and(|(value, _)| *value == nonce) {
                                    peer.ping = None;
                                    peer.last_pong = Some(Instant::now());
                                    peer.last_pong_system = Some(SystemTime::now());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = action {
            let code = reason(&error);
            if code == Reason::Transport {
                s.peers.remove(&id);
                return Err(error);
            }
            if local_failure(code) {
                return Err(error);
            }
            self.reject(s, id, code, bans)?;
        }
        Ok(())
    }
    fn inventory_page(&self, peer: &mut Peer) -> Result<()> {
        if !peer.negotiated
            || peer.inventory_sent
            || peer.outbound.len() >= 2
            || peer.operations() + INVENTORY_PAGE > PEER_OPERATIONS
        {
            return Ok(());
        }
        let page = self.device.store(|store| {
            let ids = store.inventory_page(peer.inventory_cursor.as_deref())?;
            ids.into_iter()
                .map(|id| Ok((id.clone(), store.heads(&id)?)))
                .collect::<Result<Vec<_>>>()
        })?;
        let done = page.len() < INVENTORY_PAGE;
        peer.inventory_cursor = page
            .last()
            .map(|(id, _)| id.clone())
            .or_else(|| peer.inventory_cursor.clone());
        for (id, _) in &page {
            peer.domains.insert(id.clone());
        }
        peer.queue(Inner::Inventory {
            done,
            domains: page,
        })?;
        peer.inventory_sent = done;
        Ok(())
    }
    fn peer_action(
        &self,
        s: &mut Session,
        id: [u8; 32],
        bans: &mut BTreeMap<[u8; 32], Ban>,
        action: impl FnOnce(&mut Peer) -> Result<()>,
    ) -> Result<()> {
        let Some(peer) = s.peers.get_mut(&id) else {
            return Ok(());
        };
        match action(peer) {
            Err(error) if reason(&error) == Reason::PeerBadData => {
                self.reject(s, id, Reason::PeerBadData, bans)
            }
            result => result,
        }
    }
    fn announce(
        &self,
        s: &mut Session,
        id: [u8; 32],
        domain: &str,
        heads: &[ChangeHash],
        bans: &mut BTreeMap<[u8; 32], Ban>,
    ) -> Result<()> {
        self.peer_action(s, id, bans, |peer| {
            if !peer.negotiated {
                return Ok(());
            }
            peer.domains.insert(domain.to_owned());
            peer.synced.remove(domain);
            peer.queue(Inner::Inventory {
                done: true,
                domains: vec![(domain.to_owned(), heads.to_vec())],
            })?;
            peer.generate(domain.to_owned())
        })
    }
    fn publish(&self, s: &Session) -> Result<()> {
        let ready = s.peers.values().any(Peer::ready);
        self.status.send_replace(if ready {
            Status::Synced
        } else if s.peers.is_empty() {
            Status::Offline
        } else {
            Status::Connecting
        });
        let mut records = self
            .peers
            .lock()
            .map_err(|_| Failure(Reason::LockPoisoned))?;
        for (&id, p) in &s.peers {
            let record = records.entry(id).or_insert(PeerStatus {
                device: id,
                sending_bad_data: false,
                reason: None,
                last_synced: None,
            });
            if p.domains == p.synced
                && p.inventory_received
                && p.last_pong.is_some_and(|at| at.elapsed() < HEARTBEAT * 2)
            {
                record.last_synced = p.last_pong_system;
                record.sending_bad_data = false;
                record.reason = None;
            }
        }
        Ok(())
    }
    async fn session(
        &self,
        ws: &mut impl Transport,
        ready: serde_json::Value,
        group: &Group,
        bans: &mut BTreeMap<[u8; 32], Ban>,
    ) -> Result<()> {
        self.session_from(ws, ready, group, bans, Session::default())
            .await
    }
    async fn session_from(
        &self,
        ws: &mut impl Transport,
        ready: serde_json::Value,
        group: &Group,
        bans: &mut BTreeMap<[u8; 32], Ban>,
        mut s: Session,
    ) -> Result<()> {
        let mut tasks = tokio::task::JoinSet::new();
        let capacity = Arc::new(tokio::sync::Semaphore::new(2));
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        let mut relay_ping = Instant::now();
        let mut awaiting_relay = false;
        let mut membership_drain = false;
        for p in ready["peers"].as_array().context("missing peers")? {
            if let Some(p) = p.as_str().and_then(decode_key) {
                self.probe(ws, &mut s, p, group).await?;
            }
        }
        loop {
            if let Some(code) = self.fatal()? {
                return Err(Failure(code).into());
            }
            if membership_drain && s.peers.values().all(|p| p.outbound.is_empty()) {
                return Ok(());
            }
            if !membership_drain {
                for id in s.peers.keys().copied().collect::<Vec<_>>() {
                    self.peer_action(&mut s, id, bans, |peer| self.inventory_page(peer))?;
                }
            }
            while s.operations() > TOTAL_OPERATIONS {
                let id = *s
                    .peers
                    .iter()
                    .max_by_key(|(_, p)| p.operations())
                    .ok_or(Failure(Reason::Protocol))?
                    .0;
                self.reject(&mut s, id, Reason::PeerBadData, bans)?;
            }
            ensure!(s.memory() <= BUFFER_BUDGET, Failure(Reason::Transport));
            if !membership_drain {
                self.schedule(&mut s, &mut tasks, &capacity)?;
            }
            self.publish(&s)?;
            let work = s.peers.values().any(|p| {
                !p.outbound.is_empty() || (!p.busy && !p.pending.is_empty()) || !p.inventory_sent
            });
            tokio::select! {
                frame=ws.recv()=>match frame? {
                    Frame::Binary(frame)=>self.envelope(ws,&mut s,frame,group,bans).await?,
                    Frame::Text(text)=>{
                        let Ok(value)=serde_json::from_str::<serde_json::Value>(&text) else{continue;};
                        match value["type"].as_str() {
                            Some("joined")=>{if let Some(id)=value["key"].as_str().and_then(decode_key) {self.probe(ws,&mut s,id,group).await?;}},
                            Some("left")=>{if let Some(id)=value["key"].as_str().and_then(decode_key) {s.peers.remove(&id);}},
                            Some("pong")=>awaiting_relay=false,
                            _=>{},
                        }
                    },
                },
                result=tasks.join_next(),if !tasks.is_empty()=>{
                    let completed=result.ok_or(Failure(Reason::WorkerCrashed))?.map_err(|_|Failure(Reason::WorkerCrashed))?;
                    self.completed(&mut s,completed,bans)?;
                },
                _=self.notify.notified()=>{
                    if let Some(code)=self.fatal()? {return Err(Failure(code).into());}
                    let latest=self.group()?;
                    if latest.members()!=group.members() {
                        for peer in s.peers.values_mut() {peer.queue(Inner::Membership(latest.enrollments.clone()))?;}
                        // Drain through the ordinary receive/send loop before reauthentication.
                        membership_drain = true;
                        continue;
                    }
                    let dirty=std::mem::take(&mut *self.dirty.lock().map_err(|_|Failure(Reason::LockPoisoned))?);
                    for domain in dirty {let heads=self.device.store(|store|store.heads(&domain))?;for id in s.peers.keys().copied().collect::<Vec<_>>() {self.announce(&mut s,id,&domain,&heads,bans)?;}}
                },
                _=timer.tick()=>{
                    let now=Instant::now();
                    if relay_ping.elapsed()>=HEARTBEAT {ensure!(!awaiting_relay,Failure(Reason::Transport));awaiting_relay=true;relay_ping=now;ws.send(Frame::Text("{\"type\":\"ping\"}".into())).await?;}
                    let ids:Vec<_>=s.peers.keys().copied().collect();
                    for id in ids {
                        let peer=s.peers.get_mut(&id).ok_or(Failure(Reason::Protocol))?;
                        if peer.heartbeat_expired() {self.reject(&mut s,id,Reason::Transport,bans)?;continue;}
                        if peer.ping.is_none() && (peer.ping_at.elapsed()>=HEARTBEAT || peer.last_pong.is_none()) {
                            let nonce=random32();let out=peer.cipher.encrypt_control(&Inner::Ping(nonce).encode()?)?;
                            ws.send(Frame::Binary(crypto::seal(&self.device.identity,group,id,3,peer.cipher.session,&out))).await?;
                            peer.ping=Some((nonce,now));peer.ping_at=now;
                        }
                        if peer.born.elapsed()>=Duration::from_secs(60) && peer.last_pong.is_some_and(|at|at.elapsed()<HEARTBEAT*2) {self.healthy_session.store(true,Ordering::Relaxed);}
                    }
                    s.probes.retain(|_,(_,at)|at.elapsed()<Duration::from_secs(10));s.challenges.retain(|_,c|c.at.elapsed()<Duration::from_secs(10));s.handshakes.retain(|_,(_,_,at)|at.elapsed()<Duration::from_secs(10));
                    for id in group.members() {if !s.peers.contains_key(&id) && bans.get(&id).is_none_or(|b|now>=b.until) {self.probe(ws,&mut s,id,group).await?;}}
                },
                _=tokio::time::sleep(Duration::from_millis(1)),if work=>{self.send_one(ws,&mut s,group).await?;},
            }
        }
    }
    fn schedule(
        &self,
        s: &mut Session,
        tasks: &mut tokio::task::JoinSet<Completed>,
        capacity: &Arc<tokio::sync::Semaphore>,
    ) -> Result<()> {
        let ids = s.ordered_ids(s.compute_cursor);
        for id in ids {
            let Ok(permit) = capacity.clone().try_acquire_owned() else {
                break;
            };
            let peer = s.peers.get_mut(&id).ok_or(Failure(Reason::Protocol))?;
            if peer.busy || peer.pending.is_empty() {
                continue;
            }
            let Some(mut worker) = peer.worker.take() else {
                continue;
            };
            let operation = peer.pending.pop_front().ok_or(Failure(Reason::Protocol))?;
            peer.queued.remove(&operation.domain);
            let loaded = peer
                .loaded
                .as_ref()
                .filter(|(id, _)| id == &operation.domain)
                .map(|(_, version)| *version);
            let snapshot = self
                .device
                .store(|store| store.snapshot(&operation.domain, loaded))?;
            let version = snapshot.version;
            let incoming_bytes = operation.incoming.len();
            let request = WorkRequest {
                domain: operation.domain.clone(),
                actor: self.device.identity.public(),
                snapshot,
                incoming: operation.incoming,
                ack_previous: std::mem::take(&mut peer.ack_previous),
            };
            let session = peer.cipher.session;
            let domain = operation.domain;
            peer.busy = true;
            s.compute_cursor = Some(id);
            s.inflight_bytes += incoming_bytes;
            tasks.spawn(async move {
                let _permit = permit;
                let result = worker.request(&request).await;
                Completed {
                    id,
                    session,
                    domain,
                    version,
                    incoming_bytes,
                    worker,
                    result,
                }
            });
        }
        Ok(())
    }
    fn completed(
        &self,
        s: &mut Session,
        completed: Completed,
        bans: &mut BTreeMap<[u8; 32], Ban>,
    ) -> Result<()> {
        let Completed {
            id,
            session,
            domain,
            version,
            incoming_bytes,
            worker,
            result,
        } = completed;
        s.inflight_bytes = s.inflight_bytes.saturating_sub(incoming_bytes);
        let Some(peer) = s.peers.get_mut(&id).filter(|p| p.cipher.session == session) else {
            return Ok(());
        };
        peer.busy = false;
        peer.worker = Some(worker);
        let reply = match result {
            Ok(reply) => reply,
            Err(error) => {
                let code = match reason(&error) {
                    Reason::CorruptDocument => Reason::PeerBadData,
                    code => code,
                };
                if local_failure(code) {
                    return Err(error);
                }
                self.reject(s, id, code, bans)?;
                return Ok(());
            }
        };
        let before = self.device.store(|store| store.heads(&domain))?;
        let persistence = if reply.received {
            self.device.store(|store| {
                store.append(
                    &domain,
                    version,
                    Commit {
                        creator: reply.creator,
                        chunks: reply.delta,
                        heads: reply.heads.clone(),
                        bytes: reply.bytes,
                        compact: reply.compact,
                        orphans: reply.orphans,
                    },
                    &id,
                )
            })
        } else {
            Ok(version)
        };
        let stored = match persistence {
            Ok(version) => version,
            Err(error) if reason(&error) == Reason::ConcurrentEdit => {
                peer.loaded = Some((domain.clone(), version));
                // Retry the hot document before another domain can replace uncommitted work.
                peer.queued.insert(domain.clone());
                peer.pending.push_front(Operation {
                    domain,
                    incoming: Vec::new(),
                });
                return Ok(());
            }
            Err(error)
                if matches!(
                    reason(&error),
                    Reason::DocumentTooLarge | Reason::DocumentQuota | Reason::StoreFull
                ) =>
            {
                self.reject(s, id, reason(&error), bans)?;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        peer.loaded = Some((domain.clone(), stored));
        peer.ack_previous = reply.received;
        if reply.received
            && let Err(error) = peer.generate(domain.clone())
        {
            self.reject(s, id, reason(&error), bans)?;
            return Ok(());
        }
        if reply.caught_up {
            peer.synced.insert(domain.clone());
        } else {
            peer.synced.remove(&domain);
        }
        if reply.received && before != reply.heads {
            let revision = self.applied.fetch_add(1, Ordering::Relaxed) + 1;
            self.updates.send_replace(Some(Applied {
                revision,
                domain: domain.clone(),
                heads: reply.heads.clone(),
                at: std::time::Instant::now(),
            }));
            for other in s.peers.keys().copied().collect::<Vec<_>>() {
                if other != id {
                    self.announce(s, other, &domain, &reply.heads, bans)?;
                }
            }
        }
        if !reply.outgoing.is_empty() {
            s.peers
                .get_mut(&id)
                .ok_or(Failure(Reason::Protocol))?
                .queue(Inner::Sync {
                    domain,
                    bytes: reply.outgoing,
                })?;
        }
        ensure!(s.memory() <= BUFFER_BUDGET, Failure(Reason::Transport));
        Ok(())
    }
    async fn send_one(
        &self,
        ws: &mut impl Transport,
        s: &mut Session,
        group: &Group,
    ) -> Result<()> {
        for id in s.ordered_ids(s.send_cursor) {
            let peer = s.peers.get_mut(&id).ok_or(Failure(Reason::Protocol))?;
            let Some((message, offset)) = peer.outbound.front_mut() else {
                continue;
            };
            let (encrypted, end) = peer.cipher.encrypt_fragment(message, *offset)?;
            *offset = end;
            if end == message.len() {
                peer.outbound.pop_front();
            }
            ws.send(Frame::Binary(crypto::seal(
                &self.device.identity,
                group,
                id,
                3,
                peer.cipher.session,
                &encrypted,
            )))
            .await?;
            s.send_cursor = Some(id);
            break;
        }
        Ok(())
    }
}
struct Operation {
    domain: String,
    incoming: Vec<u8>,
}
struct Peer {
    negotiated: bool,
    cipher: Cipher,
    worker: Option<Worker>,
    busy: bool,
    loaded: Option<(String, i64)>,
    ack_previous: bool,
    domains: BTreeSet<String>,
    advertised: BTreeSet<String>,
    synced: BTreeSet<String>,
    pending: VecDeque<Operation>,
    queued: BTreeSet<String>,
    outbound: VecDeque<(Vec<u8>, usize)>,
    inventory_cursor: Option<String>,
    inventory_sent: bool,
    inventory_received: bool,
    ping: Option<([u8; 32], Instant)>,
    last_pong: Option<Instant>,
    last_pong_system: Option<SystemTime>,
    ping_at: Instant,
    born: Instant,
}
impl Peer {
    fn operations(&self) -> usize {
        self.pending.len() + usize::from(self.busy)
    }
    fn memory(&self) -> usize {
        self.cipher.pending_bytes()
            + usize::from(self.busy) * OPERATION_COST
            + self
                .pending
                .iter()
                .map(|o| o.incoming.len().max(OPERATION_COST) + o.domain.len())
                .sum::<usize>()
            + self.outbound.iter().map(|(b, _)| b.len()).sum::<usize>()
    }
    fn ready(&self) -> bool {
        self.inventory_sent
            && self.inventory_received
            && self.domains == self.synced
            && self.pending.is_empty()
            && !self.busy
            && self
                .last_pong
                .is_some_and(|at| at.elapsed() < HEARTBEAT * 2)
    }
    fn heartbeat_expired(&self) -> bool {
        self.ping
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= HEARTBEAT * 2)
    }
    fn queue(&mut self, message: Inner) -> Result<()> {
        self.outbound.push_back((message.encode()?, 0));
        Ok(())
    }
    fn generate(&mut self, domain: String) -> Result<()> {
        if !self.queued.contains(&domain) {
            ensure!(
                self.operations() < PEER_OPERATIONS,
                Failure(Reason::PeerBadData)
            );
            self.queued.insert(domain.clone());
            self.pending.push_back(Operation {
                domain,
                incoming: Vec::new(),
            });
        }
        Ok(())
    }
}
struct Challenge {
    probe: [u8; 32],
    nonce: [u8; 32],
    at: Instant,
}
struct Ban {
    until: Instant,
    strikes: u32,
    decayed_at: Instant,
}
impl Ban {
    fn strike(&mut self, now: Instant) {
        use rand::Rng;
        let elapsed = now.saturating_duration_since(self.decayed_at);
        let decay = (elapsed.as_secs() / 1800).min(u32::MAX as u64) as u32;
        if decay > 0 {
            self.strikes = self.strikes.saturating_sub(decay);
            self.decayed_at += Duration::from_secs(u64::from(decay) * 1800);
        }
        self.strikes = self.strikes.saturating_add(1);
        let ceiling = (100u64 << self.strikes.min(12)).min(300_000);
        self.until =
            now + Duration::from_millis(rand::thread_rng().gen_range(ceiling / 2..=ceiling));
    }
}
impl Default for Ban {
    fn default() -> Self {
        Self {
            until: Instant::now(),
            strikes: 0,
            decayed_at: Instant::now(),
        }
    }
}
#[derive(Default)]
struct Session {
    peers: BTreeMap<[u8; 32], Peer>,
    probes: BTreeMap<[u8; 32], ([u8; 32], Instant)>,
    challenges: BTreeMap<[u8; 32], Challenge>,
    handshakes: BTreeMap<[u8; 32], (snow::HandshakeState, [u8; 32], Instant)>,
    compute_cursor: Option<[u8; 32]>,
    send_cursor: Option<[u8; 32]>,
    inflight_bytes: usize,
}
impl Session {
    fn operations(&self) -> usize {
        self.peers.values().map(Peer::operations).sum()
    }
    fn enqueue(&mut self, id: [u8; 32], operation: Operation) -> Result<()> {
        ensure!(
            !operation.incoming.is_empty() && self.operations() < TOTAL_OPERATIONS,
            Failure(Reason::PeerBadData)
        );
        let peer = self.peers.get_mut(&id).ok_or(Failure(Reason::Protocol))?;
        ensure!(
            peer.operations() < PEER_OPERATIONS,
            Failure(Reason::PeerBadData)
        );
        peer.pending.push_back(operation);
        Ok(())
    }

    fn memory(&self) -> usize {
        self.inflight_bytes + self.peers.values().map(Peer::memory).sum::<usize>()
    }
    fn ordered_ids(&self, after: Option<[u8; 32]>) -> Vec<[u8; 32]> {
        let ids: Vec<_> = self.peers.keys().copied().collect();
        ids.iter()
            .filter(|id| after.is_none_or(|a| **id > a))
            .chain(ids.iter().filter(|id| after.is_some_and(|a| **id <= a)))
            .copied()
            .collect()
    }
}
struct Completed {
    id: [u8; 32],
    session: [u8; 32],
    domain: String,
    version: i64,
    incoming_bytes: usize,
    worker: Worker,
    result: Result<WorkResponse>,
}
fn decode_key(value: &str) -> Option<[u8; 32]> {
    hex::decode(value).ok()?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Sink;
    #[async_trait::async_trait]
    impl Transport for Sink {
        async fn send(&mut self, _: Frame) -> Result<()> {
            Ok(())
        }
        async fn recv(&mut self) -> Result<Frame> {
            std::future::pending().await
        }
        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }
    fn client(device: Arc<Device>, group: Group) -> Result<SyncClient> {
        Ok(SyncClient {
            device,
            group: Mutex::new(group),
            relay: RelayUrl::local("ws://127.0.0.1:8787")?,
            pool: Pool::new(std::env::current_exe()?)?,
            notify: Notify::new(),
            dirty: Mutex::new(BTreeSet::new()),
            fatal: Mutex::new(None),
            documents: Mutex::new(BTreeMap::new()),
            healthy_session: AtomicBool::new(false),
            #[cfg(test)]
            retry_ceilings: Mutex::new(Vec::new()),
            connections: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            updates: watch::channel(None).0,
            status: watch::channel(Status::Offline).0,
            peers: Mutex::new(BTreeMap::new()),
        })
    }
    fn peer(group: &Group, a: [u8; 32], b: [u8; 32]) -> Result<Peer> {
        Ok(peer_pair(group, a, b)?.0)
    }
    fn peer_pair(group: &Group, a: [u8; 32], b: [u8; 32]) -> Result<(Peer, Cipher)> {
        let fresh = [8; 64];
        let mut first = crypto::handshake(group, a, b, true, &fresh)?;
        let mut second = crypto::handshake(group, b, a, false, &fresh)?;
        let mut buffer = [0; 1024];
        let mut output = [0; 1024];
        let n = first.write_message(&[], &mut buffer)?;
        second.read_message(&buffer[..n], &mut output)?;
        let n = second.write_message(&[], &mut buffer)?;
        first.read_message(&buffer[..n], &mut output)?;
        Ok((
            Peer {
                negotiated: true,
                cipher: Cipher::from_handshake(second)?,
                worker: None,
                busy: false,
                loaded: None,
                ack_previous: false,
                domains: BTreeSet::new(),
                advertised: BTreeSet::new(),
                synced: BTreeSet::new(),
                pending: VecDeque::new(),
                queued: BTreeSet::new(),
                outbound: VecDeque::new(),
                inventory_cursor: None,
                inventory_sent: true,
                inventory_received: true,
                ping: None,
                last_pong: Some(Instant::now()),
                last_pong_system: Some(SystemTime::now()),
                ping_at: Instant::now(),
                born: Instant::now(),
            },
            Cipher::from_handshake(first)?,
        ))
    }
    #[tokio::test(start_paused = true)]
    async fn f8_real_heartbeat_loop_expires_peer_despite_relay_pongs() -> Result<()> {
        struct RelayPongs;
        #[async_trait::async_trait]
        impl Transport for RelayPongs {
            async fn send(&mut self, _: Frame) -> Result<()> {
                Ok(())
            }
            async fn recv(&mut self) -> Result<Frame> {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(Frame::Text("{\"type\":\"pong\"}".into()))
            }
            async fn close(&mut self) -> Result<()> {
                Ok(())
            }
        }
        let dir = tempfile::tempdir()?;
        let owner = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
        let other = Device::open(&dir.path().join("peer.sqlite"))?;
        let mut group = Group::new(&owner.identity);
        group.enroll(&owner.identity, other.identity.public());
        let client = client(owner.clone(), group.clone())?;
        let mut session = Session::default();
        session.peers.insert(
            other.identity.public(),
            peer(&group, other.identity.public(), owner.identity.public())?,
        );
        let mut bans = BTreeMap::new();
        let mut relay = RelayPongs;
        tokio::select! {
            result=client.session_from(&mut relay,serde_json::json!({"peers":[]}),&group,&mut bans,session)=>{result?;anyhow::bail!("session exited before heartbeat assertion");},
            result=async {
                // The first ping is sent after one heartbeat and expires two heartbeats later.
                tokio::time::timeout(HEARTBEAT * 3 + Duration::from_secs(5),async {
                    loop {
                        if client.peer_statuses()?.iter().any(|p|p.device==other.identity.public() && p.reason==Some(Reason::Transport)) {break;}
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    Ok::<_,anyhow::Error>(())
                }).await?
            }=>result?,
        }
        assert_ne!(*client.status.borrow(), Status::Synced);
        Ok(())
    }
    #[tokio::test]
    async fn f2_busy_inventory_page_boundary_and_dirty_queue_are_peer_local() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let owner = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
        let group = Group::new(&owner.identity);
        let client = client(owner.clone(), group.clone())?;
        for n in 0..INVENTORY_PAGE {
            owner.store(|store| store.edit(&format!("meeting:z{n:03}"), |_| Ok(())))?;
        }
        let bad = [2; 32];
        let good = [3; 32];
        let mut delayed = peer(&group, bad, owner.identity.public())?;
        delayed.busy = true;
        delayed.inventory_sent = false;
        for n in 0..PEER_OPERATIONS - INVENTORY_PAGE {
            delayed.generate(format!("meeting:a{n:03}"))?;
        }
        client.inventory_page(&mut delayed)?;
        assert!(delayed.inventory_cursor.is_none());
        assert!(delayed.outbound.is_empty());
        assert_eq!(delayed.operations(), INVENTORY_PAGE + 1);
        let operation = delayed.pending.pop_front().expect("queued work");
        delayed.queued.remove(&operation.domain);
        client.inventory_page(&mut delayed)?;
        assert!(delayed.inventory_cursor.is_some());
        assert_eq!(delayed.operations(), INVENTORY_PAGE);
        for n in 0..INVENTORY_PAGE {
            delayed.generate(format!("meeting:b{n:03}"))?;
        }
        assert_eq!(delayed.operations(), PEER_OPERATIONS);

        delayed.inventory_sent = true;
        let mut session = Session::default();
        session.peers.insert(bad, delayed);
        session
            .peers
            .insert(good, peer(&group, good, owner.identity.public())?);
        client
            .dirty
            .lock()
            .expect("dirty lock")
            .insert("tasks".into());
        client.notify.notify_one();
        let mut bans = BTreeMap::new();
        let mut sink = Sink;
        tokio::select! {
            result = client.session_from(&mut sink, serde_json::json!({"peers":[]}), &group, &mut bans, session) => {
                result?;
                anyhow::bail!("queue overflow unwound shared session");
            },
            result = async {
                tokio::time::timeout(Duration::from_secs(2), async {
                    loop {
                        let statuses = client.peer_statuses()?;
                        if statuses.iter().any(|p| p.device == bad && p.reason == Some(Reason::PeerBadData)) {
                            assert!(statuses.iter().any(|p| p.device == good && p.reason.is_none()));
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                    Ok::<_, anyhow::Error>(())
                }).await?
            } => result?,
        }
        assert_eq!(bans.len(), 1);
        assert!(bans.contains_key(&bad));
        assert_eq!(client.fatal()?, None);
        Ok(())
    }
    #[tokio::test]
    async fn f2_encrypted_empty_and_tiny_floods_reject_only_the_delayed_peer() -> Result<()> {
        for empty in [true, false] {
            let dir = tempfile::tempdir()?;
            let owner = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
            let sender = Device::open(&dir.path().join("peer.sqlite"))?;
            let mut group = Group::new(&owner.identity);
            group.enroll(&owner.identity, sender.identity.public());
            let client = client(owner.clone(), group.clone())?;
            let id = sender.identity.public();
            let (mut peer, mut cipher) = peer_pair(&group, id, owner.identity.public())?;
            peer.busy = true;
            peer.domains.insert("tasks".into());
            let session_id = peer.cipher.session;
            let mut session = Session::default();
            session.peers.insert(id, peer);
            let mut bans = BTreeMap::new();
            for _ in 0..PEER_OPERATIONS * 2 {
                let message = Inner::Sync {
                    domain: "tasks".into(),
                    bytes: if empty { Vec::new() } else { vec![0x42] },
                }
                .encode()?;
                let (encrypted, _) = cipher.encrypt_fragment(&message, 0)?;
                let frame = crypto::seal(
                    &sender.identity,
                    &group,
                    owner.identity.public(),
                    3,
                    session_id,
                    &encrypted,
                );
                client
                    .envelope(&mut Sink, &mut session, frame, &group, &mut bans)
                    .await?;
                assert!(session.operations() <= PEER_OPERATIONS);
            }
            assert!(!session.peers.contains_key(&id));
            assert!(bans.contains_key(&id));
            assert!(
                client
                    .peer_statuses()?
                    .iter()
                    .any(|p| p.device == id && p.reason == Some(Reason::PeerBadData))
            );
            assert_eq!(client.fatal()?, None);
        }
        Ok(())
    }
    #[tokio::test]
    async fn f1_peer_quota_completion_keeps_other_peers_and_owner_running() -> Result<()> {
        use crate::store::Limits;
        let dir = tempfile::tempdir()?;
        let mut store = Store::open_with_limits(
            &dir.path().join("owner.sqlite"),
            b"owner",
            Limits {
                document_bytes: 1024,
                ..Limits::default()
            },
        )?;
        let identity = Identity::load_or_create(&store)?;
        store.set_actor(&identity.public());
        let owner = Arc::new(Device {
            store: Mutex::new(store),
            identity,
        });
        let group = Group::new(&owner.identity);
        let client = client(owner.clone(), group.clone())?;
        let id = [2; 32];
        let other = [3; 32];
        let mut session = Session::default();
        session
            .peers
            .insert(id, peer(&group, id, owner.identity.public())?);
        session
            .peers
            .insert(other, peer(&group, other, owner.identity.public())?);
        let completed = Completed {
            id,
            session: session.peers[&id].cipher.session,
            domain: "tasks".into(),
            version: 0,
            incoming_bytes: 1,
            worker: Pool::adjacent()?.spawn()?,
            result: Ok(WorkResponse {
                creator: Some(id),
                delta: Vec::new(),
                heads: Vec::new(),
                bytes: 1000,
                compact: None,
                orphans: 0,
                outgoing: Vec::new(),
                caught_up: false,
                received: true,
            }),
        };
        client.completed(&mut session, completed, &mut BTreeMap::new())?;
        assert!(!session.peers.contains_key(&id));
        assert!(session.peers.contains_key(&other));
        assert_eq!(client.fatal()?, None);
        client.edit("tasks", |d| {
            crate::model::create(d, "one", "owner still edits")
        })?;
        Ok(())
    }
    #[tokio::test]
    async fn creator_completion_persists_author_separately_from_relay() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("owner.sqlite");
        let owner = Arc::new(Device::open(&path)?);
        let group = Group::new(&owner.identity);
        let client = client(owner.clone(), group.clone())?;
        let relay = [2; 32];
        let author = [3; 32];
        let mut session = Session::default();
        session
            .peers
            .insert(relay, peer(&group, relay, owner.identity.public())?);
        let completed = Completed {
            id: relay,
            session: session.peers[&relay].cipher.session,
            domain: "tasks".into(),
            version: 0,
            incoming_bytes: 1,
            worker: Pool::adjacent()?.spawn()?,
            result: Ok(WorkResponse {
                creator: Some(author),
                delta: Vec::new(),
                heads: Vec::new(),
                bytes: 0,
                compact: None,
                orphans: 0,
                outgoing: Vec::new(),
                caught_up: false,
                received: true,
            }),
        };
        client.completed(&mut session, completed, &mut BTreeMap::new())?;
        let db = rusqlite::Connection::open(path)?;
        let creator: Vec<u8> =
            db.query_row("SELECT creator FROM doc_meta WHERE id='tasks'", [], |r| {
                r.get(0)
            })?;
        let sender: Vec<u8> =
            db.query_row("SELECT sender FROM doc_senders WHERE id='tasks'", [], |r| {
                r.get(0)
            })?;
        assert_eq!(creator, author);
        assert_eq!(sender, relay);
        Ok(())
    }
    #[tokio::test]
    async fn worker_corrupt_document_rejects_peer_without_stopping_owner_or_others() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let owner = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
        let group = Group::new(&owner.identity);
        let client = client(owner.clone(), group.clone())?;
        let bad = [2; 32];
        let good = [3; 32];
        let mut session = Session::default();
        session
            .peers
            .insert(bad, peer(&group, bad, owner.identity.public())?);
        session
            .peers
            .insert(good, peer(&group, good, owner.identity.public())?);
        let completed = Completed {
            id: bad,
            session: session.peers[&bad].cipher.session,
            domain: "tasks".into(),
            version: 0,
            incoming_bytes: 1,
            worker: Pool::adjacent()?.spawn()?,
            result: Err(Failure(Reason::CorruptDocument).into()),
        };
        let mut bans = BTreeMap::new();
        client.completed(&mut session, completed, &mut bans)?;
        assert!(!session.peers.contains_key(&bad));
        assert!(session.peers.contains_key(&good));
        assert_eq!(bans.len(), 1);
        assert_eq!(client.fatal()?, None);
        assert!(client.peer_statuses()?.iter().any(|p| p.device == bad
            && p.reason == Some(Reason::PeerBadData)
            && p.sending_bad_data));
        client.edit("tasks", |d| {
            crate::model::create(d, "one", "owner still edits")
        })?;
        assert!(
            local_failure(Reason::CorruptDocument),
            "actual local corruption remains fatal"
        );
        Ok(())
    }
    #[tokio::test]
    async fn f8_fatal_errors_stop_the_actual_run_loop() -> Result<()> {
        for code in [
            Reason::LocalStore,
            Reason::CorruptDocument,
            Reason::StoreFull,
            Reason::LockPoisoned,
        ] {
            let dir = tempfile::tempdir()?;
            let device = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
            let client = Arc::new(client(device.clone(), Group::new(&device.identity))?);
            let error = client
                .edit("tasks", |_| Err::<(), _>(Failure(code).into()))
                .expect_err("local failure");
            assert_eq!(reason(&error), code);
            let (_stop, rx) = watch::channel(false);
            let error = client.clone().run(rx).await.expect_err("run must stop");
            assert_eq!(reason(&error), code);
            assert_eq!(*client.status.borrow(), Status::Failed(code));
            assert_eq!(client.connections.load(Ordering::Relaxed), 0);
        }
        Ok(())
    }
    #[tokio::test(start_paused = true)]
    async fn f8_short_authenticated_sessions_do_not_reset_run_backoff() -> Result<()> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(error) => return Err::<(), anyhow::Error>(error.into()),
                };
                let mut ws = tokio_tungstenite::accept_async(stream).await?;
                ws.send(Message::Text(
                    serde_json::json!({"type":"challenge","nonce":"c".repeat(64)})
                        .to_string()
                        .into(),
                ))
                .await?;
                let auth = ws.next().await.context("auth missing")??;
                ensure!(auth.is_text(), "expected auth");
                ws.send(Message::Text("{\"type\":\"ready\",\"peers\":[]}".into()))
                    .await?;
                ws.close(None).await?;
            }
        });
        let dir = tempfile::tempdir()?;
        let device = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
        let mut instance = client(device.clone(), Group::new(&device.identity))?;
        instance.relay = RelayUrl::local(&format!("ws://{address}"))?;
        let client = Arc::new(instance);
        let (stop, rx) = watch::channel(false);
        let run = tokio::spawn(client.clone().run(rx));
        tokio::time::timeout(Duration::from_secs(3600), async {
            loop {
                if client.retry_ceilings.lock().expect("trace").len() >= 14 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        stop.send(true)?;
        run.await??;
        server.abort();
        let ceilings = client.retry_ceilings.lock().expect("trace");
        for (index, ceiling) in ceilings.iter().take(14).enumerate() {
            assert_eq!(
                *ceiling,
                Duration::from_millis((100u64 << (index + 1)).min(300_000))
            );
        }
        assert!(client.connections.load(Ordering::Relaxed) >= 14);
        Ok(())
    }
    #[test]
    fn f7_membership_dpapi_failure_is_local_and_never_peer_bad_data() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("owner.sqlite");
        let device = Arc::new(Device::open(&path)?);
        let group = Group::new(&device.identity);
        device.merge_group(&group)?;
        let db = rusqlite::Connection::open(path)?;
        db.execute(
            "UPDATE protected SET data=x'00' WHERE id LIKE 'group:%'",
            [],
        )?;
        let client = client(device, group.clone())?;
        let error = client
            .update_membership(group.enrollments.clone())
            .expect_err("invalid DPAPI blob");
        assert_eq!(reason(&error), Reason::LocalStore);
        assert!(client.peer_statuses()?.is_empty());
        Ok(())
    }
    #[test]
    fn f4_strikes_survive_a_healthy_minute_and_decay_slowly() {
        let mut ban = Ban::default();
        let start = ban.decayed_at;
        ban.strike(start);
        ban.strike(start + Duration::from_secs(61));
        assert_eq!(ban.strikes, 2);
        ban.strike(start + Duration::from_secs(1801));
        assert_eq!(ban.strikes, 2);
        assert!(ban.until > start + Duration::from_secs(1801));
    }
    #[test]
    fn f2_empty_and_tiny_floods_stay_bounded_while_worker_is_delayed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let owner = Device::open(&dir.path().join("owner.sqlite"))?;
        let group = Group::new(&owner.identity);
        let mut session = Session::default();
        for n in 1..=8u8 {
            let id = [n; 32];
            let mut p = peer(&group, id, owner.identity.public())?;
            p.busy = true;
            session.peers.insert(id, p);
        }
        for n in 1..=8u8 {
            let id = [n; 32];
            for _ in 0..2000 {
                let empty = session.enqueue(
                    id,
                    Operation {
                        domain: "tasks".into(),
                        incoming: Vec::new(),
                    },
                );
                assert_eq!(
                    reason(&empty.expect_err("empty network payload")),
                    Reason::PeerBadData
                );
                let _ = session.enqueue(
                    id,
                    Operation {
                        domain: "tasks".into(),
                        incoming: vec![0x42],
                    },
                );
                assert!(session.operations() <= TOTAL_OPERATIONS);
                assert!(session.peers[&id].pending.len() < PEER_OPERATIONS);
            }
        }
        let queued: usize = session.peers.values().map(|p| p.pending.len()).sum();
        assert!(queued > 0);
        assert_eq!(session.operations(), TOTAL_OPERATIONS);
        assert!(session.memory() >= queued * OPERATION_COST);
        Ok(())
    }
    #[tokio::test]
    async fn f1_local_document_quota_is_reported_without_stopping_run() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let device = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
        let client = Arc::new(client(device.clone(), Group::new(&device.identity))?);
        let error = client
            .edit("tasks", |_| {
                Err::<(), _>(Failure(Reason::DocumentTooLarge).into())
            })
            .expect_err("local quota");
        assert_eq!(reason(&error), Reason::DocumentTooLarge);
        assert_eq!(
            client.document_status("tasks")?,
            Some(Reason::DocumentTooLarge)
        );
        assert_eq!(client.fatal()?, None);
        let (stop, receiver) = watch::channel(true);
        client.clone().run(receiver).await?;
        drop(stop);
        client.edit("tasks", |d| {
            crate::model::create(d, "one", "owner can edit")
        })?;
        assert_eq!(client.document_status("tasks")?, None);
        Ok(())
    }
    #[tokio::test]
    async fn c2_c10_replayed_initiation_keeps_live_session_and_bad_peer_is_isolated() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let a = Arc::new(Device::open(&dir.path().join("a.sqlite"))?);
        let b = Arc::new(Device::open(&dir.path().join("b.sqlite"))?);
        let (sender, receiver) = if a.identity.public() < b.identity.public() {
            (a, b)
        } else {
            (b, a)
        };
        let c = Device::open(&dir.path().join("c.sqlite"))?;
        let mut group = Group::new(&receiver.identity);
        group.enroll(&receiver.identity, sender.identity.public());
        group.enroll(&receiver.identity, c.identity.public());
        let client = client(receiver.clone(), group.clone())?;
        let id = sender.identity.public();
        let other = c.identity.public();
        let mut session = Session::default();
        let mut bans = BTreeMap::new();
        session
            .peers
            .insert(id, peer(&group, id, receiver.identity.public())?);
        session
            .peers
            .insert(other, peer(&group, other, receiver.identity.public())?);
        let old_session = session.peers[&id].cipher.session;
        session.challenges.insert(
            id,
            Challenge {
                probe: [6; 32],
                nonce: [9; 32],
                at: Instant::now(),
            },
        );
        let stale = crypto::seal(
            &sender.identity,
            &group,
            receiver.identity.public(),
            1,
            [0; 32],
            &[7; 64],
        );
        client
            .envelope(&mut Sink, &mut session, stale, &group, &mut bans)
            .await?;
        assert_eq!(session.peers[&id].cipher.session, old_session);
        assert_eq!(session.challenges[&id].nonce, [9; 32]);
        assert!(bans.is_empty());
        let bad = crypto::seal(
            &sender.identity,
            &group,
            receiver.identity.public(),
            3,
            old_session,
            &[0; 20],
        );
        let mut forged = bad.clone();
        let last = forged.len() - 1;
        forged[last] ^= 1;
        client
            .envelope(&mut Sink, &mut session, forged, &group, &mut bans)
            .await?;
        assert!(session.peers.contains_key(&id));
        assert!(bans.is_empty());
        let error = client
            .envelope(&mut Sink, &mut session, bad, &group, &mut bans)
            .await
            .expect_err("relay replay/decrypt failure reconnects");
        assert_eq!(reason(&error), Reason::Transport);
        assert!(!session.peers.contains_key(&id));
        assert!(session.peers.contains_key(&other));
        assert!(bans.is_empty());
        assert!(client.peer_statuses()?.iter().all(|p| !p.sending_bad_data));
        Ok(())
    }
    #[test]
    fn c10_relay_pong_cannot_keep_a_data_blackhole_synced() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let a = Device::open(&dir.path().join("a.sqlite"))?;
        let b = Device::open(&dir.path().join("b.sqlite"))?;
        let mut group = Group::new(&a.identity);
        group.enroll(&a.identity, b.identity.public());
        let mut p = peer(&group, a.identity.public(), b.identity.public())?;
        assert!(p.ready());
        p.last_pong = Some(
            Instant::now()
                .checked_sub(HEARTBEAT * 3)
                .expect("fixture clock supports the heartbeat age"),
        );
        p.ping = Some((
            [1; 32],
            Instant::now()
                .checked_sub(HEARTBEAT * 3)
                .expect("fixture clock supports the heartbeat age"),
        ));
        assert!(!p.ready());
        assert!(p.heartbeat_expired());
        Ok(())
    }
    #[tokio::test]
    async fn f7_replay_near_global_limit_never_strikes_an_honest_peer() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let owner = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
        let sender = Device::open(&dir.path().join("sender.sqlite"))?;
        let id = sender.identity.public();
        let mut group = Group::new(&owner.identity);
        group.enroll(&owner.identity, id);
        let client = client(owner.clone(), group.clone())?;
        for mode in [
            "inactive",
            "stale-session",
            "replayed-noise",
            "fresh-pressure",
        ] {
            let (peer, mut cipher) = peer_pair(&group, id, owner.identity.public())?;
            let session_id = peer.cipher.session;
            let mut session = Session::default();
            if mode != "inactive" {
                session.peers.insert(id, peer);
            }
            let encrypted = cipher.encrypt_control(&Inner::Pong([1; 32]).encode()?)?;
            let frame = crypto::seal(
                &sender.identity,
                &group,
                owner.identity.public(),
                3,
                if mode == "stale-session" {
                    [0; 32]
                } else {
                    session_id
                },
                &encrypted,
            );
            let mut bans = BTreeMap::new();
            if mode == "replayed-noise" {
                client
                    .envelope(&mut Sink, &mut session, frame.clone(), &group, &mut bans)
                    .await?;
            }
            session.inflight_bytes = BUFFER_BUDGET;
            let result = client
                .envelope(&mut Sink, &mut session, frame, &group, &mut bans)
                .await;
            if ["inactive", "stale-session"].contains(&mode) {
                result?;
                assert_eq!(session.peers.contains_key(&id), mode == "stale-session");
            } else {
                assert_eq!(
                    reason(&result.expect_err("reconnect without blame")),
                    Reason::Transport
                );
            }
            assert!(bans.is_empty(), "{mode}");
            assert_eq!(client.rejected.load(Ordering::Relaxed), 0);
            assert!(client.peer_statuses()?.iter().all(|p| !p.sending_bad_data));
        }
        let mut session = Session {
            inflight_bytes: BUFFER_BUDGET + 1,
            ..Session::default()
        };
        session
            .peers
            .insert(id, peer(&group, id, owner.identity.public())?);
        let mut bans = BTreeMap::new();
        let error = client
            .session_from(
                &mut Sink,
                serde_json::json!({"peers":[]}),
                &group,
                &mut bans,
                session,
            )
            .await
            .expect_err("shared pressure reconnects");
        assert_eq!(reason(&error), Reason::Transport);
        assert!(bans.is_empty());
        assert_eq!(client.rejected.load(Ordering::Relaxed), 0);
        Ok(())
    }
    #[tokio::test]
    async fn inventory_heads_skip_cold_work_across_small_pages_and_queue_differences() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("owner.sqlite");
        let owner = Arc::new(Device::open(&path)?);
        let sender = Device::open(&dir.path().join("sender.sqlite"))?;
        let mut group = Group::new(&owner.identity);
        let id = sender.identity.public();
        group.enroll(&owner.identity, id);
        for domain in ["tasks", "memory"] {
            owner.store(|store| {
                store.edit(domain, |d| crate::model::create(d, "one", "synthetic"))
            })?;
        }
        let heads = owner.store(|store| Ok([store.heads("tasks")?, store.heads("memory")?]))?;
        let client = client(owner.clone(), group.clone())?;
        let (mut peer, mut cipher) = peer_pair(&group, id, owner.identity.public())?;
        peer.inventory_sent = false;
        peer.inventory_received = false;
        client.inventory_page(&mut peer)?;
        assert!(
            peer.pending.is_empty(),
            "advertising alone must not cold-load documents"
        );
        peer.generate("tasks".into())?;
        let mut session = Session::default();
        session.peers.insert(id, peer);
        let mut bans = BTreeMap::new();
        for (index, domain) in ["tasks", "memory"].into_iter().enumerate() {
            let message = Inner::Inventory {
                done: index == 1,
                domains: vec![(domain.into(), heads[index].clone())],
            }
            .encode()?;
            let (encrypted, _) = cipher.encrypt_fragment(&message, 0)?;
            let frame = crypto::seal(
                &sender.identity,
                &group,
                owner.identity.public(),
                3,
                cipher.session,
                &encrypted,
            );
            client
                .envelope(&mut Sink, &mut session, frame, &group, &mut bans)
                .await?;
            assert!(session.peers[&id].pending.is_empty());
            assert_eq!(session.peers[&id].loaded, None);
        }
        assert_eq!(
            session.peers[&id].synced,
            BTreeSet::from(["tasks".into(), "memory".into()])
        );
        assert!(session.peers[&id].inventory_received);
        let message = Inner::Inventory {
            done: true,
            domains: vec![("tasks".into(), Vec::new())],
        }
        .encode()?;
        let (encrypted, _) = cipher.encrypt_fragment(&message, 0)?;
        let frame = crypto::seal(
            &sender.identity,
            &group,
            owner.identity.public(),
            3,
            cipher.session,
            &encrypted,
        );
        client
            .envelope(&mut Sink, &mut session, frame, &group, &mut bans)
            .await?;
        assert_eq!(session.peers[&id].pending.len(), 1);
        assert_eq!(session.peers[&id].pending[0].domain, "tasks");
        assert!(!session.peers[&id].synced.contains("tasks"));
        Ok(())
    }
    #[tokio::test]
    async fn c2_poisoned_store_has_a_reason_code_and_is_fatal() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let device = Arc::new(Device::open(&dir.path().join("a.sqlite"))?);
        let client = Arc::new(client(device.clone(), Group::new(&device.identity))?);
        let cloned = device.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = cloned.store.lock().expect("fresh lock");
                panic!("synthetic poison");
            })
            .join()
            .is_err()
        );
        let error = device.store(|_| Ok(())).expect_err("poison must fail");
        assert_eq!(reason(&error), Reason::LockPoisoned);
        assert!(local_failure(reason(&error)));
        let edit_error = client
            .edit("tasks", |_| Ok(()))
            .expect_err("poisoned store edit");
        assert_eq!(reason(&edit_error), Reason::LockPoisoned);
        let (_stop, rx) = watch::channel(false);
        let run_error = client
            .clone()
            .run(rx)
            .await
            .expect_err("actual run is fatal");
        assert_eq!(reason(&run_error), Reason::LockPoisoned);
        assert_eq!(
            *client.status.borrow(),
            Status::Failed(Reason::LockPoisoned)
        );
        for code in [
            Reason::LocalStore,
            Reason::CorruptDocument,
            Reason::StoreFull,
        ] {
            assert!(local_failure(code));
        }
        Ok(())
    }
    #[tokio::test]
    async fn qualification_hostile_frames_preserve_heads_and_allow_a_fresh_session() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let owner = Arc::new(Device::open(&dir.path().join("owner.sqlite"))?);
        let sender = Device::open(&dir.path().join("sender.sqlite"))?;
        owner.store(|s| {
            s.edit("tasks", |d| {
                crate::model::create(d, "retained", "Synthetic retained content")
            })
        })?;
        let mut group = Group::new(&owner.identity);
        group.enroll(&owner.identity, sender.identity.public());
        let client = client(owner.clone(), group.clone())?;
        let heads = owner.store(|s| s.heads("tasks"))?;
        let id = sender.identity.public();
        for mode in [
            "replay",
            "truncated",
            "oversized",
            "wrong-group",
            "expired-session",
        ] {
            let (mut peer, mut cipher) = peer_pair(&group, id, owner.identity.public())?;
            peer.domains.insert("tasks".into());
            let session_id = peer.cipher.session;
            let mut session = Session::default();
            session.peers.insert(id, peer);
            let mut bans = BTreeMap::new();
            let ping = Inner::Ping([7; 32]).encode()?;
            let encrypted = cipher.encrypt_control(&ping)?;
            let mut frame = crypto::seal(
                &sender.identity,
                &group,
                owner.identity.public(),
                3,
                session_id,
                &encrypted,
            );
            match mode {
                "replay" => {
                    client
                        .envelope(&mut Sink, &mut session, frame.clone(), &group, &mut bans)
                        .await?;
                }
                "truncated" => {
                    frame.pop();
                }
                "oversized" => {
                    frame = crypto::seal(
                        &sender.identity,
                        &group,
                        owner.identity.public(),
                        3,
                        session_id,
                        &vec![0; crate::transport::MAX_FRAME + 1],
                    );
                }
                "wrong-group" => {
                    let mut wrong = Group::new(&sender.identity);
                    wrong.enroll(&sender.identity, owner.identity.public());
                    frame = crypto::seal(
                        &sender.identity,
                        &wrong,
                        owner.identity.public(),
                        3,
                        session_id,
                        &encrypted,
                    );
                }
                "expired-session" => {
                    let (_, mut expired) = peer_pair(&group, id, owner.identity.public())?;
                    assert!(expired.session != session_id);
                    let bytes = expired.encrypt_control(&ping)?;
                    frame = crypto::seal(
                        &sender.identity,
                        &group,
                        owner.identity.public(),
                        3,
                        expired.session,
                        &bytes,
                    );
                }
                _ => unreachable!(),
            }
            let result = client
                .envelope(&mut Sink, &mut session, frame, &group, &mut bans)
                .await;
            if matches!(mode, "replay" | "oversized") {
                assert!(result.is_err(), "invalid ciphertext was accepted");
            } else {
                result?;
            }
            assert_eq!(
                owner.store(|s| s.heads("tasks"))?,
                heads,
                "hostile frame changed accepted heads"
            );
            assert_eq!(session.operations(), 0, "hostile data was queued");
            let (mut fresh, mut honest) = peer_pair(&group, id, owner.identity.public())?;
            fresh.domains.insert("tasks".into());
            let session_id = fresh.cipher.session;
            session.peers.insert(id, fresh);
            let message = Inner::Inventory {
                done: true,
                domains: vec![("tasks".into(), heads.clone())],
            }
            .encode()?;
            let (encrypted, end) = honest.encrypt_fragment(&message, 0)?;
            assert_eq!(end, message.len());
            let frame = crypto::seal(
                &sender.identity,
                &group,
                owner.identity.public(),
                3,
                session_id,
                &encrypted,
            );
            client
                .envelope(&mut Sink, &mut session, frame, &group, &mut bans)
                .await?;
            assert!(
                session.peers[&id].inventory_received
                    && session.peers[&id].synced.contains("tasks"),
                "fresh honest session did not resume"
            );
            assert_eq!(owner.store(|s| s.heads("tasks"))?, heads);
        }
        Ok(())
    }
}
