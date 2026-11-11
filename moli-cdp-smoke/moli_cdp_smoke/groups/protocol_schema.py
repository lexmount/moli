from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from .external_process import run_external_json_process


async def run_protocol_schema_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    script = Path(__file__).resolve().parents[1] / "protocol_schema_smoke.cjs"
    await run_external_json_process(
        "protocol-schema",
        [os.environ.get("NODE", "node"), str(script), endpoint, fixture],
        results,
    )
