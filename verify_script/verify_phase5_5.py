import requests
import time
import uuid

API_URL = "http://localhost:7700/api"

def send_trace(version, name, has_error=False, deviant=False):
    trace_id = str(uuid.uuid4())
    events = [
        {
            "trace_id": trace_id,
            "span_id": str(uuid.uuid4()),
            "parent_span_id": None,
            "timestamp_ms": int(time.time() * 1000),
            "event_type": "function_enter",
            "function_name": "main",
            "module": "app",
            "file": "main.py",
            "line": 10,
            "language": "python",
            "version": version
        }
    ]
    
    if deviant:
        events.append({
            "trace_id": trace_id,
            "span_id": str(uuid.uuid4()),
            "parent_span_id": events[0]["span_id"],
            "timestamp_ms": int(time.time() * 1000) + 10,
            "event_type": "function_enter",
            "function_name": "malicious_hook",
            "module": "app",
            "file": "main.py",
            "line": 50,
            "language": "python",
            "version": version
        })
    
    if has_error:
        events.append({
            "trace_id": trace_id,
            "span_id": str(uuid.uuid4()),
            "parent_span_id": events[-1]["span_id"],
            "timestamp_ms": int(time.time() * 1000) + 20,
            "event_type": "exception",
            "function_name": "process",
            "module": "app",
            "file": "main.py",
            "line": 100,
            "language": "python",
            "error_message": "Segmentation fault at 0xdeadbeef",
            "version": version
        })

    r = requests.post(f"{API_URL}/ingest", json=events)
    if r.status_code != 200:
        print(f"   [Error] Ingest failed: {r.status_code} - {r.text}")
    return trace_id, r.status_code

def test_intel_features():
    print("🚀 Starting Phase 5.5 Intelligence Verification...")
    
    # 1. Ingest Version 1.0.0 (Baseline)
    print("📦 Building baseline for v1.0.0...")
    for _ in range(5):
        send_trace("1.0.0", "normal_run")
    
    # 2. Ingest Version 1.1.0 (Regression)
    print("📦 Ingesting v1.1.0 with a new execution path...")
    tid, status = send_trace("1.1.0", "regression_run", deviant=True)
    print(f"   Sent deviant trace in v1.1.0: {tid} (Status: {status})")

    # 3. Check Regressions API
    print("🔍 Verifying Regression Detection API...")
    r = requests.get(f"{API_URL}/intel/regressions")
    regressions = r.json()
    print(f"   Detected regressions: {len(regressions)}")
    for reg in regressions:
        print(f"   - {reg['from_version']} -> {reg['to_version']}: {reg['new_execution_paths']} new paths")

    # 4. Trigger Failure and Check AI Root Cause
    print("🔥 Triggering failure for AI Analysis...")
    fail_tid, _ = send_trace("1.1.0", "failing_run", has_error=True)
    print(f"   Sent failing trace: {fail_tid}")
    
    time.sleep(2) # Wait for ingestion/summarization
    
    print("🧠 Verifying AI Root Cause API...")
    r = requests.post(f"{API_URL}/intel/root-cause/{fail_tid}")
    if r.status_code != 200:
        print(f"   [Error] AI Root Cause failed: {r.status_code} - {r.text}")
        return
    analysis = r.json()
    print(f"   AI Root Cause: {analysis['root_cause']}")
    if analysis.get('suggested_patch'):
        print(f"   AI Patch Suggestion: {analysis['suggested_patch']}")

    # 5. Check Performance API
    print("⚡ Verifying Performance Optimization API...")
    r = requests.get(f"{API_URL}/intel/performance/{fail_tid}")
    perf = r.json()
    print(f"   AI Perf Recommendation: {perf['overall_recommendation']}")

    print("✅ Phase 5.5 Intelligence Verification Complete!")

if __name__ == "__main__":
    test_intel_features()
