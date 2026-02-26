use tonic::{Request, Response, Status};
use uuid::Uuid;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::store::DbHandle;
use crate::ws::WsBroadcaster;
use crate::models::{self, AgentLanguage, EventType, TraceEvent, LocalVar};

pub mod proto {
    tonic::include_proto!("orionis");
}

pub struct IngestService {
    pub db: DbHandle,
    pub ws: WsBroadcaster,
}

#[tonic::async_trait]
impl proto::ingest_server::Ingest for IngestService {
    async fn stream_events(
        &self,
        request: Request<tonic::Streaming<proto::TraceEvent>>,
    ) -> Result<Response<proto::IngestResponse>, Status> {
        let mut stream = request.into_inner();

        while let Some(msg_res) = stream.next().await {
            let msg = msg_res?;
            
            // Map Protobuf to our Native models
            let trace_id = Uuid::parse_str(&msg.trace_id).unwrap_or_else(|_| Uuid::nil());
            let span_id = Uuid::parse_str(&msg.span_id).unwrap_or_else(|_| Uuid::nil());
            let parent_span_id = if msg.parent_span_id.is_empty() {
                None
            } else {
                Uuid::parse_str(&msg.parent_span_id).ok()
            };

            let event_type = match msg.event_type {
                1 => EventType::FunctionEnter,
                2 => EventType::FunctionExit,
                3 => EventType::Exception,
                4 => EventType::AsyncSpawn,
                5 => EventType::AsyncResume,
                6 => EventType::HttpRequest,
                7 => EventType::HttpResponse,
                8 => EventType::DbQuery,
                _ => continue, // ignore unknown
            };

            let language = match msg.language {
                1 => AgentLanguage::Python,
                2 => AgentLanguage::Go,
                3 => AgentLanguage::Rust,
                4 => AgentLanguage::Cpp,
                _ => AgentLanguage::Python, // default fallback
            };

            let locals = if msg.locals.is_empty() {
                None
            } else {
                Some(msg.locals.into_iter().map(|l| LocalVar {
                    name: l.name,
                    value: l.value,
                    type_name: l.type_name,
                }).collect())
            };

            let ev = TraceEvent {
                trace_id,
                span_id,
                parent_span_id,
                timestamp_ms: msg.timestamp_ms,
                event_type,
                function_name: msg.function_name,
                module: msg.module,
                file: msg.file,
                line: msg.line,
                locals,
                error_message: if msg.error_message.is_empty() { None } else { Some(msg.error_message) },
                duration_us: msg.duration_us,
                language,
                thread_id: if msg.thread_id.is_empty() { None } else { Some(msg.thread_id) },
                http_request: msg.http_request.map(|hr| crate::models::HttpRequest {
                    method: hr.method,
                    url: hr.url,
                    headers: hr.headers,
                    body: if hr.body.is_empty() { None } else { Some(hr.body) },
                }),
                db_query: msg.db_query.map(|dq| crate::models::DbQuery {
                    query: dq.query,
                    driver: dq.driver,
                    duration_us: dq.duration_us,
                }),
            };

            // Broadcast to WebSockets live
            if let Ok(json) = serde_json::to_string(&ev) {
                self.ws.broadcast(json);
            }

            // Ingest to database immediately
            if let Err(e) = crate::store::ingest_events(&self.db, vec![ev]) {
                tracing::error!("Database error during gRPC stream: {}", e);
            }
        }

        Ok(Response::new(proto::IngestResponse {
            success: true,
            message: "Stream completed successfully".into(),
        }))
    }
}
