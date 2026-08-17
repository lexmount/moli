from __future__ import annotations

from . import SmokeState
from ..assertions import SmokeError, assert_equal
from ..png_image import decode_png
from ..raw_cdp import discover_websocket_url


async def run_classic_scrollbar_group(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp
    websocket_url = await discover_websocket_url(state.endpoint)
    is_moli = websocket_url.endswith("/devtools/browser/moli-browser")
    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; }
          #scroller {
            width: 200px;
            height: 100px;
            overflow: auto;
            scrollbar-color: rgb(255, 0, 0) rgb(0, 0, 255);
          }
          #content { width: 400px; height: 300px; background: rgb(0, 255, 0); }
          .probe { width: 200px; height: 100px; overflow: scroll; }
          #thin { scrollbar-width: thin; }
          #none { scrollbar-width: none; }
          .gutter-probe { width: 200px; height: 100px; overflow: auto; }
          .gutter-probe > div { height: 20px; }
          #stable { scrollbar-gutter: stable; }
          #both-edges { scrollbar-gutter: stable both-edges; }
          #rtl { direction: rtl; scrollbar-color: rgb(255, 0, 0) rgb(0, 0, 255); }
        </style>
        <div id="scroller"><div id="content"></div></div>
        <div id="thin" class="probe"><div></div></div>
        <div id="none" class="probe"><div></div></div>
        <div id="stable" class="gutter-probe"><div></div></div>
        <div id="both-edges" class="gutter-probe"><div></div></div>
        <div id="feedback" class="gutter-probe"><div style="width:200px;height:200px"></div></div>
        <div id="rtl" class="probe"><div style="width:400px;height:300px"></div></div>
        <script>
          thin.firstElementChild.style.cssText = none.firstElementChild.style.cssText =
            "width:400px;height:300px";
          window.__scrollbarDomEvents = [];
          for (const name of [
            "pointerdown", "pointermove", "pointerup",
            "mousedown", "mousemove", "mouseup", "click"
          ]) {
            document.addEventListener(name, () => __scrollbarDomEvents.push(name), true);
          }
        </script>
        """,
        wait_until="domcontentloaded",
    )
    metrics = await page.evaluate(
        """() => [
          scroller.clientWidth, scroller.clientHeight,
          thin.clientWidth, thin.clientHeight,
          none.clientWidth, none.clientHeight,
          scroller.scrollWidth, scroller.scrollHeight,
          stable.clientWidth, stable.clientLeft, stable.firstElementChild.offsetWidth,
          Math.round(stable.firstElementChild.getBoundingClientRect().left),
          document.getElementById("both-edges").clientWidth,
          document.getElementById("both-edges").clientLeft,
          document.getElementById("both-edges").firstElementChild.offsetWidth,
          Math.round(
            document.getElementById("both-edges").firstElementChild.getBoundingClientRect().left
          ),
          feedback.clientWidth, feedback.clientHeight,
          rtl.clientWidth, rtl.clientLeft, rtl.scrollWidth, rtl.scrollLeft,
        ]"""
    )
    assert_equal(
        metrics,
        [
            185, 85, 190, 90, 200, 100, 400, 300,
            185, 0, 185, 0, 170, 15, 170, 15,
            185, 85, 185, 15, 400, 0,
        ],
        "Chromium classic scrollbar client and scroll geometry",
    )

    image = decode_png(await page.screenshot())
    assert_equal(image.pixel(192, 8), (255, 0, 0, 255), "custom up-arrow paint")
    assert_equal(image.pixel(190, 30), (255, 0, 0, 255), "custom vertical thumb paint")
    assert_equal(image.pixel(190, 60), (0, 0, 255, 255), "custom vertical track paint")
    assert_equal(image.pixel(190, 90), (0, 0, 255, 255), "custom scrollbar corner paint")
    assert_equal(image.pixel(192, 508), (139, 139, 139, 255), "default up-arrow paint")
    assert_equal(image.pixel(186, 508), (252, 252, 252, 255), "default button paint")
    assert_equal(image.pixel(5, 620), (255, 0, 0, 255), "RTL vertical thumb paint")

    async def mouse(event_type: str, x: int, y: int, *, pressed: bool) -> None:
        await cdp.send(
            "Input.dispatchMouseEvent",
            {
                "type": event_type,
                "x": x,
                "y": y,
                "button": "left" if event_type != "mouseMoved" else "none",
                "buttons": 1 if pressed else 0,
                "clickCount": 1,
            },
        )

    await mouse("mousePressed", 190, 30, pressed=True)
    await mouse("mouseMoved", 190, 50, pressed=True)
    await mouse("mouseReleased", 190, 50, pressed=False)
    await mouse("mousePressed", 50, 90, pressed=True)
    await mouse("mouseMoved", 90, 90, pressed=True)
    await mouse("mouseReleased", 90, 90, pressed=False)

    state_after_drag = await page.evaluate(
        """() => ({
          left: scroller.scrollLeft,
          top: scroller.scrollTop,
          events: __scrollbarDomEvents,
        })"""
    )
    control_scroll = None
    if is_moli:
        if state_after_drag.get("left", 0) <= 50 or state_after_drag.get("top", 0) <= 100:
            raise SmokeError(
                f"classic scrollbar thumb drag did not scroll both axes: {state_after_drag!r}"
            )
        assert_equal(
            state_after_drag.get("events"),
            [],
            "Moli UA scrollbar controls stay outside DOM pointer/mouse dispatch",
        )
        before_controls = float(state_after_drag["top"])
        await mouse("mousePressed", 190, 5, pressed=True)
        await mouse("mouseReleased", 190, 5, pressed=False)
        after_back = float(await page.evaluate("() => scroller.scrollTop"))
        await mouse("mousePressed", 190, 80, pressed=True)
        await mouse("mouseReleased", 190, 80, pressed=False)
        after_forward = float(await page.evaluate("() => scroller.scrollTop"))
        await mouse("mousePressed", 190, 60, pressed=True)
        await mouse("mouseReleased", 190, 60, pressed=False)
        after_track = float(await page.evaluate("() => scroller.scrollTop"))
        control_scroll = [before_controls, after_back, after_forward, after_track]
        expected_track = before_controls + 85 * 0.875
        if (
            abs(after_back - (before_controls - 40)) > 0.01
            or abs(after_forward - before_controls) > 0.01
            or abs(after_track - expected_track) > 0.01
        ):
            raise SmokeError(f"classic scrollbar button/track steps diverged: {control_scroll!r}")
        assert_equal(
            await page.evaluate("() => __scrollbarDomEvents"),
            [],
            "Moli scrollbar button and track stay outside DOM dispatch",
        )
    else:
        # Chromium 145 on Linux/Xvfb routes synthetic raw-CDP mouse input at
        # this painted control through DOM hit testing instead of its native
        # scrollbar widget. Keep that executable comparison explicit while
        # retaining Moli's useful CDP thumb-drag capability above.
        assert_equal(
            [state_after_drag.get("left"), state_after_drag.get("top")],
            [0, 0],
            "Chromium raw-CDP input does not drag the native scrollbar",
        )
        if not state_after_drag.get("events"):
            raise SmokeError(
                "Chromium raw-CDP scrollbar probe should route through DOM mouse events"
            )
    state.record(
        "classic_scrollbar_layout_paint_and_cdp_drag",
        {
            "engine": "moli" if is_moli else "chromium",
            "methods": ["Input.dispatchMouseEvent", "Page.captureScreenshot"],
            "metrics": metrics,
            "scroll": [state_after_drag.get("left"), state_after_drag.get("top")],
            "controlScroll": control_scroll,
            "cdpNativeDrag": is_moli,
        },
    )
    await _run_root_scrollbar_workflow(state, is_moli)


async def _run_root_scrollbar_workflow(state: SmokeState, is_moli: bool) -> None:
    page = state.page
    cdp = state.cdp
    await page.set_viewport_size({"width": 800, "height": 600})
    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; }
          html { scrollbar-color: rgb(255, 0, 0) rgb(0, 0, 255); }
        </style>
        <div style="width:200vw;height:200vh"></div>
        <script>
          window.__rootScrollbarDomEvents = [];
          for (const name of [
            "pointerdown", "pointermove", "pointerup",
            "mousedown", "mousemove", "mouseup", "click"
          ]) {
            document.addEventListener(name, () => __rootScrollbarDomEvents.push(name), true);
          }
        </script>
        """,
        wait_until="domcontentloaded",
    )
    metrics = await page.evaluate(
        """() => ({
          innerWidth,
          innerHeight,
          clientWidth: document.documentElement.clientWidth,
          clientHeight: document.documentElement.clientHeight,
          scrollWidth: document.documentElement.scrollWidth,
          scrollHeight: document.documentElement.scrollHeight,
        })"""
    )
    assert_equal(
        [
            metrics["innerWidth"] - metrics["clientWidth"],
            metrics["innerHeight"] - metrics["clientHeight"],
        ],
        [15, 15],
        "root classic scrollbar gutters",
    )
    if (
        metrics["scrollWidth"] <= metrics["clientWidth"]
        or metrics["scrollHeight"] <= metrics["clientHeight"]
    ):
        raise SmokeError(f"root scrollbar fixture did not overflow: {metrics!r}")

    image = decode_png(await page.screenshot())
    right = metrics["innerWidth"] - 5
    bottom = metrics["innerHeight"] - 5
    assert_equal(image.pixel(right, 30), (255, 0, 0, 255), "root vertical thumb paint")
    assert_equal(image.pixel(right, 500), (0, 0, 255, 255), "root vertical track paint")
    assert_equal(image.pixel(30, bottom), (255, 0, 0, 255), "root horizontal thumb paint")
    assert_equal(image.pixel(right, bottom), (0, 0, 255, 255), "root scrollbar corner paint")

    scroll = None
    if is_moli:
        x = metrics["innerWidth"] - 5
        y = metrics["innerHeight"] - 5

        async def mouse(event_type: str, mouse_x: int, mouse_y: int, *, pressed: bool) -> None:
            await cdp.send(
                "Input.dispatchMouseEvent",
                {
                    "type": event_type,
                    "x": mouse_x,
                    "y": mouse_y,
                    "button": "left" if event_type != "mouseMoved" else "none",
                    "buttons": 1 if pressed else 0,
                    "clickCount": 1,
                },
            )

        await mouse("mousePressed", x, 30, pressed=True)
        await mouse("mouseMoved", x, 100, pressed=True)
        await mouse("mouseReleased", x, 100, pressed=False)
        await mouse("mousePressed", 30, y, pressed=True)
        await mouse("mouseMoved", 100, y, pressed=True)
        await mouse("mouseReleased", 100, y, pressed=False)
        state_after_drag = await page.evaluate(
            "() => [window.scrollX, window.scrollY, __rootScrollbarDomEvents]"
        )
        if state_after_drag[0] <= 50 or state_after_drag[1] <= 50:
            raise SmokeError(
                f"root scrollbar thumb drag did not scroll both axes: {state_after_drag!r}"
            )
        assert_equal(
            state_after_drag[2],
            [],
            "Moli root scrollbar stays outside DOM pointer/mouse dispatch",
        )
        scroll = state_after_drag[:2]

    state.record(
        "root_classic_scrollbar_layout_and_cdp_drag",
        {"engine": "moli" if is_moli else "chromium", "metrics": metrics, "scroll": scroll},
    )

    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; height: 100%; }
          #wide { width: 1600px; height: 50%; }
          #fixed {
            position: fixed;
            right: 0;
            bottom: 0;
            width: 10px;
            height: 10px;
          }
        </style>
        <div id="wide"></div><div id="fixed"></div>
        """,
        wait_until="domcontentloaded",
    )
    containing_block = await page.evaluate(
        """() => {
          const html = document.documentElement.getBoundingClientRect();
          const body = document.body.getBoundingClientRect();
          const wide = document.getElementById("wide").getBoundingClientRect();
          const fixed = document.getElementById("fixed").getBoundingClientRect();
          return [
            document.documentElement.clientWidth,
            document.documentElement.clientHeight,
            document.documentElement.scrollWidth,
            document.documentElement.scrollHeight,
            html.x, html.width, html.height,
            body.x, body.width, body.height,
            wide.width, wide.height,
            fixed.x, fixed.y,
          ];
        }"""
    )
    assert_equal(
        containing_block,
        [800, 585, 1600, 585, 0, 800, 585, 0, 800, 585, 1600, 292.5, 790, 575],
        "root scrollbar resizes the initial containing block",
    )

    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; height: 100%; }
          html { overflow: auto; scrollbar-gutter: stable both-edges; }
        </style>
        <div id="content"></div>
        """,
        wait_until="domcontentloaded",
    )

    async def stable_metrics() -> list[float]:
        return await page.evaluate(
            """() => {
              const html = document.documentElement.getBoundingClientRect();
              const body = document.body.getBoundingClientRect();
              return [
                document.documentElement.clientWidth,
                document.documentElement.clientHeight,
                document.documentElement.scrollWidth,
                document.documentElement.scrollHeight,
                html.x, html.width, body.x, body.width,
              ];
            }"""
        )

    stable_empty = await stable_metrics()
    assert_equal(
        stable_empty,
        [800, 600, 770, 600, 15, 770, 15, 770],
        "empty root stable both-edges gutter",
    )
    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; height: 100%; }
          html { overflow: auto; scrollbar-gutter: stable both-edges; }
          #content { height: 1200px; }
        </style>
        <div id="content"></div>
        """,
        wait_until="domcontentloaded",
    )
    stable_overflow = await stable_metrics()
    assert_equal(
        stable_overflow,
        [785, 600, 770, 1200, 15, 770, 15, 770],
        "overflowing root stable both-edges gutter",
    )

    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; height: 100%; }
          html {
            direction: rtl;
            scrollbar-color: rgb(255, 0, 0) rgb(0, 0, 255);
          }
          #content { height: 1200px; }
        </style>
        <div id="content"></div>
        """,
        wait_until="domcontentloaded",
    )
    rtl_image = decode_png(await page.screenshot())
    assert_equal(
        rtl_image.pixel(795, 30),
        (255, 0, 0, 255),
        "RTL root scrollbar remains on the physical right",
    )
    assert_equal(
        rtl_image.pixel(5, 30),
        (255, 255, 255, 255),
        "RTL root does not paint a left-edge viewport scrollbar",
    )
    state.record(
        "root_scrollbar_initial_containing_block_and_gutters",
        {
            "engine": "moli" if is_moli else "chromium",
            "containingBlock": containing_block,
            "stableEmpty": stable_empty,
            "stableOverflow": stable_overflow,
            "rtlScrollbarSide": "right",
        },
    )
