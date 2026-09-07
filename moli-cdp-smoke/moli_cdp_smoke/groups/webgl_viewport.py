from __future__ import annotations

import json
from typing import Any

from ..assertions import assert_equal, record_contract
from ..config import REPO_ROOT


async def run_webgl_viewport_group(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        script = (REPO_ROOT / "moli-renderer-v8/tests/fixtures/webgl-viewport.js").read_text()
        observed = json.loads(await page.evaluate(script))
        assert_equal(
            observed,
            ["html:webgl", "offscreen:webgl", "html:webgl2", "offscreen:webgl2"],
            "WebGL viewport state matrix",
        )
        record_contract(
            results,
            "webgl_viewport_state",
            contract=(
                "WebGL1/2 on HTMLCanvasElement and OffscreenCanvas preserve context-local "
                "viewport/error state, typed copies, WebIDL conversion, limits and resize semantics. "
                "This is a state contract, not a GPU rasterization test."
            ),
            source="Chromium 145.0.7632.116 executable probe (2026-09-07)",
            commands=["Runtime.evaluate"],
            observed=observed,
        )
    finally:
        await context.close()
