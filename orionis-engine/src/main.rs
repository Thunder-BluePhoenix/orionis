mod models;
mod store;
mod store_clickhouse;
mod server;
mod auth;
mod ws;
mod grpc;
mod clustering;
mod wal;
mod metrics;

use tracing::info;
use tracing_subscriber::fmt;
use server::{AppState, build_router};
use ws::WsBroadcaster;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Logging ────────────────────────────────────────────────────────────
    fmt::init();

    let db_path = std::env::var("ORIONIS_DB_PATH").unwrap_or_else(|_| "orionis-data".to_string());
    let http_addr = std::env::var("ORIONIS_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:7700".to_string());
    let grpc_addr = std::env::var("ORIONIS_GRPC_ADDR").unwrap_or_else(|_| "0.0.0.0:7701".to_string());

    info!("
 ██████╗ ██████╗ ██╗ ██████╗ ███╗   ██╗██╗███████╗
██╔═══██╗██╔══██╗██║██╔═══██╗████╗  ██║██║██╔════╝
██║   ██║██████╔╝██║██║   ██║██╔██╗ ██║██║███████╗
██║   ██║██╔══██╗██║██║   ██║██║╚██╗██║██║╚════██║
╚██████╔╝██║  ██║██║╚██████╔╝██║ ╚████║██║███████║
 ╚═════╝ ╚═╝  ╚═╝╚═╝ ╚═════╝ ╚═╝  ╚═══╝╚═╝╚══════╝
 Runtime Intelligence Engine — v0.1.0
");

    // ── Storage ─────────────────────────────────────────────────────────────
    let storage_backend = std::env::var("ORIONIS_STORAGE").unwrap_or_else(|_| "redb".to_string());
    
    let db: store::DbHandle = if storage_backend == "clickhouse" {
        info!("Opening ClickHouse database at {}", db_path);
        let ch = store_clickhouse::ClickHouseStore::new(&db_path).await?;
        std::sync::Arc::new(ch)
    } else {
        info!("Opening Redb local storage at {}", db_path);
        store::open(&db_path)?
    };

    // ── WAL (Write Ahead Log) ──────────────────────────────────────────────
    let wal = std::sync::Arc::new(wal::WalManager::new(&db_path)?);
    let ingestion_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
        std::env::var("ORION_INGESTION_LIMIT").unwrap_or_else(|_| "500".to_string()).parse().unwrap_or(500)
    ));
    
    // Auto-replay on startup
    info!("Checking for WAL recovery...");
    let replay_entries = wal.replay()?;
    if !replay_entries.is_empty() {
        info!("Recovering {} events from WAL...", replay_entries.is_empty());
        let mut tenant_batches: std::collections::HashMap<Option<String>, Vec<models::TraceEvent>> = std::collections::HashMap::new();
        for entry in replay_entries {
            tenant_batches.entry(entry.tenant_id).or_default().push(entry.event);
        }
        for (tid, events) in tenant_batches {
            let _ = db.ingest_events(events, tid).await;
        }
        wal.checkpoint()?; // Safe to clear after recovery
        info!("WAL recovery complete.");
    }

    // ── Clustering ──────────────────────────────────────────────────────────
    let cluster_manager = std::sync::Arc::new(clustering::ClusterManager::new(
        db.clone(),
        http_addr.clone(),
        grpc_addr.clone(),
    ));
    // start() takes ownership of its Arc, so we clone it to run in background
    cluster_manager.clone().start().await;

    // ── WebSocket broadcaster ────────────────────────────────────────────────
    let ws = WsBroadcaster::new();

    // ── HTTP Server ──────────────────────────────────────────────────────────
    let state = AppState { 
        db: db.clone(), 
        ws: ws.clone(),
        cluster: cluster_manager.clone(),
        wal: wal.clone(),
        ingestion_limit: ingestion_limit.clone(),
    };
    let router = build_router(state);

    let axum_listener = tokio::net::TcpListener::bind(&http_addr).await?;
    info!("Orionis engine (HTTP/WS) listening on http://{}", http_addr);
    info!("Dashboard: http://localhost:{}", http_addr.split(':').last().unwrap_or("7700"));
    info!("Ingest endpoint: POST http://{}/api/ingest", http_addr);
    info!("WebSocket live: ws://{}/ws/live", http_addr);

    let axum_server = axum::serve(axum_listener, router);

    // ── gRPC Server ──────────────────────────────────────────────────────────
    let grpc_socket_addr = grpc_addr.parse()?;
    info!("Orionis engine (gRPC) listening on grpc://{}", grpc_addr);
    let ingest_service = grpc::IngestService {
        db: db,
        ws: ws,
        wal: wal,
        ingestion_limit: ingestion_limit,
    };
    
    let grpc_server = tonic::transport::Server::builder()
        .add_service(grpc::proto::ingest_server::IngestServer::new(ingest_service))
        .serve(grpc_socket_addr);

    // ── Background Maintenance (Cleanup) ──────────────────────────────────
    let retention_days: u32 = std::env::var("ORION_RETENTION_DAYS")
        .unwrap_or_else(|_| "7".to_string())
        .parse()
        .unwrap_or(7);
    
    let db_for_cleanup = db.clone();
    tokio::spawn(async move {
        loop {
            info!("Running background data retention cleanup ({} days)...", retention_days);
            if let Err(e) = db_for_cleanup.cleanup_expired_data(retention_days).await {
                tracing::error!("Retention cleanup failed: {}", e);
            }
            // Repeat every 24 hours (86400 seconds)
            tokio::time::sleep(tokio::time::Duration::from_secs(86400)).await;
        }
    });

    // Run both servers concurrently
    tokio::try_join!(
        async { axum_server.await.map_err(|e| anyhow::anyhow!(e)) },
        async { grpc_server.await.map_err(|e| anyhow::anyhow!(e)) }
    )?;
    Ok(())
}
