// Orionis Rust Agent — native tracing macros and instrumentation for Rust apps.
// Captures function enter/exit via macros, panics via panic hook, and sends to the engine.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH, Instant};
use std::thread;
use std::collections::VecDeque;

pub use uuid::Uuid;

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
}

// ── Agent ─────────────────────────────────────────────────────────────────────

pub struct Agent {
    pub engine_url: String,
    pub trace_id: String,
    batch: Mutex<VecDeque<TraceEvent>>,
}

static GLOBAL: OnceLock<Arc<Agent>> = OnceLock::new();

impl Agent {
    fn new(engine_url: &str) -> Arc<Self> {
        Arc::new(Self {
            engine_url: engine_url.to_string(),
            trace_id: Uuid::new_v4().to_string(),
            batch: Mutex::new(VecDeque::new()),
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
        let url = format!("{}/api/ingest", self.engine_url);
        let body = serde_json::to_vec(&events).unwrap_or_default();
        let _ = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_bytes(&body);
    }
}

/// Initialise the Orionis Rust agent. Call this once at the start of main().
pub fn start(engine_url: &str) {
    let agent = Agent::new(engine_url);
    let _ = GLOBAL.set(agent.clone());
    install_panic_hook(agent.clone());
    start_sender(agent);
    send_env_snapshot(engine_url);
    eprintln!("[Orionis] Rust agent started — engine: {}", engine_url);
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
            trace_id: ag.trace_id.clone(),
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
                trace_id: ag.trace_id.clone(),
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
                trace_id: ag.trace_id.clone(),
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
