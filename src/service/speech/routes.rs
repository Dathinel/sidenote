use crate::core::speech_sessions::{Error, MAX_PIECE_BYTES, PieceRequest, SessionRequest, Store};
use axum::{
    Extension, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub(super) struct Owner(pub String);
pub(super) fn failure(error: Error) -> Response {
    let status = match error {
        Error::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
        Error::Missing => StatusCode::NOT_FOUND,
        Error::Conflict => StatusCode::CONFLICT,
        Error::Storage => StatusCode::SERVICE_UNAVAILABLE,
        Error::Full(position) => {
            let mut response = reply(
                StatusCode::TOO_MANY_REQUESTS,
                json!({"code":"SPEECH_QUEUE_FULL","queue_pos":position}),
            );
            response
                .headers_mut()
                .insert("retry-after", "1".parse().expect("fixed header"));
            return response;
        }
    };
    reply(status, json!({"code":"SPEECH_REQUEST_REJECTED"}))
}
fn reply(status: StatusCode, body: serde_json::Value) -> Response {
    (
        status,
        [
            ("content-type", "application/json"),
            ("cache-control", "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}
async fn create(
    State(store): State<Arc<Store>>,
    Extension(owner): Extension<Owner>,
    bytes: Bytes,
) -> Response {
    match serde_json::from_slice::<SessionRequest>(&bytes)
        .map_err(Error::from)
        .and_then(|request| store.create(&owner.0, request))
        .and_then(|(id, fresh)| {
            let events = store.events(&owner.0, &id, 0)?;
            Ok((id, fresh, events.last().map_or(0, |e| e.seq)))
        }) {
        Ok((id, fresh, seq)) => reply(
            if fresh {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            json!({"next_seq":seq,"session":id}),
        ),
        Err(e) => failure(e),
    }
}
async fn upload(
    State(store): State<Arc<Store>>,
    Extension(owner): Extension<Owner>,
    Path((session, piece)): Path<(String, String)>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let metadata = (|| {
        if headers.get_all("x-piece").iter().count() != 1
            || headers.get("content-type").and_then(|v| v.to_str().ok()) != Some("audio/wav")
        {
            return Err(Error::Invalid);
        }
        let value = headers.get("x-piece").ok_or(Error::Invalid)?.as_bytes();
        let metadata: PieceRequest = serde_json::from_slice(value)?;
        let result = store.put(&owner.0, &session, &piece, metadata.clone(), &bytes)?;
        store.emit(
            &session,
            "piece.accepted",
            json!({"piece":piece,"track":metadata.track,
            "index":metadata.index,"bytes":bytes.len(),"queue_pos":result.1.max(1)}),
        )?;
        Ok(result)
    })();
    match metadata {
        Ok((fresh, position)) => reply(
            if fresh {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            json!({"piece":piece,"state":if fresh {"accepted"} else {"have"},"queue_pos":position}),
        ),
        Err(e) => failure(e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finish {
    pieces_total: usize,
}
async fn finish(
    State(store): State<Arc<Store>>,
    Extension(owner): Extension<Owner>,
    Path(session): Path<String>,
    bytes: Bytes,
) -> Response {
    match serde_json::from_slice::<Finish>(&bytes)
        .map_err(Error::from)
        .and_then(|request| store.finish(&owner.0, &session, request.pieces_total))
    {
        Ok(()) => reply(StatusCode::OK, json!({"state":"accepted"})),
        Err(e) => failure(e),
    }
}
async fn snapshot(
    State(store): State<Arc<Store>>,
    Extension(owner): Extension<Owner>,
    Path((session, piece)): Path<(String, String)>,
) -> Response {
    match store.snapshot(&owner.0, &session, &piece) {
        Ok(value) => reply(StatusCode::OK, value),
        Err(e) => failure(e),
    }
}
pub(super) fn router(root: PathBuf) -> Router {
    Router::new()
        .route("/v1/sessions/{session}/pieces/{piece}", get(snapshot))
        .route("/v1/sessions", post(create))
        .route("/v1/sessions/{session}/events", get(super::events::events))
        .route("/v1/sessions/{session}/pieces/{piece}", put(upload))
        .route("/v1/sessions/{session}/finish", post(finish))
        .layer(DefaultBodyLimit::max(MAX_PIECE_BYTES))
        .with_state(Arc::new(Store(root)))
}
