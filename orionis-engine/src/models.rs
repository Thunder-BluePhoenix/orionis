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
}

/// A serialized local variable captured at a trace point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalVar {
    pub name: String,
    pub value: String,  // repr() / Debug string, truncated
    pub type_name: String,
}

/// Environment snapshot captured at the start of a trace session.
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
}

/// Which language/runtime sent this event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentLanguage {
    Python,
    Go,
    Rust,
    Cpp,
}

/// A complete trace — a logical unit of execution (e.g. one request, one test, one crash).
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
}

/// Incoming payload from an agent — either a single event or a batch.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum IngestPayload {
    Single(TraceEvent),
    Batch(Vec<TraceEvent>),
}
