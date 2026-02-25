"""Orionis Python Agent — package init."""
from .agent import start, stop, reset_trace

# Internal symbols exposed for the pytest plugin and tooling.
# Prefixed with _ to signal these are not part of the public SDK API.
from .agent import _flush, _get_trace_id, _active  # noqa: F401

__version__ = "0.1.0"
__all__ = ["start", "stop", "reset_trace"]
