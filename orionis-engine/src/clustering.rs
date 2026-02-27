use crate::models::{ClusterNode, NodeStatus};
use crate::store::DbHandle;
use crate::grpc::proto::{self, ingest_client::IngestClient};
use tonic::transport::Channel;

pub struct ClusterManager {
    db: DbHandle,
    node_id: String,
    http_addr: String,
    grpc_addr: String,
}

impl ClusterManager {
    pub fn new(db: DbHandle, http_addr: String, grpc_addr: String) -> Self {
        Self {
            db,
            node_id: Uuid::new_v4().to_string(),
            http_addr,
            grpc_addr,
        }
    }

    pub async fn start(self: std::sync::Arc<Self>) {
        let node_id = self.node_id.clone();
        let http_addr = self.http_addr.clone();
        let grpc_addr = self.grpc_addr.clone();
        let db = self.db.clone();

        tokio::spawn(async move {
            loop {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(); 

                let node = ClusterNode {
                    node_id: node_id.clone(),
                    http_addr: http_addr.clone(), 
                    grpc_addr: grpc_addr.clone(), 
                    status: NodeStatus::Active,
                    last_seen: now, 
                };

                if let Err(e) = db.register_node(node).await { 
                    tracing::error!("Failed to register node: {}", e); 
                }

                sleep(Duration::from_secs(5)).await;
            }
        });
    }

    /// Returns the unique ID of this node.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Returns the HTTP address this node is listening on.
    pub fn http_addr(&self) -> &str {
        &self.http_addr
    }

    pub async fn get_owner(&self, trace_id: uuid::Uuid) -> Option<ClusterNode> { // Changed trace_id type to uuid::Uuid and made async
        let nodes = self.db.list_nodes().await.ok()?; // Changed to self.db.list_nodes().await
        if nodes.is_empty() { return None; }

        let mut sorted_nodes = nodes;
        sorted_nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));

        // Simple hash-based mapping
        let hash = trace_id.as_bytes().iter().fold(0u64, |acc, &x| acc.wrapping_add(x as u64));
        let index = (hash % sorted_nodes.len() as u64) as usize;
        
        Some(sorted_nodes[index].clone())
    }

    pub async fn forward_event(&self, node: &ClusterNode, event: crate::models::TraceEvent, tenant_id: Option<String>) -> anyhow::Result<()> {
        if node.node_id == self.node_id {
            return Ok(()); // Already owner
        }

        let grpc_url = format!("http://{}", node.grpc_addr);
        let channel = Channel::from_shared(grpc_url)?
            .connect()
            .await?;
            
        let mut client = IngestClient::new(channel);

        // Map native model to proto
        let proto_ev = proto::TraceEvent {
            trace_id: event.trace_id.to_string(),
            span_id: event.span_id.to_string(),
            parent_span_id: event.parent_span_id.map(|u| u.to_string()).unwrap_or_default(),
            timestamp_ms: event.timestamp_ms,
            event_type: match event.event_type {
                crate::models::EventType::FunctionEnter => 1,
                crate::models::EventType::FunctionExit => 2,
                crate::models::EventType::Exception => 3,
                crate::models::EventType::AsyncSpawn => 4,
                crate::models::EventType::AsyncResume => 5,
                crate::models::EventType::HttpRequest => 6,
                crate::models::EventType::HttpResponse => 7,
                crate::models::EventType::DbQuery => 8,
            },
            function_name: event.function_name,
            module: event.module,
            file: event.file,
            line: event.line,
            locals: event.locals.unwrap_or_default().into_iter().map(|l| proto::LocalVar {
                name: l.name,
                value: l.value,
                type_name: l.type_name,
            }).collect(),
            error_message: event.error_message.unwrap_or_default(),
            duration_us: event.duration_us,
            language: match event.language {
                crate::models::AgentLanguage::Python => 1,
                crate::models::AgentLanguage::Go => 2,
                crate::models::AgentLanguage::Rust => 3,
                crate::models::AgentLanguage::Cpp => 4,
                crate::models::AgentLanguage::C => 5,
                crate::models::AgentLanguage::Java => 6,
                crate::models::AgentLanguage::Node => 7,
                crate::models::AgentLanguage::Unknown => 0,
            },
            thread_id: event.thread_id.unwrap_or_default(),
            http_request: event.http_request.map(|r| proto::HttpRequest {
                method: r.method,
                url: r.url,
                headers: r.headers,
                body: r.body.unwrap_or_default(),
            }),
            db_query: event.db_query.map(|q| proto::DbQuery {
                query: q.query,
                driver: q.driver,
                duration_us: q.duration_us,
            }),
            tenant_id: tenant_id.unwrap_or_default(),
        };

        let stream = tokio_stream::iter(vec![proto_ev]);
        client.stream_events(stream).await?;
            
        Ok(())
    }
}
