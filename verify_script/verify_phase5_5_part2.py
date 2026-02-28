import requests
import time
import uuid

API_URL = "http://127.0.0.1:7705/api"

def test_intel_part2():
    print("🚀 Starting Phase 5.5 Part 2 Intelligence Verification...")

    tenant_id = f"tenant_{uuid.uuid4().hex[:8]}"
    print(f"\n[1] 📦 Creating isolated SaaS tenant... ({tenant_id})")

    # 1. Test Memory Leak Detection
    print("\n🧠 Testing Memory Leak Detection AI...")
    trace_id = str(uuid.uuid4())
    headers = {"X-Orionis-Tenant": tenant_id}
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
            "memory_usage_bytes": 1024,
            "version": "1.2.0"
        },
        {
            "trace_id": trace_id,
            "span_id": str(uuid.uuid4()),
            "parent_span_id": None,
            "timestamp_ms": int(time.time() * 1000) + 50,
            "event_type": "function_exit",
            "function_name": "main",
            "module": "app",
            "file": "main.py",
            "line": 100,
            "language": "python",
            "memory_usage_bytes": 15_000_000, # 15 MB retained (Leak!)
            "version": "1.2.0"
        }
    ]
    r = requests.post(f"{API_URL}/ingest", json=events, headers=headers)
    if r.status_code == 200:
        print(f"   Sent trace: {trace_id}")
    else:
        print(f"   [Error] Ingest failed: {r.status_code}")

    r = requests.get(f"{API_URL}/intel/memory-leaks/{trace_id}", headers=headers)
    if r.status_code == 200:
        leak_data = r.json()
        print(f"   AI Status: {leak_data['status']}")
        print(f"   Total Retained: {leak_data['total_abnormal_retention_bytes']} bytes")
        if leak_data['leaking_spans']:
            print(f"   Suggestion: {leak_data['leaking_spans'][0]['suggestion']}")
    else:
        print(f"   [Error] Memory API failed: {r.status_code}")

    # 2. Test Sandbox Replay Package
    print("\n📦 Testing Sandbox Replay Extraction...")
    r = requests.get(f"{API_URL}/intel/sandbox-replay/{trace_id}", headers=headers)
    if r.status_code == 200:
        replay_data = r.json()
        print(f"   Replay Ticket Generated: {replay_data['replay_ticket']}")
        print(f"   Instructions: {replay_data['instructions']}")
        print(f"   Events Packaged: {replay_data['event_count']}")
    else:
        print(f"   [Error] Sandbox API failed: {r.status_code}")

    # 3. Test Failure Simulation Engine
    print("\n🔥 Testing Failure Simulation Rules...")
    sim_rule = {
        "rule_id": str(uuid.uuid4()),
        "target_function": "process_payment",
        "target_module": "billing",
        "action": "exception",
        "error_payload": "payment_gateway_timeout",
        "delay_ms": 2000,
        "active": True
    }
    r = requests.post(f"{API_URL}/intel/simulate-failure", json=sim_rule, headers=headers)
    if r.status_code == 200:
        print(f"   Simulation Rule Saved: {r.json()['rule_id']}")
    else:
        print(f"   [Error] Simulation Rule API failed: {r.status_code} - {r.text}")

    # 4. Test Plugin Webhook Registration
    print("\n🔌 Testing Plugin SDK Registration...")
    plugin = {
        "plugin_id": str(uuid.uuid4()),
        "name": "PagerDuty Integration",
        "webhook_url": "https://events.pagerduty.com/v2/enqueue",
        "subscribed_events": ["security_alert"],
        "active": True
    }
    r = requests.post(f"{API_URL}/plugins/register", json=plugin, headers=headers)
    if r.status_code == 200:
        print(f"   Plugin Registered successfully: {r.json()['plugin_id']}")
    else:
        print(f"   [Error] Plugin API failed: {r.status_code} - {r.text}")

    # 5. Test SaaS Tenant Quotas
    print("\n☁️ Testing SaaS Tenant Tier Quotas (Free Tier = 1000/day)...")
    print("   Simulating 1,001 traces to trigger HTTP 429 Too Many Requests...")
    
    # We already sent 1 trace. Let's blast 1000 more (we just need the status code).
    # To avoid overwhelming the tiny script/server test setup, we'll send a bulk array,
    # but since the quota increments per request in our naive implementation,
    # we'll loop until we hit 429. (Assuming `increment_tenant_usage` bumps by 1 or block bumps).
    # Wait, our code increments by 1 per *batch* request. Let's send 1001 empty/small batches.
    
    hit_429 = False
    for i in range(1005):
        mini_event = [{
            "trace_id": str(uuid.uuid4()),
            "span_id": str(uuid.uuid4()),
            "timestamp_ms": int(time.time() * 1000),
            "event_type": "function_enter",
            "function_name": "noop",
            "version": "1.0"
        }]
        res = requests.post(f"{API_URL}/ingest", json=mini_event, headers=headers)
        if res.status_code == 429:
            print(f"   ✅ Quota Exceeded! HTTP 429 triggered at request count {i + 2}.")
            print(f"   Message: {res.text}")
            hit_429 = True
            break
            
    if not hit_429:
        print("   [Error] Did not receive HTTP 429 as expected.")

    print("\n✅ Phase 5.5 Part 2 Verification Complete!")

if __name__ == "__main__":
    test_intel_part2()
