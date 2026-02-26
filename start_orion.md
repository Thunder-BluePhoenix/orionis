# 🚀 Orionis: Start-to-Finish Guide

Welcome to Orionis! This guide will walk you through setting up the engine and instrumenting your first multi-language application.

---

## 1. Launch the Engine

The Engine is the central brain that collects, stores, and analyzes your traces.

1. Navigate to the engine directory:
   ```bash
   cd orionis-engine
   ```
2. Build and run:
   ```bash
   cargo run --release
   ```
   *The Engine will now be listening on port **7700** (HTTP) and **7701** (gRPC).*

---

## 2. Instrumenting Python

The Python agent provides global tracing for any Flask, Django, or generic Python application.

1. **Install the Agent:**
   Ensure the `agents/python/orionis` folder is in your Python path.

2. **Instrument your code:**
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

3. **Enable Data Propagation:**
   Wrap your app with the middleware to stitch incoming traces from other services:
   ```python
   app.wsgi_app = orionis.WSGIMiddleware(app.wsgi_app)
   ```

---

## 3. Instrumenting Go

The Go agent is designed for high-performance manual and semi-automatic tracing.

1. **Install:**
   Add the `orionis` package to your project.

2. **Initialize:**
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

3. **Trace Functions:**
   ```go
   func ProcessOrder() {
       defer orionis.Trace()() // Automatically captures entry, exit, and timing
       // ... your logic
   }
   ```

4. **Outgoing Requests:**
   Use the `TraceRoundTripper` to automatically inject trace headers:
   ```go
   client := &http.Client{
       Transport: &orionis.TraceRoundTripper{},
   }
   ```

---

## 4. Using the Dashboard

Open your browser to `http://localhost:7700` (or open `dashboard/index.html`).

### 📊 Timeline View
Click any trace in the sidebar to see the full execution flow. Events like SQL queries and Exceptions are highlighted.

### 🤖 AI Insight
If a trace is active, click the **"Connect AI"** toggle and look for the **AI Insight** panel. It will automatically explain what happened and suggest fixes for failures.

### 📁 Failure Clusters
Go to the **"Failure Clusters"** tab. Orionis groups identical failure patterns together so you can see which bug is impacting your users the most.

### 🌗 Visual Diff
1. View a "Bad" trace.
2. Click **"Compare"** on a "Good" trace in the list.
3. The UI will highlight the **Point of Departure** (the exact moment the execution diverged) in red.

---

## 💡 Pro-Tips
- **Mode Toggle:** Use `mode="error"` in production to only capture traces that fail, saving storage and performance.
- **SQL Capture:** Always use `capture_query` for critical DB operations to get the syntax-highlighted lens in the UI.
