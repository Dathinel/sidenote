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

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{RoleServer, Service, service::RequestContext};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn remote_requests_without_grants_fail_closed() {
        let (transport, _client) = tokio::io::duplex(4096);
        let running = rmcp::service::serve_directly::<RoleServer, _, _, _, _>(
            Meetings { remote: false },
            transport,
            None,
        );
        for request in [
            json!({"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}}}),
            json!({"method":"ping"}),
            json!({"method":"tools/list"}),
            json!({"method":"tools/call","params":{"name":"propose_memory","arguments":{}}}),
            json!({"method":"resources/read","params":{"uri":"meeting://synthetic"}}),
            json!({"method":"resources/templates/list"}),
            json!({"method":"resources/list"}),
            json!({"method":"prompts/list"}),
        ] {
            for has_parts in [false, true] {
                let mut context = RequestContext::new(
                    rmcp::model::NumberOrString::Number(1),
                    running.peer().clone(),
                );
                if has_parts {
                    context.extensions.insert(Request::new(()).into_parts().0);
                }
                let error = Meetings { remote: true }
                    .handle_request(serde_json::from_value(request.clone()).unwrap(), context)
                    .await
                    .unwrap_err();
                assert_eq!(error.message, "MCP_ACCESS_DENIED");
            }
        }
        running.cancel().await.unwrap();
    }
    #[tokio::test]
    async fn request_limit_precedes_token_file_access() {
        let root = tempfile::tempdir().unwrap();
        let root = Arc::new(root.path().to_path_buf());
        let slots = Arc::new(tokio::sync::Semaphore::new(0));
        let router = Router::new()
            .fallback(|| async { "unreachable" })
            .layer(middleware::from_fn(move |request, next| {
                authorize(root.clone(), slots.clone(), request, next)
            }));
        let response = hyper_util::service::TowerToHyperService::new(router);
        use hyper::service::Service;
        let request = Request::builder()
            .uri("/mcp")
            .header("host", "127.0.0.1:6791")
            .header(
                "authorization",
                format!("Bearer synthetic.{}", "0".repeat(64)),
            )
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            response.call(request).await.unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    async fn server() -> (
        std::net::SocketAddr,
        tokio::task::JoinHandle<io::Result<()>>,
    ) {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        (
            address,
            tokio::spawn(serve(
                listener,
                Router::new().fallback(|| async { "ready" }),
            )),
        )
    }
    #[tokio::test]
    async fn idle_connections_are_capped_before_headers() {
        let (address, server) = server().await;
        let mut held = Vec::new();
        for _ in 0..CONNECTIONS {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
            held.push(stream);
            tokio::task::yield_now().await;
        }
        let mut extra = tokio::net::TcpStream::connect(address).await.unwrap();
        let _ = extra
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await;
        let result =
            tokio::time::timeout(Duration::from_millis(500), extra.read(&mut [0; 64])).await;
        server.abort();
        assert!(
            matches!(result, Ok(Ok(0)) | Ok(Err(_))),
            "over-cap connection was served or retained"
        );
    }
    #[tokio::test]
    async fn blocked_response_writes_release_connection_permits() {
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let (mut client, stream) = tokio::io::duplex(64);
        let router = Router::new().fallback(|| async { vec![b'x'; 128 * 1024] });
        let mut stalled = tokio::spawn(connection(
            stream,
            router,
            slots.clone().try_acquire_owned().unwrap(),
        ));
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), client.read_exact(&mut [0; 1]))
            .await
            .expect("response must start before testing write backpressure")
            .unwrap();
        // Keep the reader open but stop consuming: the 128 KiB body cannot fit in 64 bytes.
        assert_eq!(slots.available_permits(), 0);
        assert!(!stalled.is_finished());
        tokio::time::timeout(CONNECTION_TIMEOUT + Duration::from_secs(1), &mut stalled)
            .await
            .expect("blocked response writes retained the connection permit")
            .unwrap();
        assert_eq!(slots.available_permits(), 1);

        let (mut next, stream) = tokio::io::duplex(64);
        let served = tokio::spawn(connection(
            stream,
            Router::new().fallback(|| async { "ready" }),
            slots.clone().try_acquire_owned().unwrap(),
        ));
        next.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), next.read_to_end(&mut response))
            .await
            .expect("a subsequent connection must be served")
            .unwrap();
        served.await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with(b"ready"));
        assert_eq!(slots.available_permits(), 1);
    }
    #[tokio::test]
    async fn partial_headers_expire() {
        let (address, server) = server().await;
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\nHost:").await.unwrap();
        let mut response = Vec::new();
        let result = tokio::time::timeout(
            HEADER_TIMEOUT + Duration::from_secs(1),
            stream.read_to_end(&mut response),
        )
        .await;
        server.abort();
        assert!(
            result.is_ok(),
            "partial headers retained the connection beyond its deadline"
        );
    }
}
