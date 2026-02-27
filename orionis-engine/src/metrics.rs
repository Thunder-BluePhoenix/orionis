use prometheus::{Registry, Counter, Gauge, Opts, register_counter_with_registry, register_gauge_with_registry};
use lazy_static::lazy_static;

lazy_static! {
    pub static ref REGISTRY: Registry = Registry::new();

    pub static ref INGESTION_TOTAL: Counter = register_counter_with_registry!(
        Opts::new("orionis_ingestion_total", "Total events ingested"),
        REGISTRY
    ).unwrap();

    pub static ref INGESTION_ERRORS: Counter = register_counter_with_registry!(
        Opts::new("orionis_ingestion_errors", "Total ingestion errors"),
        REGISTRY
    ).unwrap();

    pub static ref ACTIVE_CONNECTIONS: Gauge = register_gauge_with_registry!(
        Opts::new("orionis_active_connections", "Current active WebSocket connections"),
        REGISTRY
    ).unwrap();

    pub static ref WAL_ENTRIES: Counter = register_counter_with_registry!(
        Opts::new("orionis_wal_entries_total", "Total entries written to WAL"),
        REGISTRY
    ).unwrap();
    
    pub static ref CONCURRENCY_AVAILABLE: Gauge = register_gauge_with_registry!(
        Opts::new("orionis_concurrency_available", "Available permits in the ingestion semaphore"),
        REGISTRY
    ).unwrap();
}

pub fn gather() -> String {
    use prometheus::Encoder;
    let encoder = prometheus::TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}
