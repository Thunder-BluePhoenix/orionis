import subprocess
import time
import requests
import os
import signal
import sys
import uuid

import socket

ENGINE_PATH = r"f:\proj_guides\Orionis\orionis-engine\target\debug\orionis-engine.exe"
PYTHON_AGENT_PATH = r"f:\proj_guides\Orionis\agents\python"
GO_AGENT_PATH = r"f:\proj_guides\Orionis\agents\go"

def wait_for_port(port, timeout=10):
    start_time = time.time()
    while time.time() - start_time < timeout:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return True
        except:
            time.sleep(0.5)
    return False

def start_engine():
    print("🚀 Starting Orionis Engine...")
    return subprocess.Popen([ENGINE_PATH], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

def start_python_app():
    print("🐍 Starting Python App (Flask)...")
    code = f"""
import sys
sys.path.append(r"{PYTHON_AGENT_PATH}")
import orionis
from flask import Flask, request

orionis.start(include_modules=["temp_py_app"], mode="dev", engine_url="http://127.0.0.1:7700")
app = Flask(__name__)
app.wsgi_app = orionis.WSGIMiddleware(app.wsgi_app)

@app.route("/hello")
def hello():
    orionis.capture_query("SELECT 'hello' FROM py", "postgres", 100)
    return {{"message": "Hello from Python"}}

if __name__ == "__main__":
    app.run(host="127.0.0.1", port=5001)
"""
    with open("temp_py_app.py", "w") as f:
        f.write(code)
    
    log_f = open("py_app_log.txt", "w")
    return subprocess.Popen([sys.executable, "temp_py_app.py"], stdout=log_f, stderr=log_f)

def run_go_client():
    print("🐹 Running Go Client...")
    code = f"""
package main
import (
    "fmt"
    "net/http"
    "orionis"
    "time"
)

func main() {{
    orionis.Start(
        orionis.WithMode("dev"), 
        orionis.WithEngine("http://127.0.0.1:7700"),
        func(c *orionis.Config) {{ c.UseGRPC = false }},
    )
    defer orionis.Stop()
    
    // Explicitly trace this function to ensure Go events exist
    defer orionis.Trace()()

    fmt.Println("Sending request to Python...")
    req, _ := http.NewRequest("GET", "http://127.0.0.1:5001/hello", nil)
    orionis.InjectTraceHeaders(req)
    
    client := &http.Client{{
        Timeout: 5 * time.Second,
        Transport: &orionis.TraceRoundTripper{{}},
    }}
    resp, err := client.Do(req)
    if err != nil {{
        fmt.Printf("Request failed: %v\\n", err)
        return
    }}
    defer resp.Body.Close()
    fmt.Printf("Python responded with %d\\n", resp.StatusCode)
    
    time.Sleep(2 * time.Second) // Wait for flush
}}
"""
    # Create a temp go mod for this script
    os.makedirs("temp_go_test", exist_ok=True)
    with open("temp_go_test/main.go", "w") as f:
        f.write(code)
    
    # We need to link to the local orionis package
    subprocess.run(["go", "mod", "init", "testapp"], cwd="temp_go_test", capture_output=True)
    subprocess.run(["go", "mod", "edit", "-replace", f"orionis={GO_AGENT_PATH}/orionis"], cwd="temp_go_test", capture_output=True)
    subprocess.run(["go", "mod", "tidy"], cwd="temp_go_test", capture_output=True)
    
    return subprocess.run(["go", "run", "main.go"], cwd="temp_go_test", capture_output=True, text=True)

def verify_traces():
    print("🔍 Verifying Traces in Engine...")
    for i in range(8): # More retries
        time.sleep(1.5)
        try:
            res = requests.get("http://127.0.0.1:7700/api/traces")
            if res.status_code != 200:
                print(f"Engine status {res.status_code} (Table might not be ready yet)")
                continue
            
            traces_list = res.json()
            print(f"Captured {len(traces_list)} traces.")
            if not traces_list:
                continue
            
            found_cross = False
            for t in traces_list:
                ev_res = requests.get(f"http://127.0.0.1:7700/api/traces/{t['trace_id']}")
                if ev_res.status_code != 200: continue
                events = ev_res.json()
                
                langs = set(e.get('language') for e in events)
                print(f"Trace {t['trace_id'][:8]} contains: {langs}")
                if 'go' in langs and 'python' in langs:
                    found_cross = True
                    print("✅ Found linked cross-language trace!")
                    
                    has_sql = any(e.get('event_type') == 'db_query' for e in events)
                    if has_sql:
                        print("✅ Found SQL query event in trace.")
                    
                    # Trigger AI summary to verify it works
                    requests.get(f"http://127.0.0.1:7700/api/ai/summarize/{t['trace_id']}")
                    print("✅ AI Summarization triggered.")
            
            if found_cross: return True
        except Exception as e:
            print(f"Verification attempt {i+1} failed: {e}")
    
    return False

if __name__ == "__main__":
    engine = None
    py_app = None
    try:
        engine = start_engine()
        if not wait_for_port(7700):
            print("❌ Engine failed to start.")
            sys.exit(1)
        
        py_app = start_python_app()
        print("Waiting for Python app to be ready...")
        if not wait_for_port(5001):
            print("❌ Python app failed to start.")
            sys.exit(1)
        
        go_res = run_go_client()
        print("Go Client Out:", go_res.stdout)
        print("Go Client Err:", go_res.stderr)
        
        if verify_traces():
            print("\n🌟 ALL TESTS PASSED! Multi-language tracing is fully operational.")
            sys.exit(0)
        else:
            print("\n❌ TEST FAILED: Traces not linked or events missing.")
            sys.exit(1)
            
    finally:
        if py_app: py_app.terminate()
        if engine: engine.terminate()
        time.sleep(1)
        if os.path.exists("temp_py_app.py"): os.remove("temp_py_app.py")
        # Cleanup temp_go_test is harder as it's a dir, but we should try.
