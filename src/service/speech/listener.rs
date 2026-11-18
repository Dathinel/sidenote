use crate::mcp::access;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use std::{io, path::PathBuf, sync::Arc, time::Duration};

const BODY_LIMIT: usize = 10_000_000;
fn reject(status: StatusCode) -> Response {
    (status, "SPEECH_REQUEST_REJECTED").into_response()
}
async fn authorize(
    root: Arc<PathBuf>,
    hosts: Arc<Vec<String>>,
    slots: Arc<tokio::sync::Semaphore>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let Ok(_slot) = slots.try_acquire_owned() else {
        return reject(StatusCode::SERVICE_UNAVAILABLE);
    };
    if request.headers().contains_key("origin")
        || request.uri().query().is_some()
        || request.headers().get_all("host").iter().count() != 1
        || !request
            .headers()
            .get("host")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|host| hosts.iter().any(|allowed| allowed == host))
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
        .and_then(|b| access::authenticate_speech(&root, b).ok());
    let Some(grant) = grant else {
        return reject(StatusCode::UNAUTHORIZED);
    };
    let Ok(identity) = serde_json::to_vec(&grant) else {
        return reject(StatusCode::UNAUTHORIZED);
    };
    request.extensions_mut().insert(grant.clone());
    request
        .extensions_mut()
        .insert(super::routes::Owner(crate::core::speech_sessions::digest(
            &identity,
        )));
    if request
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<u64>().ok())
        .is_some_and(|n| n > BODY_LIMIT as u64)
    {
        return reject(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let (parts, body) = request.into_parts();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let body = to_bytes(body, BODY_LIMIT)
            .await
            .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
        Ok::<_, StatusCode>(next.run(Request::from_parts(parts, Body::from(body))).await)
    })
    .await;
    if access::check(&root, &grant).is_err() {
        return reject(StatusCode::UNAUTHORIZED);
    }
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(status)) => reject(status),
        Err(_) => reject(StatusCode::GATEWAY_TIMEOUT),
    }
}
fn argument(name: &str) -> io::Result<Option<std::ffi::OsString>> {
    #[cfg(all(test, feature = "asr-runtime"))]
    if super::proofs::active() {
        return Ok(match name {
            "--speech-port" => Some("0".into()),
            "--runtime-library" => std::env::var_os("SIDENOTE_L4_RUNTIME"),
            _ => None,
        });
    }
    let args: Vec<_> = std::env::args_os().collect();
    let positions: Vec<_> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == name)
        .collect();
    match positions.as_slice() {
        [] => Ok(None),
        [(index, _)] => args
            .get(index + 1)
            .cloned()
            .map(Some)
            .ok_or_else(|| io::ErrorKind::InvalidInput.into()),
        _ => Err(io::ErrorKind::InvalidInput.into()),
    }
}
pub(super) fn runtime_library() -> io::Result<PathBuf> {
    Ok(argument("--runtime-library")?
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?.with_file_name(crate::platform::asr_library_name())))
}
pub(super) fn runtime_probe() -> io::Result<serde_json::Value> {
    let library = runtime_library()?;
    #[cfg(feature = "asr-runtime")]
    let (state, reason, version) = {
        static PROBE: std::sync::OnceLock<(&str, Option<&str>, Option<&str>)> =
            std::sync::OnceLock::new();
        *PROBE.get_or_init(|| {
            if crate::core::asr::initialize(&library).is_ok() {
                ("ready", None, crate::core::asr::runtime_version())
            } else {
                ("unavailable", Some("runtime-refused"), None)
            }
        })
    };
    #[cfg(not(feature = "asr-runtime"))]
    let (state, reason, version) = {
        let _ = library;
        (
            "unavailable",
            Some("built-without-asr-runtime"),
            None::<&str>,
        )
    };
    let store = crate::core::model_catalog::ModelStore::default();
    let mut languages = Vec::new();
    let mut models = Vec::new();
    if state == "ready" && store.installed_paths("silero-vad")?.is_some() {
        for language in ["hi", "te"] {
            let id = format!("indicconformer-{language}-ctc");
            if store.installed_paths(&id)?.is_some() {
                languages.push(language);
                models.push(json!({"id":id,"sha256":crate::core::model_catalog::model(&id)?.files[0].sha256}));
            }
        }
    }
    Ok(
        json!({"state":state,"reason":reason,"runtime_version":version,"languages":languages,"models":models}),
    )
}
pub(super) fn bind() -> io::Result<(tokio::net::TcpListener, Router)> {
    let port = argument("--speech-port")?
        .map(|v| v.to_string_lossy().parse::<u16>())
        .transpose()
        .map_err(|_| io::ErrorKind::InvalidInput)?
        .unwrap_or(6792);
    let socket = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))?;
    socket.set_nonblocking(true)?;
    let address = socket.local_addr()?;
    let mut hosts = vec![address.to_string()];
    if let Some(host) = argument("--speech-host")? {
        let host = host.to_str().ok_or(io::ErrorKind::InvalidInput)?;
        if host.len() > 253
            || !host.ends_with(".ts.net")
            || !host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        hosts.extend([host.to_owned(), format!("{host}:443")]);
    }
    let host = json!({"api":1,"role":"server","asr":runtime_probe()?,
        "limits":{"max_piece_bytes":BODY_LIMIT},"speech_jobs":true});
    let hosts = Arc::new(hosts);
    let root = Arc::new(crate::core::state_dir());
    let slots = Arc::new(tokio::sync::Semaphore::new(8));
    let router = Router::new()
        .merge(super::routes::router(root.join("local/speech")))
        .route(
            "/v1/host",
            get(move || {
                let mut host = host.clone();
                async move {
                    match super::events::host() {
                        Ok(status) => {
                            host["queue"] = status["queue"].clone();
                            host["asr"] = status["asr"].clone();
                        }
                        Err(_) => return reject(StatusCode::SERVICE_UNAVAILABLE),
                    }
                    let body = host.to_string();
                    (
                        [
                            ("content-type", "application/json"),
                            ("cache-control", "no-store"),
                        ],
                        body,
                    )
                        .into_response()
                }
            }),
        )
        .layer(middleware::from_fn(move |request, next| {
            authorize(root.clone(), hosts.clone(), slots.clone(), request, next)
        }));
    crate::core::store_io::write(
        &crate::core::state_dir().join("local/speech/endpoint.json"),
        &json!({"address":address.to_string()}),
    )?;
    Ok((tokio::net::TcpListener::from_std(socket)?, router))
}
