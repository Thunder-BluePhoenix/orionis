mod models;
mod store;
mod server;
mod ws;
mod grpc;

use tracing::info;
use tracing_subscriber::fmt;
use server::{AppState, build_router};
use ws::WsBroadcaster;

const DB_PATH: &str = "orionis-data";
const BIND_ADDR: &str = "0.0.0.0:7700";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Logging ────────────────────────────────────────────────────────────
    fmt::init();

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
    info!("Opening database at {}", DB_PATH);
    let db = store::open(DB_PATH)?;

    // ── WebSocket broadcaster ────────────────────────────────────────────────
    let ws = WsBroadcaster::new();

    // ── HTTP Server ──────────────────────────────────────────────────────────
    let state = AppState { db: db.clone(), ws: ws.clone() };
    let router = build_router(state);

    let axum_listener = tokio::net::TcpListener::bind(BIND_ADDR).await?;
    info!("Orionis engine (HTTP/WS) listening on http://{}", BIND_ADDR);
    info!("Dashboard: http://localhost:7700");
    info!("Ingest endpoint: POST http://localhost:7700/api/ingest");
    info!("WebSocket live: ws://localhost:7700/ws/live");

    let axum_server = axum::serve(axum_listener, router);

    // ── gRPC Server ──────────────────────────────────────────────────────────
    let grpc_addr = "0.0.0.0:7701".parse()?;
    info!("Orionis engine (gRPC) listening on grpc://0.0.0.0:7701");
    let ingest_service = grpc::IngestService {
        db: db,
        ws: ws,
    };
    
    let grpc_server = tonic::transport::Server::builder()
        .add_service(grpc::proto::ingest_server::IngestServer::new(ingest_service))
        .serve(grpc_addr);

    // Run both servers concurrently
    tokio::try_join!(
        async { axum_server.await.map_err(|e| anyhow::anyhow!(e)) },
        async { grpc_server.await.map_err(|e| anyhow::anyhow!(e)) }
    )?;
    Ok(())
}
