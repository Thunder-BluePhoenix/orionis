"""
orion — Orionis CLI
====================
The command-line interface for the Orionis Runtime Intelligence Engine.

Commands:
    orion init          Auto-detect language, scaffold agent config
    orion start         Start the Orionis engine daemon
    orion stop          Stop the running engine
    orion status        Show engine status and active traces
    orion list          List all captured traces
    orion replay <id>   Open a trace in the dashboard
    orion export <id>   Export a trace to .otrace file
    orion import <file> Import a .otrace file into the engine
    orion diff <a> <b>  Open two traces in side-by-side diff view
    orion watch         Watch for file changes and re-run on change
    orion clean         Delete all stored traces
    orion config        Show current configuration
"""

import sys
import os
import json
import time
import signal
import platform
import subprocess
import urllib.request
import urllib.error
import webbrowser
from pathlib import Path

# ── Config ────────────────────────────────────────────────────────────────────

def _find_engine_bin():
    """Locate the orionis-engine binary relative to orion CLI location."""
    base = Path(__file__).parent.parent
    candidates = [
        base / "orionis-engine" / "target" / "release" / "orionis-engine",
        base / "orionis-engine" / "target" / "release" / "orionis-engine.exe",
        base / "orionis-engine" / "target" / "debug"   / "orionis-engine",
        base / "orionis-engine" / "target" / "debug"   / "orionis-engine.exe",
    ]
    for c in candidates:
        if c.exists():
            return str(c)
    return "orionis-engine"

ENGINE_URL   = os.environ.get("ORIONIS_URL", "http://localhost:7700")
ENGINE_BIN   = os.environ.get("ORIONIS_BIN",  _find_engine_bin())
PID_FILE     = Path.home() / ".orionis" / "engine.pid"
CONFIG_FILE  = Path.home() / ".orionis" / "config.json"

CYAN  = "\033[96m"
GREEN = "\033[92m"
RED   = "\033[91m"
YELLOW= "\033[93m"
BOLD  = "\033[1m"
RESET = "\033[0m"

# ── Helpers ───────────────────────────────────────────────────────────────────

def _req(method: str, path: str, data: bytes = None):
    url = ENGINE_URL + path
    req = urllib.request.Request(url, data=data, method=method)
    if data:
        req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return json.loads(r.read()) if r.headers.get("Content-Type", "").startswith("application/json") else r.read()
    except urllib.error.URLError:
        return None

def _engine_alive() -> bool:
    return _req("GET", "/api/traces") is not None

def _print_banner():
    print(f"\n{BOLD}{CYAN} ORIONIS — Runtime Intelligence Engine — v0.1.0{RESET}\n")

def _ok(msg):   print(f"{GREEN}✓{RESET}  {msg}")
def _err(msg):  print(f"{RED}✕{RESET}  {msg}")
def _info(msg): print(f"{CYAN}→{RESET}  {msg}")
def _warn(msg): print(f"{YELLOW}!{RESET}  {msg}")

# ── Commands ──────────────────────────────────────────────────────────────────

def cmd_init(_args):
    """Detect project language and scaffold an orionis config."""
    cwd = Path.cwd()
    _print_banner()
    _info(f"Scanning project at: {cwd}")

    detected = []
    if list(cwd.glob("*.py")) or (cwd / "pyproject.toml").exists() or (cwd / "setup.py").exists():
        detected.append("python")
    if (cwd / "go.mod").exists():
        detected.append("go")
    if (cwd / "Cargo.toml").exists():
        detected.append("rust")
    if list(cwd.glob("*.cpp")) or list(cwd.glob("*.cc")):
        detected.append("cpp")

    if not detected:
        _warn("Could not detect language — creating generic config.")
        detected = ["python"]

    config = {
        "engine_url": ENGINE_URL,
        "languages": detected,
        "mode": "dev",
        "include_modules": [],
        "exclude_modules": [],
    }
    cfg_path = cwd / ".orionis.json"
    cfg_path.write_text(json.dumps(config, indent=2))

    _ok(f"Detected: {', '.join(detected)}")
    _ok(f"Config written: {cfg_path}")
    print()
    print("  Next steps:")
    if "python" in detected:
        print(f"    {CYAN}import orionis{RESET}")
        print(f"    {CYAN}orionis.start(){RESET}")
    if "go" in detected:
        print(f"    {CYAN}orionis.Start(){RESET}  // in your main()")
    if "rust" in detected:
        print(f"    {CYAN}orionis_agent::start(\"http://localhost:7700\");{RESET}")
    if "cpp" in detected:
        print(f"    {CYAN}#include \"orionis.hpp\"{RESET}")
        print(f"    {CYAN}orionis::start();  // in main(){RESET}")
    print()
    print(f"  Then run:  {CYAN}orion start{RESET}")


def cmd_start(_args):
    """Start the Orionis engine daemon."""
    if _engine_alive():
        _warn("Engine is already running.")
        _info(f"Dashboard: {ENGINE_URL}")
        return

    if not ENGINE_BIN or not Path(ENGINE_BIN).exists():
        _err(f"Engine binary not found: {ENGINE_BIN}")
        _info("Build it with:  cargo build --release  (in orionis-engine/)")
        sys.exit(1)

    PID_FILE.parent.mkdir(parents=True, exist_ok=True)
    proc = subprocess.Popen(
        [ENGINE_BIN],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    PID_FILE.write_text(str(proc.pid))

    # Wait for engine to come up
    for _ in range(20):
        time.sleep(0.3)
        if _engine_alive():
            _ok(f"Orionis engine started (PID {proc.pid})")
            _ok(f"Dashboard: {ENGINE_URL}")
            webbrowser.open(ENGINE_URL)
            return

    _err("Engine did not start in time. Check logs.")
    sys.exit(1)


def cmd_stop(_args):
    """Stop the running Orionis engine."""
    if not PID_FILE.exists():
        _warn("No engine PID found — is it running?")
        return
    pid = int(PID_FILE.read_text().strip())
    try:
        if platform.system() == "Windows":
            subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        else:
            os.kill(pid, signal.SIGTERM)
        PID_FILE.unlink(missing_ok=True)
        _ok(f"Engine stopped (PID {pid})")
    except ProcessLookupError:
        _warn("Process not found — it may have already stopped.")
        PID_FILE.unlink(missing_ok=True)


def cmd_status(_args):
    """Show engine status and trace count."""
    _print_banner()
    if not _engine_alive():
        _err("Engine is NOT running.")
        _info("Start it with:  orion start")
        return

    traces = _req("GET", "/api/traces") or []
    _ok(f"Engine running at {ENGINE_URL}")
    _info(f"Traces stored: {len(traces)}")
    errors = sum(1 for t in traces if t.get("has_error"))
    if errors:
        _warn(f"Traces with errors: {errors}")


def cmd_list(_args):
    """List all captured traces."""
    traces = _req("GET", "/api/traces")
    if traces is None:
        _err("Cannot reach engine. Run:  orion start")
        return
    if not traces:
        _info("No traces yet. Instrument your app and run it.")
        return

    print(f"\n{'ID':<38} {'Function':<30} {'Lang':<8} {'Events':<8} {'Error'}")
    print("─" * 90)
    for t in traces[:50]:
        tid   = t.get("trace_id", "?")[:36]
        name  = t.get("name", "?")[:28]
        lang  = t.get("language", "?")
        evts  = t.get("event_count", 0)
        err   = f"{RED}✕{RESET}" if t.get("has_error") else f"{GREEN}✓{RESET}"
        print(f"{tid:<38} {name:<30} {lang:<8} {evts:<8} {err}")
    print()


def cmd_replay(args):
    """Open a trace in the dashboard for time-travel replay."""
    if not args:
        _err("Usage: orion replay <trace-id>")
        sys.exit(1)
    trace_id = args[0]
    url = f"{ENGINE_URL}?trace={trace_id}"
    _ok(f"Opening trace {trace_id[:8]}… in browser")
    webbrowser.open(url)


def cmd_export(args):
    """Export a trace to a .otrace file."""
    if not args:
        _err("Usage: orion export <trace-id>")
        sys.exit(1)
    trace_id = args[0]
    events = _req("GET", f"/api/traces/{trace_id}")
    if not events:
        _err(f"Trace {trace_id} not found or engine not running.")
        sys.exit(1)
    out = {
        "version": "0.1",
        "trace_id": trace_id,
        "exported_at": int(time.time() * 1000),
        "events": events,
    }
    fname = f"{trace_id[:8]}.otrace"
    Path(fname).write_text(json.dumps(out, indent=2))
    _ok(f"Exported to {fname}  ({len(events)} events)")


def cmd_import(args):
    """Import a .otrace file into the engine."""
    if not args:
        _err("Usage: orion import <file.otrace>")
        sys.exit(1)
    path = Path(args[0])
    if not path.exists():
        _err(f"File not found: {path}")
        sys.exit(1)
    data = json.loads(path.read_text())
    events = data.get("events", [])
    payload = json.dumps(events).encode()
    result = _req("POST", "/api/ingest", data=payload)
    if result is None:
        _err("Import failed — engine not running?")
        sys.exit(1)
    _ok(f"Imported {len(events)} events from {path.name}")


def cmd_diff(args):
    """Open two traces side by side in the dashboard."""
    if len(args) < 2:
        _err("Usage: orion diff <trace-id-1> <trace-id-2>")
        sys.exit(1)
    a, b = args[0], args[1]
    url = f"{ENGINE_URL}?diff={a},{b}"
    _ok(f"Opening diff: {a[:8]}… vs {b[:8]}…")
    webbrowser.open(url)


def cmd_watch(args):
    """Watch source files for changes and re-run a command on each change."""
    import glob

    cwd = Path.cwd()
    cfg_path = cwd / ".orionis.json"

    # Determine the command to re-run
    watch_cmd = None
    if args:
        # orion watch python myscript.py
        watch_cmd = args
    elif cfg_path.exists():
        try:
            cfg = json.loads(cfg_path.read_text())
            raw = cfg.get("watch_cmd")
            if raw:
                watch_cmd = raw if isinstance(raw, list) else raw.split()
        except Exception:
            pass

    if not watch_cmd:
        _err("No watch command specified.")
        _info("Usage:  orion watch python my_script.py")
        _info("   or:  set \"watch_cmd\" in .orionis.json")
        sys.exit(1)

    _print_banner()
    _info(f"Watching  : {cwd}")
    _info(f"Command   : {' '.join(watch_cmd)}")
    _info("Ctrl+C to stop.")
    print()

    # File extensions to watch
    PATTERNS = ["**/*.py", "**/*.go", "**/*.rs", "**/*.cpp", "**/*.cc", "**/*.c", "**/*.h", "**/*.hpp"]

    def _scan_mtimes():
        """Return dict: filepath → mtime for all source files."""
        result = {}
        for pat in PATTERNS:
            for p in cwd.glob(pat):
                try:
                    result[str(p)] = p.stat().st_mtime
                except OSError:
                    pass
        return result

    def _run():
        """Run the watched command in the current directory."""
        print(f"\n{CYAN}→ Running: {' '.join(watch_cmd)}{RESET}")
        try:
            result = subprocess.run(watch_cmd, cwd=str(cwd))
            if result.returncode == 0:
                _ok("Exited OK")
            else:
                _err(f"Exited with code {result.returncode}")
        except FileNotFoundError as e:
            _err(f"Command not found: {e}")
        except KeyboardInterrupt:
            pass

    mtimes = _scan_mtimes()
    _run()  # run once immediately on start

    try:
        while True:
            time.sleep(0.5)
            new_mtimes = _scan_mtimes()
            changed = [f for f, t in new_mtimes.items()
                       if t != mtimes.get(f)]
            if changed:
                for f in changed:
                    _info(f"Changed: {Path(f).relative_to(cwd)}")
                mtimes = new_mtimes
                _run()
    except KeyboardInterrupt:
        print(f"\n{YELLOW}Watch stopped.{RESET}")


def cmd_clean(_args):
    """Delete all stored traces from the engine."""
    result = _req("DELETE", "/api/clear")
    if result is None:
        _err("Cannot reach engine.")
        sys.exit(1)
    _ok("All traces cleared.")


def cmd_config(_args):
    """Show current configuration."""
    _print_banner()
    cfg_path = Path.cwd() / ".orionis.json"
    if cfg_path.exists():
        _info(f"Project config: {cfg_path}")
        print(json.dumps(json.loads(cfg_path.read_text()), indent=2))
    else:
        _warn("No .orionis.json found in current directory.")
        _info("Run  orion init  to create one.")
    print()
    _info(f"Engine URL: {ENGINE_URL}")
    _info(f"Engine binary: {ENGINE_BIN or 'not found'}")


def cmd_help(_args):
    print(__doc__)


# ── Dispatch ──────────────────────────────────────────────────────────────────

COMMANDS = {
    "init":    cmd_init,
    "start":   cmd_start,
    "stop":    cmd_stop,
    "status":  cmd_status,
    "list":    cmd_list,
    "replay":  cmd_replay,
    "export":  cmd_export,
    "import":  cmd_import,
    "diff":    cmd_diff,
    "watch":   cmd_watch,
    "clean":   cmd_clean,
    "config":  cmd_config,
    "help":    cmd_help,
    "--help":  cmd_help,
    "-h":      cmd_help,
}

def main():
    args = sys.argv[1:]
    if not args:
        cmd_help([])
        return
    cmd = args[0].lower()
    handler = COMMANDS.get(cmd)
    if not handler:
        _err(f"Unknown command: {cmd}")
        _info("Run  orion help  to see available commands.")
        sys.exit(1)
    handler(args[1:])


if __name__ == "__main__":
    main()
