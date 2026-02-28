# Orionis — Product & Engineering Plan

> **Product name:** Orionis
> **Type:** Standalone runtime intelligence engine
> **Model:** Open-source first, solo long-term project
> **Start date:** 2026

---

## Vision Statement

Orionis is an intelligent runtime observability and time-travel debugging engine that lets developers **see, replay, and understand** their systems across services — in real time and in history.

---

## The Four Core Pillars

### Pillar 1 — Intelligent Stack Trace Visualizer
Transform raw stack traces into a navigable **visual execution graph**:
- Variable states at each frame
- Async chain linking
- Highlighted root cause guess
- AI explanation panel

### Pillar 2 — Runtime Time-Travel Debugger
Record and replay execution like Git for code — but for runtime:
- Step backward/forward through execution
- Inspect state at any timestamp
- Replay failed requests
- Compare two execution runs

### Pillar 3 — Live Dependency Graph Inspector
Interactive graph showing:
- Service-to-service calls
- Module-to-module dependencies
- DB interactions, event emissions, background jobs
- Click a node → see latency, error rate, recent failures

### Pillar 4 — Cross-Service Request Tracer
Full trace timeline for requests flowing across services:
- `Frontend → API Gateway → Auth → Order → Payment → DB`
- Latency breakdown per service
- State snapshot at the point of failure

---

## Architecture

### Three-Layer Design

```
┌─────────────────────────────┐
│  Orionis Agent (SDK Layer)  │  Python / Go / Rust / C++
│  Captures execution events  │
└─────────────┬───────────────┘
              │ gRPC / TCP
┌─────────────▼───────────────┐
│  Orionis Engine (Rust Core) │  Event ingestion, storage,
│  Replay, Analysis, Graph    │  replay engine, graph builder
└─────────────┬───────────────┘
              │ WebSocket + HTTP
┌─────────────▼───────────────┐
│  Orionis Dashboard (UI)     │  HTML + CSS + Vanilla JS
│  Timeline, Stack, Locals    │  D3.js for graph viz
└─────────────────────────────┘
```

---

## Tech Stack

| Component | Technology | Reason |
|---|---|---|
| Core Engine | **Rust** + Tokio + Axum | Memory safe, high-perf, great concurrency, FFI |
| Storage (v1) | **SQLite** (embedded) | Simple, ship-fast, local-first |
| Storage (v2+) | **RocksDB** or ClickHouse | Time-indexed, scalable |
| Protocol (v1) | **HTTP JSON** | Fast to develop, correct first |
| Protocol (v2+) | **gRPC (tonic)** | High-performance production |
| Python Agent | **sys.setprofile()** | Function-level global tracing, lower overhead |
| Dashboard | **HTML + CSS + Vanilla JS + D3.js** | No framework bloat |
| Live updates | **WebSockets** | Real-time trace streaming |
| License | **Apache 2.0** or **MIT** | Enterprise-friendly open source |

---

## Event Model (Core Data Structure)

```rust
struct TraceEvent {
    trace_id: Uuid,
    span_id: Uuid,
    parent_span_id: Option<Uuid>,
    timestamp: u64,
    event_type: EventType,
    function_name: String,
    file: String,
    line: u32,
    locals: Option<SerializedState>,
}

enum EventType {
    FunctionEnter,
    FunctionExit,
    Exception,
}
```

This is the foundation. Every feature is built on top of this event stream.

---

## Tracing Mode Decision

**Choice: Global Tracing Mode (B)**
- Use `sys.setprofile()` for whole-process tracing
- Filter noise via module inclusion/exclusion lists:
  ```python
  orionis.start(
      include_modules=["myapp"],
      exclude_modules=["asyncio", "logging"]
  )
  ```
- Path-based filtering: trace `/project/` only, ignore `/usr/lib/python/`

### Variable Capture Policy (v1)
- Shallow copy of locals only
- Max string length: 200 chars
- Max list length: 50 items
- Max depth: 2 levels
- `repr()` fallback for complex objects

---

## Tracing Modes

| Mode | Behavior | Use Case |
|---|---|---|
| **DEV MODE** | Full capture, all locals, deep tracing | Local development |
| **SAFE MODE** | Function calls only, no locals, sampling | Staging |
| **ERROR MODE** | Lightweight always, full capture on exception | **Production** |

> **Smart Production Strategy:** lightweight trace always → exception raised → escalate to full capture → persist that trace only.
> Low overhead. Rich failure data. Enterprise-grade design.

---

## Repository Structure

```
orionis/
 ├── orionis-engine/          # Rust core engine
 │    ├── src/
 │    └── Cargo.toml
 │
 ├── agents/
 │    ├── python/              # Python agent — sys.setprofile(), pytest plugin
 │    ├── go/                  # Go agent — goroutine-aware, middleware
 │    ├── rust/                # Rust agent — macro-based native instrumentation
 │    ├── cpp/                 # C++ agent — RAII, exception hooks, signals
 │    ├── c/                   # C agent — cyg_profile hooks, signal capture
 │    ├── node/                # Node.js agent — async_hooks, require hook
 │    └── java/                # Java agent — JVM bytecode instrument, -javaagent
 │
 ├── orion-cli/               # `orion` CLI tool (Python)
 │
 ├── dashboard/               # Frontend UI
 │    ├── index.html
 │    ├── app.js
 │    └── styles.css
 │
 ├── proto/                   # Protobuf definitions
 │
 └── truth_docs/              # Philosophy, plan, session summaries
```

---

## Dashboard Routes

| Route | Purpose |
|---|---|
| `/` | Main UI |
| `/api/traces` | List all traces |
| `/api/trace/{id}` | Fetch single trace |
| `/ws/live` | WebSocket live stream |

**Dashboard panels:**
- **Left**: Trace list (searchable, filterable)
- **Center**: Timeline slider (time-travel control)
- **Right**: Stack tree view
- **Bottom**: Locals/variable viewer

---

## Phased Build Plan

### Phase 1 — Foundation: All 7 Agents + CLI (Months 1–6)
**Goal: Ship a working local runtime intelligence engine with all 7 language agents from Day 1.**

> All seven language agents — Python, Go, Rust, C++, C, Java, Node.js — are built simultaneously. No phasing of languages. This is non-negotiable.

| Month | Deliverable |
|---|---|
| Month 1 | Rust engine core — event ingestion, SQLite storage, WebSocket broadcaster, HTTP server |
| Month 2 | Python agent — `sys.setprofile()`, global tracing, module filter, variable shallow capture |
| Month 3 | Go agent — middleware wrappers, goroutine-aware context propagation, panic interceptors |
| Month 4 | Rust agent — native instrumentation macros, FFI event bridge to Orionis engine |
| Month 5 | C++ agent — RAII-based instrumentation, macro tracing, exception hooks, signal capture |
| Month 5 | C agent — lightweight `__cyg_profile_func_enter/exit` hooks, `SIGSEGV`/`SIGABRT` signal capture |
| Month 5 | Node.js agent — `--require` hook, `async_hooks` API, unhandledRejection + uncaughtException capture |
| Month 6 | Java agent — JVM bytecode agent via `-javaagent`, method enter/exit + exception capture |
| Month 6 | `orion` CLI + UI + test integration — ship **v0.1** |

**`orion` CLI (Day 1 commands):**
```
orion init                            # Auto-detect language, scaffold agent config in project
orion start                          # Start the Orionis engine daemon
orion stop                           # Stop the engine
orion status                         # Show engine status + active traces
orion replay <trace-id>              # Replay a specific trace in the UI
orion list                           # List all captured traces
orion export <trace-id>              # Export trace to .otrace file
orion import <file.otrace>           # Import and load a shared trace
orion diff <trace-id-1> <trace-id-2> # Side-by-side execution diff
orion watch                          # Auto-capture trace on file change + re-run
orion clean                          # Clear all stored traces
orion config                         # Show/edit configuration
```

**v0.1 Scope (exact):**
- ✅ Rust engine (single binary, local, SQLite)
- ✅ Python tracing agent — global mode, module filter, variable snapshot
- ✅ Go tracing agent — goroutine-aware, middleware, panic capture
- ✅ Rust tracing agent — macro-based native instrumentation
- ✅ C++ tracing agent — RAII instrumentation, exception hooks, signal capture
- ✅ C tracing agent — `cyg_profile` hooks, signal interception, crash reporting
- ✅ Node.js tracing agent — `async_hooks`, require hook, unhandledRejection capture
- ✅ Java tracing agent — JVM `-javaagent`, bytecode instrumentation, exception capture
- ✅ `orion init` — one-command setup, auto-detects language, scaffolds agent config
- ✅ `orion` CLI with start / stop / replay / list / export / import / diff / watch
- ✅ Panic/crash zero-config auto-detection — Rust panics, Go panics, C++ signals, Python exceptions, Node.js rejections, Java exceptions auto-caught with NO setup needed beyond `orion start`
- ✅ Environment snapshot — capture env vars, OS info, config values at trace start (solves "works on my machine" forever)
- ✅ Test framework auto-capture — `pytest`, `cargo test`, `go test`, `npm test`, `mvn test` failures auto-traced
- ✅ Trace export/import — `.otrace` portable file format for sharing failures
- ✅ Dashboard UI — stack tree, timeline slider, variable panel, replay controls
- ✅ DEV / SAFE / ERROR tracing modes
- ❌ No AI yet
- ❌ No cross-service graph yet
- ❌ No distributed tracing yet
- ❌ No gRPC yet (HTTP JSON for now)

---

### Phase 2 — Cross-Service + Developer Power Features (Months 7–11)
**Goal: Make Orionis multi-service aware. Add the power features devs demand.**

| Month | Deliverable |
|---|---|
| Month 7 | Cross-service trace ID propagation — W3C TraceContext header support across all 4 agents |
| Month 8 | Live service dependency graph — interactive D3 node map, latency + error rate overlays |
| Month 9 | Flamegraph overlay — performance flame graph layered over the execution tree |
| Month 10 | HTTP request replay — re-fire the exact HTTP request that triggered a failure |
| Month 11 | Async/concurrent execution visualizer — goroutines, Tokio tasks, Python asyncio shown as parallel lanes |

**v0.2 Scope:**
- ✅ Cross-service trace stitching via propagated trace ID headers
- ✅ Interactive service dependency graph (D3)
- ✅ Flamegraph view on execution tree
- ✅ HTTP request replay (capture request context → re-fire)
- ✅ Async/concurrent execution lanes in timeline
- ✅ Protocol upgrade: HTTP JSON → gRPC (tonic)
- ✅ Storage upgrade: SQLite → RocksDB
- ✅ OpenTelemetry compatibility — import/export OTEL traces (works alongside existing infra)
- ✅ `orion replay`, `orion diff`, `orion watch` fully stable
- ❌ No AI yet
- ❌ No DB query lens yet
- ❌ No cluster yet

---

### Phase 3 — AI Intelligence + Deep Analysis (Months 12–16)
**Goal: Make Orionis the smartest tool in the developer's arsenal.**

| Month | Deliverable |
|---|---|
| Month 12 | AI trace summarizer — LLM compresses 2000-line trace into 4 bullet root-cause points |
| Month 13 | AI root cause suggester — cluster similar failures, detect recurrence + regression patterns |
| Month 14 | Database query lens — SQL queries + timing embedded in trace timeline, slow query highlighting |
| Month 15 | Diff mode — side-by-side visual execution diff across two traces or two releases |
| Month 16 | Production hardening — benchmark agent overhead across all 4 languages, full test suite |

**v0.3 Scope:**
- ✅ AI summarizer (pluggable LLM backend — local or cloud)
- ✅ AI root cause suggester with failure clustering
- ✅ DB query lens (SQL + timing + slow query detection in trace)
- ✅ `orion diff` — full visual execution diff between any two traces
- ✅ Benchmarked agent overhead: < 3% CPU in SAFE MODE
- ✅ Full automated test suite across all 4 agents
- ❌ No enterprise auth yet
- ❌ No multi-node cluster yet

---

### Phase 4 — Enterprise + Collaboration + Scale (Months 17–21)
**Goal: Make Orionis team-ready and enterprise-deployable at scale.**

| Month | Deliverable |
|---|---|
| Month 17 | Multi-node Orionis cluster — horizontal scaling for high-volume trace ingestion |
| Month 18 | ClickHouse backend — high-performance time-indexed analytics at scale |
| Month 19 | Enterprise auth — SSO/OIDC, role-based access control, team namespaces |
| Month 20 | Team collaboration — trace annotations, comments per frame, assign trace to dev, share link |
| Month 21 | CI/CD integration — GitHub Actions + GitLab CI plugin; auto-replay failed test traces from pipeline |

**v1.0 Scope:**
- ✅ Horizontally scalable multi-node engine cluster
- ✅ ClickHouse time-indexed analytics backend
- ✅ SSO / OIDC / RBAC enterprise auth
- ✅ Multi-tenant team dashboards (per-team trace isolation)
- ✅ Team annotations — comment, tag, assign per trace frame
- ✅ CI/CD replay — failed CI trace replay with `orion replay`
- ✅ On-prem Docker + Kubernetes deployment manifests
- ✅ Stable public API + protocol spec published
- ✅ Full documentation site

---

### Phase 5 — Final Product: Orionis v2.0 (Months 25–30)
**Goal: Deep state analysis, deterministic replay, and measurable trust.**

| Feature | Description |
|---|---|
| **Deterministic Replay Mode** | Freeze time, mock network/DB, isolate side-effects → Replay production failures with total fidelity in sandbox |
| **Execution Fingerprinting** | Structural hashing of execution graphs → cluster identical paths independent of variable values |
| **Span-Level State Heatmap**| Visual mutation intensity map: variable deltas, memory growth, object expansion per function |
| **Adaptive Sampling Engine** | Auto-escalate to FULL capture on anomaly; downgrade to SAFE under pressure |
| **Trace Integrity Score** | Reliability metric for each trace: completeness, ordering, dedupe confidence |
| **Failure Probability Estimator**| Statistical ranking of unstable modules & risky execution paths (non-LLM) |
| **Agent Self-Health Telemetry** | Agent buffer pressure, throttle state, dropped events visible in dashboard |
| **Timeline Compression Engine** | Structural graph compression to reduce storage pressure and accelerate replay |
| **Practical Runtime Security**| Detect abnormal control flow, privilege escalation paths, suspicious env mutations |
| **Developer Ergonomics CLI Layer**| `orion doctor`, `orion validate`, `orion benchmark`, `orion explain` |

**v2.0 — Deep Core State:**
- ✅ Deterministic time-travel debugging with side-effect mocking
- ✅ Execution graph fingerprinting & structural clustering
- ✅ State mutation heatmap visualization
- ✅ Adaptive sampling for high-load environments
- ✅ Trace integrity scoring & measurable trust
- ✅ Statistical failure probability modeling
- ✅ Agent self-telemetry & buffer visibility
- ✅ Timeline compression for scalable storage
- ✅ Runtime graph-based security detection
- ✅ Full CLI health & benchmarking suite
- ✅ Enterprise cluster remains stable & hardened

---

### Phase 5.5 — Intelligence Platform Expansion (Months 31–36)
**Goal: Expand Orionis into a complete developer intelligence platform, powered by the deterministic core.**

| Feature | Description |
|---|---|
| **AI Root Cause & Patch Suggestions** | AI analyzes trace + fingerprint → propose contextual code fixes |
| **Regression Detection Engine** | Auto-compare traces across releases → detect new failure paths |
| **Performance Optimization AI** | Analyze flamegraph + state heatmap → suggest algorithmic improvements |
| **Production Crash Auto-Replay** | Replay real production failure in isolated sandbox with deterministic inputs |
| **Failure Simulation Engine** | Inject failure states during replay to test resilience |
| **Memory Leak Detection** | Track heap allocation diffs across spans and traces |
| **Orionis Cloud** | Fully managed SaaS tier (Free / Pro / Enterprise) |
| **Plugin Ecosystem SDK** | Public SDK for analyzers, exporters, language agents, IDE integrations |
| **Collaboration Intelligence** | AI-assisted trace annotation, shared investigation context |

**v2.5 — Orionis Intelligence Platform:**
- ✅ All 7 language agents stable & backpressure-aware
- ✅ Deterministic replay + fingerprinting + clustering
- ✅ AI summarizer + root cause + patch suggestions
- ✅ Regression + performance + memory anomaly detection
- ✅ Failure simulation sandbox
- ✅ Adaptive sampling in production
- ✅ Enterprise cluster with ClickHouse + RBAC + SSO
- ✅ Open-source core (Apache 2.0) + Commercial Enterprise tier
- ✅ Orionis Cloud SaaS
- ✅ Plugin ecosystem & IDE integrations
- ✅ Competes with Datadog / Sentry / Jaeger — at deeper execution layer

---

### Phase 6 — True Resiliency & Scale (Months 37–42)
**Goal: Rebuild the core intelligence engine for high-cardinality, multi-release, hyper-scale environments.**

| Feature | Description |
|---|---|
| **Semantic Abstraction Layer** | Map raw function names to standardized categories (e.g., `DB_READ`, `AUTH`) for cross-service clustering |
| **Fuzzy Structural Similarity** | Replace exact hashes with Locality-Sensitive Hashing (LSH) and cosine distance for fuzzy matching |
| **Statistical Risk Scoring**| Base alerts on frequency, latency variance, and structural distance, eliminating false positive floods |
| **Dynamic Cluster Lifecycle** | Manage clusters through explicit stages (Candidate → Stable → Decaying) tracking stability |
| **Aggressive Edge Sampling**| Discard 99.9% of stable traces; escalate to capturing 100% of anomalies, retaining only statistical summaries |
| **Tiered Storage Model** | Store only representative traces per cluster and statistical metrics (P50/P99, error ratios) |
| **Adaptive Similarity Thresholding** | Dynamic cluster-specific similarity bands to prevent fragmentation over slight code variations |
| **Cluster Cardinality Governor** | Hard cap on cluster memory boundaries; entropy monitoring to fold cold clusters automatically |
| **Concept Drift Window Modeling** | Sliding time windows for behavioral drift to auto-accept gradual evolutionary software changes |
| **Deterministic Sampling Seeds** | Cluster-pinned seeds to ensure statistical integrity and zero blind spots during aggressive sampling |

**v3.0 — The Statistical Execution Engine:**
- ✅ Exact hashing replaced by Semantic Fuzzy Clustering + LSH Distance
- ✅ Adaptive Thresholding & Cardinality Governor prevent storage meltdown
- ✅ Concept Drift Windows eliminate alert fatigue over logical evolution
- ✅ Tiered statistical metrics storage in ClickHouse
- ✅ Aggressive Edge Sampling (99.9% drop) with Deterministic Seeds
- ✅ Deterministic trace replay strictly maintained below the probabilistic layer

---

## Complete Phase Summary

| Phase | Months | Version | Key Unlock |
|---|---|---|---|
| Phase 1 — Foundation | 1–6 | v0.1 | All 7 agents Day 1, CLI, UI, local storage |
| Phase 2 — Cross-Service | 7–11 | v0.2 | Cross-service graph, flamegraph, HTTP replay, gRPC |
| Phase 3 — AI Intelligence | 12–16 | v0.3 | AI summarizer + root cause, DB lens, diff mode |
| Phase 4 — Enterprise | 17–21 | v1.0 | Cluster, ClickHouse, SSO/RBAC, team collab |
| Phase 4.5 — Reliability | 22–24 | v1.1 | Durable WAL, Idempotency, Backpressure |
| Phase 5 — Core v2.0 | 25–30 | v2.0 | Deterministic Replay, State Heatmaps, Fingerprinting |
| Phase 5.5 — Intel v2.5 | 31–36 | v2.5 | AI Patches, Regression Engine, Cloud, Plugins |
| Phase 6 — Scale & Resiliency | 37–42 | v3.0 | Semantic Clustering, Adaptive Sampling, Tiered Storage |

---

## Long-Term Expansion (Post v2.5)

| Feature | Description |
|---|---|
| Orionis Cloud | Fully managed SaaS hosted version |
| Plugin marketplace | Community-built trace analyzers and exporters |
| Language agent: Kotlin/Scala | JVM ecosystem extension — Kotlin, Scala via existing Java agent base |
| Language agent: Ruby | `TracePoint` API-based tracing for Ruby/Rails |
| Language agent: PHP | `xdebug`-compatible hooks for PHP trace capture |
| WebAssembly agent | Trace WASM execution in browser and edge environments |
| IDE integrations | VS Code + JetBrains plugins for inline trace viewing |

---

## Monetization Strategy

| Tier | Model |
|---|---|
| Free | Local dev version (full open source) |
| Paid | Production cluster mode |
| Enterprise | Observability suite + SLA + on-prem license |
| Premium | AI-enhanced tier |

---

## Open-Source Strategy

- **License:** Apache 2.0 (enterprise-friendly, no copyleft restrictions)
- Make core engine clean and modular
- Publish protocol specification publicly
- Write strong documentation from v0.1
- Ship working demo early — developers contribute when:
  - The idea is clear
  - The architecture is clean
  - The code is readable

---

## Engineering Principles

```
Correctness > Performance (v1)
Simplicity > Scalability (v1)
Stability > Features (v1)
```

Do NOT build early:
- ❌ AI integration (Phase 3+)
- ❌ Distributed tracing (Phase 2+)
- ❌ Kubernetes / enterprise auth (Phase 4)
- ❌ Multi-tenant dashboards (Phase 4)
- ❌ Cloud / SaaS (Phase 5)

**Ship Phase 1 first. All 7 agents. The CLI. The UI. Correct and clean.**

---

## What Makes Orionis Elite Even at v0.1

Even without AI. Even without cross-service graphs.

If Orionis delivers at v0.1:
- ✅ All 7 language agents working on Day 1 (Python, Go, Rust, C++, C, Node.js, Java)
- ✅ `orion init` — zero friction setup, language auto-detected
- ✅ Panic/crash auto-caught with zero config — just `orion start`
- ✅ Environment snapshot at every trace start
- ✅ Full-process execution tree
- ✅ Variable snapshots per frame
- ✅ Time slider replay with step-back
- ✅ `orion` CLI that just works
- ✅ Auto-capture test failures from pytest / cargo test / go test / npm test / mvn test
- ✅ Export a `.otrace` file and share a bug with a teammate in seconds

**That is already something no existing dev tool provides in one package.**

That alone is rare. That alone is worth shipping.

---

## What We Are Not Forgetting (Also Planned)

Things confirmed for correct phases but not in Phase 1:

| Feature | Phase |
|---|---|
| `orion init` zero-friction setup | Phase 1 (Day 1) |
| Panic/crash zero-config auto-detection | Phase 1 (Day 1) |
| Environment snapshot at trace start | Phase 1 (Day 1) |
| OpenTelemetry import/export compatibility | Phase 2 |
| Async/concurrent execution lanes (goroutines, asyncio, Tokio, Node.js event loop) | Phase 2 |
| HTTP request replay | Phase 2 |
| Flamegraph overlay on execution tree | Phase 2 |
| AI trace summarizer + root cause | Phase 3 |
| Database query lens (SQL timing in trace) | Phase 3 |
| Diff mode — two executions side by side | Phase 3 |
| Durable Ingestion (WAL) & Idempotency | Phase 4.5 |
| Backpressure & Agent Circuit Breakers | Phase 4.5 |
| Real-time observability metrics (/metrics) | Phase 4.5 |
| `orion-loadgen` Benchmarking Tool | Phase 4.5 |
| Memory heap allocation tracking per span | Phase 5 |
| Deterministic Replay Mode (Mocking/Time-freeze) | Phase 5 |
| Adaptive Sampling Engine | Phase 5 |
| Trace Integrity Scoring | Phase 5 |
| Agent Self-Health Telemetry | Phase 5 |
| Execution Graph Fingerprinting | Phase 5 |
| AI code patch suggestions | Phase 5 |
| Practical Runtime Security | Phase 5 |
| Team annotations + trace sharing | Phase 4 |
| IDE plugins — VS Code + JetBrains | Post v2.0 |
| Kotlin/Scala/Ruby/PHP agents | Post v2.0 |
| WASM agent | Post v2.0 |

---

*Product name: Orionis | Plan version: 3.0 | Date: 2026-02-27 | Agents: Python, Go, Rust, C++, C, Node.js, Java*
