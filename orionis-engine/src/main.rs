mod models;
mod store;
mod store_clickhouse;
mod server;
mod auth;
mod ws;
mod grpc;
mod clustering;

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
    };
    
    let grpc_server = tonic::transport::Server::builder()
        .add_service(grpc::proto::ingest_server::IngestServer::new(ingest_service))
        .serve(grpc_socket_addr);

    // Run both servers concurrently
    tokio::try_join!(
        async { axum_server.await.map_err(|e| anyhow::anyhow!(e)) },
        async { grpc_server.await.map_err(|e| anyhow::anyhow!(e)) }
    )?;
    Ok(())
}
