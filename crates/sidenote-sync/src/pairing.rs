//! Short-code PAKE. The first three decimal digits locate a room; the other
//! five are secret and never sent to the relay or hashed into its room name.
use crate::{
    client::Device,
    identity::{Group, Identity, hash, verify},
    status::{Failure, Reason},
    transport::{Frame, RelayUrl, Transport, WebSocketTransport, authenticate},
};
use anyhow::{Context, Result, bail, ensure};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use rand::{Rng, rngs::OsRng};
use sha2::Sha256;
use spake2::{Ed25519Group, Identity as PakeIdentity, Password, Spake2};
use std::{
    future::Future,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub const INVITATION_TTL: Duration = Duration::from_secs(600);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(150);

fn room(code: &str) -> Result<String> {
    ensure!(
        code.len() == 8 && code.bytes().all(|c| c.is_ascii_digit()),
        "expected eight decimal digits"
    );
    Ok(hex::encode(hash(
        format!("sidenote-pair-v1:{}", &code[..3]).as_bytes(),
    )))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receipt {
    Confirmed,
    /// Enrollment is durable, but no bundle send was attempted. Start a new invitation.
    NotSent,
    /// A bundle send was attempted. Check the guest before starting recovery.
    Unknown,
}
pub struct Grant {
    pub group: Group,
    pub receipt: Receipt,
}

pub type SecretCode = Zeroizing<String>;

#[derive(Clone)]
pub struct Confirmation {
    pub phrase: String,
    pub peer_name: String,
    pub peer: [u8; 32],
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control)
}

pub struct PairHost {
    transport: WebSocketTransport,
    code: Zeroizing<String>,
    expires: Instant,
}

impl PairHost {
    pub async fn open(relay: &RelayUrl, identity: &Identity) -> Result<Self> {
        // Locator collisions are public; retry with a fresh code without exposing it in errors.
        for attempt in 0..16 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let code = Zeroizing::new(format!("{:08}", OsRng.gen_range(0..100_000_000u32)));
            let room = room(&code)?;
            let mut transport = match WebSocketTransport::connect(
                &relay.endpoint("pair", &room, "owner"),
            )
            .await
            {
                Ok(transport) => transport,
                Err(error) => {
                    let collision = matches!(error.downcast_ref::<tokio_tungstenite::tungstenite::Error>(),Some(tokio_tungstenite::tungstenite::Error::Http(response)) if [409,410,429].contains(&response.status().as_u16()));
                    if collision {
                        continue;
                    } else {
                        return Err(error);
                    }
                }
            };
            if authenticate(&mut transport, &room, "owner", identity, None)
                .await
                .is_ok()
            {
                return Ok(Self {
                    transport,
                    code,
                    expires: Instant::now() + INVITATION_TTL,
                });
            }
        }
        bail!("no pairing locator available")
    }
    /// Display locally only. The invitation is deliberately neither Debug nor Serialize.
    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn remaining(&self) -> Duration {
        self.expires.saturating_duration_since(Instant::now())
    }

    pub async fn accept_named<F, Fut>(
        mut self,
        device: &Device,
        existing: Option<Group>,
        name: &str,
        confirm: F,
    ) -> Result<Grant>
    where
        F: FnMut(Confirmation) -> Fut,
        Fut: Future<Output = bool>,
    {
        accept_named_transport(
            &mut self.transport,
            &self.code,
            self.expires,
            device,
            existing,
            name,
            confirm,
        )
        .await
    }

    /// The callback must present the SAS and obtain explicit local confirmation.
    /// Every exchange consumes this invitation, including rejection or timeout.
    pub async fn accept<F, Fut>(
        mut self,
        device: &Device,
        existing: Option<Group>,
        confirm: F,
    ) -> Result<Grant>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = bool>,
    {
        accept_transport(
            &mut self.transport,
            &self.code,
            self.expires,
            device,
            existing,
            confirm,
        )
        .await
    }
}

async fn accept_transport<F, Fut>(
    transport: &mut impl Transport,
    code: &str,
    expires: Instant,
    device: &Device,
    existing: Option<Group>,
    mut confirm: F,
) -> Result<Grant>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = bool>,
{
    accept_named_transport(
        transport,
        code,
        expires,
        device,
        existing,
        "Device",
        move |info| confirm(info.phrase),
    )
    .await
}

async fn accept_named_transport<F, Fut>(
    transport: &mut impl Transport,
    code: &str,
    expires: Instant,
    device: &Device,
    existing: Option<Group>,
    name: &str,
    mut confirm: F,
) -> Result<Grant>
where
    F: FnMut(Confirmation) -> Fut,
    Fut: Future<Output = bool>,
{
    let group = existing.unwrap_or_else(|| Group::new(&device.identity));
    group.validate(device.identity.public())?;
    let deadline = tokio::time::Instant::from_std(expires);
    let mut granted = None;
    let result = tokio::time::timeout_at(deadline, async {
        let first = binary(transport).await?;
        let exchange = exchange(
            transport,
            code,
            device,
            name,
            &mut confirm,
            Side::Host {
                group: &group,
                first,
                grant: &mut granted,
            },
        );
        match tokio::time::timeout(ATTEMPT_TIMEOUT, exchange).await {
            Ok(Ok(group)) => {
                let _ = transport
                    .send(Frame::Text("{\"type\":\"complete\"}".into()))
                    .await;
                Ok(Grant {
                    group,
                    receipt: Receipt::Confirmed,
                })
            }
            Ok(Err(error)) => Err(error),
            Err(_) => bail!("pairing attempt timed out"),
        }
    })
    .await;
    match result {
        Ok(Ok(grant)) => Ok(grant),
        _ if granted.is_some() => Ok(granted.take().context("missing grant")?),
        Ok(Err(error)) => Err(error),
        Err(_) => bail!("invitation expired"),
    }
}

pub async fn join<F, Fut>(
    relay: &RelayUrl,
    code: &str,
    device: &Device,
    confirm: F,
) -> Result<Group>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let identity = &device.identity;
    let room = room(code)?;
    let mut transport =
        WebSocketTransport::connect(&relay.endpoint("pair", &room, "guest")).await?;
    authenticate(&mut transport, &room, "guest", identity, None).await?;
    let result = join_transport(&mut transport, code, device, confirm).await;
    let _ = transport.close().await;
    result
}

pub async fn join_named<F, Fut>(
    relay: &RelayUrl,
    code: &str,
    device: &Device,
    name: &str,
    confirm: F,
) -> Result<Group>
where
    F: FnMut(Confirmation) -> Fut,
    Fut: Future<Output = bool>,
{
    let identity = &device.identity;
    let room = room(code)?;
    let mut transport =
        WebSocketTransport::connect(&relay.endpoint("pair", &room, "guest")).await?;
    authenticate(&mut transport, &room, "guest", identity, None).await?;
    let result = join_named_transport(&mut transport, code, device, name, confirm).await;
    let _ = transport.close().await;
    result
}

async fn join_transport<F, Fut>(
    transport: &mut impl Transport,
    code: &str,
    device: &Device,
    mut confirm: F,
) -> Result<Group>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = bool>,
{
    join_named_transport(transport, code, device, "Device", move |info| {
        confirm(info.phrase)
    })
    .await
}

async fn join_named_transport<F, Fut>(
    transport: &mut impl Transport,
    code: &str,
    device: &Device,
    name: &str,
    mut confirm: F,
) -> Result<Group>
where
    F: FnMut(Confirmation) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut stored = None;
    let result = tokio::time::timeout(
        ATTEMPT_TIMEOUT,
        exchange(
            transport,
            code,
            device,
            name,
            &mut confirm,
            Side::Guest {
                stored: &mut stored,
            },
        ),
    )
    .await;
    match result {
        Ok(Ok(group)) => Ok(group),
        _ if stored.is_some() => Ok(stored.take().context("missing stored grant")?),
        Ok(Err(error)) => Err(error),
        Err(_) => bail!("pairing timed out"),
    }
}

struct Channel {
    cipher: ChaCha20Poly1305,
    transcript: [u8; 32],
    send: u64,
    recv: u64,
}
impl Channel {
    fn new(shared: &[u8], transcript: [u8; 32], host: bool) -> Result<Self> {
        let mut key = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(Some(&transcript), shared)
            .expand(b"sidenote-pair-aead-v1", key.as_mut())
            .map_err(|_| anyhow::anyhow!("pair KDF failed"))?;
        Ok(Self {
            cipher: ChaCha20Poly1305::new_from_slice(key.as_ref())
                .map_err(|_| anyhow::anyhow!("pair cipher failed"))?,
            transcript,
            send: if host { 0 } else { 1 },
            recv: if host { 1 } else { 0 },
        })
    }
    async fn send(&mut self, transport: &mut impl Transport, bytes: &[u8]) -> Result<()> {
        let nonce = nonce(self.send);
        let encrypted = self
            .cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: &self.transcript,
                },
            )
            .map_err(|_| anyhow::anyhow!("pair encryption failed"))?;
        self.send = self.send.checked_add(2).context("pair nonce exhausted")?;
        transport.send(Frame::Binary(encrypted)).await
    }
    async fn recv(&mut self, transport: &mut impl Transport) -> Result<Zeroizing<Vec<u8>>> {
        let data = binary(transport).await?;
        let nonce = nonce(self.recv);
        let bytes = self
            .cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &data,
                    aad: &self.transcript,
                },
            )
            .map_err(|_| anyhow::anyhow!("pairing authentication failed"))?;
        self.recv = self.recv.checked_add(2).context("pair nonce exhausted")?;
        Ok(Zeroizing::new(bytes))
    }
}
fn nonce(n: u64) -> [u8; 12] {
    let mut out = [0; 12];
    out[4..].copy_from_slice(&n.to_be_bytes());
    out
}

async fn binary(transport: &mut impl Transport) -> Result<Vec<u8>> {
    loop {
        match transport.recv().await? {
            Frame::Binary(b) => return Ok(b),
            Frame::Text(s) => {
                let v: serde_json::Value =
                    serde_json::from_str(&s).map_err(|_| Failure(Reason::Protocol))?;
                ensure!(
                    ["joined", "left"].iter().any(|t| v["type"] == *t),
                    "unexpected pairing control"
                );
            }
        }
    }
}

enum Side<'a> {
    Host {
        group: &'a Group,
        first: Vec<u8>,
        grant: &'a mut Option<Grant>,
    },
    Guest {
        stored: &'a mut Option<Group>,
    },
}
fn validate_point(message: &[u8]) -> Result<()> {
    use curve25519_dalek::{edwards::CompressedEdwardsY, traits::IsIdentity};
    ensure!(message.len() == 33, "invalid PAKE length");
    let point = CompressedEdwardsY(message[1..].try_into()?)
        .decompress()
        .context("invalid PAKE point")?;
    ensure!(
        !point.is_identity() && !point.is_small_order() && point.is_torsion_free(),
        "unsafe PAKE point"
    );
    Ok(())
}
async fn exchange<F, Fut>(
    transport: &mut impl Transport,
    code: &str,
    device: &Device,
    name: &str,
    confirm: &mut F,
    side: Side<'_>,
) -> Result<Group>
where
    F: FnMut(Confirmation) -> Fut,
    Fut: Future<Output = bool>,
{
    ensure!(valid_name(name), "invalid device name");
    let identity = &device.identity;
    let host = matches!(side, Side::Host { .. });
    // Host waits for the joiner's first PAKE message; no pairing messages are buffered by the relay.
    let room = room(code)?;
    let id_a = PakeIdentity::new(format!("sidenote-pair-v1:{room}:A").as_bytes());
    let id_b = PakeIdentity::new(format!("sidenote-pair-v1:{room}:B").as_bytes());
    let password = Password::new(&code.as_bytes()[3..]);
    let (pake, out) = if host {
        Spake2::<Ed25519Group>::start_a(&password, &id_a, &id_b)
    } else {
        Spake2::<Ed25519Group>::start_b(&password, &id_a, &id_b)
    };
    let incoming = if let Side::Host { first, .. } = &side {
        first.clone()
    } else {
        transport.send(Frame::Binary(out.clone())).await?;
        binary(transport).await?
    };
    validate_point(&incoming)?;
    if host {
        transport.send(Frame::Binary(out.clone())).await?;
    }
    let shared = Zeroizing::new(
        pake.finish(&incoming)
            .map_err(|_| anyhow::anyhow!("PAKE failed"))?,
    );
    let mut transcript = format!("sidenote-pair-v1:{room}").into_bytes();
    if host {
        transcript.extend_from_slice(&out);
        transcript.extend_from_slice(&incoming);
    } else {
        transcript.extend_from_slice(&incoming);
        transcript.extend_from_slice(&out);
    }
    let transcript = hash(&transcript);
    let mut channel = Channel::new(&shared, transcript, host)?;
    let mut signed = transcript.to_vec();
    signed.extend_from_slice(name.as_bytes());
    let mut proof = identity.public().to_vec();
    proof.extend_from_slice(&identity.sign(&signed));
    proof.extend_from_slice(name.as_bytes());
    channel.send(transport, &proof).await?;
    let peer_proof = channel.recv(transport).await?;
    ensure!(
        (97..=416).contains(&peer_proof.len()),
        "invalid identity proof"
    );
    let peer_name = std::str::from_utf8(&peer_proof[96..]).context("invalid device name")?;
    ensure!(valid_name(peer_name), "invalid device name");
    let peer: [u8; 32] = peer_proof[..32].try_into()?;
    ensure!(
        peer != identity.public(),
        "cannot pair identity with itself"
    );
    let mut peer_signed = transcript.to_vec();
    peer_signed.extend_from_slice(peer_name.as_bytes());
    verify(&peer, &peer_signed, &peer_proof[32..96])?;
    let mut sas_bytes = transcript.to_vec();
    if host {
        sas_bytes.extend_from_slice(&identity.public());
        sas_bytes.extend_from_slice(&peer);
    } else {
        sas_bytes.extend_from_slice(&peer);
        sas_bytes.extend_from_slice(&identity.public());
    }
    let (first, second) = if host {
        (name, peer_name)
    } else {
        (peer_name, name)
    };
    sas_bytes.extend_from_slice(first.as_bytes());
    sas_bytes.push(0);
    sas_bytes.extend_from_slice(second.as_bytes());
    let sas = hex::encode(&hash(&sas_bytes)[..4]);
    let accepted = confirm(Confirmation {
        phrase: sas,
        peer_name: peer_name.into(),
        peer,
    })
    .await;
    channel
        .send(transport, if accepted { b"confirmed" } else { b"rejected" })
        .await?;
    ensure!(
        accepted && channel.recv(transport).await?.as_slice() == b"confirmed",
        "confirmation declined"
    );
    if let Side::Host { group, grant, .. } = side {
        let mut group = group.clone();
        ensure!(
            group.members().len() < 16 || group.members().contains(&peer),
            "group full"
        );
        group.enroll(identity, peer);
        group = device.merge_group(&group)?;
        *grant = Some(Grant {
            group: group.clone(),
            receipt: Receipt::NotSent,
        });
        transport
            .send(Frame::Text("{\"type\":\"grant\"}".into()))
            .await?;
        loop {
            match transport.recv().await? {
                Frame::Text(text) => {
                    let control: serde_json::Value =
                        serde_json::from_str(&text).map_err(|_| Failure(Reason::Protocol))?;
                    if control["type"] == "grant-ready" {
                        break;
                    }
                }
                Frame::Binary(_) => bail!("unexpected grant data"),
            }
        }
        let bytes =
            Zeroizing::new(serde_json::to_vec(&group).map_err(|_| Failure(Reason::Protocol))?);
        // A failed/cancelled send can still have delivered bytes.
        grant.as_mut().context("missing grant")?.receipt = Receipt::Unknown;
        channel.send(transport, &bytes).await?;
        ensure!(
            channel.recv(transport).await?.as_slice() == b"stored",
            "missing pairing receipt"
        );
        Ok(group)
    } else {
        let bytes = channel.recv(transport).await?;
        let mut group: Group =
            serde_json::from_slice(&bytes).map_err(|_| Failure(Reason::Protocol))?;
        group.validate(identity.public())?;
        ensure!(group.members().contains(&peer), "host not in group");
        group = device.merge_group(&group)?;
        if let Side::Guest { stored } = side {
            *stored = Some(group.clone());
        }
        let _ = channel.send(transport, b"stored").await;
        Ok(group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curve25519_dalek::{
        constants::{ED25519_BASEPOINT_POINT, EIGHT_TORSION},
        traits::Identity as _,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    #[tokio::test(start_paused = true)]
    async fn expired_invitation_cannot_grant_or_send_content() -> Result<()> {
        assert_eq!(INVITATION_TTL, Duration::from_secs(600));
        let dir = tempfile::tempdir()?;
        let device = Device::open(&dir.path().join("expired.sqlite"))?;
        let used = Arc::new(AtomicBool::new(false));
        let grants = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let mut transport = MemoryRelay {
            host: true,
            fault: None,
            used: used.clone(),
            grants: grants.clone(),
            retries_after_grant: Arc::new(AtomicUsize::new(0)),
            incoming: rx,
            self_tx: tx.clone(),
            other: tx,
        };
        let result = accept_transport(
            &mut transport,
            "00112345",
            Instant::now() - Duration::from_secs(1),
            &device,
            None,
            |_| async { true },
        )
        .await;
        assert!(result.is_err());
        assert!(!used.load(Ordering::Relaxed));
        assert_eq!(grants.load(Ordering::Relaxed), 0);
        Ok(())
    }
    #[test]
    fn device_names_are_bounded_before_the_confirmation_screen() {
        assert!(valid_name("Synthetic laptop"));
        for invalid in [String::new(), " ".into(), "a\nb".into(), "x".repeat(81)] {
            assert!(!valid_name(&invalid));
        }
    }
    struct MemoryRelay {
        host: bool,
        fault: Option<&'static str>,
        used: Arc<AtomicBool>,
        grants: Arc<AtomicUsize>,
        retries_after_grant: Arc<AtomicUsize>,
        incoming: tokio::sync::mpsc::Receiver<Result<Frame, ()>>,
        self_tx: tokio::sync::mpsc::Sender<Result<Frame, ()>>,
        other: tokio::sync::mpsc::Sender<Result<Frame, ()>>,
    }
    #[async_trait::async_trait]
    impl Transport for MemoryRelay {
        async fn send(&mut self, frame: Frame) -> Result<()> {
            if let Frame::Text(text) = &frame {
                let value: serde_json::Value =
                    serde_json::from_str(text).map_err(|_| Failure(Reason::Protocol))?;
                if value["type"] == "grant" {
                    if self.fault == Some("grant-control") {
                        let _ = self.other.send(Err(())).await;
                        bail!("grant control send failed");
                    }
                    self.grants.fetch_add(1, Ordering::Relaxed);
                    ensure!(!self.used.swap(true, Ordering::Relaxed), "already consumed");
                    if self.fault == Some("grant-ready") {
                        let _ = self.self_tx.send(Err(())).await;
                        let _ = self.other.send(Err(())).await;
                        return Ok(());
                    }
                    self.self_tx
                        .send(Ok(Frame::Text("{\"type\":\"grant-ready\"}".into())))
                        .await
                        .map_err(|_| anyhow::anyhow!("test pipe closed"))?;
                } else if value["type"] == "retry" && self.used.load(Ordering::Relaxed) {
                    self.retries_after_grant.fetch_add(1, Ordering::Relaxed);
                }
                return Ok(());
            }
            if self.host && self.used.load(Ordering::Relaxed) && self.fault == Some("bundle") {
                let _ = self.other.send(Err(())).await;
                bail!("bundle send failed");
            }
            if !self.host
                && self.used.load(Ordering::Relaxed)
                && matches!(self.fault, Some("receipt" | "receipt-stall"))
            {
                self.other
                    .send(Err(()))
                    .await
                    .map_err(|_| anyhow::anyhow!("test pipe closed"))?;
                if self.fault == Some("receipt-stall") {
                    return std::future::pending().await;
                }
                bail!("receipt send failed");
            } else {
                self.other
                    .send(Ok(frame))
                    .await
                    .map_err(|_| anyhow::anyhow!("test pipe closed"))?;
            }
            Ok(())
        }
        async fn recv(&mut self) -> Result<Frame> {
            self.incoming
                .recv()
                .await
                .context("test pipe closed")?
                .map_err(|_| anyhow::anyhow!("lost receipt"))
        }
        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }
    #[tokio::test(start_paused = true)]
    async fn retired_group_pairing_is_refused_whichever_device_hosts() -> Result<()> {
        for old_survivor_hosts in [true, false] {
            let dir = tempfile::tempdir()?;
            let a = Device::open(&dir.path().join("a.sqlite"))?;
            let b = Device::open(&dir.path().join("b.sqlite"))?;
            let c = Device::open(&dir.path().join("c.sqlite"))?;
            let mut old = Group::new(&a.identity);
            old.enroll(&a.identity, b.identity.public());
            old.enroll(&a.identity, c.identity.public());
            a.merge_group(&old)?;
            b.merge_group(&old)?;
            a.store(|s| s.retire_group(&old.room()))?;
            let fresh = Group::new(&a.identity);
            a.merge_group(&fresh)?;
            let (atx, arx) = tokio::sync::mpsc::channel(16);
            let (btx, brx) = tokio::sync::mpsc::channel(16);
            let used = Arc::new(AtomicBool::new(false));
            let grants = Arc::new(AtomicUsize::new(0));
            let retries = Arc::new(AtomicUsize::new(0));
            let mut host = MemoryRelay {
                host: true,
                fault: None,
                used: used.clone(),
                grants: grants.clone(),
                retries_after_grant: retries.clone(),
                incoming: arx,
                self_tx: atx.clone(),
                other: btx.clone(),
            };
            let mut guest = MemoryRelay {
                host: false,
                fault: None,
                used,
                grants,
                retries_after_grant: retries,
                incoming: brx,
                self_tx: btx,
                other: atx,
            };
            let (host_device, guest_device) = if old_survivor_hosts {
                (&b, &a)
            } else {
                (&a, &b)
            };
            let code = Zeroizing::new(format!("{:08}", OsRng.gen_range(0..100_000_000u32)));
            let (offered, joined) = tokio::join!(
                accept_transport(
                    &mut host,
                    &code,
                    Instant::now() + INVITATION_TTL,
                    host_device,
                    Some(old.clone()),
                    |_| async { true }
                ),
                join_transport(&mut guest, &code, guest_device, |_| async { true })
            );
            assert!(joined.is_err());
            assert!(offered.is_err() || offered.is_ok_and(|g| g.receipt != Receipt::Confirmed));
            assert!(a.merge_group(&old).is_err());
            a.store(|s| s.check_group_active(&fresh.room()))?;
        }
        Ok(())
    }
    #[tokio::test(start_paused = true)]
    async fn expired_confirmation_consumes_invitation_without_a_retry_or_grant() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let a = Device::open(&dir.path().join("a.sqlite"))?;
        let b = Device::open(&dir.path().join("b.sqlite"))?;
        let used = Arc::new(AtomicBool::new(false));
        let grants = Arc::new(AtomicUsize::new(0));
        let retries = Arc::new(AtomicUsize::new(0));
        let (atx, arx) = tokio::sync::mpsc::channel(16);
        let (btx, brx) = tokio::sync::mpsc::channel(16);
        let mut host = MemoryRelay {
            host: true,
            fault: None,
            used: used.clone(),
            grants: grants.clone(),
            retries_after_grant: retries.clone(),
            incoming: arx,
            self_tx: atx.clone(),
            other: btx.clone(),
        };
        let mut guest = MemoryRelay {
            host: false,
            fault: None,
            used: used.clone(),
            grants: grants.clone(),
            retries_after_grant: retries.clone(),
            incoming: brx,
            self_tx: btx,
            other: atx,
        };
        let prompts = AtomicUsize::new(0);
        let code = Zeroizing::new(format!("{:08}", OsRng.gen_range(0..100_000_000u32)));
        let started = tokio::time::Instant::now();
        let (host, guest) = tokio::join!(
            accept_transport(
                &mut host,
                &code,
                Instant::now() + INVITATION_TTL,
                &a,
                None,
                |_| async {
                    prompts.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_secs(90)).await;
                    false
                }
            ),
            join_transport(&mut guest, &code, &b, |_| async { true })
        );
        assert!(host.is_err() && guest.is_err());
        assert!(started.elapsed() < Duration::from_secs(91));
        assert_eq!(prompts.load(Ordering::Relaxed), 1);
        assert_eq!(grants.load(Ordering::Relaxed), 0);
        assert_eq!(retries.load(Ordering::Relaxed), 0);
        assert!(!used.load(Ordering::Relaxed));
        Ok(())
    }
    #[tokio::test(start_paused = true)]
    async fn c6_grant_recovery_distinguishes_unsent_bundle_and_durable_guest() -> Result<()> {
        for fault in [
            None,
            Some("receipt"),
            Some("receipt-stall"),
            Some("grant-control"),
            Some("grant-ready"),
            Some("bundle"),
            Some("full-recovery"),
        ] {
            let dir = tempfile::tempdir()?;
            let a = Device::open(&dir.path().join("a.sqlite"))?;
            let b = Device::open(&dir.path().join("b.sqlite"))?;
            let used = Arc::new(AtomicBool::new(false));
            let grants = Arc::new(AtomicUsize::new(0));
            let retries = Arc::new(AtomicUsize::new(0));
            let (atx, arx) = tokio::sync::mpsc::channel(16);
            let (btx, brx) = tokio::sync::mpsc::channel(16);
            let mut host = MemoryRelay {
                host: true,
                fault,
                used: used.clone(),
                grants: grants.clone(),
                retries_after_grant: retries.clone(),
                incoming: arx,
                self_tx: atx.clone(),
                other: btx.clone(),
            };
            let mut guest = MemoryRelay {
                host: false,
                fault,
                used: used.clone(),
                grants: grants.clone(),
                retries_after_grant: retries.clone(),
                incoming: brx,
                self_tx: btx,
                other: atx,
            };
            let existing = if fault == Some("full-recovery") {
                let mut group = Group::new(&a.identity);
                group.enroll(&a.identity, b.identity.public());
                for n in 0..14 {
                    let member = Device::open(&dir.path().join(format!("member{n}.sqlite")))?;
                    group.enroll(&a.identity, member.identity.public());
                }
                Some(group)
            } else {
                None
            };
            let members = if existing.is_some() { 16 } else { 2 };
            let host = accept_transport(
                &mut host,
                "00112345",
                Instant::now() + INVITATION_TTL,
                &a,
                existing,
                |_| async { true },
            );
            let guest = join_transport(&mut guest, "00112345", &b, |_| async { true });
            let (host, guest) = tokio::join!(host, guest);
            let host = host?;
            assert_eq!(
                host.receipt,
                match fault {
                    None | Some("full-recovery") => Receipt::Confirmed,
                    Some("grant-control" | "grant-ready") => Receipt::NotSent,
                    _ => Receipt::Unknown,
                }
            );
            assert_eq!(used.load(Ordering::Relaxed), fault != Some("grant-control"));
            assert_eq!(
                grants.load(Ordering::Relaxed),
                usize::from(fault != Some("grant-control"))
            );
            assert_eq!(retries.load(Ordering::Relaxed), 0);
            assert_eq!(
                a.store(|s| Group::load(s, &host.group.room()))?
                    .members()
                    .len(),
                members
            );
            if matches!(
                fault,
                None | Some("receipt" | "receipt-stall" | "full-recovery")
            ) {
                let guest = guest?;
                assert_eq!(
                    b.store(|s| Group::load(s, &guest.room()))?.members().len(),
                    members
                );
            } else {
                assert!(guest.is_err());
                assert!(b.store(|s| Group::load(s, &host.group.room())).is_err());
            }
        }
        Ok(())
    }
    #[test]
    fn c8_rejects_identity_small_order_and_mixed_torsion_points() {
        let points = [
            curve25519_dalek::edwards::EdwardsPoint::identity(),
            EIGHT_TORSION[1],
            ED25519_BASEPOINT_POINT + EIGHT_TORSION[1],
        ];
        for point in points {
            let mut message = vec![b'B'];
            message.extend_from_slice(point.compress().as_bytes());
            assert!(validate_point(&message).is_err());
        }
    }
    #[test]
    fn c8_fresh_rustcrypto_messages_pass_the_point_filter() {
        let password = Password::new(b"synthetic");
        let a = PakeIdentity::new(b"a");
        let b = PakeIdentity::new(b"b");
        let (_, first) = Spake2::<Ed25519Group>::start_a(&password, &a, &b);
        let (_, second) = Spake2::<Ed25519Group>::start_b(&password, &a, &b);
        assert!(validate_point(&first).is_ok());
        assert!(validate_point(&second).is_ok());
    }
    #[test]
    fn c11_owned_password_buffer_can_be_zeroized() {
        use zeroize::Zeroize;
        let mut password = Password::new(b"synthetic");
        password.zeroize();
        assert_eq!(password.len(), 9);
        assert_eq!(password.as_slice(), &[0; 9]);
    }
}
