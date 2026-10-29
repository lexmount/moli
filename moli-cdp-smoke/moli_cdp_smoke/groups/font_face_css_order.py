from __future__ import annotations

import base64
from typing import Any

from ..assertions import assert_equal, record_contract
from .font_face_matching import font_matching_payload


async def run_css_font_order_contract(browser: Any, results: list[dict[str, Any]]) -> None:
    context = await browser.new_context()
    try:
        fonts = font_matching_payload()
        observed = {}
        for names in [("narrow", "wide"), ("wide", "narrow")]:
            rules = "".join(
                "@font-face{font-family:PureCssOrder;src:url(data:font/ttf;base64,"
                + fonts[name + "Data"] + ")}"
                for name in names
            )
            html = (
                "<!doctype html><script>globalThis.fontsReads=0;"
                "Object.defineProperty(document,'fonts',{get(){++fontsReads;"
                "throw new Error('document.fonts must remain unobserved')}})</script>"
                "<style>" + rules + "</style>"
                '<span id="probe" style="display:inline-block;font:40px PureCssOrder">AAAA</span>'
            )
            page = await context.new_page()
            try:
                url = "data:text/html;base64," + base64.b64encode(html.encode()).decode("ascii")
                await page.goto(url, wait_until="load", timeout=10_000)
                observed["-".join(names)] = await page.evaluate(
                    "({width: document.getElementById('probe').getBoundingClientRect().width, fontsReads})"
                )
            finally:
                await page.close()
        assert_equal(observed, {
            "narrow-wide": {"width": 160, "fontsReads": 0},
            "wide-narrow": {"width": 80, "fontsReads": 0},
        }, "CSS font declaration order without observing document.fonts")
        record_contract(
            results,
            "css_font_order_without_font_face_set_projection",
            contract="Pure CSS font selection follows reverse declaration order without accessing document.fonts.",
            source="Chromium 147 and pinned Chromium 145.0.7632.6",
            commands=["Page.navigate", "Runtime.evaluate"],
            observed=observed,
        )
    finally:
        await context.close()


async def run_projected_local_font_order_contract(browser: Any, results: list[dict[str, Any]]) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto("about:blank", wait_until="load", timeout=10_000)
        observed = await page.evaluate(r"""async ({narrowData, wideData}) => {
          let localName;
          for (const name of ['DejaVu Sans Mono', 'Liberation Mono', 'Courier New', 'Menlo', 'Consolas']) {
            const face = new FontFace('AvailableLocalOrderFont', `local(${JSON.stringify(name)})`);
            try { await face.load(); localName = name; break; } catch {}
          }
          if (!localName) throw new Error('No supported local monospace font is installed');
          const style = document.createElement('style');
          style.textContent = `@font-face{font-family:ProjectedLocalOrder;src:local(${JSON.stringify(localName)})}
            @font-face{font-family:ProjectedLocalOrder;src:url(data:font/ttf;base64,${wideData})}`;
          document.head.append(style);
          const faces = [...document.fonts].filter(face => face.family === 'ProjectedLocalOrder');
          await Promise.all(faces.map(face => face.load()));
          globalThis.localOrderProbe = document.createElement('span');
          localOrderProbe.textContent = 'AAAA';
          localOrderProbe.style.cssText = 'display:inline-block;font:40px ProjectedLocalOrder';
          document.body.append(localOrderProbe);
          const before = localOrderProbe.getBoundingClientRect().width;
          const other = document.createElement('style');
          other.textContent = `@font-face{font-family:UnrelatedLocalOrder;src:url(data:font/ttf;base64,${narrowData})}`;
          document.head.append(other);
          return {before, states: faces.map(face => face.status)};
        }""", font_matching_payload())
        # Observe after the stylesheet mutation's native resource reconciliation,
        # without rereading document.fonts to republish the projected order.
        observed["after"] = await page.evaluate("localOrderProbe.getBoundingClientRect().width")
        assert_equal(observed, {"before": 160, "states": ["loaded", "loaded"], "after": 160},
                     "native font fetch updates preserve projected local-only CSS face order")
        record_contract(
            results,
            "projected_local_css_font_order_survives_native_reconciliation",
            contract="An exposed FontFaceSet retains complete CSS order, including local-only faces, when native font requests change.",
            source="Chromium 147 and pinned Chromium 145.0.7632.6",
            commands=["Runtime.evaluate"],
            observed=observed,
        )
    finally:
        await context.close()
