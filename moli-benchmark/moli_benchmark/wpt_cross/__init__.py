"""Cross-engine WPT runner.

Runs identical Web Platform Tests cases against multiple headless engines
(Moli, Lightpanda, Chrome, Obscura) using the same fixture server and upstream
WPT case URLs. It supports both testharness completion and manifest-backed
layout reftest screenshots.

The goal is to produce an objective WebAPI coverage matrix: same case set,
same harness, same timeout, no per-engine source patches.
"""

from .engine import EngineDriver, ENGINES, build_driver
from .server import WptFixtureServer
from .runner import CaseResult, EngineRunResult, run_engine_on_cases

__all__ = [
    "EngineDriver",
    "ENGINES",
    "build_driver",
    "WptFixtureServer",
    "CaseResult",
    "EngineRunResult",
    "run_engine_on_cases",
]
