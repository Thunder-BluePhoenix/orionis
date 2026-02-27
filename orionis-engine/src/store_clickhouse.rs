use std::sync::Arc;
use anyhow::{Result, Context};
use async_trait::async_trait;
use uuid::Uuid;
use clickhouse::{Client, Row};
use serde::{Deserialize, Serialize};

use crate::models::{TraceSummary, TraceEvent};
use crate::store::StorageBackend;

/// ClickHouse implementation of the Orionis Storage Backend
pub struct ClickHouseStore {
    client: Client,
}

impl ClickHouseStore {
    pub async fn new(url: &str) -> Result<Self> {
        let client = Client::default()
            .with_url(url)
            .with_database("orionis"); // Ensure DB exists
        
        // Initialize schema if not exists
        client.query("CREATE DATABASE IF NOT EXISTS orionis").execute().await?;
        
        client.query(r#"
            CREATE TABLE IF NOT EXISTS traces (
                trace_id UUID,
                name String,
                started_at UInt64,
                duration_ms Nullable(UInt64),
                has_error UInt8,
                language String,
                event_count UInt32,
                thread_ids Array(String),
                ai_cluster_key Nullable(String),
                ai_summary Nullable(String)
            ) ENGINE = ReplacingMergeTree()
            ORDER BY (trace_id)
        "#).execute().await?;

        client.query(r#"
            CREATE TABLE IF NOT EXISTS events (
                trace_id UUID,
                span_id UUID,
                parent_span_id Nullable(UUID),
                timestamp_ms UInt64,
                event_type String,
                function_name String,
                module String,
                file String,
                line UInt32,
                locals String,
                error_message Nullable(String),
                duration_us Nullable(UInt64),
                language String,
                thread_id Nullable(String),
                http_request String,
                db_query String
            ) ENGINE = MergeTree()
            ORDER BY (trace_id, timestamp_ms, span_id)
        "#).execute().await?;

        client.query(r#"
            CREATE TABLE IF NOT EXISTS nodes (
                node_id String,
                http_addr String,
                grpc_addr String,
                status String,
                last_seen UInt64
            ) ENGINE = ReplacingMergeTree()
            ORDER BY (node_id)
        "#).execute().await?;

        client.query(r#"
            CREATE TABLE IF NOT EXISTS comments (
                comment_id String,
                trace_id String,
                span_id Nullable(String),
                user_id String,
                text String,
                timestamp_ms UInt64
            ) ENGINE = MergeTree()
            ORDER BY (trace_id, timestamp_ms)
        "#).execute().await?;

        Ok(Self { client })
    }
}

// Helper structs for ClickHouse Row insertion (ClickHouse crate requires specific deriving for arrays/nulls)
#[derive(Row, Serialize, Deserialize)]
struct ChTraceSummary {
    trace_id: String,
    name: String,
    started_at: u64,
    duration_ms: Option<u64>,
    has_error: u8,
    language: String,
    event_count: u32,
    thread_ids: Vec<String>,
    ai_cluster_key: Option<String>,
    ai_summary: Option<String>,
}

#[derive(Row, Serialize, Deserialize)]
struct ChEvent {
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
    timestamp_ms: u64,
    event_type: String,
    function_name: String,
    module: String,
    file: String,
    line: u32,
    locals: String,
    error_message: Option<String>,
    duration_us: Option<u64>,
    language: String,
    thread_id: Option<String>,
    http_request: String,
    db_query: String,
}

#[derive(Row, Serialize, Deserialize)]
struct ChNode {
    node_id: String,
    http_addr: String,
    grpc_addr: String,
    status: String,
    last_seen: u64,
}

#[derive(Row, Serialize, Deserialize)]
struct ChComment {
    comment_id: String,
    trace_id: String,
    span_id: Option<String>,
    user_id: String,
    text: String,
    timestamp_ms: u64,
}

#[async_trait]
impl StorageBackend for ClickHouseStore {
    async fn ingest_events(&self, events: Vec<TraceEvent>) -> Result<()> {
        if events.is_empty() { return Ok(()); }
        
        let mut event_inserter = self.client.insert::<ChEvent>("events").await?;
        let mut trace_inserter = self.client.insert::<ChTraceSummary>("traces").await?;
        
        let mut summaries = std::collections::HashMap::new();

        for event in events {
            // Upsert mechanism for trace summary
            let summary = summaries.entry(event.trace_id).or_insert_with(|| {
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
            });

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

            let ch_event = ChEvent {
                trace_id: event.trace_id.to_string(),
                span_id: event.span_id.to_string(),
                parent_span_id: event.parent_span_id.map(|u| u.to_string()),
                timestamp_ms: event.timestamp_ms,
                event_type: serde_json::to_string(&event.event_type).unwrap_or_default(),
                function_name: event.function_name,
                module: event.module,
                file: event.file,
                line: event.line,
                locals: serde_json::to_string(&event.locals).unwrap_or_default(),
                error_message: event.error_message,
                duration_us: event.duration_us,
                language: serde_json::to_string(&event.language).unwrap_or_default(),
                thread_id: event.thread_id,
                http_request: serde_json::to_string(&event.http_request).unwrap_or_default(),
                db_query: serde_json::to_string(&event.db_query).unwrap_or_default(),
            };
            event_inserter.write(&ch_event).await?;
        }
        
        for (_, summary) in summaries {
            // Explicitly map over ai_cluster_key rather than assigning directly if needed, but it should be fine.
            let ch_trace = ChTraceSummary {
                trace_id: summary.trace_id.to_string(),
                name: summary.name,
                started_at: summary.started_at,
                duration_ms: summary.duration_ms,
                has_error: if summary.has_error { 1 } else { 0 },
                language: serde_json::to_string(&summary.language).unwrap_or_default(),
                event_count: summary.event_count as u32,
                thread_ids: summary.thread_ids,
                ai_cluster_key: summary.ai_cluster_key.clone(),
                ai_summary: summary.ai_summary.clone(),
            };
            trace_inserter.write(&ch_trace).await?;
        }

        event_inserter.end().await?;
        trace_inserter.end().await?;
        
        Ok(())
    }

    async fn list_traces(&self) -> Result<Vec<TraceSummary>> {
        let rows = self.client.query("SELECT * FROM traces ORDER BY started_at DESC LIMIT 500").fetch_all::<ChTraceSummary>().await?;
        let traces = rows.into_iter().map(|r| TraceSummary {
            trace_id: Uuid::parse_str(&r.trace_id).unwrap_or_default(),
            name: r.name,
            started_at: r.started_at,
            duration_ms: r.duration_ms,
            has_error: r.has_error == 1,
            language: serde_json::from_str(&r.language).unwrap_or(crate::models::AgentLanguage::Unknown),
            event_count: r.event_count as usize,
            thread_ids: r.thread_ids,
            ai_cluster_key: r.ai_cluster_key,
            ai_summary: r.ai_summary,
        }).collect();
        Ok(traces)
    }

    async fn get_trace_events(&self, trace_id: Uuid) -> Result<Vec<TraceEvent>> {
        let tid_str = trace_id.to_string();
        let rows = self.client.query("SELECT * FROM events WHERE trace_id = ? ORDER BY timestamp_ms ASC")
            .bind(tid_str)
            .fetch_all::<ChEvent>().await?;
            
        let events = rows.into_iter().map(|r| TraceEvent {
            trace_id: Uuid::parse_str(&r.trace_id).unwrap_or_default(),
            span_id: Uuid::parse_str(&r.span_id).unwrap_or_default(),
            parent_span_id: r.parent_span_id.and_then(|s| Uuid::parse_str(&s).ok()),
            timestamp_ms: r.timestamp_ms,
            event_type: serde_json::from_str(&r.event_type).unwrap_or(crate::models::EventType::FunctionEnter),
            function_name: r.function_name,
            module: r.module,
            file: r.file,
            line: r.line,
            locals: serde_json::from_str(&r.locals).unwrap_or(None),
            error_message: r.error_message,
            duration_us: r.duration_us,
            language: serde_json::from_str(&r.language).unwrap_or(crate::models::AgentLanguage::Unknown),
            thread_id: r.thread_id,
            http_request: serde_json::from_str(&r.http_request).unwrap_or(None),
            db_query: serde_json::from_str(&r.db_query).unwrap_or(None),
        }).collect();
        
        Ok(events)
    }

    async fn clear_all(&self) -> Result<()> {
        self.client.query("TRUNCATE TABLE traces").execute().await?;
        self.client.query("TRUNCATE TABLE events").execute().await?;
        Ok(())
    }

    async fn get_service_graph(&self) -> Result<serde_json::Value> {
        // Fallback implementation, can be done via complex SQL later
        Ok(serde_json::json!({
            "nodes": [],
            "links": []
        }))
    }

    async fn get_event_by_span_id(&self, span_id: Uuid) -> Result<Option<TraceEvent>> {
        let sid_str = span_id.to_string();
        let row = self.client.query("SELECT * FROM events WHERE span_id = ? LIMIT 1")
            .bind(sid_str)
            .fetch_optional::<ChEvent>().await?;
            
        if let Some(r) = row {
            Ok(Some(TraceEvent {
                trace_id: Uuid::parse_str(&r.trace_id).unwrap_or_default(),
                span_id: Uuid::parse_str(&r.span_id).unwrap_or_default(),
                parent_span_id: r.parent_span_id.and_then(|s| Uuid::parse_str(&s).ok()),
                timestamp_ms: r.timestamp_ms,
                event_type: serde_json::from_str(&r.event_type).unwrap_or(crate::models::EventType::FunctionEnter),
                function_name: r.function_name,
                module: r.module,
                file: r.file,
                line: r.line,
                locals: serde_json::from_str(&r.locals).unwrap_or(None),
                error_message: r.error_message,
                duration_us: r.duration_us,
                language: serde_json::from_str(&r.language).unwrap_or(crate::models::AgentLanguage::Unknown),
                thread_id: r.thread_id,
                http_request: serde_json::from_str(&r.http_request).unwrap_or(None),
                db_query: serde_json::from_str(&r.db_query).unwrap_or(None),
            }))
        } else {
            Ok(None)
        }
    }

    async fn get_clusters(&self) -> Result<serde_json::Value> {
        Ok(serde_json::json!([])) // simplified
    }

    async fn update_trace_ai(&self, trace_id: Uuid, summary: String, cluster_key: Option<String>) -> Result<()> {
        Ok(())
    }

    async fn register_node(&self, node: crate::models::ClusterNode) -> Result<()> {
        let mut inserter = self.client.insert::<ChNode>("nodes").await?;
        let ch_node = ChNode {
            node_id: node.node_id,
            http_addr: node.http_addr,
            grpc_addr: node.grpc_addr,
            status: serde_json::to_string(&node.status).unwrap_or_default(),
            last_seen: node.last_seen,
        };
        inserter.write(&ch_node).await?;
        inserter.end().await?;
        Ok(())
    }

    async fn list_nodes(&self) -> Result<Vec<crate::models::ClusterNode>> {
        let rows = self.client.query("SELECT * FROM nodes FINAL").fetch_all::<ChNode>().await?;
        let nodes = rows.into_iter().map(|r| crate::models::ClusterNode {
            node_id: r.node_id,
            http_addr: r.http_addr,
            grpc_addr: r.grpc_addr,
            status: serde_json::from_str(&r.status).unwrap_or(crate::models::NodeStatus::Active),
            last_seen: r.last_seen,
        }).collect();
        Ok(nodes)
    }

    async fn add_comment(&self, comment: crate::models::TraceComment) -> Result<()> {
        let mut inserter = self.client.insert::<ChComment>("comments").await?;
        let ch_comment = ChComment {
            comment_id: comment.comment_id.to_string(),
            trace_id: comment.trace_id.to_string(),
            span_id: comment.span_id.map(|u| u.to_string()),
            user_id: comment.user_id,
            text: comment.text,
            timestamp_ms: comment.timestamp_ms,
        };
        inserter.write(&ch_comment).await?;
        inserter.end().await?;
        Ok(())
    }

    async fn get_comments(&self, trace_id: Uuid) -> Result<Vec<crate::models::TraceComment>> {
        let tid_str = trace_id.to_string();
        let rows = self.client.query("SELECT * FROM comments WHERE trace_id = ? ORDER BY timestamp_ms ASC")
            .bind(tid_str)
            .fetch_all::<ChComment>().await?;
            
        let comments = rows.into_iter().map(|r| crate::models::TraceComment {
            comment_id: Uuid::parse_str(&r.comment_id).unwrap_or_default(),
            trace_id: Uuid::parse_str(&r.trace_id).unwrap_or_default(),
            span_id: r.span_id.and_then(|s| Uuid::parse_str(&s).ok()),
            user_id: r.user_id,
            text: r.text,
            timestamp_ms: r.timestamp_ms,
        }).collect();
        
        Ok(comments)
    }
}
