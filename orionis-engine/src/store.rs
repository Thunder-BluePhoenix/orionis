use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;
use redb::{Database, TableDefinition, ReadableTable};

use crate::models::{TraceSummary, TraceEvent};

// ── Storage Trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait StorageBackend: Send + Sync {
    async fn ingest_events(&self, events: Vec<TraceEvent>, tenant_id: Option<String>) -> Result<()>;
    async fn list_traces(&self, tenant_id: Option<String>) -> Result<Vec<TraceSummary>>;
    async fn get_trace_events(&self, trace_id: Uuid, tenant_id: Option<String>) -> Result<Vec<TraceEvent>>;
    async fn clear_all(&self, tenant_id: Option<String>) -> Result<()>;
    async fn get_service_graph(&self, tenant_id: Option<String>) -> Result<serde_json::Value>;
    async fn get_event_by_span_id(&self, span_id: Uuid, tenant_id: Option<String>) -> Result<Option<TraceEvent>>;
    async fn get_clusters(&self, tenant_id: Option<String>) -> Result<serde_json::Value>;
    async fn update_trace_ai(&self, trace_id: Uuid, summary: String, cluster_key: Option<String>) -> Result<()>;
    async fn register_node(&self, node: crate::models::ClusterNode) -> Result<()>;
    async fn list_nodes(&self) -> Result<Vec<crate::models::ClusterNode>>;
    async fn add_comment(&self, comment: crate::models::TraceComment, tenant_id: Option<String>) -> Result<()>;
    async fn get_comments(&self, trace_id: Uuid, tenant_id: Option<String>) -> Result<Vec<crate::models::TraceComment>>;
    async fn update_trace_metadata(&self, trace_id: Uuid, tags: Option<Vec<String>>, assigned_to: Option<String>, tenant_id: Option<String>) -> Result<()>;
    async fn cleanup_expired_data(&self, retention_days: u32) -> Result<()>;
}

pub type DbHandle = Arc<dyn StorageBackend>;

// ── LocalStore (redb) Implementation ─────────────────────────────────────────

const TRACES_TABLE: TableDefinition<&[u8; 16], &str> = TableDefinition::new("traces");
const EVENTS_TABLE: TableDefinition<(&[u8; 16], u64, &str), &str> = TableDefinition::new("events");
const NODES_TABLE: TableDefinition<&str, &str> = TableDefinition::new("nodes"); // node_id -> json
const COMMENTS_TABLE: TableDefinition<&[u8; 16], &str> = TableDefinition::new("comments"); // comment_id -> json

pub struct LocalStore {
    db: Arc<Database>,
}

impl LocalStore {
    pub fn new(path: &str) -> Result<Self> {
        let db = Database::builder().create(path)?;
        Ok(Self { db: Arc::new(db) })
    }
}

#[async_trait]
impl StorageBackend for LocalStore {
    async fn ingest_events(&self, events: Vec<TraceEvent>, tenant_id: Option<String>) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut traces_table = write_txn.open_table(TRACES_TABLE)?;
            let mut events_table = write_txn.open_table(EVENTS_TABLE)?;

            for mut event in events {
                event.tenant_id = tenant_id.clone();
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
                        tenant_id: tenant_id.clone(),
                        tags: Vec::new(),
                        assigned_to: None,
                    }
                };

                summary.event_count += 1;
                if let Some(tid) = &event.thread_id {
                    if !summary.thread_ids.contains(tid) {
                        summary.thread_ids.push(tid.clone());
                    }
                }
                if let Some(err) = &event.error_message {
                    summary.has_error = true;
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
                let eid = event.event_id.as_deref().unwrap_or("");
                events_table.insert((&tid_bytes, event.timestamp_ms, eid), event_json.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn list_traces(&self, tenant_id: Option<String>) -> Result<Vec<TraceSummary>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRACES_TABLE)?;
        let mut summaries = Vec::new();

        for item in table.iter()? {
            let (_, value_access) = item?;
            let summary: TraceSummary = serde_json::from_str(value_access.value())?;
            
            // Filter by tenant if provided
            if let Some(tid) = &tenant_id {
                if summary.tenant_id.as_ref() != Some(tid) {
                    continue;
                }
            } else if summary.tenant_id.is_some() {
                // If no tenant provided but data has tenant, skip by default (security)
                continue;
            }

            summaries.push(summary);
        }

        summaries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(summaries.into_iter().take(500).collect())
    }

    async fn get_trace_events(&self, trace_id: Uuid, tenant_id: Option<String>) -> Result<Vec<TraceEvent>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(EVENTS_TABLE)?;
        let mut events = Vec::new();

        let tid_bytes = trace_id.into_bytes();
        let range = table.range((&tid_bytes, 0, "")..(&tid_bytes, u64::MAX, "\u{10ffff}"))?;

        for item in range {
            let (_key_access, value_access) = item?;
            let event: TraceEvent = serde_json::from_str(value_access.value())?;
            
            if let Some(tid) = &tenant_id {
                if event.tenant_id.as_ref() != Some(tid) {
                    continue;
                }
            } else if event.tenant_id.is_some() {
                continue;
            }

            events.push(event);
        }

        Ok(events)
    }

    async fn clear_all(&self, tenant_id: Option<String>) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut traces_table = write_txn.open_table(TRACES_TABLE)?;
            let mut events_table = write_txn.open_table(EVENTS_TABLE)?;
            
            let mut tids_to_remove = Vec::new();
            for item in traces_table.iter()? {
                let (key_access, val_access) = item?;
                let summary: TraceSummary = serde_json::from_str(val_access.value())?;
                
                if let Some(tid) = &tenant_id {
                    if summary.tenant_id.as_ref() == Some(tid) {
                        tids_to_remove.push(*key_access.value());
                    }
                } else if summary.tenant_id.is_none() {
                    tids_to_remove.push(*key_access.value());
                }
            }
            for tid in tids_to_remove {
                traces_table.remove(&tid)?;
            }
            
            let mut ev_keys_to_remove = Vec::new();
            for item in events_table.iter()? {
                let (key_access, val_access) = item?;
                let event: TraceEvent = serde_json::from_str(val_access.value())?;
                
                if let Some(tid) = &tenant_id {
                    if event.tenant_id.as_ref() == Some(tid) {
                        let (t, ts, e) = key_access.value();
                        ev_keys_to_remove.push((*t, ts, e.to_string()));
                    }
                } else if event.tenant_id.is_none() {
                    let (t, ts, e) = key_access.value();
                    ev_keys_to_remove.push((*t, ts, e.to_string()));
                }
            }
            for kid in ev_keys_to_remove {
                let (t, ts, e) = kid;
                events_table.remove((&t, ts, e.as_str()))?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_service_graph(&self, tenant_id: Option<String>) -> Result<serde_json::Value> {
        use std::collections::{HashMap, HashSet};
        
        let read_txn = self.db.begin_read()?;
        let events_table = read_txn.open_table(EVENTS_TABLE)?;
        
        let mut nodes = HashSet::new();
        let mut edges = HashMap::new();
        
        let mut all_events = Vec::new();
        for item in events_table.iter()? {
            let (_, val_access) = item?;
            let event: TraceEvent = serde_json::from_str(val_access.value())?;
            
            if let Some(tid) = &tenant_id {
                if event.tenant_id.as_ref() != Some(tid) { continue; }
            } else if event.tenant_id.is_some() { continue; }

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

    async fn get_event_by_span_id(&self, span_id: Uuid, tenant_id: Option<String>) -> Result<Option<TraceEvent>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(EVENTS_TABLE)?;
        
        for item in table.iter()? {
            let (_, val_access) = item?;
            let event: TraceEvent = serde_json::from_str(val_access.value())?;
            if event.span_id == span_id {
                if let Some(tid) = &tenant_id {
                    if event.tenant_id.as_ref() != Some(tid) { return Ok(None); }
                } else if event.tenant_id.is_some() { return Ok(None); }
                return Ok(Some(event));
            }
        }
        Ok(None)
    }

    async fn get_clusters(&self, tenant_id: Option<String>) -> Result<serde_json::Value> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRACES_TABLE)?;
        
        let mut clusters: std::collections::HashMap<String, Vec<TraceSummary>> = std::collections::HashMap::new();
        
        for item in table.iter()? {
            let (_, value_access) = item?;
            let summary: TraceSummary = serde_json::from_str(value_access.value())?;
            
            if let Some(tid) = &tenant_id {
                if summary.tenant_id.as_ref() != Some(tid) { continue; }
            } else if summary.tenant_id.is_some() { continue; }

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

    async fn update_trace_ai(&self, trace_id: Uuid, summary: String, cluster_key: Option<String>) -> Result<()> {
        let write_txn = self.db.begin_write()?;
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

    async fn register_node(&self, node: crate::models::ClusterNode) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(NODES_TABLE)?;
            let json = serde_json::to_string(&node)?;
            table.insert(node.node_id.as_str(), json.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn list_nodes(&self) -> Result<Vec<crate::models::ClusterNode>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(NODES_TABLE)?;
        let mut nodes = Vec::new();

        for item in table.iter()? {
            let (_, val_access) = item?;
            let node: crate::models::ClusterNode = serde_json::from_str(val_access.value())?;
            nodes.push(node);
        }
        Ok(nodes)
    }

    async fn add_comment(&self, comment: crate::models::TraceComment, tenant_id: Option<String>) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(COMMENTS_TABLE)?;
            let mut comment = comment;
            comment.tenant_id = tenant_id;
            let json = serde_json::to_string(&comment)?;
            table.insert(&comment.comment_id.into_bytes(), json.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_comments(&self, trace_id: Uuid, tenant_id: Option<String>) -> Result<Vec<crate::models::TraceComment>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(COMMENTS_TABLE)?;
        let mut comments = Vec::new();

        for item in table.iter()? {
            let (_, val_access) = item?;
            let comment: crate::models::TraceComment = serde_json::from_str(val_access.value())?;
            if comment.trace_id == trace_id {
                if let Some(tid) = &tenant_id {
                    if comment.tenant_id.as_ref() != Some(tid) { continue; }
                } else if comment.tenant_id.is_some() { continue; }
                comments.push(comment);
            }
        }
        comments.sort_by(|a, b| a.timestamp_ms.cmp(&b.timestamp_ms));
        Ok(comments)
    }

    async fn update_trace_metadata(&self, trace_id: Uuid, tags: Option<Vec<String>>, assigned_to: Option<String>, tenant_id: Option<String>) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TRACES_TABLE)?;
            let tid_bytes = trace_id.into_bytes();
            let s_opt = {
                let guard = table.get(&tid_bytes)?;
                if let Some(access) = guard {
                    let s: TraceSummary = serde_json::from_str(access.value())?;
                    if let Some(tid) = &tenant_id {
                        if s.tenant_id.as_ref() != Some(tid) { return Ok(()); }
                    } else if s.tenant_id.is_some() { return Ok(()); }
                    Some(s)
                } else {
                    None
                }
            };

            if let Some(mut s) = s_opt {
                if let Some(t) = tags { s.tags = t; }
                if let Some(a) = assigned_to { s.assigned_to = Some(a); }
                let json = serde_json::to_string(&s)?;
                table.insert(&tid_bytes, json.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn cleanup_expired_data(&self, retention_days: u32) -> Result<()> {
        let cutoff_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64 - (retention_days as u64 * 24 * 60 * 60 * 1000);
            
        let write_txn = self.db.begin_write()?;
        {
            let mut traces_table = write_txn.open_table(TRACES_TABLE)?;
            let mut events_table = write_txn.open_table(EVENTS_TABLE)?;
            
            let mut tids_to_remove = Vec::new();
            for item in traces_table.iter()? {
                let (key, val) = item?;
                let summary: TraceSummary = serde_json::from_str(val.value())?;
                if summary.started_at < cutoff_ms {
                    tids_to_remove.push(*key.value());
                }
            }
            
            for tid in tids_to_remove {
                traces_table.remove(&tid)?;
                // Remove all associated events
                let range = events_table.range((&tid, 0, "")..(&tid, u64::MAX, "\u{10ffff}"))?;
                let mut ev_keys_to_remove = Vec::new();
                for ev_item in range {
                    let (ev_key, _) = ev_item?;
                    ev_keys_to_remove.push((*ev_key.value().0, ev_key.value().1, ev_key.value().2.to_string()));
                }
                for (t, ts, e) in ev_keys_to_remove {
                    events_table.remove((&t, ts, e.as_str()))?;
                }
            }
        }
        write_txn.commit()?;
        Ok(())
    }
}

pub fn open(path: &str) -> Result<DbHandle> {
    let store = LocalStore::new(path)?;
    Ok(Arc::new(store))
}
