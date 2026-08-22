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
    await page.set_viewport_size({"width": 800, "height": 700})
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
    assert_equal(
        (image.width, image.height),
        (800, 700),
        "classic scrollbar smoke owns its screenshot viewport",
    )
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
    await _run_multi_scroller_drag_workflow(state, is_moli)
    await _run_root_scrollbar_workflow(state, is_moli)
    await _run_painted_surface_workflow(state, is_moli)
    await _run_viewport_policy_and_numeric_gutter_workflow(state, is_moli)


async def _run_multi_scroller_drag_workflow(state: SmokeState, is_moli: bool) -> None:
    page = state.page
    cdp = state.cdp
    await page.set_viewport_size({"width": 800, "height": 600})
    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; overflow: hidden; }
          #outer {
            position: absolute; left: 20px; top: 20px;
            width: 280px; height: 200px; overflow: auto;
          }
          #outer-content { position: relative; width: 620px; height: 500px; }
          #inner {
            position: absolute; left: 30px; top: 30px;
            width: 180px; height: 120px; overflow: auto;
          }
          .large { width: 420px; height: 320px; }
          #sibling {
            position: absolute; left: 340px; top: 20px;
            width: 180px; height: 120px; overflow: auto;
          }
          #horizontal {
            position: absolute; left: 340px; top: 180px;
            width: 220px; height: 100px; overflow: auto;
          }
          #horizontal > div { width: 600px; height: 60px; }
          #thin-child {
            position: absolute; left: 600px; top: 20px;
            width: 160px; height: 120px; overflow: auto; scrollbar-width: thin;
          }
          #rtl-child {
            position: absolute; left: 600px; top: 180px;
            width: 160px; height: 120px; overflow: auto; direction: rtl;
          }
        </style>
        <div id="outer">
          <div id="outer-content">
            <div id="inner"><div class="large"></div></div>
          </div>
        </div>
        <div id="sibling"><div class="large"></div></div>
        <div id="horizontal"><div></div></div>
        <div id="thin-child"><div style="width:400px;height:320px"></div></div>
        <div id="rtl-child"><div style="width:400px;height:320px"></div></div>
        <script>
          window.__multiScrollbarDomEvents = [];
          for (const name of [
            "pointerdown", "pointermove", "pointerup",
            "mousedown", "mousemove", "mouseup", "click"
          ]) {
            document.addEventListener(name, () => __multiScrollbarDomEvents.push(name), true);
          }
        </script>
        """,
        wait_until="domcontentloaded",
    )
    metrics = await page.evaluate(
        """() => {
          const outer = document.getElementById("outer");
          const inner = document.getElementById("inner");
          const sibling = document.getElementById("sibling");
          const horizontal = document.getElementById("horizontal");
          const thin = document.getElementById("thin-child");
          const rtl = document.getElementById("rtl-child");
          return [
            outer.clientWidth, outer.clientHeight, outer.scrollWidth, outer.scrollHeight,
            inner.clientWidth, inner.clientHeight, inner.scrollWidth, inner.scrollHeight,
            sibling.clientWidth, sibling.clientHeight,
            horizontal.clientWidth, horizontal.clientHeight,
            horizontal.scrollWidth, horizontal.scrollHeight,
            thin.clientWidth, thin.clientHeight,
            rtl.clientWidth, rtl.clientHeight, rtl.clientLeft, rtl.scrollLeft,
          ];
        }"""
    )
    assert_equal(
        metrics,
        [
            265, 185, 620, 500, 165, 105, 420, 320, 165, 105,
            220, 85, 600, 85, 150, 110, 145, 105, 15, 0,
        ],
        "nested and sibling classic scrollbar geometry",
    )

    scroll = None
    if is_moli:
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

        # Release over a sibling scrollbar: capture must remain on the inner
        # thumb until mouseReleased and must not leak input into either DOM.
        await mouse("mousePressed", 225, 75, pressed=True)
        await mouse("mouseMoved", 515, 115, pressed=True)
        await mouse("mouseReleased", 515, 115, pressed=False)
        first_drag = await page.evaluate(
            """() => [
              document.getElementById("inner").scrollTop,
              document.getElementById("outer").scrollTop,
              document.getElementById("sibling").scrollTop,
            ]"""
        )
        if first_drag[0] <= 180 or first_drag[1:] != [0, 0]:
            raise SmokeError(f"nested scrollbar capture retargeted unexpectedly: {first_drag!r}")

        await mouse("mousePressed", 515, 45, pressed=True)
        await mouse("mouseMoved", 515, 85, pressed=True)
        await mouse("mouseReleased", 515, 85, pressed=False)
        await mouse("mousePressed", 370, 275, pressed=True)
        await mouse("mouseMoved", 450, 275, pressed=True)
        await mouse("mouseReleased", 450, 275, pressed=False)

        await mouse("mousePressed", 755, 45, pressed=True)
        await mouse("mouseMoved", 755, 75, pressed=True)
        await mouse("mouseReleased", 755, 75, pressed=False)
        await mouse("mousePressed", 630, 135, pressed=True)
        await mouse("mouseMoved", 690, 135, pressed=True)
        await mouse("mouseReleased", 690, 135, pressed=False)

        await mouse("mousePressed", 607, 205, pressed=True)
        await mouse("mouseMoved", 607, 235, pressed=True)
        await mouse("mouseReleased", 607, 235, pressed=False)
        await mouse("mousePressed", 720, 292, pressed=True)
        await mouse("mouseMoved", 660, 292, pressed=True)
        await mouse("mouseReleased", 660, 292, pressed=False)

        await page.evaluate(
            """() => {
              document.getElementById("outer").scrollTop = 50;
              document.getElementById("inner").scrollTop = 0;
            }"""
        )
        await mouse("mousePressed", 225, 25, pressed=True)
        await mouse("mouseMoved", 700, 45, pressed=True)
        await mouse("mouseReleased", 700, 45, pressed=False)
        await mouse("mousePressed", 295, 65, pressed=True)
        await mouse("mouseMoved", 295, 125, pressed=True)
        await mouse("mouseReleased", 295, 125, pressed=False)

        state_after = await page.evaluate(
            """() => ({
              outer: document.getElementById("outer").scrollTop,
              inner: document.getElementById("inner").scrollTop,
              sibling: document.getElementById("sibling").scrollTop,
              horizontal: document.getElementById("horizontal").scrollLeft,
              thinLeft: document.getElementById("thin-child").scrollLeft,
              thinTop: document.getElementById("thin-child").scrollTop,
              rtlLeft: document.getElementById("rtl-child").scrollLeft,
              rtlTop: document.getElementById("rtl-child").scrollTop,
              events: __multiScrollbarDomEvents,
            })"""
        )
        if (
            state_after["outer"] <= 200
            or state_after["inner"] <= 90
            or state_after["sibling"] <= 180
            or state_after["horizontal"] <= 250
            or state_after["thinLeft"] <= 180
            or state_after["thinTop"] <= 100
            or state_after["rtlLeft"] >= -180
            or state_after["rtlTop"] <= 130
        ):
            raise SmokeError(f"multi-scroller thumb drag matrix diverged: {state_after!r}")
        assert_equal(
            state_after["events"],
            [],
            "captured nested and sibling scrollbars stay outside DOM dispatch",
        )
        scroll = {
            key: state_after[key]
            for key in (
                "outer", "inner", "sibling", "horizontal",
                "thinLeft", "thinTop", "rtlLeft", "rtlTop",
            )
        }

    state.record(
        "nested_sibling_and_horizontal_scrollbar_drag_matrix",
        {
            "engine": "moli" if is_moli else "chromium",
            "metrics": metrics,
            "scroll": scroll,
            "cdpNativeDrag": is_moli,
        },
    )


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


async def _run_painted_surface_workflow(state: SmokeState, is_moli: bool) -> None:
    page = state.page
    cdp = state.cdp
    await page.set_viewport_size({"width": 800, "height": 600})
    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; }
          #scroller {
            position: absolute; left: 20px; top: 20px;
            width: 200px; height: 100px; overflow: scroll;
          }
          #large { width: 400px; height: 300px; }
          #overlay {
            position: absolute; left: 200px; top: 20px;
            width: 40px; height: 100px; z-index: 10;
          }
        </style>
        <div id="scroller"><div id="large"></div></div>
        <div id="overlay"></div>
        <script>
          window.__paintedSurfaceEvents = [];
          for (const name of [
            "pointerdown", "mousedown", "pointerup", "mouseup", "click"
          ]) {
            document.addEventListener(name, event => {
              __paintedSurfaceEvents.push(`${name}:${event.target.id || event.target.localName}`);
            }, true);
          }
        </script>
        """,
        wait_until="domcontentloaded",
    )
    await page.evaluate(
        """() => {
          scroller.scrollTop = 80;
          return [scroller.clientWidth, scroller.clientHeight, overlay.getBoundingClientRect().x];
        }"""
    )

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

    # This point is simultaneously inside the overlay and the vertical
    # scrollbar beneath it. The ordinary painted sibling must win.
    await mouse("mousePressed", 210, 30, pressed=True)
    await mouse("mouseReleased", 210, 30, pressed=False)
    covered = await page.evaluate(
        "() => [scroller.scrollTop, __paintedSurfaceEvents.slice()]"
    )
    assert_equal(covered[0], 80, "covered scrollbar does not steal overlay input")
    assert_equal(
        covered[1],
        [
            "pointerdown:overlay",
            "mousedown:overlay",
            "pointerup:overlay",
            "mouseup:overlay",
            "click:overlay",
        ],
        "topmost painted overlay receives pointer and mouse dispatch",
    )

    await page.evaluate(
        """() => {
          overlay.style.display = "none";
          __paintedSurfaceEvents.length = 0;
          return scroller.getBoundingClientRect().width;
        }"""
    )
    # Moli intentionally caps rendering at 50 ms. A screenshot is an explicit
    # render trigger, so the second input probes the new painted state instead
    # of the still-valid throttled snapshot from the first click.
    await page.screenshot()
    # The lower-right 15x15 intersection is painted UA chrome. Moli exposes
    # it as a consume-only control surface rather than a DOM target.
    await mouse("mousePressed", 210, 110, pressed=True)
    await mouse("mouseReleased", 210, 110, pressed=False)
    corner = await page.evaluate(
        "() => [scroller.scrollTop, __paintedSurfaceEvents.slice()]"
    )
    assert_equal(corner[0], 80, "scrollbar corner has no scroll action")
    if is_moli:
        assert_equal(
            corner[1],
            [],
            "Moli painted scrollbar corner stays outside DOM dispatch",
        )

    state.record(
        "paint_ordered_scrollbar_and_corner_input_surfaces",
        {
            "engine": "moli" if is_moli else "chromium",
            "covered": covered,
            "corner": corner,
            "cornerConsumedOutsideDom": is_moli,
        },
    )


async def _run_viewport_policy_and_numeric_gutter_workflow(
    state: SmokeState, is_moli: bool
) -> None:
    page = state.page
    cdp = state.cdp
    await page.set_viewport_size({"width": 800, "height": 600})
    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; }
          .scroller {
            width: 200px; height: 100px;
            overflow: scroll;
            scrollbar-gutter: stable both-edges;
          }
          .scroller > div { width: 100%; height: 100%; }
          #flex { display: flex; }
          #grid { display: grid; }
        </style>
        <div id="block" class="scroller"><div></div></div>
        <div id="flex" class="scroller"><div></div></div>
        <div id="grid" class="scroller"><div></div></div>
        <div id="overflow" class="scroller"><div style="width:400px;height:200px"></div></div>
        """,
        wait_until="domcontentloaded",
    )
    both_edges = await page.evaluate(
        """() => ["block", "flex", "grid", "overflow"].map(id => {
          const scroller = document.getElementById(id);
          const child = scroller.firstElementChild;
          return [
            scroller.clientWidth, scroller.clientHeight,
            scroller.scrollWidth, scroller.scrollHeight,
            child.offsetWidth, child.offsetHeight,
          ];
        })"""
    )
    assert_equal(
        both_edges,
        [
            [170, 85, 170, 85, 170, 85],
            [170, 85, 170, 85, 170, 85],
            [170, 85, 170, 85, 170, 85],
            [170, 85, 400, 200, 400, 200],
        ],
        "both-edge gutters participate in numeric layout without inflating scroll ranges",
    )

    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; }
          .case {
            margin: 0;
            overflow: scroll;
            scrollbar-gutter: stable both-edges;
          }
          .case > div { width: 100%; height: 100%; }
          #border {
            box-sizing: border-box;
            width: 200px; height: 100px;
            padding: 10px; border: 5px solid;
          }
          #content {
            box-sizing: content-box;
            width: 200px; height: 100px;
            padding: 10px; border: 5px solid;
          }
          .auto {
            box-sizing: border-box;
            width: 200px; height: auto;
            min-height: 80px; max-height: 120px;
          }
          #auto-small > div { width: 400px; height: 20px; }
          #auto-mid > div { width: 400px; height: 90px; }
          #auto-large > div { width: 400px; height: 150px; }
          #ratio {
            box-sizing: border-box;
            width: 200px; height: auto;
            aspect-ratio: 2;
          }
          #vertical {
            box-sizing: border-box;
            width: 200px; height: 100px;
            writing-mode: vertical-rl;
          }
        </style>
        <div id="border" class="case"><div></div></div>
        <div id="content" class="case"><div></div></div>
        <div id="auto-small" class="case auto"><div></div></div>
        <div id="auto-mid" class="case auto"><div></div></div>
        <div id="auto-large" class="case auto"><div></div></div>
        <div id="ratio" class="case"><div></div></div>
        <div id="vertical" class="case"><div></div></div>
        """,
        wait_until="domcontentloaded",
    )
    resolved_edge_insets = await page.evaluate(
        """() => [
          "border", "content", "auto-small", "auto-mid", "auto-large", "ratio", "vertical"
        ].map(id => {
          const scroller = document.getElementById(id);
          const child = scroller.firstElementChild;
          const outer = scroller.getBoundingClientRect();
          const inner = child.getBoundingClientRect();
          return [
            scroller.offsetWidth, scroller.offsetHeight,
            scroller.clientWidth, scroller.clientHeight,
            scroller.clientLeft, scroller.clientTop,
            inner.x - outer.x, inner.y - outer.y, inner.width, inner.height,
          ];
        })"""
    )
    assert_equal(
        resolved_edge_insets,
        [
            [200, 100, 160, 75, 20, 5, 30, 15, 140, 55],
            [230, 130, 190, 105, 20, 5, 30, 15, 170, 85],
            [200, 80, 170, 65, 15, 0, 15, 0, 400, 20],
            [200, 105, 170, 90, 15, 0, 15, 0, 400, 90],
            [200, 120, 170, 105, 15, 0, 15, 0, 400, 150],
            [200, 100, 170, 85, 15, 0, 15, 0, 170, 85],
            [200, 100, 185, 70, 0, 15, 0, 15, 185, 70],
        ],
        "physical scrollbar insets participate in box sizing, intrinsic sizing, aspect ratio, and vertical writing mode",
    )

    await page.set_content(
        """
        <!doctype html>
        <style>
          html { margin: 0; overflow: visible; }
          body { margin: 0; overflow: hidden; }
          main { height: 1200px; }
        </style>
        <main></main>
        """,
        wait_until="domcontentloaded",
    )

    async def wheel(delta_y: int) -> None:
        await cdp.send(
            "Input.dispatchMouseEvent",
            {
                "type": "mouseWheel",
                "x": 10,
                "y": 10,
                "deltaX": 0,
                "deltaY": delta_y,
            },
        )

    hidden_before = await page.evaluate(
        """() => {
          scrollTo(0, 0);
          return [
            document.documentElement.clientWidth,
            document.documentElement.scrollHeight,
            scrollY,
          ];
        }"""
    )
    assert_equal(hidden_before, [800, 1200, 0], "body hidden propagates to viewport")
    await wheel(80)
    await page.wait_for_timeout(100)
    hidden_wheel = await page.evaluate("() => scrollY")
    assert_equal(hidden_wheel, 0, "body hidden disables user viewport scrolling")
    hidden_script = await page.evaluate("() => { scrollTo(0, 100); return scrollY; }")
    assert_equal(hidden_script, 100, "body hidden retains script viewport scrolling")

    auto_width = await page.evaluate(
        """() => {
          document.body.style.overflow = "auto";
          scrollTo(0, 0);
          return document.documentElement.clientWidth;
        }"""
    )
    assert_equal(auto_width, 785, "body auto exposes the viewport scrollbar")
    await wheel(80)
    await page.wait_for_function("() => scrollY > 0", timeout=2_000)
    auto_wheel = await page.evaluate("() => scrollY")
    assert_equal(auto_wheel, 80, "body auto permits user viewport scrolling")

    clip_width = await page.evaluate(
        """() => {
          document.body.style.overflow = "clip";
          void document.documentElement.clientWidth;
          scrollTo(0, 100);
          return document.documentElement.clientWidth;
        }"""
    )
    assert_equal(clip_width, 800, "body clip maps to hidden at the viewport")
    await wheel(80)
    await page.wait_for_timeout(100)
    clip_wheel = await page.evaluate("() => scrollY")
    assert_equal(clip_wheel, 100, "clip-derived viewport disables user scrolling")

    await page.set_content(
        """
        <!doctype html>
        <style>
          html { margin: 0; overflow: visible; scrollbar-gutter: stable; }
          body { margin: 0; }
        </style>
        """,
        wait_until="domcontentloaded",
    )

    async def root_gutter_metrics() -> list[float]:
        return await page.evaluate(
            """() => {
              const html = document.documentElement.getBoundingClientRect();
              const body = document.body.getBoundingClientRect();
              return [
                document.documentElement.clientWidth,
                html.x, html.width, body.x, body.width,
              ];
            }"""
        )

    stable = await root_gutter_metrics()
    assert_equal(stable, [800, 0, 785, 0, 785], "default-visible root stable gutter")
    await page.evaluate(
        "() => { document.documentElement.style.scrollbarGutter = 'stable both-edges'; }"
    )
    await page.screenshot()
    both_edge_root = await root_gutter_metrics()
    assert_equal(
        both_edge_root,
        [800, 15, 770, 15, 770],
        "default-visible root stable both-edge gutters",
    )

    await page.set_content(
        """
        <!doctype html>
        <style>
          html { margin: 0; overflow: visible; }
          body { display: contents; overflow: scroll; }
        </style>
        """,
        wait_until="domcontentloaded",
    )
    display_contents_width = await page.evaluate(
        "() => document.documentElement.getBoundingClientRect().width"
    )
    assert_equal(
        display_contents_width,
        800,
        "display:contents body does not propagate overflow to viewport",
    )
    await page.evaluate("() => { document.body.style.display = 'block'; }")
    await page.screenshot()
    principal_body_width = await page.evaluate(
        "() => document.documentElement.getBoundingClientRect().width"
    )
    assert_equal(
        principal_body_width,
        785,
        "principal body propagates overflow:scroll to viewport",
    )

    state.record(
        "viewport_overflow_policy_and_numeric_scrollbar_gutters",
        {
            "engine": "moli" if is_moli else "chromium",
            "bothEdges": both_edges,
            "resolvedEdgeInsets": resolved_edge_insets,
            "bodyHidden": [hidden_before, hidden_wheel, hidden_script],
            "bodyAuto": [auto_width, auto_wheel],
            "bodyClip": [clip_width, clip_wheel],
            "rootStable": stable,
            "rootBothEdges": both_edge_root,
            "displayContentsBody": display_contents_width,
            "principalBody": principal_body_width,
        },
    )
