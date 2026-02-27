use clap::Parser;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::time::{Duration, Instant};
use uuid::Uuid;
use rand::Rng;

pub mod proto {
    tonic::include_proto!("orionis");
}

use proto::ingest_client::IngestClient;
use proto::TraceEvent as ProtoEvent;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "http://localhost:7700")]
    engine_url: String,

    #[arg(short, long, default_value = "grpc://localhost:7701")]
    grpc_url: String,

    #[arg(short, long, default_value = "1000")]
    rate: u64, // spans per second

    #[arg(short, long, default_value = "10")]
    concurrency: u64,

    #[arg(short, long, default_value = "30")]
    duration: u64, // seconds

    #[arg(short, long, default_value = "grpc")]
    mode: String, // "http" or "grpc"
}

struct Stats {
    success: AtomicU64,
    errors: AtomicU64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let stats = Arc::new(Stats {
        success: AtomicU64::new(0),
        errors: AtomicU64::new(0),
    });

    println!("🚀 Starting Orionis LoadGen");
    println!("Mode: {}, Rate: {} spans/s, Concurrency: {}, Duration: {}s", 
             args.mode, args.rate, args.concurrency, args.duration);

    let start_time = Instant::now();
    let duration = Duration::from_secs(args.duration);
    
    let mut tasks = Vec::new();
    let rate_per_worker = args.rate / args.concurrency;

    for i in 0..args.concurrency {
        let args = args.clone();
        let stats = stats.clone();
        
        tasks.push(tokio::spawn(async move {
            if args.mode == "grpc" {
                run_grpc_worker(args, stats, rate_per_worker, i, duration).await;
            } else {
                run_http_worker(args, stats, rate_per_worker, i, duration).await;
            }
        }));
    }

    // Progress reporter
    let stats_reporter = stats.clone();
    tokio::spawn(async move {
        let mut last_success = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let current = stats_reporter.success.load(Ordering::Relaxed);
            let errors = stats_reporter.errors.load(Ordering::Relaxed);
            println!("  📊 Throughput: {} spans/s | Total: {} | Errors: {}", current - last_success, current, errors);
            last_success = current;
        }
    });

    futures::future::join_all(tasks).await;

    let total_success = stats.success.load(Ordering::Relaxed);
    let total_elapsed = start_time.elapsed().as_secs_f64();
    
    println!("\n🏁 Benchmark Finished");
    println!("Total Spans: {}", total_success);
    println!("Average Rate: {:.2} spans/s", total_success as f64 / total_elapsed);
    println!("Total Errors: {}", stats.errors.load(Ordering::Relaxed));

    Ok(())
}

async fn run_grpc_worker(args: Args, stats: Arc<Stats>, rate: u64, id: u64, total_duration: Duration) {
    let mut client = match IngestClient::connect(args.grpc_url.clone()).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Worker {} failed to connect: {}", id, e);
            return;
        }
    };

    let start = Instant::now();
    let interval = Duration::from_micros(1_000_000 / rate.max(1));
    let mut ticker = tokio::time::interval(interval);

    while start.elapsed() < total_duration {
        ticker.tick().await;

        let event = generate_proto_event();
        let request = tonic::Request::new(futures::stream::iter(vec![event]));

        match client.stream_events(request).await {
            Ok(_) => { stats.success.fetch_add(1, Ordering::Relaxed); }
            Err(_) => { stats.errors.fetch_add(1, Ordering::Relaxed); }
        }
    }
}

async fn run_http_worker(args: Args, stats: Arc<Stats>, rate: u64, _id: u64, total_duration: Duration) {
    let client = reqwest::Client::new();
    let url = format!("{}/api/ingest", args.engine_url);
    
    let start = Instant::now();
    let interval = Duration::from_micros(1_000_000 / rate.max(1));
    let mut ticker = tokio::time::interval(interval);

    while start.elapsed() < total_duration {
        ticker.tick().await;

        let event = generate_json_event();
        match client.post(&url)
            .header("X-Orionis-Api-Key", "secret-agent-key") // use default or env
            .json(&event)
            .send()
            .await {
                Ok(resp) if resp.status().is_success() => { stats.success.fetch_add(1, Ordering::Relaxed); }
                _ => { stats.errors.fetch_add(1, Ordering::Relaxed); }
            }
    }
}

fn generate_proto_event() -> ProtoEvent {
    let mut rng = rand::thread_rng();
    ProtoEvent {
        trace_id: Uuid::new_v4().to_string(),
        span_id: Uuid::new_v4().to_string(),
        parent_span_id: "".to_string(),
        timestamp_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
        event_type: "function_enter".to_string(),
        function_name: "benchmark_test".to_string(),
        module: "loadgen".to_string(),
        file: "loadgen.rs".to_string(),
        line: rng.gen_range(1..1000),
        error_message: "".to_string(),
        duration_us: 0,
        language: "rust".to_string(),
        thread_id: "main".to_string(),
        tenant_id: "benchmark-tenant".to_string(),
        http_request: None,
        db_query: None,
    }
}

fn generate_json_event() -> serde_json::Value {
    let mut rng = rand::thread_rng();
    serde_json::json!({
        "trace_id": Uuid::new_v4(),
        "span_id": Uuid::new_v4(),
        "timestamp_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
        "event_type": "function_enter",
        "function_name": "benchmark_test",
        "module": "loadgen",
        "file": "loadgen.rs",
        "line": rng.gen_range(1..1000),
        "language": "rust",
        "tenant_id": "benchmark-tenant"
    })
}
