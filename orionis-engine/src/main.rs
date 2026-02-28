use anyhow::Result;
use tracing::info;

mod models;
mod store;
mod store_clickhouse;
mod wal;
mod clustering;
mod ws;
mod server;
mod analyzer;
mod auth;
mod metrics;
mod grpc;
mod replay;
mod intel;

use server::{AppState, build_router};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    info!("Starting Orionis Engine v2.0...");

    // ── Configuration ────────────────────────────────────────────────────────
    let http_addr    = std::env::var("ORIONIS_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:7700".to_string());
    let grpc_addr    = std::env::var("ORIONIS_GRPC_ADDR").unwrap_or_else(|_| "0.0.0.0:7701".to_string());
    let data_dir     = std::env::var("ORIONIS_DATA_DIR").unwrap_or_else(|_| "orionis-data".to_string());
    let storage_backend = std::env::var("ORIONIS_STORAGE").unwrap_or_else(|_| "local".to_string());
    let db_path      = std::env::var("ORIONIS_DB_PATH")
        .unwrap_or_else(|_| format!("{}/engine.db", data_dir));

    // Create data directory idempotently
    let data_path = std::path::Path::new(&data_dir);
    if data_path.exists() && !data_path.is_dir() {
        std::fs::remove_file(data_path)?;
    }
    if !data_path.exists() {
        std::fs::create_dir_all(data_path)?;
    }

    // ── Storage ─────────────────────────────────────────────────────────────
    let db: store::DbHandle = if storage_backend == "clickhouse" {
        info!("Opening ClickHouse database at {}", db_path);
        let ch = store_clickhouse::ClickHouseStore::new(&db_path).await?;
        std::sync::Arc::new(ch)
    } else {
        info!("Opening Redb local storage at {}", db_path);
        store::open(&db_path)?
    };
    info!("Storage backend initialized.");

    // ── WAL (Write Ahead Log) ──────────────────────────────────────────────
    let wal = std::sync::Arc::new(wal::WalManager::new(&data_dir)?);
    info!("WAL manager initialized.");
    let ingestion_limit = std::sync::Arc::new(tokio::sync::Semaphore::new(
        std::env::var("ORION_INGESTION_LIMIT").unwrap_or_else(|_| "500".to_string()).parse().unwrap_or(500)
    ));

    // Auto-replay on startup
    info!("Checking for WAL recovery...");
    let replay_entries = wal.replay()?;
    if !replay_entries.is_empty() {
        info!("Recovering {} events from WAL...", replay_entries.len());
        let mut tenant_batches: std::collections::HashMap<Option<String>, Vec<models::TraceEvent>> = std::collections::HashMap::new();
        for entry in replay_entries {
            tenant_batches.entry(entry.tenant_id).or_default().push(entry.event);
        }
        for (tid, events) in tenant_batches {
            let _ = db.ingest_events(events, tid).await;
        }
        wal.checkpoint()?;
        info!("WAL recovery complete.");
    } else {
        info!("No WAL recovery needed.");
    }

    // ── Clustering ──────────────────────────────────────────────────────────
    let cluster_manager = std::sync::Arc::new(clustering::ClusterManager::new(
        db.clone(),
        http_addr.clone(),
        grpc_addr.clone(),
    ));
    info!("Cluster manager initialized.");
    cluster_manager.clone().start().await;
    info!("Cluster manager started.");

    // ── WebSocket broadcaster ────────────────────────────────────────────────
    let ws = ws::WsBroadcaster::new();
    info!("WebSocket broadcaster initialized.");

    // ── HTTP + Router ────────────────────────────────────────────────────────
    let state = AppState {
        db: db.clone(),
        ws: ws.clone(),
        cluster: cluster_manager.clone(),
        wal: wal.clone(),
        ingestion_limit: ingestion_limit.clone(),
    };
    let router = build_router(state);
    info!("Router built successfully.");

    let axum_listener = tokio::net::TcpListener::bind(&http_addr).await?;
    info!("Orionis engine (HTTP/WS) listening on http://{}", http_addr);
    info!("Dashboard: http://localhost:{}", http_addr.split(':').last().unwrap_or("7700"));
    let axum_server = axum::serve(axum_listener, router);

    // ── gRPC Server ──────────────────────────────────────────────────────────
    let grpc_socket_addr = grpc_addr.parse()?;
    info!("Orionis engine (gRPC) listening on grpc://{}", grpc_addr);
    let ingest_service = grpc::IngestService {
        db: db.clone(),
        ws: ws.clone(),
        wal: wal.clone(),
        ingestion_limit: ingestion_limit.clone(),
    };
    let control_service = grpc::ControlService {
        ingestion_limit: ingestion_limit.clone(),
        cluster: cluster_manager.clone(),
    };
    let grpc_server = tonic::transport::Server::builder()
        .add_service(grpc::proto::ingest_server::IngestServer::new(ingest_service))
        .add_service(grpc::proto::control_server::ControlServer::new(control_service))
        .serve(grpc_socket_addr);

    // ── Background Maintenance ───────────────────────────────────────────────
    let retention_days: u32 = std::env::var("ORION_RETENTION_DAYS")
        .unwrap_or_else(|_| "7".to_string())
        .parse()
        .unwrap_or(7);
    let db_for_cleanup = db.clone();
    tokio::spawn(async move {
        loop {
            if let Err(e) = db_for_cleanup.cleanup_expired_data(retention_days).await {
                tracing::error!("Retention cleanup failed: {}", e);
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(86400)).await;
        }
    });

    // ── Run HTTP + gRPC concurrently ─────────────────────────────────────────
    tokio::try_join!(
        async { axum_server.await.map_err(|e| anyhow::anyhow!("HTTP: {}", e)) },
        async { grpc_server.await.map_err(|e| anyhow::anyhow!("gRPC: {}", e)) }
    )?;

    Ok(())
}
