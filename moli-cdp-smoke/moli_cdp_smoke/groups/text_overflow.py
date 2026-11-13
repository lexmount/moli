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
    ("plaintext-direction", "ABCDEFGHIJKL", "ABCD...", "direction:rtl;unicode-bidi:plaintext"),
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
    ("hidden-atomic-text-mask", 'ABC<span class="atomic" style="width:80px">INNER</span>XYZ',
     'ABC<span class="atomic" style="width:0;visibility:hidden">INNER</span>...',
     "background:red;background-clip:text;-webkit-background-clip:text;color:transparent"),
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
        await _check_positioned_descendants(page, font, results)
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
        for mixed in (False, True):
            for rtl, start, end, visible in (
                (False, 5, 12, False), (True, 0, 7, False),
                (False, 0, 2, True), (True, 10, 12, True),
            ):
                html = f"""<!doctype html><style>
                  @font-face{{font-family:OverflowTest;src:url(data:font/ttf;base64,{font})}}
                  body{{margin:0;background:white}}
                  #probe{{position:absolute;left:20px;top:20px;width:90px;
                    font:20px/24px OverflowTest;white-space:nowrap;overflow:hidden;
                    text-overflow:ellipsis;direction:{'rtl' if rtl else 'ltr'}}}
                </style><div id="probe">{'<span style="display:inline-block;width:0;height:0"></span>' if mixed else ''}ABCDEFGHIJKL</div>"""
                await page.goto("data:text/html;base64," + base64.b64encode(html.encode()).decode(),
                                wait_until="load")
                await page.evaluate("document.fonts.ready")
                capture = {"x": 0, "y": 0, "width": 200, "height": 100}
                before = decode_png(await page.screenshot(clip=capture))
                geometry = await page.evaluate("""() => {
                  const range = document.createRange();
                  range.selectNodeContents(document.getElementById('probe'));
                  return range.getBoundingClientRect().toJSON();
                }""")
                await page.evaluate("""([start, end]) => {
                  const text = document.getElementById('probe').lastChild;
                  getSelection().setBaseAndExtent(text, start, text, end);
                }""", [start, end])
                after = decode_png(await page.screenshot(clip=capture))
                different = sum(before.rgba[i:i+4] != after.rgba[i:i+4]
                                for i in range(0, len(before.rgba), 4))
                name = f"selection-{'mixed' if mixed else 'bulk'}-{'rtl' if rtl else 'ltr'}-{'visible' if visible else 'hidden'}"
                assert_equal(different > 0, visible, f"{name}: paint only visible selection")
                after_geometry = await page.evaluate("""() => {
                  const range = document.createRange();
                  range.selectNodeContents(document.getElementById('probe'));
                  return range.getBoundingClientRect().toJSON();
                }""")
                assert_equal(geometry, after_geometry, f"{name}: preserve original range geometry")
                record_contract(results, f"text_overflow_{name}",
                    contract="Selection paint follows truncation in bulk and mixed IFCs; source geometry is preserved.",
                    source="Blink TextFragmentPainter selection uses the visible fragment range",
                    commands=["Page.captureScreenshot", "Runtime.evaluate"],
                    observed={"differentPixels": different, "sourceGeometry": geometry})
    finally:
        await context.close()


async def _check_positioned_descendants(page: Any, font: str,
                                        results: list[dict[str, Any]]) -> None:
    for position, opacity in (("absolute", 1), ("fixed", 1), ("absolute", 0.5), ("fixed", 0.5)):
        html = f"""<!doctype html><style>
          @font-face{{font-family:OverflowTest;src:url(data:font/ttf;base64,{font})}}
          body{{margin:0;background:white}}
          #probe{{position:relative;width:90px;overflow:hidden;text-overflow:ellipsis;
            font:20px/24px OverflowTest;white-space:nowrap}}
          .atomic{{position:relative;display:inline-block;width:80px;height:20px;background:lime;opacity:{opacity}}}
          .inner{{position:{position};left:0;top:0;color:red}}
        </style><div id="probe">ABC<span class="atomic"><span class="inner">INNER</span></span>XYZ</div>"""
        await page.goto("data:text/html;base64," + base64.b64encode(html.encode()).decode(),
                        wait_until="load")
        await page.evaluate("document.fonts.ready")
        image = decode_png(await page.screenshot(clip={"x": 0, "y": 0, "width": 200, "height": 100}))
        pixels = [image.rgba[i:i+4] for i in range(0, len(image.rgba), 4)]
        green = sum(g > r and g > b for r, g, b, _ in pixels)
        red = sum(r > g and r > b for r, g, b, _ in pixels)
        pure_red = sum(pixel == bytes((255, 0, 0, 255)) for pixel in pixels)
        assert_equal(green > 0, position == "fixed",
                     f"{position}: retain the atomic only when OOF descendants propagate")
        assert_equal(red > 0, True, f"{position}: independently painted descendant survives")
        assert_equal(pure_red > 0, opacity == 1,
                     f"{position}: descendant retains the hidden stacking root's opacity")
        record_contract(results, f"text_overflow_positioned_{position}_{opacity}",
            contract="A truncated atomic background and independent positioned descendants have separate visibility.",
            source="Blink LineTruncator::HideChild; PhysicalFragment OofPositionedDescendants",
            commands=["Page.captureScreenshot", "Runtime.evaluate"],
            observed={"greenPixels": green, "redPixels": red, "pureRedPixels": pure_red,
                      "opacity": opacity})
