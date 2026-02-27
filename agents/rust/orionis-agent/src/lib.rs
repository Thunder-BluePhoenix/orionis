// Orionis Rust Agent — native tracing macros and instrumentation for Rust apps.
// Captures function enter/exit via macros, panics via panic hook, and sends to the engine.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH, Instant};
use std::thread;
use std::collections::VecDeque;

pub use uuid::Uuid;

// ── Generated Proto ─────────────────────────────────────────────────────────

pub mod proto {
    tonic::include_proto!("orionis");
}

// ── Types (mirror engine models) ──────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    FunctionEnter,
    FunctionExit,
    Exception,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalVar {
    pub name: String,
    pub value: String,
    pub type_name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TraceEvent {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub timestamp_ms: u64,
    pub event_type: EventType,
    pub function_name: String,
    pub module: String,
    pub file: String,
    pub line: u32,
    pub locals: Option<Vec<LocalVar>>,
    pub error_message: Option<String>,
    pub duration_us: Option<u64>,
    pub language: String,
    pub thread_id: String,
    pub http_request: Option<HttpRequest>,
}

// ── Senders ──────────────────────────────────────────────────────────────────

trait Sender: Send + Sync {
    fn send(&self, events: Vec<TraceEvent>);
}

struct HttpSender {
    url: String,
}

impl Sender for HttpSender {
    fn send(&self, events: Vec<TraceEvent>) {
        let url = format!("{}/api/ingest", self.url);
        let body = serde_json::to_vec(&events).unwrap_or_default();
        let _ = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_bytes(&body);
    }
}

struct GrpcSender {
    endpoint: String,
}

impl Sender for GrpcSender {
    fn send(&self, events: Vec<TraceEvent>) {
        let endpoint = self.endpoint.clone();
        // Since we are in a library that might be used in async or sync contexts, 
        // we spawn a blocking task or use a handle if available.
        // For simplicity in the agent, we fire and forget via a small internal runtime or thread.
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let mut client = match proto::ingest_client::IngestClient::connect(endpoint).await {
                    Ok(c) => c,
                    Err(_) => return,
                };

                let mut pb_events = Vec::new();
                for ev in events {
                    let mut pb_ev = proto::TraceEvent {
                        trace_id: ev.trace_id,
                        span_id: ev.span_id,
                        parent_span_id: ev.parent_span_id.unwrap_or_default(),
                        timestamp_ms: ev.timestamp_ms,
                        event_type: match ev.event_type {
                            EventType::FunctionEnter => proto::EventType::EventFunctionEnter as i32,
                            EventType::FunctionExit => proto::EventType::EventFunctionExit as i32,
                            EventType::Exception => proto::EventType::EventException as i32,
                        },
                        function_name: ev.function_name,
                        module: ev.module,
                        file: ev.file,
                        line: ev.line,
                        locals: ev.locals.unwrap_or_default().into_iter().map(|l| proto::LocalVar {
                            name: l.name,
                            value: l.value,
                            type_name: l.type_name,
                        }).collect(),
                        error_message: ev.error_message.unwrap_or_default(),
                        duration_us: ev.duration_us,
                        language: proto::AgentLanguage::LangRust as i32,
                    };
                    pb_events.push(pb_ev);
                }

                let stream = tokio_stream::iter(pb_events);
                let _ = client.stream_events(stream).await;
            });
        });
    }
}

// ── Agent ─────────────────────────────────────────────────────────────────────

thread_local! {
    static THREAD_TRACE_ID: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
}

pub struct Agent {
    pub engine_url: String,
    pub grpc_url: String,
    pub use_grpc: bool,
    batch: Mutex<VecDeque<TraceEvent>>,
    sender: Box<dyn Sender>,
}

static GLOBAL: OnceLock<Arc<Agent>> = OnceLock::new();

impl Agent {
    fn new(engine_url: &str, grpc_url: &str, use_grpc: bool) -> Arc<Self> {
        let sender: Box<dyn Sender> = if use_grpc {
            Box::new(GrpcSender { endpoint: grpc_url.to_string() })
        } else {
            Box::new(HttpSender { url: engine_url.to_string() })
        };

        Arc::new(Self {
            engine_url: engine_url.to_string(),
            grpc_url: grpc_url.to_string(),
            use_grpc,
            batch: Mutex::new(VecDeque::new()),
            sender,
        })
    }

    pub fn enqueue(&self, ev: TraceEvent) {
        let mut b = self.batch.lock().unwrap();
        b.push_back(ev);
    }

    pub fn flush(&self) {
        let events: Vec<TraceEvent> = {
            let mut b = self.batch.lock().unwrap();
            b.drain(..).collect()
        };
        if events.is_empty() { return; }
        self.sender.send(events);
    }
}

pub fn reset_trace() {
    THREAD_TRACE_ID.with(|tid| {
        *tid.borrow_mut() = Some(Uuid::new_v4().to_string());
    });
}

pub fn get_trace_id() -> String {
    THREAD_TRACE_ID.with(|tid| {
        let mut t = tid.borrow_mut();
        if t.is_none() {
            *t = Some(Uuid::new_v4().to_string());
        }
        t.clone().unwrap()
    })
}

pub fn inject_trace_headers(headers: &mut std::collections::HashMap<String, String>) {
    let tid = get_trace_id().replace("-", "");
    headers.insert("traceparent".to_string(), format!("00-{}-0000000000000000-01", tid));
}

pub fn extract_trace_headers(headers: &std::collections::HashMap<String, String>) {
    if let Some(tp) = headers.get("traceparent") {
        if tp.len() >= 55 {
            let tid_raw = &tp[3..35];
            let tid = format!("{}-{}-{}-{}-{}", 
                &tid_raw[0..8], &tid_raw[8..12], &tid_raw[12..16], &tid_raw[16..20], &tid_raw[20..32]);
            THREAD_TRACE_ID.with(|t| {
                *t.borrow_mut() = Some(tid);
            });
        }
    }
}

/// Initialise the Orionis Rust agent. Call this once at the start of main().
pub fn start(engine_url: &str) {
    let grpc_url = "http://localhost:7701";
    let agent = Agent::new(engine_url, grpc_url, true);
    let _ = GLOBAL.set(agent.clone());
    install_panic_hook(agent.clone());
    start_sender(agent);
    send_env_snapshot(engine_url);
    eprintln!("[Orionis] Rust agent started — engine: {} | grpc: {}", engine_url, grpc_url);
}

pub fn agent() -> Option<&'static Arc<Agent>> {
    GLOBAL.get()
}

pub fn install_panic_hook(ag: Arc<Agent>) {
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.to_string();
        let (file, line) = info.location()
            .map(|l| (l.file().to_string(), l.line()))
            .unwrap_or_else(|| ("unknown".into(), 0));

        ag.enqueue(TraceEvent {
            trace_id: get_trace_id(),
            span_id: Uuid::new_v4().to_string(),
            parent_span_id: None,
            timestamp_ms: now_ms(),
            event_type: EventType::Exception,
            function_name: "panic".into(),
            module: env!("CARGO_PKG_NAME").into(),
            file,
            line,
            locals: None,
            error_message: Some(msg),
            duration_us: None,
            language: "rust".into(),
            thread_id: format!("{:?}", thread::current().id()),
            http_request: None,
        });
        ag.flush();
    }));
}

fn start_sender(ag: Arc<Agent>) {
    thread::spawn(move || loop {
        thread::sleep(std::time::Duration::from_millis(100));
        ag.flush();
    });
}

fn send_env_snapshot(engine_url: &str) {
    let snap = serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "working_dir": std::env::current_dir().unwrap_or_default().display().to_string(),
        "captured_at": now_ms(),
    });
    let _ = ureq::post(&format!("{}/api/ingest", engine_url))
        .set("Content-Type", "application/json")
        .send_string(&snap.to_string());
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

// ── Span Guard (RAII) ─────────────────────────────────────────────────────────

/// Drop guard that records a FunctionExit event when it goes out of scope.
pub struct SpanGuard {
    span_id: String,
    fn_name: String,
    module: String,
    file: String,
    line: u32,
    start: Instant,
}

impl SpanGuard {
    pub fn new(fn_name: &str, module: &str, file: &str, line: u32) -> Self {
        let span_id = Uuid::new_v4().to_string();
        if let Some(ag) = GLOBAL.get() {
            ag.enqueue(TraceEvent {
                trace_id: get_trace_id(),
                span_id: span_id.clone(),
                parent_span_id: None,
                timestamp_ms: now_ms(),
                event_type: EventType::FunctionEnter,
                function_name: fn_name.to_string(),
                module: module.to_string(),
                file: file.to_string(),
                line,
                locals: None,
                error_message: None,
                duration_us: None,
                language: "rust".into(),
                thread_id: format!("{:?}", thread::current().id()),
                http_request: None,
            });
        }
        Self { span_id, fn_name: fn_name.into(), module: module.into(), file: file.into(), line, start: Instant::now() }
    }
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        let dur = self.start.elapsed().as_micros() as u64;
        if let Some(ag) = GLOBAL.get() {
            ag.enqueue(TraceEvent {
                trace_id: get_trace_id(),
                span_id: Uuid::new_v4().to_string(),
                parent_span_id: Some(self.span_id.clone()),
                timestamp_ms: now_ms(),
                event_type: EventType::FunctionExit,
                function_name: self.fn_name.clone(),
                module: self.module.clone(),
                file: self.file.clone(),
                line: self.line,
                locals: None,
                error_message: None,
                duration_us: Some(dur),
                language: "rust".into(),
                thread_id: format!("{:?}", thread::current().id()),
                http_request: None,
            });
        }
    }
}

// ── Instrumentation Macro ─────────────────────────────────────────────────────

/// Instrument a function block. Place at the top of a Rust function body.
///
/// ```rust
/// fn my_function() {
///     orion_trace!();
///     // ... rest of function
/// }
/// ```
#[macro_export]
macro_rules! orion_trace {
    () => {
        let _orion_guard = $crate::SpanGuard::new(
            &format!("{}::{}", module_path!(), {
                fn f() {}
                fn type_name_of<T>(_: T) -> &'static str { std::any::type_name::<T>() }
                let name = type_name_of(f);
                &name[..name.len() - 3]
            }),
            module_path!(),
            file!(),
            line!(),
        );
    };
}
