use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

/// The type of runtime event captured by an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    FunctionEnter,
    FunctionExit,
    Exception,
    AsyncSpawn,
    AsyncResume,
    HttpRequest,
    HttpResponse,
    DbQuery,
}

/// A serialized local variable captured at a trace point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalVar {
    pub name: String,
    pub value: String,  // repr() / Debug string, truncated
    pub type_name: String,
}

/// Environment snapshot captured at the start of a trace session.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvSnapshot {
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub env_vars: Vec<(String, String)>,   // filtered key-value pairs
    pub working_dir: String,
    pub captured_at: u64,                   // unix timestamp ms
}

/// The atomic unit of trace data. Every agent emits TraceEvents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub trace_id: Uuid,
    pub span_id: Uuid,
    pub parent_span_id: Option<Uuid>,
    pub timestamp_ms: u64,
    pub event_type: EventType,
    pub function_name: String,
    pub module: String,
    pub file: String,
    pub line: u32,
    pub locals: Option<Vec<LocalVar>>,
    pub error_message: Option<String>,
    pub duration_us: Option<u64>,   // microseconds, set on FunctionExit
    pub language: AgentLanguage,
    pub thread_id: Option<String>,
    pub http_request: Option<HttpRequest>,
    pub db_query: Option<DbQuery>,
    pub tenant_id: Option<String>,
    pub event_id: Option<String>,
    pub memory_usage_bytes: Option<u64>,
    pub fingerprint: Option<String>,
    pub is_folded: Option<bool>,
    pub fold_count: Option<u32>,
    pub version: Option<String>,
    pub environment: Option<String>,
}

impl TraceEvent {
    pub fn calculate_event_id(&mut self) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        self.trace_id.hash(&mut hasher);
        self.span_id.hash(&mut hasher);
        self.timestamp_ms.hash(&mut hasher);
        // Event type as string for hashing
        format!("{:?}", self.event_type).hash(&mut hasher);
        
        self.event_id = Some(format!("{:x}", hasher.finish()));
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbQuery {
    pub query: String,
    pub driver: String,
    pub duration_us: u64,
}

/// Which language/runtime sent this event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentLanguage {
    Python,
    Go,
    Rust,
    Cpp,
    C,
    Java,
    Node,
    Unknown,
}

/// A complete trace — a logical unit of execution (e.g. one request, one test, one crash).
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub trace_id: Uuid,
    pub name: String,              // e.g. function name or request path
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub has_error: bool,
    pub language: AgentLanguage,
    pub env: Option<EnvSnapshot>,
    pub event_count: usize,
    pub tenant_id: Option<String>,
    pub version: Option<String>,
    pub environment: Option<String>,
}

/// Summary of a trace for the list view (no events, just metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSummary {
    pub trace_id: Uuid,
    pub name: String,
    pub started_at: u64,
    pub duration_ms: Option<u64>,
    pub has_error: bool,
    pub language: AgentLanguage,
    pub event_count: usize,
    pub thread_ids: Vec<String>,
    pub ai_cluster_key: Option<String>,
    pub ai_summary: Option<String>,
    pub tenant_id: Option<String>,
    pub tags: Vec<String>,
    pub assigned_to: Option<String>,
    pub structural_hash: Option<String>,
    pub integrity_score: Option<u8>,
    pub version: Option<String>,
    pub environment: Option<String>,
    pub ai_root_cause: Option<String>,
    pub ai_patch_suggestion: Option<String>,
}

impl TraceSummary {
    pub fn calculate_structural_hash(events: &[TraceEvent]) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn build_pattern(parent_id: Option<Uuid>, events: &[TraceEvent]) -> String {
            let mut children: Vec<_> = events
                .iter()
                .filter(|e| e.parent_span_id == parent_id && e.event_type == EventType::FunctionEnter)
                .collect();
            children.sort_by_key(|e| e.timestamp_ms);

            let mut pattern = String::new();
            for child in children {
                pattern.push_str(&format!("{}.{}", child.module, child.function_name));
                let sub = build_pattern(Some(child.span_id), events);
                if !sub.is_empty() {
                    pattern.push('(');
                    pattern.push_str(&sub);
                    pattern.push(')');
                }
            }
            pattern
        }

        let pattern = build_pattern(None, events);
        let mut hasher = DefaultHasher::new();
        pattern.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    pub fn calculate_integrity_score(events: &[TraceEvent]) -> u8 {
        if events.is_empty() { return 0; }

        let mut enter_spans = std::collections::HashSet::new();
        let mut exit_spans = std::collections::HashSet::new();
        let mut spans_with_locals = 0;
        let mut connected_spans = 0;
        let mut total_spans = 0;

        for ev in events {
            if ev.event_type == EventType::FunctionEnter {
                enter_spans.insert(ev.span_id);
                total_spans += 1;
                if ev.locals.is_some() && !ev.locals.as_ref().unwrap().is_empty() {
                    spans_with_locals += 1;
                }
                if ev.parent_span_id.is_some() || total_spans == 1 {
                    connected_spans += 1;
                }
            } else if ev.event_type == EventType::FunctionExit {
                exit_spans.insert(ev.span_id);
            }
        }

        if total_spans == 0 { return 0; }

        // 1. Sequence completeness (40%) - Balanced Enter/Exit
        let matched_pairs = enter_spans.intersection(&exit_spans).count();
        let sequence_score = (matched_pairs as f32 / total_spans as f32) * 40.0;

        // 2. Data Depth (30%) - Presence of locals
        let data_score = (spans_with_locals as f32 / total_spans as f32) * 30.0;

        // 3. Connectivity (20%) - Parent-child linkage
        let connectivity_score = (connected_spans as f32 / total_spans as f32) * 20.0;

        // 4. Metadata presence (10%) - Constant score if basic data exists
        let metadata_score = 10.0;

        (sequence_score + data_score + connectivity_score + metadata_score).min(100.0) as u8
    }

    /// Compresses a sequence of events by folding repetitive structural patterns (loops).
    pub fn compress_timeline(events: Vec<TraceEvent>) -> Vec<TraceEvent> {
        if events.is_empty() { return events; }

        // Step 1: Group events by parent_span_id to identify siblings
        let mut by_parent: std::collections::HashMap<Option<Uuid>, Vec<TraceEvent>> = std::collections::HashMap::new();
        let _event_map: std::collections::HashMap<Uuid, TraceEvent> = events.iter().cloned().map(|e| (e.span_id, e)).collect();

        // Only consider FunctionEnter events for folding structure
        for ev in events.iter().filter(|e| e.event_type == EventType::FunctionEnter) {
            by_parent.entry(ev.parent_span_id).or_default().push(ev.clone());
        }

        // Step 2: For each parent, find repetitive sequences of siblings
        let mut folded_span_ids = std::collections::HashSet::new();
        let mut fold_counts: std::collections::HashMap<Uuid, u32> = std::collections::HashMap::new();

        for (_parent_id, mut siblings) in by_parent {
            if siblings.len() < 2 { continue; }
            siblings.sort_by_key(|s| s.timestamp_ms);

            let mut i = 0;
            while i < siblings.len() {
                let current_fingerprint = &siblings[i].fingerprint;
                if current_fingerprint.is_none() { 
                    i += 1; 
                    continue; 
                }

                let mut j = i + 1;
                while j < siblings.len() && siblings[j].fingerprint == *current_fingerprint {
                    folded_span_ids.insert(siblings[j].span_id);
                    j += 1;
                }

                if j > i + 1 {
                    let count = (j - i) as u32;
                    fold_counts.insert(siblings[i].span_id, count);
                }
                i = j;
            }
        }

        // Step 3: Reconstruct the event list, omitting folded events and marking representatives
        let mut result = Vec::new();
        for ev in events {
            if folded_span_ids.contains(&ev.span_id) {
                continue;
            }
            
            let mut final_ev = ev;
            if let Some(count) = fold_counts.get(&final_ev.span_id) {
                final_ev.is_folded = Some(true);
                final_ev.fold_count = Some(*count);
            }
            result.push(final_ev);
        }

        result
    }

    /// Recursively calculates fingerprints for all spans in the trace.
    pub fn calculate_span_fingerprints(events: &mut [TraceEvent]) {
        if events.is_empty() { return; }

        let mut children_map: std::collections::HashMap<Option<Uuid>, Vec<Uuid>> = std::collections::HashMap::new();
        let mut event_index_map: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();

        for (i, ev) in events.iter().enumerate() {
            if ev.event_type == EventType::FunctionEnter {
                children_map.entry(ev.parent_span_id).or_default().push(ev.span_id);
            }
            event_index_map.insert(ev.span_id, i);
        }

        fn compute_for_span(
            span_id: Uuid,
            events: &mut [TraceEvent],
            children_map: &std::collections::HashMap<Option<Uuid>, Vec<Uuid>>,
            event_index_map: &std::collections::HashMap<Uuid, usize>,
        ) -> String {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let idx = *event_index_map.get(&span_id).unwrap();
            let module = events[idx].module.clone();
            let function = events[idx].function_name.clone();

            let mut pattern = format!("{}.{}", module, function);
            
            if let Some(children) = children_map.get(&Some(span_id)) {
                let mut child_patterns = Vec::new();
                for child_id in children {
                    child_patterns.push(compute_for_span(*child_id, events, children_map, event_index_map));
                }
                if !child_patterns.is_empty() {
                    pattern.push('(');
                    pattern.push_str(&child_patterns.join("|"));
                    pattern.push(')');
                }
            }

            let mut hasher = DefaultHasher::new();
            pattern.hash(&mut hasher);
            let fingerprint = format!("{:x}", hasher.finish());
            events[idx].fingerprint = Some(fingerprint.clone());
            fingerprint
        }

        // Identify roots (spans with no parent in this batch)
        let root_ids: Vec<Uuid> = children_map.get(&None).cloned().unwrap_or_default();
        for rid in root_ids {
            compute_for_span(rid, events, &children_map, &event_index_map);
        }
    }
}

/// Incoming payload from an agent — either a single event or a batch.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum IngestPayload {
    Single(TraceEvent),
    Batch(Vec<TraceEvent>),
}

/// Information about a node in the Orionis cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNode {
    pub node_id: String,
    pub http_addr: String,
    pub grpc_addr: String,
    pub status: NodeStatus,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    Active,
    Inactive,
    Diverged,
}

/// A comment or annotation left by a user on a specific trace or span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceComment {
    pub comment_id: Uuid,
    pub trace_id: Uuid,
    pub span_id: Option<Uuid>,
    pub user_id: String,
    pub text: String,
    pub timestamp_ms: u64,
    pub tenant_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MutationMetrics {
    pub mutation_count: usize,
    pub memory_delta_bytes: i64,
    pub changed_keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StateHeatmap {
    pub metrics: HashMap<Uuid, MutationMetrics>, // span_id -> metrics
}

impl StateHeatmap {
    pub fn calculate(events: &[TraceEvent]) -> Self {
        let mut metrics = HashMap::new();
        let mut enters: HashMap<Uuid, &TraceEvent> = HashMap::new();
        let mut exits: HashMap<Uuid, &TraceEvent> = HashMap::new();

        for event in events {
            match event.event_type {
                EventType::FunctionEnter => { enters.insert(event.span_id, event); }
                EventType::FunctionExit => { exits.insert(event.span_id, event); }
                _ => {}
            }
        }

        for (span_id, enter) in enters {
            if let Some(exit) = exits.get(&span_id) {
                let mut mutation_count = 0;
                let mut changed_keys = Vec::new();

                let enter_locals = enter.locals.as_ref();
                let exit_locals = exit.locals.as_ref();

                if let (Some(en), Some(ex)) = (enter_locals, exit_locals) {
                    let en_map: HashMap<String, String> = en.iter().map(|l| (l.name.clone(), l.value.clone())).collect();
                    for l in ex {
                        if let Some(old_val) = en_map.get(&l.name) {
                            if *old_val != l.value {
                                mutation_count += 1;
                                changed_keys.push(l.name.clone());
                            }
                        } else {
                            // New variable added
                            mutation_count += 1;
                            changed_keys.push(l.name.clone());
                        }
                    }
                }

                let memory_delta_bytes = match (enter.memory_usage_bytes, exit.memory_usage_bytes) {
                    (Some(en), Some(ex)) => ex as i64 - en as i64,
                    _ => 0,
                };

                metrics.insert(span_id, MutationMetrics {
                    mutation_count,
                    memory_delta_bytes,
                    changed_keys,
                });
            }
        }

        Self { metrics }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TracingMode {
    Dev,   // Full capture
    Safe,  // Sampled, no locals
    Error, // Only capture on exception
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHealth {
    pub agent_id: String,
    pub hostname: String,
    pub language: AgentLanguage,
    pub buffer_usage_percent: u64,
    pub events_dropped: u64,
    pub active_spans: u64,
    pub pid: String,
    pub last_heartbeat: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityAlert {
    pub alert_id: Uuid,
    pub trace_id: Uuid,
    pub alert_type: String, // e.g., "flow_deviation", "privilege_escalation"
    pub severity: String,   // "high", "medium", "low"
    pub message: String,
    pub timestamp_ms: u64,
    pub tenant_id: Option<String>,
}
