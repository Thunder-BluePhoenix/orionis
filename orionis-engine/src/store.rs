use std::sync::Arc;
use anyhow::Result;
use uuid::Uuid;
use redb::{Database, TableDefinition, ReadableTable};

use crate::models::{TraceSummary, TraceEvent};

pub type DbHandle = Arc<Database>;

const TRACES_TABLE: TableDefinition<&[u8; 16], &str> = TableDefinition::new("traces");
const EVENTS_TABLE: TableDefinition<(&[u8; 16], u64, &[u8; 16]), &str> = TableDefinition::new("events");

pub fn open(path: &str) -> Result<DbHandle> {
    let db = Database::builder().create(path)?;
    Ok(Arc::new(db))
}

pub fn ingest_events(db: &DbHandle, events: Vec<TraceEvent>) -> Result<()> {
    let write_txn = db.begin_write()?;
    {
        let mut traces_table = write_txn.open_table(TRACES_TABLE)?;
        let mut events_table = write_txn.open_table(EVENTS_TABLE)?;

        for event in events {
            let tid_bytes = event.trace_id.into_bytes();
            let sid_bytes = event.span_id.into_bytes();

            let summary_opt = {
                let guard = traces_table.get(&tid_bytes)?;
                if let Some(access) = guard {
                    Some(serde_json::from_str::<TraceSummary>(access.value())?)
                } else {
                    None
                }
            };

            let mut summary = if let Some(s) = summary_opt {
                s
            } else {
                TraceSummary {
                    trace_id: event.trace_id,
                    name: event.function_name.clone(),
                    started_at: event.timestamp_ms,
                    duration_ms: Some(0),
                    has_error: false,
                    language: event.language.clone(),
                    event_count: 0,
                    thread_ids: Vec::new(),
                    ai_cluster_key: None,
                    ai_summary: None,
                }
            };
            // Guard is dropped here because 'summary_opt' scope ended

            summary.event_count += 1;
            if let Some(tid) = &event.thread_id {
                if !summary.thread_ids.contains(tid) {
                    summary.thread_ids.push(tid.clone());
                }
            }
            if let Some(err) = &event.error_message {
                summary.has_error = true;
                // Generate a stable cluster key based on error + location
                if summary.ai_cluster_key.is_none() {
                    summary.ai_cluster_key = Some(format!("{}:{}:{}", err, event.file, event.line));
                }
            }
            if event.timestamp_ms > (summary.started_at + summary.duration_ms.unwrap_or(0)) {
                summary.duration_ms = Some(event.timestamp_ms - summary.started_at);
            }

            let summary_json = serde_json::to_string(&summary)?;
            traces_table.insert(&tid_bytes, summary_json.as_str())?;
            let event_json = serde_json::to_string(&event)?;
            events_table.insert((&tid_bytes, event.timestamp_ms, &sid_bytes), event_json.as_str())?;
        }
    }
    write_txn.commit()?;
    Ok(())
}

pub fn list_traces(db: &DbHandle) -> Result<Vec<TraceSummary>> {
    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(TRACES_TABLE)?;
    let mut summaries = Vec::new();

    for item in table.iter()? {
        let (key_access, value_access) = item?;
        let _key = key_access.value();
        let summary: TraceSummary = serde_json::from_str(value_access.value())?;
        summaries.push(summary);
    }

    summaries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    Ok(summaries.into_iter().take(500).collect())
}

pub fn get_trace_events(db: &DbHandle, trace_id: Uuid) -> Result<Vec<TraceEvent>> {
    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(EVENTS_TABLE)?;
    let mut events = Vec::new();

    let tid_bytes = trace_id.into_bytes();
    let range = table.range((&tid_bytes, 0, &[0u8; 16])..(&tid_bytes, u64::MAX, &[255u8; 16]))?;

    for item in range {
        let (_key_access, value_access) = item?;
        let event: TraceEvent = serde_json::from_str(value_access.value())?;
        events.push(event);
    }

    Ok(events)
}

pub fn clear_all(db: &DbHandle) -> Result<()> {
    let write_txn = db.begin_write()?;
    {
        let mut traces_table = write_txn.open_table(TRACES_TABLE)?;
        let mut events_table = write_txn.open_table(EVENTS_TABLE)?;
        
        // In redb 2.x, we can use retain or just iterate and remove.
        let mut tids = Vec::new();
        for item in traces_table.iter()? {
            let (key_access, _) = item?;
            tids.push(*key_access.value());
        }
        for tid in tids {
            traces_table.remove(&tid)?;
        }
        
        let mut ekids = Vec::new();
        for item in events_table.iter()? {
            let (key_access, _) = item?;
            let (t, ts, s) = key_access.value();
            ekids.push((*t, ts, *s));
        }
        for kid in ekids {
            let (t, ts, s) = kid;
            events_table.remove((&t, ts, &s))?;
        }
    }
    write_txn.commit()?;
    Ok(())
}

pub fn get_service_graph(db: &DbHandle) -> Result<serde_json::Value> {
    use std::collections::{HashMap, HashSet};
    
    let read_txn = db.begin_read()?;
    let events_table = read_txn.open_table(EVENTS_TABLE)?;
    
    let mut nodes = HashSet::new();
    let mut edges = HashMap::new(); // (source, target) -> (count, total_ms)
    
    let mut all_events = Vec::new();
    for item in events_table.iter()? {
        let (_, val_access) = item?;
        let event: TraceEvent = serde_json::from_str(val_access.value())?;
        all_events.push(event);
    }
    
    let event_map: HashMap<Uuid, &TraceEvent> = all_events.iter().map(|e| (e.span_id, e)).collect();
    
    for ev in &all_events {
        nodes.insert(ev.module.clone());
        
        if let Some(pid) = ev.parent_span_id {
            if let Some(parent) = event_map.get(&pid) {
                if parent.module != ev.module {
                    let key = (parent.module.clone(), ev.module.clone());
                    let (count, total_dur) = edges.entry(key).or_insert((0, 0));
                    *count += 1;
                    if let Some(dur) = ev.duration_us {
                        *total_dur += dur / 1000;
                    }
                }
            }
        }
    }
    
    let nodes_out: Vec<serde_json::Value> = nodes.into_iter().map(|n| {
        serde_json::json!({ "id": n, "label": n, "type": "service" })
    }).collect();
    
    let edges_out: Vec<serde_json::Value> = edges.into_iter().map(|((s, t), (c, d))| {
        serde_json::json!({ 
            "source": s, 
            "target": t, 
            "count": c, 
            "latency": if c > 0 { d / c } else { 0 } 
        })
    }).collect();
    
    Ok(serde_json::json!({
        "nodes": nodes_out,
        "links": edges_out
    }))
}

pub fn get_event_by_span_id(db: &DbHandle, span_id: Uuid) -> Result<Option<TraceEvent>> {
    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(EVENTS_TABLE)?;
    
    for item in table.iter()? {
        let (_, val_access) = item?;
        let event: TraceEvent = serde_json::from_str(val_access.value())?;
        if event.span_id == span_id {
            return Ok(Some(event));
        }
    }
    Ok(None)
}

pub fn get_clusters(db: &DbHandle) -> Result<serde_json::Value> {
    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(TRACES_TABLE)?;
    
    let mut clusters: std::collections::HashMap<String, Vec<TraceSummary>> = std::collections::HashMap::new();
    
    for item in table.iter()? {
        let (_, value_access) = item?;
        let summary: TraceSummary = serde_json::from_str(value_access.value())?;
        
        if let Some(key) = &summary.ai_cluster_key {
            clusters.entry(key.clone()).or_default().push(summary);
        }
    }
    
    let out: Vec<serde_json::Value> = clusters.into_iter().map(|(key, traces)| {
        let count = traces.len();
        let representative = traces[0].clone();
        serde_json::json!({
            "cluster_key": key,
            "count": count,
            "representative": representative,
            "traces": traces
        })
    }).collect();
    
    Ok(serde_json::json!(out))
}

pub fn update_trace_ai(db: &DbHandle, trace_id: Uuid, summary: String, cluster_key: Option<String>) -> Result<()> {
    let write_txn = db.begin_write()?;
    {
        let mut table = write_txn.open_table(TRACES_TABLE)?;
        let tid_bytes = trace_id.into_bytes();
        let s_opt = {
            let guard = table.get(&tid_bytes)?;
            if let Some(access) = guard {
                Some(serde_json::from_str::<TraceSummary>(access.value())?)
            } else {
                None
            }
        };

        if let Some(mut s) = s_opt {
            s.ai_summary = Some(summary);
            if let Some(ck) = cluster_key {
                s.ai_cluster_key = Some(ck);
            }
            let summary_json = serde_json::to_string(&s)?;
            table.insert(&tid_bytes, summary_json.as_str())?;
        }
    }
    write_txn.commit()?;
    Ok(())
}
