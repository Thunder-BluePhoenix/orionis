mod models;
mod store;
mod server;
mod ws;

use tracing::info;
use tracing_subscriber::fmt;
use server::{AppState, build_router};
use ws::WsBroadcaster;

const DB_PATH: &str = "orionis.db";
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
    let state = AppState { db, ws };
    let router = build_router(state);

    let listener = tokio::net::TcpListener::bind(BIND_ADDR).await?;
    info!("Orionis engine listening on http://{}", BIND_ADDR);
    info!("Dashboard: http://localhost:7700");
    info!("Ingest endpoint: POST http://localhost:7700/api/ingest");
    info!("WebSocket live: ws://localhost:7700/ws/live");

    axum::serve(listener, router).await?;
    Ok(())
}
