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
    pub event_id: Option<String>,
    pub memory_usage_bytes: Option<u64>,
}

// ── Senders ──────────────────────────────────────────────────────────────────

// ── Senders ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SendResult {
    Success,
    RetryableError, // 503, 429, or network blip
    FatalError,     // 400, 403, etc.
}

trait Sender: Send + Sync {
    fn send(&self, events: Vec<TraceEvent>) -> SendResult;
}

struct HttpSender {
    url: String,
}

impl Sender for HttpSender {
    fn send(&self, events: Vec<TraceEvent>) -> SendResult {
        let url = format!("{}/api/ingest", self.url);
        let body = serde_json::to_vec(&events).unwrap_or_default();
        
        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_bytes(&body) {
                Ok(_) => SendResult::Success,
                Err(ureq::Error::Status(503, _)) | Err(ureq::Error::Status(429, _)) => SendResult::RetryableError,
                Err(ureq::Error::Status(_, _)) => SendResult::FatalError,
                Err(_) => SendResult::RetryableError, // Network issues
            }
    }
}

struct GrpcSender {
    endpoint: String,
}

impl Sender for GrpcSender {
    fn send(&self, events: Vec<TraceEvent>) -> SendResult {
        let endpoint = self.endpoint.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        // We use a temporary runtime to keep the agent's sync API simple
        thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(r) => r,
                Err(_) => { let _ = tx.send(SendResult::FatalError); return; }
            };
            
            rt.block_on(async move {
                let mut client = match proto::ingest_client::IngestClient::connect(endpoint).await {
                    Ok(c) => c,
                    Err(_) => { let _ = tx.send(SendResult::RetryableError); return; }
                };

                let mut pb_events = Vec::new();
                for ev in events {
                    let pb_ev = proto::TraceEvent {
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
                        thread_id: ev.thread_id,
                        tenant_id: "".to_string(), // handled by API keys usually
                        http_request: None,
                        db_query: None,
                        event_id: ev.event_id.unwrap_or_default(),
                        memory_usage_bytes: ev.memory_usage_bytes,
                    };
                    pb_events.push(pb_ev);
                }

                let stream = tokio_stream::iter(pb_events);
                match client.stream_events(stream).await {
                    Ok(_) => { let _ = tx.send(SendResult::Success); }
                    Err(s) if s.code() == tonic::Code::ResourceExhausted || s.code() == tonic::Code::Unavailable => {
                        let _ = tx.send(SendResult::RetryableError);
                    }
                    Err(_) => { let _ = tx.send(SendResult::FatalError); }
                }
            });
        });

        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or(SendResult::RetryableError)
    }
}

// ── Agent ─────────────────────────────────────────────────────────────────────

const MAX_BUFFER_SIZE: usize = 10000;

thread_local! {
    static THREAD_TRACE_ID: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
}

pub struct Agent {
    pub engine_url: String,
    pub grpc_url: String,
    pub use_grpc: bool,
    batch: Mutex<VecDeque<TraceEvent>>,
    sender: Box<dyn Sender>,
    backoff_until: Mutex<Option<Instant>>,
    backoff_count: std::sync::atomic::AtomicU32,
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
            batch: Mutex::new(VecDeque::with_capacity(1000)),
            sender,
            backoff_until: Mutex::new(None),
            backoff_count: std::sync::atomic::AtomicU32::new(0),
        })
    }

    pub fn enqueue(&self, ev: TraceEvent) {
        let mut b = self.batch.lock().unwrap();
        if b.len() >= MAX_BUFFER_SIZE {
            // Data shedding: Drop oldest if full (LIFO-like logic for efficiency)
            b.pop_front(); 
        }
        b.push_back(ev);
    }

    pub fn flush(&self) {
        // Check backoff
        {
            let backoff = self.backoff_until.lock().unwrap();
            if let Some(until) = *backoff {
                if Instant::now() < until {
                    return;
                }
            }
        }

        let events: Vec<TraceEvent> = {
            let mut b = self.batch.lock().unwrap();
            if b.is_empty() { return; }
            b.drain(..).collect()
        };

        match self.sender.send(events.clone()) {
            SendResult::Success => {
                self.backoff_count.store(0, std::sync::atomic::Ordering::Relaxed);
                let mut backoff = self.backoff_until.lock().unwrap();
                *backoff = None;
            }
            SendResult::RetryableError => {
                // Return to batch (at the front)
                let mut b = self.batch.lock().unwrap();
                for ev in events.into_iter().rev() {
                    if b.len() < MAX_BUFFER_SIZE {
                        b.push_front(ev);
                    }
                }
                
                // Increase backoff
                let count = self.backoff_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let delay_ms = (100 * 2u64.pow(count.min(10))).min(10000); // Max 10s
                let mut backoff = self.backoff_until.lock().unwrap();
                *backoff = Some(Instant::now() + std::time::Duration::from_millis(delay_ms));
            }
            SendResult::FatalError => {
                // Drop the data to avoid infinite failure loop
                eprintln!("[Orionis] Fatal error sending traces. Data dropped.");
            }
        }
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
    // Note: Environmental snapshot also uses HTTP, might need its own backoff logic 
    // but usually only sent once.
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
            event_id: None,
            memory_usage_bytes: memory_stats::memory_stats().map(|s| s.physical_mem as u64),
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
                event_id: None,
                memory_usage_bytes: memory_stats::memory_stats().map(|s| s.physical_mem as u64),
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
                event_id: None,
                memory_usage_bytes: memory_stats::memory_stats().map(|s| s.physical_mem as u64),
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
