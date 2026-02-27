use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, Duration};
use uuid::Uuid;
use crate::models::{ClusterNode, NodeStatus};
use crate::store::DbHandle;

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

    pub async fn forward_event(&self, node: &ClusterNode, event: crate::models::TraceEvent) -> anyhow::Result<()> {
        if node.node_id == self.node_id {
            return Ok(()); // Already owner
        }

        let url = format!("http://{}/api/ingest", node.http_addr);
        let client = reqwest::Client::new();
        client.post(&url)
            .json(&event)
            .send()
            .await?;
            
        Ok(())
    }
}
