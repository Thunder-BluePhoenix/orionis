use crate::models::{TraceEvent, EventType};
use uuid::Uuid;
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MockData {
    pub span_id: Uuid,
    pub call_signature: String,
    pub result: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReplaySnapshot {
    pub trace_id: Uuid,
    pub mocks: Vec<MockData>,
}

pub struct ReplayManager;

impl ReplayManager {
    pub fn create_snapshot(trace_id: Uuid, events: &[TraceEvent]) -> ReplaySnapshot {
        let mut mocks = Vec::new();

        for event in events {
            match event.event_type {
                EventType::DbQuery => {
                    if let Some(query) = &event.db_query {
                        mocks.push(MockData {
                            span_id: event.span_id,
                            call_signature: format!("SQL:{}", query.query),
                            result: serde_json::json!({
                                "status": "recorded",
                                "query": query.query,
                                "duration_us": query.duration_us,
                                "driver": query.driver
                            }),
                        });
                    }
                }
                EventType::HttpResponse => {
                    if let Some(resp) = &event.http_request { // Note: we currently capture full requests, we might need a dedicated HttpResponse model later
                        mocks.push(MockData {
                            span_id: event.span_id,
                            call_signature: format!("HTTP:{} {}", resp.method, resp.url),
                            result: serde_json::json!({
                                "status": "recorded",
                                "url": resp.url,
                                "method": resp.method
                            }),
                        });
                    }
                }
                _ => {}
            }
        }

        ReplaySnapshot { trace_id, mocks }
    }
}
