use axum::{
    Router,
    extract::{Path, State, ws::WebSocketUpgrade},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{delete, get, post},
};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

use crate::{
    models::IngestPayload,
    store::{self, DbHandle},
    ws::WsBroadcaster,
};

#[derive(Clone)]
pub struct AppState {
    pub db: DbHandle,
    pub ws: WsBroadcaster,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/traces",       get(list_traces))
        .route("/api/traces/{id}",  get(get_trace))
        .route("/api/ingest",       post(ingest))
        .route("/api/clear",        delete(clear_all))
        .route("/ws/live",          get(ws_handler))
        .route("/",                 get(serve_index))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn list_traces(State(s): State<AppState>) -> impl IntoResponse {
    match store::list_traces(&s.db) {
        Ok(t)  => Json(t).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_trace(State(s): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&id) {
        Ok(u)  => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid trace ID").into_response(),
    };
    match store::get_trace_events(&s.db, uuid) {
        Ok(ev) => Json(ev).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn ingest(State(s): State<AppState>, Json(payload): Json<IngestPayload>) -> impl IntoResponse {
    let events = match payload {
        IngestPayload::Single(e) => vec![e],
        IngestPayload::Batch(v)  => v,
    };
    for ev in &events {
        if let Ok(json) = serde_json::to_string(ev) {
            s.ws.broadcast(json);
        }
    }
    match store::ingest_events(&s.db, events) {
        Ok(_)  => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn clear_all(State(s): State<AppState>) -> impl IntoResponse {
    match store::clear_all(&s.db) {
        Ok(_)  => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(s): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        s.ws.handle_socket(socket).await;
    })
}

async fn serve_index() -> impl IntoResponse {
    Html(include_str!("../../dashboard/index.html"))
}
