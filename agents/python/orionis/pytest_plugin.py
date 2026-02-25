"""
Orionis pytest plugin — automatic test failure tracing.

Zero-config: install orionis-agent and every pytest failure is auto-traced.
The trace ID is printed in the failure output so you can replay it with:

    orion replay <trace-id>

Activated automatically via the `pytest11` entry point in setup.py.
"""

import pytest
from . import agent as _agent


# ── Plugin Registration ────────────────────────────────────────────────────────

def pytest_addoption(parser):
    """Add --orionis-off flag to disable the plugin."""
    group = parser.getgroup("orionis")
    group.addoption(
        "--orionis-off",
        action="store_true",
        default=False,
        help="Disable Orionis automatic tracing for this test run.",
    )
    group.addoption(
        "--orionis-url",
        default="http://localhost:7700",
        help="Orionis engine URL (default: http://localhost:7700)",
    )
    group.addoption(
        "--orionis-mode",
        default="error",
        choices=["dev", "safe", "error"],
        help="Orionis tracing mode for pytest (default: error — captures on failure only)",
    )


# ── Session Hooks ─────────────────────────────────────────────────────────────

def pytest_sessionstart(session):
    """Start the Orionis agent at the beginning of the test session."""
    if session.config.getoption("--orionis-off", default=False):
        return

    engine_url = session.config.getoption("--orionis-url", default="http://localhost:7700")
    mode       = session.config.getoption("--orionis-mode", default="error")

    try:
        # Check engine is reachable before starting
        import urllib.request, urllib.error
        try:
            urllib.request.urlopen(f"{engine_url}/api/traces", timeout=1)
        except urllib.error.URLError:
            # Engine not running — warn but don't fail the test session
            print(f"\n[Orionis] ⚠ Engine not reachable at {engine_url} — tracing disabled for this session.")
            print(f"[Orionis]   Start it with:  orion start\n")
            session.orionis_active = False
            return

        _agent.start(engine_url=engine_url, mode=mode)
        session.orionis_active = True
        print(f"\n[Orionis] Agent active — engine: {engine_url} | mode: {mode}")
        print(f"[Orionis] Failed tests will be traced automatically.\n")

    except Exception as e:
        print(f"\n[Orionis] Failed to start agent: {e}\n")
        session.orionis_active = False


def pytest_sessionfinish(session, exitstatus):
    """Stop the Orionis agent after all tests complete."""
    if getattr(session, "orionis_active", False):
        _agent.stop()


# ── Test Failure Hook ─────────────────────────────────────────────────────────

def pytest_runtest_logreport(report):
    """
    Called after each test phase (setup / call / teardown).
    On failure in the 'call' phase — the actual test body — flush trace and
    print the trace ID so the developer can replay it.
    """
    if report.when != "call":
        return
    if not report.failed:
        return
    if not _agent._active:
        return

    # Force flush the current trace events to the engine
    _agent._flush()

    trace_id = _agent._get_trace_id()

    # Print a clearly visible replay link in the test output
    print(f"\n{'─' * 60}")
    print(f"  [Orionis] Test failure traced!")
    print(f"  Trace ID : {trace_id}")
    print(f"  Replay   : orion replay {trace_id}")
    print(f"  Dashboard: http://localhost:7700?trace={trace_id}")
    print(f"{'─' * 60}\n")

    # Start a fresh trace context for the next test
    _agent.reset_trace()


# ── Fixture: per-test trace reset ─────────────────────────────────────────────

@pytest.fixture(autouse=True)
def _orionis_reset_trace():
    """
    Auto-used fixture: resets the trace context before each test so each
    test gets its own isolated trace ID.
    """
    if _agent._active:
        _agent.reset_trace()
    yield
    # Nothing to clean up — logreport hook handles post-test work
