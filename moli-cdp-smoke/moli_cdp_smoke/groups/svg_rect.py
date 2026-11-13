from __future__ import annotations

import base64
import math
from typing import Any

from ..assertions import SmokeError, assert_equal, record_contract
from ..config import REPO_ROOT
from ..diagnostics import new_cdp_session
from ..png_image import decode_png


async def _check_svg_scene_inputs(page: Any, results: list[dict[str, Any]], kind: str) -> None:
    fixture = (REPO_ROOT / "moli-renderer-v8/tests/fixtures/inline-svg-scene-inputs.js").read_text()
    metadata = await page.evaluate(f"{fixture}\ninstallSvgSceneInputs('{kind}')")
    await page.set_viewport_size({"width": metadata["width"], "height": metadata["height"]})
    session = await new_cdp_session(page.context, page)
    try:
        response = await session.send("Page.captureScreenshot", {"format": "png"})
        image = decode_png(base64.b64decode(response["data"]))
        for case in metadata["cases"]:
            name, top = case["name"], case["top"]
            observed = await page.evaluate("name => svgSceneInputRects(name)", name)
            if kind == "units":
                if observed["target"][2] <= 0 or not all(
                    math.isclose(target, reference, abs_tol=0.01)
                    for target, reference in zip(observed["target"], observed["reference"], strict=True)
                ):
                    raise SmokeError(f"{name}: SVG zoom differs from equivalent outer scaling: {observed!r}")
                ink = 0
                for y in range(top, top + 140):
                    for x in range(200):
                        target, reference = image.pixel(x, y), image.pixel(x + 240, y)
                        if target != reference:
                            raise SmokeError(f"{name} pixel ({x},{y}): {target!r} != {reference!r}")
                        ink += 255 - reference[0]
                if ink <= 1000:
                    raise SmokeError(f"{name}: the scaling reference has no visible ink")
            else:
                assert_equal(observed["children"], 0, name + " ordinary template children")
                if observed["content"] <= 0:
                    raise SmokeError(f"{name}: missing template content")
                assert_equal(observed["rects"], [[0, top, 20, 20], [40, top, 20, 20]], name + " graphics identities")
                assert_equal(image.pixel(10, top + 10), (255, 0, 0, 255), name + " preceding paint")
                assert_equal(image.pixel(50, top + 10), (0, 0, 255, 255), name + " following paint")
            record_contract(results, "svg/scene_inputs/" + name, observed=observed,
                            contract="Scene lengths stay in SVG user units and source mapping follows serialized template contents.",
                            source="Chromium 145.0.7632.116 executable comparison (2026-11-13)",
                            commands=["Runtime.evaluate", "Page.captureScreenshot", "Emulation.setDeviceMetricsOverride"])
    finally:
        await session.detach()


async def _check_inline_svg(page: Any, results: list[dict[str, Any]]) -> None:
    session = await new_cdp_session(page.context, page)

    async def capture() -> Any:
        response = await session.send("Page.captureScreenshot", {"format": "png"})
        return decode_png(base64.b64decode(response["data"]))

    cases = [
        ("original_style_selector", '<style>rect[style]{transform:matrix(1,0,0,1,40,0);fill:blue!important}</style><rect width="20" height="20" fill="red"/>', [0, 0, 20, 20]),
        ("original_id_selector", '<style>rect[id]{transform:matrix(1,0,0,1,40,0)}</style><rect width="20" height="20" fill="red"/>', [0, 0, 20, 20]),
        ("empty_clip_geometry", '<defs><clipPath id="empty"/></defs><rect x="10" y="20" width="30" height="40" clip-path="url(#empty)"/>', [10, 20, 30, 40]),
        ("nested_visible_geometry", '<svg x="10" y="20" width="100" height="80" overflow="visible"><rect x="2" y="3" width="30" height="40"/></svg>', [12, 23, 30, 40]),
    ]
    try:
        for name, content, expected in cases:
            await page.evaluate("""content => {
                document.head.innerHTML='<style>html,body{margin:0;background:white}svg{display:block}</style>';
                document.body.innerHTML='<svg width="300" height="150">'+content+'</svg>';
            }""", content)
            image = await capture()
            observed = await page.evaluate("""() => {
                const e=document.querySelector('body>svg>svg,rect'), r=e.getBoundingClientRect();
                return {rect:[r.x,r.y,r.width,r.height], count:e.getClientRects().length};
            }""")
            assert_equal(observed, {"rect": expected, "count": 1}, name)
            if name.startswith("original_"):
                assert_equal(image.pixel(10, 10), (255, 0, 0, 255), name + " paint")
                assert_equal(image.pixel(50, 10), (255, 255, 255, 255), name + " unmatched transform")
            record_contract(results, "svg/" + name, observed=observed,
                            contract="Final computed styles and source geometry preserve original SVG XML identity.",
                            source="Chromium 145.0.7632.116 executable comparison (2026-11-13)", commands=["Runtime.evaluate", "Page.captureScreenshot"])

        await page.evaluate("""() => {
            document.head.innerHTML='<style>html,body{margin:0}#container{width:250px}.hidden{display:none}svg{font:12px "DejaVu Sans"}</style>';
            document.body.innerHTML='<div id="container"><svg class="hidden"><g><text>Code review</text></g></svg></div>';
        }""")
        await capture()
        container = await page.evaluate("document.getElementById('container').clientWidth")
        await page.evaluate("document.querySelector('svg').classList.remove('hidden')")
        await capture()
        initial = await page.evaluate("""() => {
            const r=document.querySelector('text').getBoundingClientRect();
            return [r.width,r.height];
        }""")
        await page.evaluate("""() => {
            const text=document.querySelector('text');
            text.setAttribute('x','40');text.setAttribute('y','50');
        }""")
        await capture()
        positioned = await page.evaluate("""() => {
            const r=document.querySelector('g').getBoundingClientRect();
            return [r.x,r.y,r.width,r.height];
        }""")
        observed = {"container": container, "initial": initial, "positioned": positioned}
        assert_equal(container, 250, "chart container measurement")
        if initial[0] <= 50 or initial[1] <= 8:
            raise SmokeError(f"SVG stayed hidden after visual publication: {observed!r}")
        if positioned[0] <= 39 or positioned[1] <= 30:
            raise SmokeError(f"SVG label position stayed stale after capture: {observed!r}")
        record_contract(results, "svg/chart_construction", observed=observed,
                        contract="Explicit visual captures publish revealed and repositioned SVG label geometry.",
                        source="Chromium executable comparison; Moli uses explicit frozen-layout publication.",
                        commands=["Runtime.evaluate", "Page.captureScreenshot"])

        await page.evaluate("""() => {
            document.head.innerHTML='<style>html,body{margin:0;background:white}svg{display:block}</style>';
            document.body.innerHTML='<svg width="100" height="100">'+'<circle r="1"></circle>'.repeat(30000)+'</svg>';
        }""")
        image = await capture()
        if any(channel > 2 for channel in image.pixel(0, 0)[:3]):
            raise SmokeError("690 KB SVG disappeared after applying computed styles")
        record_contract(results, "svg/original_source_budget", observed={"circles": 30000},
                        contract="Derived styles do not inflate the bounded encoded SVG input.",
                        source="Chromium 145.0.7632.116 executable comparison (2026-11-13)", commands=["Page.captureScreenshot"])

        await page.evaluate("""() => {
            document.body.innerHTML='<svg style="position:absolute;left:100px;top:0" width="200" height="100">'+
                '<rect x="10" width="10" height="10"/>'.repeat(1000)+'</svg>';
        }""")
        await capture()
        count = await page.evaluate("""() => {
            const r=document.createRange();r.selectNodeContents(document.querySelector('svg'));
            return r.getClientRects().length;
        }""")
        assert_equal(count, 1000, "SVG Range geometry batch")
        await page.evaluate("""() => {
            document.querySelector('svg').style.left='300px';
            document.querySelector('rect').setAttribute('x','30');
        }""")
        await capture()
        observed = await page.evaluate("""() => [document.querySelector('svg'),document.querySelector('rect')]
            .map(e=>e.getBoundingClientRect().x)""")
        assert_equal(observed, [300, 330], "coherent SVG geometry after capture")
        updated = await page.evaluate("document.querySelector('rect').getBoundingClientRect().x")
        assert_equal(updated, 330, "repeated published SVG geometry")
        record_contract(results, "svg/coherent_batch_publication", observed={"count": count, "published": observed, "repeated": updated},
                        contract="A visual capture publishes SVG object and viewport coordinates together; batch reads reuse that geometry.",
                        source="Chromium executable comparison; Moli uses explicit frozen-layout publication.",
                        commands=["Runtime.evaluate", "Page.captureScreenshot"])
    finally:
        await session.detach()


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
        await _check_inline_svg(page, results)
        await _check_svg_scene_inputs(page, results, "template")
        await _check_svg_scene_inputs(page, results, "units")
    finally:
        await context.close()
