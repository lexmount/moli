from __future__ import annotations

import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .assertions import record
from .fixture import FixtureServer


@dataclass
class SmokeState:
    endpoint: str
    browser: Any
    context: Any
    page: Any
    cdp: Any
    fixture: str
    fixture_server: FixtureServer
    temp_dir: Path
    results: list[dict[str, Any]]
    subresource_events: list[dict[str, Any]]
    websocket_events: list[dict[str, Any]]

    def record(self, name: str, data: dict[str, Any] | None = None) -> None:
        record(self.results, name, data)
        print(
            f"[moli-cdp-smoke] PASS scenario/{name}",
            file=sys.stderr,
            flush=True,
        )
