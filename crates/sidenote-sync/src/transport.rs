use crate::status::{Failure, Reason};
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

pub const MAX_FRAME: usize = 128 * 1024;

#[derive(Clone)]
pub struct RelayUrl(url::Url);

impl RelayUrl {
    pub fn new(url: &str) -> Result<Self> {
        let parsed = url::Url::parse(url)?;
        ensure!(
            parsed.scheme() == "wss" && parsed.port_or_known_default() == Some(443),
            "relay requires WSS port 443"
        );
        Self::validate(parsed)
    }
    /// Explicit loopback-only override for the local harness. Never accepts hostnames.
    pub fn local(url: &str) -> Result<Self> {
        let parsed = url::Url::parse(url)?;
        ensure!(
            parsed.scheme() == "ws" && parsed.host_str() == Some("127.0.0.1"),
            "local relay must be numeric loopback"
        );
        Self::validate(parsed)
    }
    fn validate(parsed: url::Url) -> Result<Self> {
        ensure!(
            parsed.username().is_empty()
                && parsed.password().is_none()
                && parsed.query().is_none()
                && parsed.fragment().is_none()
                && parsed.path() == "/",
            "relay URL must be an origin without credentials"
        );
        Ok(Self(parsed))
    }
    pub(crate) fn endpoint(&self, kind: &str, room: &str, role: &str) -> String {
        format!("{}{kind}/{room}?role={role}", self.0)
    }
}

pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
}

pub(crate) async fn control(transport: &mut impl Transport) -> Result<serde_json::Value> {
    match transport.recv().await? {
        Frame::Text(s) => Ok(serde_json::from_str(&s).map_err(|_| Failure(Reason::Protocol))?),
        _ => bail!("expected relay control"),
    }
}

pub(crate) async fn authenticate(
    transport: &mut impl Transport,
    room: &str,
    role: &str,
    identity: &crate::identity::Identity,
    group: Option<&crate::identity::Group>,
) -> Result<serde_json::Value> {
    tokio::time::timeout(std::time::Duration::from_secs(10),async {
    let challenge = control(transport).await?;
    ensure!(challenge["type"] == "challenge", "expected challenge");
    let nonce = challenge["nonce"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing challenge"))?;
    ensure!(
        nonce.len() == 64 && hex::decode(nonce).is_ok(),
        "invalid challenge"
    );
    let enrollments:Vec<_>=group.map(|g|g.enrollments.iter().map(|e|serde_json::json!({"sponsor":hex::encode(e.sponsor),"member":hex::encode(e.member),"signature":hex::encode(&e.signature)})).collect()).unwrap_or_default();
    let auth = serde_json::json!({"type":"auth","key":hex::encode(identity.public()),"signature":hex::encode(identity.sign(&crate::identity::auth_bytes(room,role,nonce))),"enrollments":enrollments});
    transport.send(Frame::Text(auth.to_string())).await?;
    let ready = control(transport).await?;
    ensure!(ready["type"] == "ready", "authentication rejected");
    Ok(ready)
    }).await.context("relay authentication timed out")?
}

#[async_trait]
pub trait Transport: Send {
    async fn send(&mut self, frame: Frame) -> Result<()>;
    async fn recv(&mut self) -> Result<Frame>;
    async fn close(&mut self) -> Result<()>;
}

const SEND_BYTES_PER_SECOND: usize = 4 * 1024 * 1024;
const SEND_FRAMES_PER_SECOND: u32 = 200;
#[derive(Default)]
struct SendBudget {
    next: Option<tokio::time::Instant>,
}
impl SendBudget {
    async fn acquire(&mut self, bytes: usize) {
        let now = tokio::time::Instant::now();
        let at = self.next.unwrap_or(now).max(now);
        tokio::time::sleep_until(at).await;
        self.next = Some(
            at + std::time::Duration::from_secs_f64(
                (bytes as f64 / SEND_BYTES_PER_SECOND as f64)
                    .max(1.0 / SEND_FRAMES_PER_SECOND as f64),
            ),
        );
    }
}
pub struct WebSocketTransport(WebSocketStream<MaybeTlsStream<TcpStream>>, SendBudget);
impl WebSocketTransport {
    pub async fn connect(url: &str) -> Result<Self> {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME))
            .max_write_buffer_size(MAX_FRAME * 2);
        Ok(Self(
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                connect_async_with_config(url, Some(config), true),
            )
            .await??
            .0,
            SendBudget::default(),
        ))
    }
}

#[async_trait]
impl Transport for WebSocketTransport {
    async fn send(&mut self, frame: Frame) -> Result<()> {
        let bytes = match &frame {
            Frame::Text(s) => s.len(),
            Frame::Binary(b) => b.len(),
        };
        ensure!(bytes <= MAX_FRAME, "oversize frame");
        self.1.acquire(bytes).await;
        let message = match frame {
            Frame::Text(s) => {
                ensure!(s.len() <= MAX_FRAME, "oversize frame");
                Message::Text(s.into())
            }
            Frame::Binary(b) => {
                ensure!(b.len() <= MAX_FRAME, "oversize frame");
                Message::Binary(b.into())
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), self.0.send(message))
            .await
            .context("send deadline")??;
        Ok(())
    }
    async fn recv(&mut self) -> Result<Frame> {
        loop {
            match self.0.next().await {
                Some(Ok(Message::Text(s))) => return Ok(Frame::Text(s.to_string())),
                Some(Ok(Message::Binary(b))) => return Ok(Frame::Binary(b.to_vec())),
                Some(Ok(Message::Ping(_))) => {
                    self.1.acquire(125).await;
                    tokio::time::timeout(std::time::Duration::from_secs(10), self.0.flush())
                        .await
                        .context("flush deadline")??;
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => bail!("relay disconnected"),
                Some(Err(_)) => bail!("WebSocket transport error"),
                _ => bail!("unsupported WebSocket frame"),
            }
        }
    }
    async fn close(&mut self) -> Result<()> {
        self.1.acquire(125).await;
        tokio::time::timeout(std::time::Duration::from_secs(10), self.0.close(None))
            .await
            .context("close deadline")??;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Silent;
    #[async_trait]
    impl Transport for Silent {
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
    #[tokio::test]
    async fn silent_relay_auth_times_out() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = crate::store::Store::open(&dir.path().join("identity.sqlite"), b"test")?;
        let identity = crate::identity::Identity::load_or_create(&store)?;
        assert!(
            authenticate(&mut Silent, &"0".repeat(64), "device", &identity, None)
                .await
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn relay_url_boundaries() {
        assert!(RelayUrl::new("wss://relay.example").is_ok());
        for bad in [
            "ws://relay.example",
            "wss://relay.example:444",
            "wss://relay.example/path",
            "wss://user@relay.example",
            "wss://relay.example?token=x",
        ] {
            assert!(RelayUrl::new(bad).is_err());
        }
        assert!(RelayUrl::local("ws://127.0.0.1:8787").is_ok());
        for bad in [
            "ws://localhost:8787",
            "ws://0.0.0.0:8787",
            "ws://192.168.1.1:8787",
        ] {
            assert!(RelayUrl::local(bad).is_err());
        }
    }
}
