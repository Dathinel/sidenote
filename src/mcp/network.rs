//! Loopback-only stateless Streamable HTTP. Encryption belongs to the private tunnel.
use super::{Meetings, access};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const CONNECTIONS: usize = 32;
const HEADER_TIMEOUT: Duration = Duration::from_secs(2);
// Includes the header and handler budgets, response-body delivery and socket writes.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
async fn connection(
    stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    router: Router,
    _slot: tokio::sync::OwnedSemaphorePermit,
) {
    let _ = tokio::time::timeout(CONNECTION_TIMEOUT, async {
        hyper::server::conn::http1::Builder::new()
            .timer(hyper_util::rt::TokioTimer::new())
            .header_read_timeout(HEADER_TIMEOUT)
            .keep_alive(false)
            .max_buf_size(16 * 1024)
            .serve_connection(
                hyper_util::rt::TokioIo::new(stream),
                hyper_util::service::TowerToHyperService::new(router),
            )
            .await
    })
    .await;
}
async fn serve(listener: tokio::net::TcpListener, router: Router) -> io::Result<()> {
    let slots = Arc::new(tokio::sync::Semaphore::new(CONNECTIONS));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
                tasks.spawn(connection(stream, router.clone(), slot));
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
}

pub(crate) struct Endpoint {
    listener: std::net::TcpListener,
    root: PathBuf,
}
fn reject(status: StatusCode) -> Response {
    (status, "MCP_REQUEST_REJECTED").into_response()
}
async fn authorize(
    root: Arc<PathBuf>,
    slots: Arc<tokio::sync::Semaphore>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let Ok(_slot) = slots.try_acquire_owned() else {
        return reject(StatusCode::SERVICE_UNAVAILABLE);
    };
    if request.headers().contains_key("origin") {
        return reject(StatusCode::FORBIDDEN);
    }
    let Some(host) = request.headers().get("host").and_then(|h| h.to_str().ok()) else {
        return reject(StatusCode::FORBIDDEN);
    };
    if !host
        .strip_prefix("127.0.0.1:")
        .is_some_and(|p| p.parse::<u16>().is_ok())
    {
        return reject(StatusCode::FORBIDDEN);
    }
    if request.headers().get_all("authorization").iter().count() != 1 {
        return reject(StatusCode::UNAUTHORIZED);
    }
    let grant = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .and_then(|b| access::authenticate(&root, b).ok());
    let Some(grant) = grant else {
        return reject(StatusCode::UNAUTHORIZED);
    };
    request.extensions_mut().insert(grant.clone());
    let response = tokio::time::timeout(Duration::from_secs(5), next.run(request)).await;
    if access::check(&root, &grant).is_err() {
        return reject(StatusCode::UNAUTHORIZED);
    }
    response.unwrap_or_else(|_| reject(StatusCode::GATEWAY_TIMEOUT))
}
impl Endpoint {
    pub(crate) fn bind(root: &std::path::Path) -> io::Result<Self> {
        let args = std::env::args_os().collect::<Vec<_>>();
        let port = match args.iter().position(|a| a == "--mcp-port") {
            Some(index) => args
                .get(index + 1)
                .and_then(|v| v.to_str())
                .and_then(|v| v.parse::<u16>().ok())
                .ok_or_else(|| io::Error::other("MCP_INVALID_PORT"))?,
            None => 6791,
        };
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
        listener.set_nonblocking(true)?;
        crate::core::store_io::write(
            &root.join("local/mcp/endpoint.json"),
            &serde_json::json!({"address":listener.local_addr()?.to_string()}),
        )?;
        Ok(Self {
            listener,
            root: root.into(),
        })
    }
    pub(crate) fn serve(self, stop: Arc<AtomicBool>) {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => return,
        };
        runtime.block_on(async {
            let listener = match self
                .listener
                .try_clone()
                .and_then(tokio::net::TcpListener::from_std)
            {
                Ok(listener) => listener,
                Err(_) => return,
            };
            let service: StreamableHttpService<Meetings, LocalSessionManager> =
                StreamableHttpService::new(
                    || Ok(Meetings { remote: true }),
                    Default::default(),
                    StreamableHttpServerConfig::default()
                        .with_legacy_session_mode(false)
                        .with_json_response(true)
                        .with_sse_keep_alive(None)
                        .with_max_request_body_bytes(64 * 1024),
                );
            let root = Arc::new(self.root.clone());
            let slots = Arc::new(tokio::sync::Semaphore::new(8));
            let router = Router::new()
                .nest_service("/mcp", service)
                .layer(middleware::from_fn(move |request, next| {
                    authorize(root.clone(), slots.clone(), request, next)
                }));
            tokio::select! {
                _ = serve(listener, router) => {},
                _ = async move {
                    while !stop.load(Ordering::Acquire) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                } => {},
            }
        });
        runtime.shutdown_timeout(Duration::from_secs(1));
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.root.join("local/mcp/endpoint.json"));
    }
}
