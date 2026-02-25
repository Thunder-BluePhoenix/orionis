use std::sync::{Arc, Mutex};

use anyhow::Result;
use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::models::{AgentLanguage, Trace, TraceSummary, TraceEvent};

pub type DbHandle = Arc<Mutex<Connection>>;

/// Open (or create) the local SQLite database and run migrations.
pub fn open(path: &str) -> Result<DbHandle> {
    let conn = Connection::open(path)?;
    run_migrations(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch("
        PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;

        CREATE TABLE IF NOT EXISTS traces (
            trace_id    TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            started_at  INTEGER NOT NULL,
            ended_at    INTEGER,
            has_error   INTEGER NOT NULL DEFAULT 0,
            language    TEXT NOT NULL,
            env_json    TEXT,
            event_count INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS events (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            trace_id        TEXT NOT NULL,
            span_id         TEXT NOT NULL,
            parent_span_id  TEXT,
            timestamp_ms    INTEGER NOT NULL,
            event_type      TEXT NOT NULL,
            function_name   TEXT NOT NULL,
            module          TEXT NOT NULL,
            file            TEXT NOT NULL,
            line            INTEGER NOT NULL,
            locals_json     TEXT,
            error_message   TEXT,
            duration_us     INTEGER,
            language        TEXT NOT NULL,

            FOREIGN KEY(trace_id) REFERENCES traces(trace_id)
        );

        CREATE INDEX IF NOT EXISTS idx_events_trace ON events(trace_id, timestamp_ms);
    ")?;
    Ok(())
}

// ─── Ingest ──────────────────────────────────────────────────────────────────

/// Insert a batch of events, creating / updating the parent trace record.
pub fn ingest_events(db: &DbHandle, events: Vec<TraceEvent>) -> Result<()> {
    let conn = db.lock().unwrap();

    for event in &events {
        let trace_id = event.trace_id.to_string();
        let lang = lang_str(&event.language);
        let locals_json = event.locals.as_ref().map(|l| serde_json::to_string(l).ok()).flatten();
        let has_error = if event.error_message.is_some() { 1i32 } else { 0 };

        // Upsert trace header
        conn.execute(
            "INSERT INTO traces (trace_id, name, started_at, has_error, language, event_count)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)
             ON CONFLICT(trace_id) DO UPDATE SET
                ended_at    = MAX(COALESCE(ended_at, 0), ?3),
                has_error   = has_error | ?4,
                event_count = event_count + 1",
            params![
                trace_id,
                event.function_name,
                event.timestamp_ms as i64,
                has_error,
                lang,
            ],
        )?;

        // Insert the event
        conn.execute(
            "INSERT INTO events
             (trace_id, span_id, parent_span_id, timestamp_ms, event_type,
              function_name, module, file, line, locals_json, error_message, duration_us, language)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                trace_id,
                event.span_id.to_string(),
                event.parent_span_id.map(|u| u.to_string()),
                event.timestamp_ms as i64,
                format!("{:?}", event.event_type),
                event.function_name,
                event.module,
                event.file,
                event.line,
                locals_json,
                event.error_message,
                event.duration_us.map(|d| d as i64),
                lang_str(&event.language),
            ],
        )?;
    }

    Ok(())
}

// ─── Queries ─────────────────────────────────────────────────────────────────

/// List all traces as summaries (no events).
pub fn list_traces(db: &DbHandle) -> Result<Vec<TraceSummary>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT trace_id, name, started_at, ended_at, has_error, language, event_count
         FROM traces ORDER BY started_at DESC LIMIT 500"
    )?;

    let rows = stmt.query_map([], |row| {
        let started: i64 = row.get(2)?;
        let ended: Option<i64> = row.get(3)?;
        let duration = ended.map(|e| (e - started).unsigned_abs());
        let lang_s: String = row.get(5)?;
        Ok(TraceSummary {
            trace_id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            name: row.get(1)?,
            started_at: started as u64,
            duration_ms: duration,
            has_error: row.get::<_, i32>(4)? != 0,
            language: parse_lang(&lang_s),
            event_count: row.get::<_, i64>(6)? as usize,
        })
    })?;

    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Get all events for a specific trace (for replay).
pub fn get_trace_events(db: &DbHandle, trace_id: Uuid) -> Result<Vec<TraceEvent>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT span_id, parent_span_id, timestamp_ms, event_type, function_name,
                module, file, line, locals_json, error_message, duration_us, language
         FROM events WHERE trace_id = ?1 ORDER BY timestamp_ms ASC"
    )?;

    let tid = trace_id.to_string();
    let rows = stmt.query_map([&tid], |row| {
        let span_id = Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default();
        let parent: Option<String> = row.get(1)?;
        let loc_json: Option<String> = row.get(8)?;
        let lang_s: String = row.get(11)?;

        Ok(TraceEvent {
            trace_id,
            span_id,
            parent_span_id: parent.and_then(|s| Uuid::parse_str(&s).ok()),
            timestamp_ms: row.get::<_, i64>(2)? as u64,
            event_type: parse_event_type(&row.get::<_, String>(3)?),
            function_name: row.get(4)?,
            module: row.get(5)?,
            file: row.get(6)?,
            line: row.get::<_, u32>(7)?,
            locals: loc_json.and_then(|j| serde_json::from_str(&j).ok()),
            error_message: row.get(9)?,
            duration_us: row.get::<_, Option<i64>>(10)?.map(|v| v as u64),
            language: parse_lang(&lang_s),
        })
    })?;

    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Delete all traces and events.
pub fn clear_all(db: &DbHandle) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute_batch("DELETE FROM events; DELETE FROM traces;")?;
    Ok(())
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn lang_str(lang: &AgentLanguage) -> &'static str {
    match lang {
        AgentLanguage::Python => "python",
        AgentLanguage::Go     => "go",
        AgentLanguage::Rust   => "rust",
        AgentLanguage::Cpp    => "cpp",
    }
}

fn parse_lang(s: &str) -> AgentLanguage {
    match s {
        "go"     => AgentLanguage::Go,
        "rust"   => AgentLanguage::Rust,
        "cpp"    => AgentLanguage::Cpp,
        _        => AgentLanguage::Python,
    }
}

fn parse_event_type(s: &str) -> crate::models::EventType {
    use crate::models::EventType::*;
    match s {
        "FunctionExit"  => FunctionExit,
        "Exception"     => Exception,
        "AsyncSpawn"    => AsyncSpawn,
        "AsyncResume"   => AsyncResume,
        "HttpRequest"   => HttpRequest,
        "HttpResponse"  => HttpResponse,
        _               => FunctionEnter,
    }
}
