from __future__ import annotations

from typing import Any

from ..assertions import assert_equal, record_contract
from ..config import REPO_ROOT


async def run_svg_rect_group(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        script = (REPO_ROOT / "moli-renderer-v8/tests/fixtures/svg-create-rect.js").read_text()
        observed = await page.evaluate(script)
        assert_equal(observed, "svg-create-rect:ok", "SVGRect interface and value contract")
        record_contract(
            results,
            "svg_create_rect",
            contract="SVG capability detection has a real detached SVGRect with restricted float fields.",
            source="Chromium 145.0.7632.116 executable probe (2026-09-07)",
            commands=["Runtime.evaluate"],
            observed=observed,
        )
    finally:
        await context.close()
