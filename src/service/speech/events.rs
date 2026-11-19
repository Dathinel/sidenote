use super::routes::{Owner, failure};
use crate::{
    core::{
        speech_jobs::Event,
        speech_sessions::{Error, Store},
    },
    mcp::access,
};
use axum::{
    Extension, Router,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    time::{Instant, Sleep},
};
#[derive(Clone)]
pub(super) struct Streaming(Arc<AtomicBool>);
struct Events {
    store: Arc<Store>,
    owner: String,
    session: String,
    grant: access::Grant,
    after: u64,
    timer: Pin<Box<Sleep>>,
    keepalive: Instant,
}
impl hyper::body::Body for Events {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Bytes>, io::Error>>> {
        if self.timer.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        let root = self
            .store
            .0
            .parent()
            .and_then(|p| p.parent())
            .expect("speech state root");
        if access::check(root, &self.grant).is_err() {
            return Poll::Ready(None);
        }
        let events = match self.store.events(&self.owner, &self.session, self.after) {
            Ok(e) => e,
            Err(_) => return Poll::Ready(Some(Err(io::Error::other("SPEECH_STORAGE")))),
        };
        self.timer
            .as_mut()
            .reset(Instant::now() + Duration::from_millis(250));
        let mut frame = String::new();
        for Event { seq, event, data } in events {
            frame.push_str(&format!("id: {seq}\nevent: {event}\ndata: {data}\n\n"));
            self.after = seq;
        }
        if self.keepalive.elapsed() >= Duration::from_secs(10) {
            frame.push_str(": keepalive\n\n");
            self.keepalive = Instant::now();
        }
        if frame.is_empty() {
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(Some(Ok(hyper::body::Frame::data(frame.into()))))
        }
    }
}
pub(super) async fn events(
    State(store): State<Arc<Store>>,
    Extension(owner): Extension<Owner>,
    Extension(grant): Extension<access::Grant>,
    Path(session): Path<String>,
    headers: HeaderMap,
    Extension(streaming): Extension<Streaming>,
) -> Response {
    let after = match headers.get_all("last-event-id").iter().count() {
        0 => 0,
        1 => match headers
            .get("last-event-id")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse::<u64>().ok())
        {
            Some(n) => n,
            None => return failure(Error::Invalid),
        },
        _ => return failure(Error::Invalid),
    };
    if let Err(e) = store.events(&owner.0, &session, after) {
        return failure(e);
    }
    streaming.0.store(true, Ordering::Relaxed);
    let mut response = Response::new(Body::new(Events {
        store,
        owner: owner.0,
        session,
        grant,
        after,
        timer: Box::pin(tokio::time::sleep(Duration::ZERO)),
        keepalive: Instant::now(),
    }));
    *response.status_mut() = StatusCode::OK;
    for (name, value) in [
        ("content-type", "text/event-stream"),
        ("cache-control", "no-store"),
        ("x-accel-buffering", "no"),
    ] {
        response
            .headers_mut()
            .insert(name, value.parse().expect("fixed header"));
    }
    response.into_response()
}
// Reset only on actual writes, so a blocked SSE peer loses its connection after 60 seconds.
struct Socket {
    stream: TcpStream,
    idle: Pin<Box<Sleep>>,
}
impl AsyncRead for Socket {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for Socket {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.idle.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
        }
        let result = Pin::new(&mut self.stream).poll_write(cx, buf);
        if matches!(result,Poll::Ready(Ok(n)) if n>0) {
            self.idle
                .as_mut()
                .reset(Instant::now() + Duration::from_secs(60));
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
pub(super) async fn serve(listener: tokio::net::TcpListener, router: Router) -> io::Result<()> {
    let slots = Arc::new(tokio::sync::Semaphore::new(32));
    let mut tasks = tokio::task::JoinSet::new();
    let mut status = tokio::time::interval(Duration::from_secs(1));
    let mut last = (serde_json::Value::Null, Instant::now());
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream,_) = accepted?;
                let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
                let streaming = Arc::new(AtomicBool::new(false));
                let router = router.clone().layer(Extension(Streaming(streaming.clone())));
                tasks.spawn(async move {
                    let _slot = slot;
                    let socket = Socket {stream,idle:Box::pin(tokio::time::sleep(Duration::from_secs(60)))};
                    let mut builder=hyper::server::conn::http1::Builder::new();
                    builder.timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(Duration::from_secs(2)).keep_alive(false).max_buf_size(16*1024);
                    let connection=builder.serve_connection(hyper_util::rt::TokioIo::new(socket),hyper_util::service::TowerToHyperService::new(router));
                    tokio::select! {
                        _ = connection => {},
                        _ = async { tokio::time::sleep(Duration::from_secs(10)).await;
                            if streaming.load(Ordering::Relaxed) { std::future::pending::<()>().await; }
                        } => {},
                    }
                });
            },
            _ = tasks.join_next(), if !tasks.is_empty() => {},
            _ = status.tick() => { host_status(&mut last)?; },
        }
    }
}
pub(super) fn host() -> io::Result<serde_json::Value> {
    let store = Store(crate::core::state_dir().join("local/speech"));
    let sessions = {
        let (_, s) = store
            .load()
            .map_err(|_| io::Error::other("SPEECH_STORAGE"))?;
        s
    };
    let pieces: Vec<_> = sessions
        .values()
        .flat_map(|s| s.pieces.values())
        .filter(|p| !p.done)
        .collect();
    let data = serde_json::json!({"asr":super::listener::runtime_probe()?,"queue":{"pieces":pieces.len(),
        "audio_seconds":pieces.iter().map(|p|(p.metadata.end_sample-p.metadata.start_sample) as f64/16000.0).sum::<f64>()}});
    Ok(data)
}
fn host_status(last: &mut (serde_json::Value, Instant)) -> io::Result<()> {
    let store = Store(crate::core::state_dir().join("local/speech"));
    let sessions = {
        let (_, s) = store
            .load()
            .map_err(|_| io::Error::other("SPEECH_STORAGE"))?;
        s
    };
    let data = host()?;
    let identity = serde_json::json!([data, sessions.keys().collect::<Vec<_>>()]);
    if last.0 == identity && last.1.elapsed() < Duration::from_secs(30) {
        return Ok(());
    }
    for id in sessions.keys() {
        // A retained session's unavailable log must not stop the shared listener.
        let _ = store.emit(id, "host.status", data.clone());
    }
    *last = (identity, Instant::now());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test]
    async fn expired_stream_write_deadline_refuses_a_slow_socket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _peer = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = Socket {
            stream,
            idle: Box::pin(tokio::time::sleep(Duration::ZERO)),
        };
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert_eq!(
            socket.write_all(b"public").await.unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }
}
