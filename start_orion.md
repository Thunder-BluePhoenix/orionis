# Orionis 🌌: Start-to-Finish Guide

Welcome to Orionis! This guide will walk you through setting up the engine and instrumenting your first multi-language application.

---

## 1. Launch the Engine

The Engine is the central brain that collects, stores, and analyzes your traces.

1. **Navigate to the engine directory:**
   ```bash
   cd orionis-engine
   ```
2. **Build and run:**
   ```bash
   cargo run --release
   ```
   *The Engine will now be listening on port **7700** (HTTP) and **7701** (gRPC).*

---

## 2. Instrumenting Python

The Python agent provides global tracing for any Flask, Django, or generic Python application.

1. **Instrument your code:**
   ```python
   import orionis

   # Start global tracing
   orionis.start(
       include_modules=["myapp"],
       engine_url="http://localhost:7700"
   )

   @app.route("/")
   def home():
       # Manual SQL capture
       orionis.capture_query("SELECT * FROM users", "postgres", 1500)
       return "Hello World"
   ```

2. **Middleware for trace propagation:**
   ```python
   app.wsgi_app = orionis.WSGIMiddleware(app.wsgi_app)
   ```

---

## 3. Instrumenting Go

The Go agent is designed for high-performance manual and semi-automated tracing.

1. **Initialize:**
   ```go
   import "orionis"

   func main() {
       orionis.Start(
           orionis.WithMode("dev"),
           orionis.WithEngine("http://localhost:7700"),
       )
       defer orionis.Stop()
   }
   ```

2. **Trace Functions:**
   ```go
   func ProcessOrder() {
       defer orionis.Trace()() // Automatically captures entry, exit, and timing
   }
   ```

---

## 4. Instrumenting Rust

The Rust agent uses `OnceLock` for global management and thread-locals for trace context.

1. **Start:**
   ```rust
   orionis::start("http://localhost:7700");
   ```

2. **Trace with Macros:**
   ```rust
   #[orionis::orion_trace]
   fn some_work() {
       // ... logic
   }
   ```

---

## 5. Instrumenting C, Java, and Node.js

### C (Pure C11)
Include `orionis.h` and use the `ORIONIS_TRACE()` macro.
```c
#include "orionis.h"
orionis_start("http://localhost:7700");
ORIONIS_TRACE();
```

### Java (JVM SDK)
Use the `Orionis.Span` try-with-resources.
```java
try (Orionis.Span span = new Orionis.Span("myFunc", "file.java", 10)) {
    // code
}
```

### Node.js (AsyncLocalStorage)
Wrap functions with `orionis.trace()`.
```javascript
const orionis = require('./orionis');
orionis.trace('myTask', async () => {
    // automagically propagates trace context
});
```

---

## 6. Using the Dashboard

Open your browser to [http://localhost:7700](http://localhost:7700).

- **Timeline View:** Click any trace to see the full execution flow.
- **AI Insight:** Look for the **AI Insight** panel on failing traces.
- **Failure Clusters:** View grouped identical failure patterns in the sidebar.
- **Visual Diff:** Click **Compare** on a "Good" trace after viewing a "Bad" one.

---

## ⚖️ Pro-Tips
- **Trace Propagation:** Ensure all services use the provided `inject/extract` helpers to maintain trace continuity across language boundaries.
- **SQL Capture:** Always use the specialized query capture for deep inspection.
