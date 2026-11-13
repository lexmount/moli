from __future__ import annotations

import base64
from typing import Any

from ..assertions import SmokeError, assert_equal, record_contract
from ..config import REPO_ROOT
from ..diagnostics import write_artifact
from ..png_image import decode_png


# The fixed face deliberately lacks U+2026. Blink falls back to three periods
# from the primary face rather than taking an ellipsis from a fallback font.
CASES = (
    ("ltr", "ABCDEFGHIJKL", "ABCD...", ""),
    ("rtl", "ABCDEFGHIJKL", "...IJKL", "direction:rtl"),
    ("line-style", '<span style="color:red">ABCDEFGHIJKL</span>',
     '<span style="color:red">ABCD</span>...', "color:blue"),
    ("padding", "ABCDEFGHIJKL", "ABCD...", "padding:10px"),
    ("letter-spacing", "ABCDEFGHIJKL", '<span>ABCD</span><span style="letter-spacing:0">...</span>',
     "letter-spacing:1px"),
    ("underline", "ABCDEFGHIJKL", 'ABCD<span style="display:inline-block;text-decoration:none">...</span>',
     "text-decoration:underline"),
    ("indent", "ABCDEFGHIJKL", "AB...", "text-indent:20px"),
    ("negative-indent-start-only", 'ABCDEFGHI<span style="color:red">JKL</span>',
     'ABCDEFGHI<span style="color:red">JKL</span>', "text-indent:-60px"),
    ("negative-indent-end-overflow", "ABCDEFGHIJKL", "ABCDEFG...", "text-indent:-30px"),
    ("negative-indent-right-aligned", 'ABCDEFGHI<span style="color:red">JKL</span>',
     'ABCDEFGHI<span style="color:red">JKL</span>', "text-indent:-60px;text-align:right"),
    ("forced-lines", "ABCDEFGHIJKL\nABCDEFGHIJKL", "ABCD...\nABCD...", "white-space:pre"),
    ("tiny", "ABCDEFGHIJKL", "A...", "width:4px"),
    ("no-overflow", "ABCD", "ABCD", ""),
    ("visible-overflow", "ABCDEFGHIJKL", "ABCDEFGHIJKL", "overflow:visible"),
    ("retained-atomic", '<span class="atomic"></span>ABCDEFGHIJKL',
     '<span class="atomic"></span>AB...', ""),
    ("hidden-atomic", 'ABC<span class="atomic" style="width:80px"></span>XYZ',
     'ABC<span class="atomic" style="width:0;visibility:hidden"></span>...', ""),
    ("first-atomic", '<span class="atomic" style="width:80px"></span>'
     '<span class="atomic" style="width:80px"></span>',
     '<span class="atomic" style="width:80px"></span>...', ""),
    ("atomic-forced-lines", '<span class="atomic"></span>ABCDEFGHIJKL\n'
     '<span class="atomic"></span>ABCDEFGHIJKL',
     '<span class="atomic"></span>AB...\n<span class="atomic"></span>AB...', "white-space:pre"),
)


async def run_text_overflow_group(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    font = base64.b64encode(
        (REPO_ROOT / "moli-layout/tests/fixtures/moli-ahem.ttf").read_bytes()
    ).decode("ascii")
    context = await browser.new_context(viewport={"width": 400, "height": 200})
    try:
        page = await context.new_page()
        for name, content, reference, extra in CASES:
            html = f"""<!doctype html><style>
              @font-face{{font-family:OverflowTest;src:url(data:font/ttf;base64,{font})}}
              body{{margin:0;background:white}}
              #probe{{position:absolute;left:20px;top:20px;width:90px;
                font:20px/24px OverflowTest;white-space:nowrap;overflow:hidden;
                text-overflow:ellipsis;{extra}}}
              .atomic{{display:inline-block;width:24px;height:20px;background:lime}}
            </style><div id="probe">{content}</div>"""
            await page.goto("data:text/html;base64," + base64.b64encode(html.encode()).decode(),
                            wait_until="load")
            await page.evaluate("document.fonts.ready")
            original = await page.evaluate("""() => {
              const e = document.getElementById('probe'), range = document.createRange();
              range.selectNodeContents(e);
              return {text:e.textContent, scroll:e.scrollWidth, client:e.clientWidth,
                width:range.getBoundingClientRect().width};
            }""")
            capture = {"x": 0, "y": 0, "width": 200, "height": 100}
            actual_png = await page.screenshot(clip=capture)
            actual = decode_png(actual_png)
            await page.evaluate("""() => {
              document.getElementById('probe').style.textOverflow = 'clip';
            }""")
            unclipped = await page.evaluate("""() => {
              const e = document.getElementById('probe'), range = document.createRange();
              range.selectNodeContents(e);
              return {text:e.textContent, scroll:e.scrollWidth, client:e.clientWidth,
                width:range.getBoundingClientRect().width};
            }""")
            assert_equal(original, unclipped, f"{name}: ellipsis leaves source geometry intact")
            await page.evaluate("""html => {
              document.getElementById('probe').innerHTML = html;
            }""", reference)
            expected_png = await page.screenshot(clip=capture)
            expected = decode_png(expected_png)
            different = sum(actual.rgba[i:i+4] != expected.rgba[i:i+4]
                            for i in range(0, len(actual.rgba), 4))
            if different:
                write_artifact(f"text-overflow-{name}", {
                    "actualPng": base64.b64encode(actual_png).decode(),
                    "referencePng": base64.b64encode(expected_png).decode(),
                    "originalGeometry": original,
                })
                raise SmokeError(f"{name}: ellipsis differs from its reference by {different} pixels")
            record_contract(results, f"text_overflow_{name}",
                contract="Visual ellipsis preserves source geometry and matches explicit reference text.",
                source="Chromium 145.0.7632.116; Blink LineTruncator",
                commands=["Page.captureScreenshot", "Runtime.evaluate"],
                observed={"differentPixels": different, **original})
    finally:
        await context.close()
