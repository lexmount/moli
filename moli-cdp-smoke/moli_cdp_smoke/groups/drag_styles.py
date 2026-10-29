"""Native eligibility consumes computed CSS, including presentation hints."""
import asyncio
import base64

from ..assertions import assert_equal
from ..fixture import TRANSPARENT_PNG
from ..helpers import capture_layout
from . import SmokeState
from .native_drag_semantics import _mouse, _session_page


async def run_drag_styles(state: SmokeState) -> None:
    image = "<img id=source src='data:image/png;base64," + base64.b64encode(TRANSPARENT_PNG).decode("ascii") + "'>"
    cases = (
        ("<div id=source>source</div>", "", "auto", False),
        ("<div id=source>source</div>", "#source{-webkit-user-drag:element}", "element", True),
        ("<div id=source draggable=true>source</div>", "#source{-webkit-user-drag:none}", "none", False),
        ("<div id=source draggable=false>source</div>", "#source{-webkit-user-drag:element}", "element", True),
        ("<div id=source draggable=true>source</div>", "#source{-webkit-user-drag:auto}", "auto", False),
        ("<a id=source href='/link'>link</a>", "#source{-webkit-user-drag:none}", "none", False),
        ("<a id=source href='/link' draggable=false>link</a>", "#source{-webkit-user-drag:auto}", "auto", True),
        ("<div id=source style='-webkit-user-drag:element'>source</div>", "#source{-webkit-user-drag:none!important}", "none", False),
        ("<div style='-webkit-user-drag:none'><a id=source href='/link'>link</a></div>", "", "auto", True),
        ("<div id=source>source</div>", "#source{--drag:element;-webkit-user-drag:var(--drag)}", "element", True),
        ("<div id=source draggable=true>source</div>", "#source{-webkit-user-drag:invalid}", "element", True),
        (image, "", "auto", True),
        (image, "#source{-webkit-user-drag:none}", "none", False),
    )
    for source, css, style, expected in cases:
        async with _session_page(state) as (page, session, received, ready):
            await page.set_content("<!doctype html><base href='https://drag-styles.test/'><body style='margin:0'><style>"
                                   "#source{display:block;width:150px;height:100px}" + css + "</style>" + source)
            await page.evaluate("() => Promise.all(Array.from(document.images, image => image.decode()))")
            await page.evaluate("""() => {window.__starts=0;
              document.addEventListener('dragstart',()=>__starts++);}""")
            await capture_layout(page)
            await session.send("Input.setInterceptDrags", {"enabled": True})
            await _mouse(session, "mouseMoved", 30, 30, 0)
            await _mouse(session, "mousePressed", 30, 30, 1)
            await _mouse(session, "mouseMoved", 45, 30, 1)
            if expected:
                await asyncio.wait_for(ready.wait(), timeout=5)
            observed = await page.evaluate("""() => [getComputedStyle(document.getElementById('source'))
              .getPropertyValue('-webkit-user-drag'),window.__starts]""")
            assert_equal(observed, [style, int(expected)], f"computed user-drag determines native eligibility: {source}, {css}")
    state.record("native_drag_computed_user_drag_and_presentation_hints")
