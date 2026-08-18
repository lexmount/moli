from __future__ import annotations

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until


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
