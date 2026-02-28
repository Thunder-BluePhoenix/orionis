use crate::models::{TraceEvent, TraceSummary, SecurityAlert};
use crate::store::DbHandle;
use uuid::Uuid;
use anyhow::Result;
use chrono::Utc;

pub struct SecurityAnalyzer {
    db: DbHandle,
}

impl SecurityAnalyzer {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    pub async fn analyze_trace(&self, trace_id: Uuid, events: &[TraceEvent], tenant_id: Option<String>) -> Result<()> {
        let structural_hash = TraceSummary::calculate_structural_hash(events);
        
        // 1. Detect Control-Flow Deviation
        // Strategy: If this hash hasn't been seen before, but it's very similar to a common one, 
        // or if we already have thousands of traces for this "Entrypoint" and this is a new path.
        let clusters_val = self.db.get_clusters(tenant_id.clone()).await?;
        if let Some(clusters) = clusters_val.as_array() {
            let mut found = false;
            let mut total_traces = 0;
            
            for cluster in clusters {
                let hash = cluster["cluster_key"].as_str().unwrap_or_default();
                let count = cluster["count"].as_u64().unwrap_or(0);
                
                if hash == structural_hash {
                    if count > 5 { // Threshold for "already known stable path"
                        found = true;
                    }
                } else {
                    total_traces += count;
                }
            }

            tracing::info!("Security analysis for trace {}: found={}, total_traces_baseline={}, current_hash={}", trace_id, found, total_traces, structural_hash);

            // Only alert if we have a significant baseline (> 100 regular traces)
            // and this is a "Unknown" execution path.
            if !found && total_traces > 100 {
                let alert = SecurityAlert {
                    alert_id: Uuid::new_v4(),
                    trace_id,
                    alert_type: "flow_deviation".to_string(),
                    severity: "medium".to_string(),
                    message: format!("New execution path detected (hash: {}) after {} stable runs.", structural_hash, total_traces),
                    timestamp_ms: Utc::now().timestamp_millis() as u64,
                    tenant_id: tenant_id.clone(),
                };
                self.db.add_security_alert(alert, tenant_id.clone()).await?;
            }
        }

        // 2. Detect Suspicious Environment Mutations
        // (Conceptual: Compare against baseline env vars if we had them)
        
        Ok(())
    }
}
