use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
