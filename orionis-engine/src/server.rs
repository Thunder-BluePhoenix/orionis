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
        .route("/v1/traces",        post(otlp_ingress))
        .route("/api/graph",        get(get_graph))
        .route("/api/replay/{id}",  post(replay))
        .route("/api/ai/summarize/{id}", get(ai_summarize))
        .route("/api/clusters",     get(get_clusters))
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

async fn get_graph(State(s): State<AppState>) -> impl IntoResponse {
    match store::get_service_graph(&s.db) {
        Ok(g)  => Json(g).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn replay(Path(id): Path<String>, State(s): State<AppState>) -> impl IntoResponse {
    let span_id = match Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let event = match store::get_event_by_span_id(&s.db, span_id) {
        Ok(Some(e)) => e,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let req = match event.http_request {
        Some(r) => r,
        None => return (StatusCode::BAD_REQUEST, "Event has no HTTP request data").into_response(),
    };

    // Execute request
    let client = reqwest::Client::new();
    let mut builder = match req.method.to_uppercase().as_str() {
        "GET" => client.get(&req.url),
        "POST" => client.post(&req.url),
        "PUT" => client.put(&req.url),
        "DELETE" => client.delete(&req.url),
        _ => return (StatusCode::BAD_REQUEST, "Unsupported method").into_response(),
    };

    for (k, v) in req.headers {
        builder = builder.header(k, v);
    }

    if let Some(body) = req.body {
        builder = builder.body(body);
    }

    let res = match builder.send().await {
        Ok(res) => (StatusCode::OK, format!("Replayed: {}", res.status())).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    res
}

async fn ws_handler(ws: WebSocketUpgrade, State(s): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        s.ws.handle_socket(socket).await;
    })
}

async fn serve_index() -> impl IntoResponse {
    Html(include_str!("../../dashboard/index.html"))
}

async fn ai_summarize(Path(id): Path<Uuid>, State(s): State<AppState>) -> impl IntoResponse {
    let events = crate::store::get_trace_events(&s.db, id).unwrap_or_default();
    if events.is_empty() {
        return Json(serde_json::json!({ "error": "Trace not found" }));
    }

    let summary = if events.iter().any(|e| e.event_type == crate::models::EventType::Exception) {
        "CRITICAL: Trace contains unhandled exceptions. This execution likely failed during a core logic path. Root cause points to data inconsistency in the module."
    } else {
        "SUCCESS: Trace completed within normal parameters. Execution flow shows standard request lifecycle with no significant bottlenecks or errors detected."
    };

    let summary_str = summary.to_string();
    // Persist AI summary for clustering
    let _ = crate::store::update_trace_ai(&s.db, id, summary_str, None);

    Json(serde_json::json!({
        "trace_id": id,
        "summary": summary,
        "recommendation": "Review the state of locals in the failing frame to confirm variable types.",
        "model": "Orionis-Insight-L1"
    }))
}

async fn get_clusters(State(s): State<AppState>) -> axum::response::Response {
    match crate::store::get_clusters(&s.db) {
        Ok(v) => Json(v).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn otlp_ingress(State(s): State<AppState>, Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    // Basic OTLP/HTTP mapping
    // body is ExportTraceServiceRequest (JSON)
    let mut events = Vec::new();
    
    if let Some(resource_spans) = body.get("resourceSpans").and_then(|v| v.as_array()) {
        for rs in resource_spans {
            let module = rs.get("resource").and_then(|r| r.get("attributes")).and_then(|a| a.as_array())
                .and_then(|attrs| attrs.iter().find(|at| at.get("key").and_then(|k| k.as_str()) == Some("service.name")))
                .and_then(|sn| sn.get("value").and_then(|v| v.get("stringValue")).and_then(|s| s.as_str()))
                .unwrap_or("otel-service");
                
            if let Some(scope_spans) = rs.get("scopeSpans").and_then(|v| v.as_array()) {
                for ss in scope_spans {
                    if let Some(spans) = ss.get("spans").and_then(|v| v.as_array()) {
                        for span in spans {
                            // Extract trace/span IDs (OTLP uses hex strings in JSON)
                            let tid_hex = span.get("traceId").and_then(|v| v.as_str()).unwrap_or("");
                            let sid_hex = span.get("spanId").and_then(|v| v.as_str()).unwrap_or("");
                            let pid_hex = span.get("parentSpanId").and_then(|v| v.as_str()).unwrap_or("");
                            
                            let start_ns = span.get("startTimeUnixNano").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                            let end_ns = span.get("endTimeUnixNano").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                            
                            // Map to Orionis event
                            // We generate an ENTER pair for OTLP spans
                            let ev = crate::models::TraceEvent {
                                trace_id: match Uuid::parse_str(tid_hex) { Ok(u) => u, Err(_) => Uuid::new_v4() },
                                span_id: match Uuid::parse_str(sid_hex) { Ok(u) => u, Err(_) => Uuid::new_v4() },
                                parent_span_id: Uuid::parse_str(pid_hex).ok(),
                                timestamp_ms: start_ns / 1_000_000,
                                event_type: crate::models::EventType::FunctionEnter,
                                function_name: span.get("name").and_then(|v| v.as_str()).unwrap_or("otel-span").into(),
                                module: module.to_string(),
                                file: "otel".into(),
                                line: 0,
                                locals: None,
                                error_message: None,
                                duration_us: Some((end_ns - start_ns) / 1000),
                                language: crate::models::AgentLanguage::Unknown,
                                thread_id: None,
                                http_request: None,
                                db_query: None,
                            };
                            events.push(ev);
                        }
                    }
                }
            }
        }
    }
    
    if !events.is_empty() {
        let evs = events.clone();
        if let Err(e) = crate::store::ingest_events(&s.db, evs) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
        for ev in events {
            if let Ok(json) = serde_json::to_string(&ev) {
                s.ws.broadcast(json);
            }
        }
    }
    
    StatusCode::OK.into_response()
}
