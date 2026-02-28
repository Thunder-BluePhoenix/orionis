use crate::models::TraceSummary;
use crate::store::DbHandle;
use uuid::Uuid;
use anyhow::Result;

pub struct IntelligenceEngine {
    db: DbHandle,
}

impl IntelligenceEngine {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    pub async fn analyze_root_cause(&self, trace_id: Uuid, tenant_id: Option<String>) -> Result<serde_json::Value> {
        let events = self.db.get_trace_events(trace_id, tenant_id.clone()).await?;
        if events.is_empty() {
            return Err(anyhow::anyhow!("Trace not found"));
        }

        let has_error = events.iter().any(|e| e.event_type == crate::models::EventType::Exception);
        
        // Context for LLM
        let failing_event = events.iter().find(|e| e.event_type == crate::models::EventType::Exception);
        let structural_hash = TraceSummary::calculate_structural_hash(&events);

        // Simulate LLM analysis
        let root_cause = if let Some(e) = failing_event {
            format!("Exception in {}.{}: {}. The control flow reached this point after abnormal state mutation.", e.module, e.function_name, e.error_message.as_deref().unwrap_or("Unknown error"))
        } else {
            "No explicit exception found, but structural analysis indicates potential logical divergence.".to_string()
        };

        let patch = if has_error {
            Some("Add a boundary check before the failing call and implement a retry mechanism for transient dependencies.".to_string())
        } else {
            None
        };

        // Persist
        self.db.update_trace_intel(trace_id, Some(root_cause.clone()), patch.clone()).await?;

        Ok(serde_json::json!({
            "trace_id": trace_id,
            "root_cause": root_cause,
            "suggested_patch": patch,
            "structural_hash": structural_hash,
            "confidence": 0.85
        }))
    }

    pub async fn get_performance_tuning(&self, trace_id: Uuid, tenant_id: Option<String>) -> Result<serde_json::Value> {
        let events = self.db.get_trace_events(trace_id, tenant_id).await?;
        
        // Find long running spans
        let mut bottlenecks = Vec::new();
        for e in &events {
            if let Some(dur) = e.duration_us {
                if dur > 500000 { // > 500ms
                    bottlenecks.push(serde_json::json!({
                        "function": e.function_name,
                        "duration_ms": dur / 1000,
                        "suggestion": "Consider caching the result or parallelizing this operation."
                    }));
                }
            }
        }

        Ok(serde_json::json!({
            "trace_id": trace_id,
            "bottlenecks": bottlenecks,
            "overall_recommendation": if bottlenecks.is_empty() { "Performance within targets." } else { "Multiple bottlenecks detected in core logic path." }
        }))
    }

    pub async fn analyze_memory_leaks(&self, trace_id: Uuid, tenant_id: Option<String>) -> Result<serde_json::Value> {
        let events = self.db.get_trace_events(trace_id, tenant_id).await?;
        if events.is_empty() { return Err(anyhow::anyhow!("Trace not found")); }

        let mut leaking_spans = Vec::new();
        let mut total_retained = 0;

        // Simplified heuristic: Find exit events with memory significantly higher than enter events
        // (Assuming a simplified data model where exits hold the delta or final state)
        for e in &events {
            if let Some(mem) = e.memory_usage_bytes {
                if mem > 10_000_000 { // Retained > 10MB
                    leaking_spans.push(serde_json::json!({
                        "span_id": e.span_id,
                        "function": e.function_name,
                        "retained_bytes": mem,
                        "suggestion": "Large continuous allocation detected. Check for unreleased collections or missing garbage collection triggers."
                    }));
                    total_retained += mem;
                }
            }
        }

        Ok(serde_json::json!({
            "trace_id": trace_id,
            "total_abnormal_retention_bytes": total_retained,
            "leaking_spans": leaking_spans,
            "status": if leaking_spans.is_empty() { "Healthy" } else { "Memory Leak Detected" }
        }))
    }

    pub async fn generate_sandbox_replay(&self, trace_id: Uuid, tenant_id: Option<String>) -> Result<serde_json::Value> {
        let events = self.db.get_trace_events(trace_id, tenant_id).await?;
        if events.is_empty() { return Err(anyhow::anyhow!("Trace not found")); }

        let structural_hash = TraceSummary::calculate_structural_hash(&events);
        
        // Strip sensitive data for the sandbox
        let mut clean_events = Vec::new();
        for mut e in events {
            e.locals = None; // Strip PII/Secrets
            clean_events.push(e);
        }

        Ok(serde_json::json!({
            "replay_ticket": Uuid::new_v4(),
            "trace_id": trace_id,
            "structural_hash": structural_hash,
            "event_count": clean_events.len(),
            "sandbox_payload": clean_events,
            "instructions": "Download this payload to run `orion replay --sandbox <file>` locally."
        }))
    }
}
