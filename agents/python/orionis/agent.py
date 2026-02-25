"""
Orionis Python Agent
====================
Global runtime tracing agent for Python applications.
Captures function enter/exit/exception events and streams them to the Orionis engine.

Usage:
    import orionis
    orionis.start(include_modules=["myapp"])

    # Or with the orion CLI:
    # orion start  →  then run your app normally
"""

import sys
import os
import time
import uuid
import json
import threading
import urllib.request
import traceback
import platform
import atexit
from typing import Optional, List, Dict, Any

# ── Configuration ─────────────────────────────────────────────────────────────

class Config:
    engine_url: str = "http://localhost:7700"
    include_modules: List[str] = []
    exclude_modules: List[str] = ["_pytest", "pytest", "orionis", "importlib", "encodings", "codecs", "abc"]
    mode: str = "dev"          # dev | safe | error
    max_string_len: int = 200
    max_list_len: int = 50
    max_depth: int = 2
    send_batch_size: int = 20
    send_interval_ms: int = 100

_cfg = Config()
_active = False
_lock = threading.Lock()
_batch: List[dict] = []
_sender_thread: Optional[threading.Thread] = None

# ── Environment Snapshot ──────────────────────────────────────────────────────

def _env_snapshot() -> dict:
    safe_vars = {k: v for k, v in os.environ.items()
                 if not any(s in k.upper() for s in ("SECRET", "TOKEN", "PASS", "KEY", "PWD"))}
    return {
        "os": platform.system(),
        "arch": platform.machine(),
        "hostname": platform.node(),
        "python_version": platform.python_version(),
        "env_vars": list(safe_vars.items())[:30],
        "working_dir": os.getcwd(),
        "captured_at": int(time.time() * 1000),
    }

# ── Variable Capture ──────────────────────────────────────────────────────────

def _serialize_value(v: Any, depth: int = 0) -> str:
    if depth >= _cfg.max_depth:
        return repr(v)[:_cfg.max_string_len]
    if isinstance(v, str):
        return v[:_cfg.max_string_len]
    if isinstance(v, (int, float, bool, type(None))):
        return repr(v)
    if isinstance(v, (list, tuple)):
        items = [_serialize_value(i, depth + 1) for i in v[:_cfg.max_list_len]]
        return f"[{', '.join(items)}]"
    if isinstance(v, dict):
        items = [f"{k!r}: {_serialize_value(val, depth+1)}" for k, val in list(v.items())[:_cfg.max_list_len]]
        return "{" + ", ".join(items) + "}"
    return repr(v)[:_cfg.max_string_len]


def _capture_locals(frame) -> List[dict]:
    try:
        result = []
        for name, value in frame.f_locals.items():
            if name.startswith("__"):
                continue
            result.append({
                "name": name,
                "value": _serialize_value(value),
                "type_name": type(value).__name__,
            })
        return result
    except Exception:
        return []

# ── Module Filter ─────────────────────────────────────────────────────────────

def _should_trace(frame) -> bool:
    filename = frame.f_code.co_filename or ""
    module = frame.f_globals.get("__name__", "") or ""

    # Always exclude standard library / third-party noise
    for exc in _cfg.exclude_modules:
        if module.startswith(exc) or exc in filename:
            return False

    # If include list is set, only trace matching modules/paths
    if _cfg.include_modules:
        cwd = os.getcwd()
        in_project = filename.startswith(cwd)
        in_module = any(module.startswith(m) for m in _cfg.include_modules)
        return in_project or in_module

    # Default: trace anything in the current working directory
    return os.path.abspath(filename).lower().startswith(os.getcwd().lower())

# ── Trace Context ─────────────────────────────────────────────────────────────

_thread_local = threading.local()

def _get_trace_id() -> str:
    if not hasattr(_thread_local, "trace_id") or _thread_local.trace_id is None:
        _thread_local.trace_id = str(uuid.uuid4())
        _thread_local.span_stack = []
    return _thread_local.trace_id

def _push_span(span_id: str):
    if not hasattr(_thread_local, "span_stack"):
        _thread_local.span_stack = []
    _thread_local.span_stack.append(span_id)

def _pop_span():
    if hasattr(_thread_local, "span_stack") and _thread_local.span_stack:
        _thread_local.span_stack.pop()

def _current_parent() -> Optional[str]:
    if hasattr(_thread_local, "span_stack") and _thread_local.span_stack:
        return _thread_local.span_stack[-1]
    return None

# ── Event Builder ─────────────────────────────────────────────────────────────

def _make_event(event_type: str, frame, error_msg: Optional[str] = None, duration_us: Optional[int] = None) -> dict:
    span_id = str(uuid.uuid4())
    capture_locals = event_type in ("function_enter", "exception") and _cfg.mode == "dev"

    return {
        "trace_id": _get_trace_id(),
        "span_id": span_id,
        "parent_span_id": _current_parent(),
        "timestamp_ms": int(time.time() * 1000),
        "event_type": event_type,
        "function_name": frame.f_code.co_name,
        "module": frame.f_globals.get("__name__", ""),
        "file": frame.f_code.co_filename,
        "line": frame.f_lineno,
        "locals": _capture_locals(frame) if capture_locals else None,
        "error_message": error_msg,
        "duration_us": duration_us,
        "language": "python",
    }

# ── Profile Hook ─────────────────────────────────────────────────────────────

_enter_times: Dict[str, int] = {}

def _profile_hook(frame, event, arg):
    if not _active:
        return
    if not _should_trace(frame):
        return

    if event == "call":
        ev = _make_event("function_enter", frame)
        _push_span(ev["span_id"])
        _enter_times[ev["span_id"]] = time.monotonic_ns()
        _enqueue(ev)

    elif event == "return":
        span_id = _current_parent()
        duration = None
        if span_id and span_id in _enter_times:
            duration = (time.monotonic_ns() - _enter_times.pop(span_id)) // 1000
        ev = _make_event("function_exit", frame, duration_us=duration)
        _pop_span()
        _enqueue(ev)

    elif event == "exception":
        exc_type, exc_val, _ = arg
        msg = f"{exc_type.__name__}: {exc_val}" if exc_type else str(exc_val)
        ev = _make_event("exception", frame, error_msg=msg)
        _enqueue(ev)

# ── Batch Sender ──────────────────────────────────────────────────────────────

def _enqueue(event: dict):
    with _lock:
        _batch.append(event)

def _sender_loop():
    while _active:
        time.sleep(_cfg.send_interval_ms / 1000)
        _flush()
    _flush()  # final flush on stop

def _flush():
    with _lock:
        if not _batch:
            return
        to_send = _batch.copy()
        _batch.clear()

    print(f"[Orionis DEBUG] Flushing {len(to_send)} events")
    payload = json.dumps(to_send).encode()
    try:
        req = urllib.request.Request(
            f"{_cfg.engine_url}/api/ingest",
            data=payload,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        res = urllib.request.urlopen(req, timeout=3)
        print(f"[Orionis DEBUG] Engine returned HTTP {res.getcode()}")
    except Exception as e:
        print(f"[Orionis] Error sending payload: {e}")

# ── Panic / Exception Handler ─────────────────────────────────────────────────

def _install_excepthook():
    original = sys.excepthook
    def orionis_excepthook(exc_type, exc_val, exc_tb):
        try:
            tb = traceback.extract_tb(exc_tb)
            if tb:
                frame_info = tb[-1]
                event = {
                    "trace_id": _get_trace_id(),
                    "span_id": str(uuid.uuid4()),
                    "parent_span_id": None,
                    "timestamp_ms": int(time.time() * 1000),
                    "event_type": "exception",
                    "function_name": frame_info.name,
                    "module": "__main__",
                    "file": frame_info.filename,
                    "line": frame_info.lineno,
                    "locals": None,
                    "error_message": f"{exc_type.__name__}: {exc_val}",
                    "duration_us": None,
                    "language": "python",
                }
                _enqueue(event)
                _flush()
        except Exception:
            pass
        original(exc_type, exc_val, exc_tb)
    sys.excepthook = orionis_excepthook

# ── Public API ────────────────────────────────────────────────────────────────

def start(
    include_modules: List[str] = [],
    exclude_modules: List[str] = [],
    mode: str = "dev",
    engine_url: str = "http://localhost:7700",
):
    """
    Start the Orionis global tracing agent.

    Args:
        include_modules: Only trace these module prefixes (e.g. ["myapp"])
        exclude_modules: Always exclude these module prefixes
        mode: "dev" (full capture) | "safe" (no locals) | "error" (only on exception)
        engine_url: URL of the running Orionis engine
    """
    global _active, _sender_thread

    _cfg.include_modules = include_modules
    _cfg.exclude_modules = _cfg.exclude_modules + exclude_modules
    _cfg.mode = mode
    _cfg.engine_url = engine_url

    # Send environment snapshot
    try:
        snap = json.dumps({"env_snapshot": _env_snapshot()}).encode()
        req = urllib.request.Request(
            f"{engine_url}/api/ingest",
            data=snap,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        urllib.request.urlopen(req, timeout=2)
    except Exception:
        pass

    _active = True
    _install_excepthook()
    sys.setprofile(_profile_hook)
    threading.setprofile(_profile_hook)

    _sender_thread = threading.Thread(target=_sender_loop, daemon=True, name="orionis-sender")
    _sender_thread.start()

    atexit.register(stop)
    print(f"[Orionis] Agent started — engine: {engine_url} | mode: {mode}")


def stop():
    """Stop the Orionis tracing agent and flush remaining events."""
    global _active
    _active = False
    sys.setprofile(None)
    threading.setprofile(None)
    _flush()
    if _sender_thread:
        _sender_thread.join(timeout=1)
    print("[Orionis] Agent stopped.")


def reset_trace():
    """Start a new trace context (new trace_id) for the current thread."""
    _thread_local.trace_id = str(uuid.uuid4())
    _thread_local.span_stack = []
