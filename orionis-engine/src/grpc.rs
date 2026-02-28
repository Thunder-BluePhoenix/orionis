use tonic::{Request, Response, Status};
use uuid::Uuid;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::store::DbHandle;
use crate::ws::WsBroadcaster;
use crate::models::{AgentLanguage, EventType, TraceEvent, LocalVar};

pub mod proto {
    tonic::include_proto!("orionis");
}

pub struct IngestService {
    pub db: DbHandle,
    pub ws: WsBroadcaster,
    pub wal: Arc<crate::wal::WalManager>,
    pub ingestion_limit: Arc<tokio::sync::Semaphore>,
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

            // Backpressure check
            let _permit = match self.ingestion_limit.try_acquire() {
                Ok(p) => p,
                Err(_) => {
                    tracing::warn!("Ingestion limit reached, dropping gRPC stream message");
                    continue; 
                }
            };
            
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

            let mut ev = TraceEvent {
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
                tenant_id: if msg.tenant_id.is_empty() { None } else { Some(msg.tenant_id.clone()) },
                event_id: if msg.event_id.is_empty() { None } else { Some(msg.event_id) },
                memory_usage_bytes: msg.memory_usage_bytes,
                fingerprint: None,
                is_folded: None,
                fold_count: None,
                version: None,
                environment: None,
            };
            ev.calculate_event_id();

            let tid = ev.tenant_id.clone();

            // WAL First (Durability)
            if let Err(e) = self.wal.append(ev.clone(), tid.clone()) {
                tracing::error!("WAL append failed during gRPC stream: {}", e);
            }

            // Broadcast to WebSockets live
            if let Ok(json) = serde_json::to_string(&ev) {
                self.ws.broadcast(json);
            }

            let tid = ev.tenant_id.clone();
            // Ingest to database immediately
            if let Err(e) = self.db.ingest_events(vec![ev], tid).await {
                crate::metrics::INGESTION_ERRORS.inc();
                tracing::error!("Database error during gRPC stream: {}", e);
            } else {
                crate::metrics::INGESTION_TOTAL.inc();
            }
        }

        Ok(Response::new(proto::IngestResponse {
            success: true,
            message: "Stream completed successfully".into(),
        }))
    }
}

pub struct ControlService {
    pub ingestion_limit: Arc<tokio::sync::Semaphore>,
    pub cluster: Arc<crate::clustering::ClusterManager>,
}

#[tonic::async_trait]
impl proto::control_server::Control for ControlService {
    type HeartbeatStreamStream = ReceiverStream<Result<proto::ControlCommand, Status>>;

    async fn heartbeat_stream(
        &self,
        request: Request<tonic::Streaming<proto::AgentHeartbeat>>,
    ) -> Result<Response<Self::HeartbeatStreamStream>, Status> {
        let mut in_stream = request.into_inner();
        let (tx, rx) = mpsc::channel(4);
        let ingestion_limit = self.ingestion_limit.clone();
        let cluster = self.cluster.clone();

        tokio::spawn(async move {
            while let Some(msg_res) = in_stream.next().await {
                match msg_res {
                    Ok(hb) => {
                        // Logic for Adaptive Sampling
                        let mut mode = proto::TracingMode::ModeDev;
                        let mut rate = 100;

                        // If engine is under extreme backpressure, force SAFE mode
                        if ingestion_limit.available_permits() < 50 {
                            mode = proto::TracingMode::ModeSafe;
                            rate = 50;
                        }

                        // If agent buffer is nearly full, force SAFE mode
                        if hb.buffer_usage_percent > 80 {
                            mode = proto::TracingMode::ModeSafe;
                            rate = 20;
                        }

                        let cmd = proto::ControlCommand {
                            mode: mode as i32,
                            sampling_rate_percent: rate,
                            kill_switch: false,
                        };

                        if let Err(_) = tx.send(Ok(cmd)).await {
                            break;
                        }

                        // Update local health state
                        let health = crate::models::AgentHealth {
                            agent_id: hb.agent_id,
                            hostname: hb.hostname,
                            language: match hb.language {
                                1 => crate::models::AgentLanguage::Python,
                                2 => crate::models::AgentLanguage::Go,
                                3 => crate::models::AgentLanguage::Rust,
                                4 => crate::models::AgentLanguage::Cpp,
                                _ => crate::models::AgentLanguage::Unknown,
                            },
                            buffer_usage_percent: hb.buffer_usage_percent,
                            events_dropped: hb.events_dropped,
                            active_spans: hb.active_spans,
                            pid: hb.pid,
                            last_heartbeat: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
                        };
                        cluster.update_agent_health(health).await;
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
