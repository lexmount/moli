from __future__ import annotations

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..raw_cdp import discover_websocket_url


def _assert_close(actual: object, expected: float, label: str, tolerance: float = 0.15) -> None:
    if not isinstance(actual, (int, float)) or abs(float(actual) - expected) > tolerance:
        raise SmokeError(
            f"{label}: expected {expected!r} ± {tolerance}, got {actual!r}"
        )


async def run_iframe_input_group(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp
    await page.set_viewport_size({"width": 1200, "height": 800})
    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; padding: 0; min-height: 1400px; }
          #outside {
            position: fixed; left: 0; top: 0; width: 30px; height: 30px;
          }
          #clip {
            position: absolute; left: 100px; top: 120px;
            width: 720px; height: 500px; overflow: hidden;
          }
          #input-frame {
            display: block;
            width: calc(100% / .78);
            height: calc(500px / .78);
            margin: 0; border: 0; padding: 0;
            transform: scale(.78);
            transform-origin: 0 0;
          }
        </style>
        <div id="outside"></div>
        <div id="clip"><iframe id="input-frame"></iframe></div>
        <script>
          const frame = document.getElementById("input-frame");
          const child = frame.contentDocument;
          child.documentElement.style.cssText = "margin:0;padding:0";
          child.body.style.cssText = "margin:0;padding:0";
          const style = child.createElement("style");
          style.textContent = `
            #hover-target {
              position: fixed; left: 220px; top: 50px;
              width: 40px; height: 80px; background: red;
              margin: 0; border: 0; padding: 0;
            }
            #hover-target:hover { background: lime; }
            #wheel-target {
              position: fixed; left: 500px; top: 180px;
              width: 120px; height: 120px; overflow: auto;
            }
            #wheel-content { width: 100px; height: 900px; }
          `;
          child.head.appendChild(style);
          child.body.innerHTML = `
            <button id="hover-target">Run</button>
            <div id="wheel-target"><div id="wheel-content"></div></div>
          `;
          frame.contentWindow.__inputEvents = [];
          for (const type of [
            "pointerover", "pointerout", "mouseover", "mouseout",
            "mousemove", "mousedown", "mouseup", "click", "focus", "wheel"
          ]) {
            child.addEventListener(type, event => {
              frame.contentWindow.__inputEvents.push({
                type,
                target: event.target.id,
                clientX: event.clientX,
                clientY: event.clientY,
                deltaY: event.deltaY || 0,
              });
            }, true);
          }
        </script>
        """,
        wait_until="domcontentloaded",
    )

    geometry = await page.evaluate(
        """() => {
          const frame = document.getElementById("input-frame");
          const rect = frame.getBoundingClientRect();
          return {
            offset: [frame.offsetWidth, frame.offsetHeight],
            rect: [rect.left, rect.top, rect.width, rect.height],
            childViewport: [frame.contentWindow.innerWidth, frame.contentWindow.innerHeight],
          };
        }"""
    )
    assert_equal(geometry["offset"], [923, 641], "transformed iframe used size")
    assert_equal(
        geometry["childViewport"],
        [923, 641],
        "child LocalFrameView uses the iframe's exact content viewport",
    )
    for actual, expected, label in zip(
        geometry["rect"],
        [100.0, 120.0, 719.94, 499.98],
        ["left", "top", "width", "height"],
    ):
        _assert_close(actual, expected, f"transformed iframe visual {label}")

    async def mouse(
        event_type: str,
        x: float,
        y: float,
        *,
        button: str = "none",
        buttons: int = 0,
        delta_y: float | None = None,
    ) -> None:
        params: dict[str, object] = {
            "type": event_type,
            "x": x,
            "y": y,
            "button": button,
            "buttons": buttons,
            "clickCount": 1,
        }
        if delta_y is not None:
            params.update({"deltaX": 0, "deltaY": delta_y})
        await cdp.send("Input.dispatchMouseEvent", params)

    # Child point (240, 90), the visible target center, maps through .78 to
    # root point (287.2, 190.2).
    await mouse("mouseMoved", 287.2, 190.2)
    hover_probe = await page.evaluate(
        """() => {
          const child = document.getElementById("input-frame").contentDocument;
          return {
            hovered: child.getElementById("hover-target").matches(":hover"),
            events: document.getElementById("input-frame").contentWindow.__inputEvents,
            childHit: child.elementFromPoint(240, 90)?.id || null,
          };
        }"""
    )
    assert_equal(hover_probe["childHit"], "hover-target", "child target fixture hit")
    hover_event = next(
        (
            event
            for event in hover_probe["events"]
            if event["type"] == "mouseover" and event["target"] == "hover-target"
        ),
        None,
    )
    if hover_event is None:
        raise SmokeError(f"transformed iframe hover target was missed: {hover_probe!r}")
    await mouse("mousePressed", 287.2, 190.2, button="left", buttons=1)
    await mouse("mouseReleased", 287.2, 190.2, button="left", buttons=0)

    # Child point (560, 240) lies in the nested overflow element.
    await mouse("mouseWheel", 536.8, 307.2, delta_y=120)

    async def wheel_applied() -> bool:
        return bool(
            await page.evaluate(
                """() =>
                  document.getElementById("input-frame").contentDocument
                    .getElementById("wheel-target").scrollTop === 120"""
            )
        )

    await wait_until(wheel_applied, "transformed iframe wheel action", timeout_ms=3_000)
    before_exit = await page.evaluate(
        """() => {
          const frame = document.getElementById("input-frame");
          const child = frame.contentDocument;
          return {
            wheelTop: child.getElementById("wheel-target").scrollTop,
            rootTop: document.scrollingElement.scrollTop,
            events: frame.contentWindow.__inputEvents,
          };
        }"""
    )
    assert_equal(before_exit["wheelTop"], 120, "child overflow scrollTop")
    assert_equal(before_exit["rootTop"], 0, "top document remains unscrolled")

    events = before_exit["events"]
    for event_type in ["mousemove", "mousedown", "mouseup", "click"]:
        event = next(
            (
                candidate
                for candidate in events
                if candidate["type"] == event_type
                and candidate["target"] == "hover-target"
            ),
            None,
        )
        if event is None:
            raise SmokeError(f"missing child {event_type} event: {events!r}")
        _assert_close(event["clientX"], 240.0, f"child {event_type}.clientX")
        _assert_close(event["clientY"], 90.0, f"child {event_type}.clientY")
    wheel = next((event for event in events if event["type"] == "wheel"), None)
    if wheel is None:
        raise SmokeError(f"missing child wheel event: {events!r}")
    assert_equal(wheel["target"], "wheel-content", "child wheel target")
    _assert_close(wheel["clientX"], 560.0, "child wheel.clientX")
    _assert_close(wheel["clientY"], 240.0, "child wheel.clientY")

    await mouse("mouseMoved", 10.0, 10.0)
    mouseout = await page.evaluate(
        """() => {
          const events = document.getElementById("input-frame").contentWindow.__inputEvents;
          return [...events].reverse().find(event => event.type === "mouseout") || null;
        }"""
    )
    if mouseout is None:
        raise SmokeError("moving from the child frame did not dispatch mouseout")
    assert_equal(mouseout["target"], "hover-target", "child mouseout target")
    # Chromium rounds cross-frame boundary-event coordinates to integral CSS
    # pixels; Moli retains the subpixel affine result.
    _assert_close(mouseout["clientX"], -115.3846, "child mouseout.clientX", 1.1)
    _assert_close(mouseout["clientY"], -141.0256, "child mouseout.clientY", 1.1)

    # Put the same transformed iframe partly below the top viewport. Its
    # button remains visible, so focusing the button must bubble that button's
    # rect to the parent rather than scrolling the entire iframe into view.
    await page.evaluate(
        """() => {
          const clip = document.getElementById("clip");
          const frame = document.getElementById("input-frame");
          clip.style.top = "400px";
          frame.contentWindow.__inputEvents = [];
          window.scrollTo(0, 0);
        }"""
    )

    async def frame_repositioned() -> bool:
        top = await page.evaluate(
            """() => document.getElementById("input-frame")
              .getBoundingClientRect().top"""
        )
        return isinstance(top, (int, float)) and abs(float(top) - 400.0) < 0.15

    await wait_until(frame_repositioned, "partially hidden iframe layout", timeout_ms=3_000)
    await mouse("mouseMoved", 287.2, 470.2)
    await mouse("mousePressed", 287.2, 470.2, button="left", buttons=1)
    scroll_after_press = await page.evaluate("() => window.scrollY")
    assert_equal(
        scroll_after_press,
        0,
        "focusing a visible child target does not scroll the whole iframe",
    )
    await mouse("mouseReleased", 287.2, 470.2, button="left", buttons=0)
    focus_probe = await page.evaluate(
        """() => {
          const frame = document.getElementById("input-frame");
          const child = frame.contentDocument;
          return {
            rootTop: window.scrollY,
            parentActive: document.activeElement === frame,
            childActive: child.activeElement === child.getElementById("hover-target"),
            events: frame.contentWindow.__inputEvents,
          };
        }"""
    )
    assert_equal(focus_probe["rootTop"], 0, "top document stays still through release")
    assert_equal(focus_probe["parentActive"], True, "parent active element is iframe")
    assert_equal(focus_probe["childActive"], True, "child button keeps focus")
    for event_type in ["mousedown", "mouseup", "click"]:
        event = next(
            (
                candidate
                for candidate in focus_probe["events"]
                if candidate["type"] == event_type
                and candidate["target"] == "hover-target"
            ),
            None,
        )
        if event is None:
            raise SmokeError(
                f"missing stable child {event_type} after focus: {focus_probe!r}"
            )
        _assert_close(event["clientX"], 240.0, f"focused child {event_type}.clientX")
        _assert_close(event["clientY"], 90.0, f"focused child {event_type}.clientY")

    state.record(
        "transformed_iframe_hover_click_wheel_coordinates",
        {
            "methods": ["Input.dispatchMouseEvent", "Runtime.evaluate"],
            "geometry": geometry,
            "wheelTop": before_exit["wheelTop"],
            "mouseout": mouseout,
            "focusWithoutParentScroll": focus_probe,
        },
    )
    await _run_nested_iframe_input_workflow(state)


async def _run_nested_iframe_input_workflow(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp
    websocket_url = await discover_websocket_url(state.endpoint)
    is_moli = websocket_url.endswith("/devtools/browser/moli-browser")
    await page.set_viewport_size({"width": 800, "height": 600})
    await page.set_content(
        """
        <!doctype html>
        <style>
          html, body { margin: 0; padding: 0; }
          #outside {
            position: fixed; left: 0; top: 0; width: 30px; height: 30px;
          }
          #parent-scroller {
            position: absolute; left: 40px; top: 30px;
            width: 400px; height: 300px; overflow: auto;
          }
          #canvas { position: relative; width: 800px; height: 600px; }
          #outer-frame {
            position: absolute; left: 100px; top: 80px;
            display: block; box-sizing: border-box;
            width: 240px; height: 180px; margin: 0;
            border: 4px solid black; padding: 6px;
            transform: scale(.75); transform-origin: 0 0;
          }
        </style>
        <div id="outside"></div>
        <div id="parent-scroller">
          <div id="canvas"></div>
        </div>
        """,
        wait_until="domcontentloaded",
    )
    await page.evaluate(
        """() => {
          const parentScroller = document.getElementById("parent-scroller");
          const outerFrame = document.createElement("iframe");
          outerFrame.id = "outer-frame";
          document.getElementById("canvas").appendChild(outerFrame);
          const child = outerFrame.contentDocument;
          child.documentElement.style.cssText = "margin:0;padding:0";
          child.body.style.cssText = "margin:0;padding:0";

          const nestedFrame = child.createElement("iframe");
          nestedFrame.id = "nested-frame";
          nestedFrame.style.cssText = [
            "position:absolute", "left:40px", "top:30px", "display:block",
            "box-sizing:border-box", "width:100px", "height:80px", "margin:0",
            "border:2px solid black", "padding:3px",
            "transform:scale(.5)", "transform-origin:0 0"
          ].join(";");
          child.body.appendChild(nestedFrame);

          const nested = nestedFrame.contentDocument;
          nested.documentElement.style.cssText = "margin:0;padding:0";
          nested.body.style.cssText = "margin:0;padding:0";
          nested.body.innerHTML = `
            <button id="nested-target" style="position:fixed;left:10px;top:10px;width:20px;height:20px;margin:0;border:0;padding:0">Go</button>
            <div id="nested-scroll" style="position:fixed;left:10px;top:30px;width:50px;height:40px;overflow:auto">
              <div id="nested-scroll-content" style="width:200px;height:200px"></div>
            </div>
          `;
          nestedFrame.contentWindow.__nestedInputEvents = [];
          for (const type of [
            "pointerover", "pointerout", "mouseover", "mouseout",
            "mousemove", "mousedown", "mouseup", "click", "focus", "wheel"
          ]) {
            nested.addEventListener(type, event => {
              nestedFrame.contentWindow.__nestedInputEvents.push({
                type,
                target: event.target.id,
                clientX: event.clientX,
                clientY: event.clientY,
                deltaX: event.deltaX || 0,
                deltaY: event.deltaY || 0,
              });
            }, true);
          }
          parentScroller.scrollTo(50, 40);
        }"""
    )

    geometry = await page.evaluate(
        """() => {
          const parentScroller = document.getElementById("parent-scroller");
          const outer = document.getElementById("outer-frame");
          const child = outer.contentDocument;
          const nested = child.getElementById("nested-frame");
          const outerRect = outer.getBoundingClientRect();
          const nestedRect = nested.getBoundingClientRect();
          return {
            parentScroll: [parentScroller.scrollLeft, parentScroller.scrollTop],
            outerViewport: [outer.contentWindow.innerWidth, outer.contentWindow.innerHeight],
            nestedViewport: [nested.contentWindow.innerWidth, nested.contentWindow.innerHeight],
            outerRect: [outerRect.left, outerRect.top, outerRect.width, outerRect.height],
            nestedRect: [nestedRect.left, nestedRect.top, nestedRect.width, nestedRect.height],
          };
        }"""
    )
    assert_equal(geometry["parentScroll"], [50, 40], "nested fixture parent scroll")
    assert_equal(geometry["outerViewport"], [220, 160], "outer iframe content viewport")
    assert_equal(geometry["nestedViewport"], [90, 70], "nested iframe content viewport")
    for actual, expected, label in zip(
        geometry["outerRect"],
        [90.0, 70.0, 180.0, 135.0],
        ["left", "top", "width", "height"],
    ):
        _assert_close(actual, expected, f"outer transformed iframe {label}")
    for actual, expected, label in zip(
        geometry["nestedRect"],
        [40.0, 30.0, 50.0, 40.0],
        ["left", "top", "width", "height"],
    ):
        _assert_close(actual, expected, f"nested transformed iframe {label}")

    async def mouse(
        event_type: str,
        x: float,
        y: float,
        *,
        button: str = "none",
        buttons: int = 0,
        delta_x: float | None = None,
        delta_y: float | None = None,
    ) -> None:
        params: dict[str, object] = {
            "type": event_type,
            "x": x,
            "y": y,
            "button": button,
            "buttons": buttons,
            "clickCount": 1,
        }
        if delta_x is not None or delta_y is not None:
            params.update({"deltaX": delta_x or 0, "deltaY": delta_y or 0})
        await cdp.send("Input.dispatchMouseEvent", params)

    # Nested client point (20, 20) crosses both frame transforms, each
    # border+padding edge, and the top document's scrolled overflow ancestor.
    await mouse("mouseMoved", 136.875, 109.375)
    await mouse("mousePressed", 136.875, 109.375, button="left", buttons=1)
    await mouse("mouseReleased", 136.875, 109.375, button="left", buttons=0)

    async def nested_click_applied() -> bool:
        return bool(
            await page.evaluate(
                """() => {
                  const outer = document.getElementById("outer-frame");
                  const frame = outer.contentDocument.getElementById("nested-frame");
                  return frame.contentWindow.__nestedInputEvents
                    .some(event => event.type === "click" && event.target === "nested-target");
                }"""
            )
        )

    await wait_until(nested_click_applied, "nested iframe click", timeout_ms=3_000)
    click_probe = await page.evaluate(
        """() => {
          const outer = document.getElementById("outer-frame");
          const child = outer.contentDocument;
          const frame = child.getElementById("nested-frame");
          const nested = frame.contentDocument;
          return {
            parentActive: document.activeElement === outer,
            childActive: child.activeElement === frame,
            nestedActive: nested.activeElement === nested.getElementById("nested-target"),
            parentScroll: [
              document.getElementById("parent-scroller").scrollLeft,
              document.getElementById("parent-scroller").scrollTop,
            ],
            events: frame.contentWindow.__nestedInputEvents,
          };
        }"""
    )
    assert_equal(
        [
            click_probe["parentActive"],
            click_probe["childActive"],
            click_probe["nestedActive"],
        ],
        [True, True, True],
        "nested focus chain",
    )
    assert_equal(click_probe["parentScroll"], [50, 40], "nested click keeps parent scroll")
    for event_type in [
        "pointerover",
        "mouseover",
        "mousemove",
        "mousedown",
        "mouseup",
        "click",
    ]:
        event = next(
            (
                candidate
                for candidate in click_probe["events"]
                if candidate["type"] == event_type
                and candidate["target"] == "nested-target"
            ),
            None,
        )
        if event is None:
            raise SmokeError(f"missing nested {event_type}: {click_probe!r}")
        _assert_close(event["clientX"], 20.0, f"nested {event_type}.clientX")
        _assert_close(event["clientY"], 20.0, f"nested {event_type}.clientY")

    # Nested client point (20, 35) lies in the innermost overflow content.
    await mouse("mouseWheel", 136.875, 115.0, delta_x=25, delta_y=30)

    async def nested_wheel_applied() -> bool:
        return bool(
            await page.evaluate(
                """() => {
                  const outer = document.getElementById("outer-frame");
                  const frame = outer.contentDocument.getElementById("nested-frame");
                  const scroller = frame.contentDocument.getElementById("nested-scroll");
                  return scroller.scrollLeft === 25 && scroller.scrollTop === 30;
                }"""
            )
        )

    await wait_until(nested_wheel_applied, "nested iframe wheel", timeout_ms=3_000)
    wheel_probe = await page.evaluate(
        """() => {
          const outer = document.getElementById("outer-frame");
          const frame = outer.contentDocument.getElementById("nested-frame");
          const nested = frame.contentDocument;
          return {
            nestedScroll: [
              nested.getElementById("nested-scroll").scrollLeft,
              nested.getElementById("nested-scroll").scrollTop,
            ],
            parentScroll: [
              document.getElementById("parent-scroller").scrollLeft,
              document.getElementById("parent-scroller").scrollTop,
            ],
            events: frame.contentWindow.__nestedInputEvents,
          };
        }"""
    )
    assert_equal(wheel_probe["nestedScroll"], [25, 30], "nested overflow wheel scroll")
    assert_equal(wheel_probe["parentScroll"], [50, 40], "nested wheel stays in inner scroller")
    wheel = next(
        (event for event in wheel_probe["events"] if event["type"] == "wheel"),
        None,
    )
    if wheel is None:
        raise SmokeError(f"missing nested wheel event: {wheel_probe!r}")
    assert_equal(wheel["target"], "nested-scroll-content", "nested wheel target")
    _assert_close(wheel["clientX"], 20.0, "nested wheel.clientX")
    _assert_close(wheel["clientY"], 35.0, "nested wheel.clientY")
    assert_equal([wheel["deltaX"], wheel["deltaY"]], [25, 30], "nested wheel deltas")

    # Boundary events retain the previously targeted nested frame transform
    # when the next root-frame point lies outside both iframes.
    await mouse("mouseMoved", 10.0, 10.0)

    async def nested_mouseout_applied() -> bool:
        return bool(
            await page.evaluate(
                """() => {
                  const outer = document.getElementById("outer-frame");
                  const frame = outer.contentDocument.getElementById("nested-frame");
                  return frame.contentWindow.__nestedInputEvents
                    .some(event => event.type === "mouseout");
                }"""
            )
        )

    await wait_until(nested_mouseout_applied, "nested iframe mouseout", timeout_ms=3_000)
    exit_probe = await page.evaluate(
        """() => {
          const outer = document.getElementById("outer-frame");
          const frame = outer.contentDocument.getElementById("nested-frame");
          return [...frame.contentWindow.__nestedInputEvents]
            .reverse()
            .find(event => event.type === "mouseout") || null;
        }"""
    )
    if exit_probe is None:
        raise SmokeError("leaving nested iframe did not dispatch mouseout")
    assert_equal(exit_probe["target"], "nested-target", "nested mouseout target")
    _assert_close(exit_probe["clientX"], -318.3333, "nested mouseout.clientX", 1.1)
    _assert_close(exit_probe["clientY"], -245.0, "nested mouseout.clientY", 1.1)

    scrollbar_top: float | None = None
    if is_moli:
        events_before_scrollbar = int(
            await page.evaluate(
                """() => {
                  const outer = document.getElementById("outer-frame");
                  const frame = outer.contentDocument.getElementById("nested-frame");
                  return frame.contentWindow.__nestedInputEvents.length;
                }"""
            )
        )
        # Nested client point (52, 50) is the vertical scrollbar's forward
        # control. Moli routes UA scrollbar input through the same two frame
        # maps while keeping it outside DOM mouse dispatch.
        await mouse("mousePressed", 148.875, 120.625, button="left", buttons=1)
        await mouse("mouseReleased", 148.875, 120.625, button="left", buttons=0)

        async def nested_scrollbar_applied() -> bool:
            return bool(
                await page.evaluate(
                    """() => {
                      const outer = document.getElementById("outer-frame");
                      const frame = outer.contentDocument.getElementById("nested-frame");
                      return frame.contentDocument.getElementById("nested-scroll").scrollTop === 70;
                    }"""
                )
            )

        await wait_until(
            nested_scrollbar_applied,
            "nested iframe scrollbar control",
            timeout_ms=3_000,
        )
        scrollbar_probe = await page.evaluate(
            """() => {
              const outer = document.getElementById("outer-frame");
              const frame = outer.contentDocument.getElementById("nested-frame");
              return {
                top: frame.contentDocument.getElementById("nested-scroll").scrollTop,
                eventCount: frame.contentWindow.__nestedInputEvents.length,
              };
            }"""
        )
        scrollbar_top = float(scrollbar_probe["top"])
        assert_equal(
            scrollbar_probe["eventCount"],
            events_before_scrollbar,
            "nested UA scrollbar stays outside DOM dispatch",
        )

    state.record(
        "nested_transformed_iframe_input_coordinates",
        {
            "engine": "moli" if is_moli else "chromium",
            "methods": ["Input.dispatchMouseEvent", "Runtime.evaluate"],
            "frameDepth": 2,
            "geometry": geometry,
            "nestedScroll": wheel_probe["nestedScroll"],
            "scrollbarTop": scrollbar_top,
            "mouseout": exit_probe,
        },
    )
