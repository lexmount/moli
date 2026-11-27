"""Moli's first-demand / frozen-screen-layout contract, through public CDP."""

from __future__ import annotations

import asyncio
import base64
import json
import traceback
from contextlib import asynccontextmanager
from dataclasses import dataclass
from html import escape
from typing import Any, AsyncIterator, Awaitable, Callable
from urllib.parse import quote

from ..assertions import SmokeError, assert_equal, record
from ..config import REPO_ROOT, is_moli_browser
from ..pdf_document import assert_pdf_envelope
from ..png_image import decode_png
from ..raw_cdp import RawCdpClient, connect_raw_cdp
from .layout_screenshot import _capture, _navigate, _open_target, _response_for_id, _success


HTML = """<!doctype html><style>
html { scrollbar-width: none; background: white }
body { margin: 0; height: 1200px }
#probe { position: absolute; left: 20px; top: 20px; width: 80px; height: 40px;
         display: grid; grid-template-columns: 1fr 1fr; background: rgb(0,255,0) }
</style><div id="probe" tabindex="0"></div>
<script>
window.inputEvents = [];
document.addEventListener('dragover', event => event.preventDefault());
for (const name of ['mousemove','wheel','touchstart','dragenter','dragover','click']) {
  document.addEventListener(name, event => {
    inputEvents.push(name);
    if (name === 'wheel' || name === 'dragenter') event.preventDefault();
  }, {passive: false});
}
</script>"""
DATA_URL = "data:text/html," + quote(HTML)

NESTED_FRAME_HTML = """<!doctype html><style>
body { margin: 0 }
#viewport-probe { width: 50vw; height: 10px }
</style><div id="viewport-probe"></div>"""
CHILD_FRAME_HTML = f"""<!doctype html><style>
body {{ margin: 0 }}
#child {{ width: 50vw; height: 40px; background: lime; touch-action: none }}
@media (min-width: 200px) {{ #child {{ background: blue }} }}
iframe {{ display: block; width: 50%; height: 20px; border: 0 }}
</style><div id="child"></div><iframe srcdoc="{escape(NESTED_FRAME_HTML)}"></iframe>"""
FRAME_DATA_URL = "data:text/html," + quote(f"""<!doctype html><style>
html {{ scrollbar-width: none; background: white }}
body {{ margin: 0 }}
section {{ width: 400px; margin: 60px 0 0 100px }}
section:has(> iframe[data-wide]) {{ width: 560px }}
#probe {{ display: block; box-sizing: border-box; width: 50%; height: 160px;
          padding: 20px; border: 10px solid black }}
#target {{ position: absolute; left: 450px; top: 60px; width: 40px; height: 40px; background: red }}
iframe[data-wide] ~ #target {{ pointer-events: none }}
</style><section><iframe id="probe" srcdoc="{escape(CHILD_FRAME_HTML)}"></iframe>
<div id="target"></div></section>""")

FRAME_GEOMETRY = """(() => {
  const frame = document.getElementById('probe');
  const child = frame.contentWindow;
  const nested = child.document.querySelector('iframe').contentWindow;
  return {
    outerWidth: frame.offsetWidth,
    viewport: [child.innerWidth, child.innerHeight],
    viewportUnitWidth: child.document.getElementById('child').offsetWidth,
    nestedWidth: nested.innerWidth,
    nestedViewportUnitWidth: nested.document.getElementById('viewport-probe').offsetWidth,
    wide: child.matchMedia('(min-width: 200px)').matches,
    siblingHit: document.elementFromPoint(460, 70)?.id === 'target',
  };
})()"""
INITIAL_FRAME_GEOMETRY = {
    "outerWidth": 200, "viewport": [140, 100], "viewportUnitWidth": 70,
    "nestedWidth": 70, "nestedViewportUnitWidth": 35, "wide": False, "siblingHit": True,
}
WIDE_FRAME_GEOMETRY = {
    "outerWidth": 280, "viewport": [220, 100], "viewportUnitWidth": 110,
    "nestedWidth": 110, "nestedViewportUnitWidth": 55, "wide": True, "siblingHit": False,
}


@dataclass
class Probe:
    client: RawCdpClient
    session: str
    object_id: str
    node_id: int
    frame_id: str

    async def call(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        result, _, _ = await _success(self.client, method, params, session_id=self.session)
        if result.get("exceptionDetails"):
            raise SmokeError(f"{method} raised an exception: {result!r}")
        return result

    async def evaluate(self, expression: str, *, await_promise: bool = False) -> Any:
        result = await self.call("Runtime.evaluate", {
            "expression": expression, "returnByValue": True, "awaitPromise": await_promise,
        })
        return result["result"].get("value")

    async def width(self) -> Any:
        return await self.evaluate("document.getElementById('probe').getBoundingClientRect().width")

    async def set_width(self, width: int) -> None:
        await self.evaluate(f"document.getElementById('probe').style.width = '{width}px'")


@asynccontextmanager
async def _fresh_probe(client: RawCdpClient, url: str) -> AsyncIterator[Probe]:
    # Only DOM/Runtime metadata is read here. No Playwright preflight, geometry
    # query, screenshot, or screencast may warm a case before its first command.
    target, session = await _open_target(client, url)
    try:
        if not is_moli_browser():
            # Chromium hit testing uses the compositor surface. Wait for the
            # initial rendered document before probing its live layout policy.
            await _success(client, "Page.bringToFront", session_id=session)
            await _capture(client, session)
        await _success(client, "DOM.enable", session_id=session)
        remote, _, _ = await _success(client, "Runtime.evaluate", {
            "expression": "document.getElementById('probe')",
        }, session_id=session)
        document, _, _ = await _success(client, "DOM.getDocument", session_id=session)
        node, _, _ = await _success(client, "DOM.querySelector", {
            "nodeId": document["root"]["nodeId"], "selector": "#probe",
        }, session_id=session)
        tree, _, _ = await _success(client, "Page.getFrameTree", session_id=session)
        yield Probe(client, session, remote["result"]["objectId"], node["nodeId"], tree["frameTree"]["frame"]["id"])
    finally:
        await _success(client, "Target.closeTarget", {"targetId": target})


Operation = Callable[[Probe, int], Awaitable[Any]]


async def _initializes_once(probe: Probe, operation: Operation) -> dict[str, Any]:
    initial = await operation(probe, 0)
    await probe.set_width(120)
    # Read only after the mutation: returning 80 proves the tested command
    # published the first layout, rather than this assertion creating it.
    assert_equal(await probe.width(), 80 if is_moli_browser() else 120, "geometry after mutation")
    warm = await operation(probe, 1)
    if is_moli_browser():
        assert_equal(warm, initial, "warm command returned the original geometry")
    assert_equal(await probe.width(), 80 if is_moli_browser() else 120, "warm geometry")
    return {"firstPublishedWidth": 80, "warmWidthAfterMutation": await probe.width()}


async def _does_not_publish(probe: Probe, operation: Operation) -> dict[str, Any]:
    await operation(probe, 0)
    await probe.set_width(120)
    assert_equal(await probe.width(), 120, "non-layout command left the page cold")
    await probe.set_width(160)
    await operation(probe, 1)
    expected = 120 if is_moli_browser() else (100 if operation is _attribute_width else 160)
    assert_equal(await probe.width(), expected, "geometry after second mutation")
    return {"firstGeometryWidth": 120, "warmWidthAfterMutation": await probe.width()}


def _command(method: str, params: Callable[[Probe, int], dict[str, Any]] | None = None) -> Operation:
    async def run(probe: Probe, phase: int) -> Any:
        return await probe.call(method, params(probe, phase) if params else None)
    return run


def _script(expression: str) -> Operation:
    async def run(probe: Probe, phase: int) -> Any:
        return await probe.evaluate(expression)
    return run


async def _attribute_width(probe: Probe, phase: int) -> None:
    await probe.call("DOM.setAttributeValue", {"nodeId": probe.node_id, "name": "style", "value": "width:100px"})


async def _idle_touch_end(probe: Probe, phase: int) -> None:
    if is_moli_browser():
        await probe.call("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    else:
        identifier = await probe.client.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []}, session_id=probe.session)
        response, _ = await _response_for_id(probe.client, identifier)
        assert_equal(response.get("error"), {"code": -32602, "message": "Must send a TouchStart first to start a new touch."}, "idle touch end validation")


def _input(method: str, params: dict[str, Any], event: str) -> Operation:
    async def run(probe: Probe, phase: int) -> None:
        await probe.evaluate("inputEvents.length = 0")
        actual = dict(params)
        expected_event = event
        if method == "Input.dispatchDragEvent" and phase == 1:
            actual["type"] = "dragOver"
            expected_event = "dragover"
        if "x" in actual:
            actual["x"] += phase
        await probe.call(method, actual)
        deadline = asyncio.get_running_loop().time() + 4
        while expected_event not in await probe.evaluate("inputEvents"):
            if asyncio.get_running_loop().time() >= deadline:
                raise SmokeError(f"{method} did not dispatch {event} in phase {phase}: " + str(await probe.evaluate("({events:inputEvents,focus:document.hasFocus(),rect:document.getElementById('probe').getBoundingClientRect().toJSON()})")))
            # smoke-timing: poll the dispatched DOM event within the four-second deadline.
            await asyncio.sleep(0.02)
        if method == "Input.dispatchTouchEvent":
            await probe.call(method, {"type": "touchEnd", "touchPoints": []})
    return run


async def _dom_reads(probe: Probe, phase: int) -> None:
    await probe.call("DOM.describeNode", {"nodeId": probe.node_id})
    await probe.call("DOM.getAttributes", {"nodeId": probe.node_id})
    await probe.call("DOM.getOuterHTML", {"nodeId": probe.node_id})


async def _style_reads(probe: Probe, phase: int) -> None:
    await probe.call("CSS.enable")
    styles = await probe.call("CSS.getComputedStyleForNode", {"nodeId": probe.node_id})
    if not styles.get("computedStyle"):
        raise SmokeError("CSS.getComputedStyleForNode returned no properties")
    await probe.evaluate("(() => { const s = getComputedStyle(document.getElementById('probe')); return [s.width, s.height, s.gridTemplateColumns]; })()")


async def _style_sheet(probe: Probe, phase: int) -> None:
    await probe.call("CSS.disable")
    _, _, seen = await _success(probe.client, "CSS.enable", session_id=probe.session)
    async with asyncio.timeout(5):
        while True:
            sheet = next((message["params"]["header"] for message in seen
                          if message.get("sessionId") == probe.session
                          and message.get("method") == "CSS.styleSheetAdded"
                          and (message["params"]["header"]["isInline"] if is_moli_browser()
                               else message["params"]["header"]["origin"] == "regular")), None)
            if sheet is not None:
                break
            seen = [await probe.client.recv()]
    params = {"styleSheetId": sheet["styleSheetId"]}
    method = "CSS.getStyleSheet" if is_moli_browser() else "CSS.getStyleSheetText"
    original = await probe.call(method, params)
    text = (original["styleSheet"]["text"] if is_moli_browser() else original["text"]) + "\nbody { color: rgb(1,2,3) }"
    await probe.call("CSS.setStyleSheetText", {**params, "text": text})
    updated = await probe.call(method, params)
    assert_equal(updated["styleSheet"]["text"] if is_moli_browser() else updated["text"], text, "stylesheet mutation applied")


async def _viewport(probe: Probe, phase: int) -> None:
    await probe.call("Emulation.setDeviceMetricsOverride", {
        "width": 900 if phase == 0 else 800, "height": 500,
        "deviceScaleFactor": 1, "mobile": False,
    })
    await probe.call("Emulation.setEmulatedMedia", {"media": "print" if phase == 0 else "screen"})


async def _pdf(probe: Probe, phase: int) -> None:
    result = await probe.call("Page.printToPDF")
    assert_pdf_envelope(base64.b64decode(result["data"], validate=True), "layout policy PDF")


async def _screenshot(probe: Probe) -> dict[str, Any]:
    first, _ = await _capture(probe.client, probe.session)
    assert_equal(decode_png(first).pixel(120, 30), (255, 255, 255, 255), "cold screenshot narrow box")
    await probe.set_width(120)
    assert_equal(await probe.width(), 80 if is_moli_browser() else 120, "geometry after screenshot and mutation")
    second, _ = await _capture(probe.client, probe.session)
    assert_equal(decode_png(second).pixel(120, 30), (0, 255, 0, 255), "warm screenshot updated pixels")
    await probe.set_width(160)
    assert_equal(await probe.width(), 120 if is_moli_browser() else 160, "geometry after warm screenshot and mutation")
    return {"coldPublishedWidth": 80, "refreshedWidth": 120, "pixelsVerified": True}


async def _frame(probe: Probe, seen: list[dict[str, Any]], previous: int | None) -> dict[str, Any]:
    def matches(message: dict[str, Any]) -> bool:
        return (message.get("sessionId") == probe.session
                and message.get("method") == "Page.screencastFrame"
                and message["params"]["sessionId"] != previous)
    for message in seen:
        if matches(message):
            return message
    async with asyncio.timeout(5):
        while True:
            message = await probe.client.recv()
            if matches(message):
                return message


async def _screencast(probe: Probe) -> dict[str, Any]:
    previous = None
    for published, mutation in [(80, 120), (120, 160)]:
        previous = await _capture_screencast_frame(probe, previous)
        await probe.set_width(mutation)
        assert_equal(await probe.width(), published if is_moli_browser() else mutation, "geometry after screencast and mutation")
    return {"firstFrameWidth": 80, "newFrameWidth": 120}


async def _capture_screencast_frame(probe: Probe, previous: int | None = None) -> int:
    _, _, seen = await _success(probe.client, "Page.startScreencast", {
        "format": "jpeg", "maxWidth": 400, "maxHeight": 300,
    }, session_id=probe.session)
    event = await _frame(probe, seen, previous if is_moli_browser() else None)
    await probe.call("Page.stopScreencast")
    if not base64.b64decode(event["params"]["data"], validate=True).startswith(b"\xff\xd8"):
        raise SmokeError("screencast frame did not contain JPEG data")
    return event["params"]["sessionId"]


async def _invalid_clip(probe: Probe) -> dict[str, Any]:
    async def reject(phase: int) -> None:
        message_id = await probe.client.send("Page.captureScreenshot", {
            "format": "png", "clip": {"x": 0, "y": 0, "width": 0, "height": 0, "scale": 1},
        }, session_id=probe.session)
        response, _ = await _response_for_id(probe.client, message_id)
        assert_equal(response.get("error", {}).get("code"), -32602 if is_moli_browser() else -32000, "empty clip parameter error")
    return await _does_not_publish(probe, lambda _, phase: reject(phase))


async def _live_metrics(probe: Probe) -> dict[str, Any]:
    initial = await probe.call("Page.getLayoutMetrics")
    assert_equal(initial["cssContentSize"]["height"], 1200, "cold content size is real layout")
    await probe.evaluate("document.body.style.height = '1600px'; window.scrollTo(0,100)")
    await probe.call("Emulation.setDeviceMetricsOverride", {
        "width": 900, "height": 500, "deviceScaleFactor": 1, "mobile": False,
    })
    warm = await probe.call("Page.getLayoutMetrics")
    if is_moli_browser():
        assert_equal(warm["cssContentSize"], initial["cssContentSize"], "content size remains frozen")
    else:
        assert_equal(warm["cssContentSize"]["height"], 1600, "Chromium updates content extent immediately")
    assert_equal(warm["cssLayoutViewport"]["clientWidth"], 900, "viewport remains live")
    assert_equal(warm["cssLayoutViewport"]["pageY"], 100, "scroll remains live")
    await _capture(probe.client, probe.session)
    refreshed = await probe.call("Page.getLayoutMetrics")
    assert_equal(refreshed["cssContentSize"]["height"], 1600, "capture updates content extent")
    assert_equal(refreshed["cssContentSize"]["width"], 900, "capture updates content width")
    return {"coldContentHeight": 1200, "warmContentHeight": warm["cssContentSize"]["height"], "refreshedContentHeight": 1600}


async def _missing_box(probe: Probe) -> dict[str, Any]:
    assert_equal(await probe.width(), 80, "initial screen layout")
    await probe.evaluate("const late = document.createElement('div'); late.id = 'late'; late.style.width = '200px'; document.body.append(late)")
    expression = "document.getElementById('late').getBoundingClientRect().width"
    assert_equal(await probe.evaluate(expression), 0 if is_moli_browser() else 200, "new node geometry")
    await probe.call("Page.getLayoutMetrics")
    assert_equal(await probe.evaluate(expression), 0 if is_moli_browser() else 200, "new node geometry after metrics")
    await _capture(probe.client, probe.session)
    assert_equal(await probe.evaluate(expression), 200, "capture publishes the new node")
    return {"missingBoxWidth": 0 if is_moli_browser() else 200, "afterCaptureWidth": 200}


def _replacement(kind: str) -> Callable[[Probe], Awaitable[dict[str, Any]]]:
    async def run(probe: Probe) -> dict[str, Any]:
        assert_equal(await probe.width(), 80, "old Document layout")
        await probe.set_width(120)
        if kind == "document.open":
            await probe.evaluate(f"document.open(); document.write({json.dumps(HTML)}); document.close()")
        elif kind == "Page.setDocumentContent":
            await probe.call(kind, {"frameId": probe.frame_id, "html": HTML})
        else:
            params = {"url": "data:text/html," + quote(HTML + "<!-- replacement -->")} if kind == "Page.navigate" else {}
            await _navigate(probe.client, probe.session, params, method=kind)
        await probe.set_width(160)
        assert_equal(await probe.width(), 160, "replacement Document gets a fresh initial layout")
        await probe.set_width(200)
        assert_equal(await probe.width(), 160 if is_moli_browser() else 200, "replacement Document geometry after mutation")
        return {"oldWidth": 80, "newDocumentWidth": 160}
    return run


async def _svg_publication(probe: Probe) -> dict[str, Any]:
    fixture = (REPO_ROOT / "moli-renderer-v8/tests/fixtures/inline-svg-layout-publication.js").read_text()
    await probe.evaluate(f"{fixture}\ninstallSvgLayoutPublication()")
    initial = await probe.evaluate("readSvgLayoutPublication()")
    expected = {"htmlBefore": 80, "svg": [100, 40, 200, 100], "object": [110, 50, 10, 10],
                "shapeCount": 1, "range": [110, 50, 10, 10], "rangeCount": 1000, "htmlAfter": 80}
    assert_equal(initial, expected, "initial HTML and SVG publication")
    await probe.evaluate("document.getElementById('html').style.width='300px'")
    html_changed = await probe.evaluate("readSvgLayoutPublication()")
    assert_equal(html_changed, expected if is_moli_browser() else
                 {**expected, "htmlBefore": 300, "htmlAfter": 300},
                 "SVG queries must follow the browser's ordinary layout policy")
    await probe.evaluate("mutateSvgLayoutPublication()")
    dirty = await probe.evaluate("readSvgLayoutPublication()")
    refreshed = {"htmlBefore": 300, "svg": [300, 40, 200, 100], "object": [330, 50, 100, 10],
                 "shapeCount": 1, "range": [310, 50, 120, 10], "rangeCount": 1000, "htmlAfter": 300}
    assert_equal(dirty, expected if is_moli_browser() else refreshed,
                 "dirty SVG single and Range queries")
    png, _ = await _capture(probe.client, probe.session)
    image = decode_png(png)
    assert_equal(image.pixel(250, 10), (255, 0, 0, 255), "published HTML paint")
    assert_equal(image.pixel(335, 55), (0, 255, 0, 255), "published SVG paint")
    published = await probe.evaluate("readSvgLayoutPublication()")
    assert_equal(published, refreshed, "capture publishes both coordinate levels")
    return {"initial": initial, "htmlChanged": html_changed, "svgChanged": dirty,
            "published": published}


async def _iframe_screenshot(probe: Probe) -> dict[str, Any]:
    assert_equal(await probe.evaluate(FRAME_GEOMETRY), INITIAL_FRAME_GEOMETRY, "initial iframe geometry")
    await probe.evaluate("probe.setAttribute('data-wide', '')")
    assert_equal(await probe.evaluate("getComputedStyle(target).pointerEvents"), "none", "live sibling style")
    before = await probe.evaluate(FRAME_GEOMETRY)
    assert_equal(before, INITIAL_FRAME_GEOMETRY if is_moli_browser() else WIDE_FRAME_GEOMETRY,
                 "geometry and hit testing after stylesheet dependencies change")

    png, _ = await _capture(probe.client, probe.session)
    assert_equal(decode_png(png).pixel(140, 100), (0, 0, 255, 255), "child media query in captured pixels")
    after = await probe.evaluate(FRAME_GEOMETRY)
    assert_equal(after, WIDE_FRAME_GEOMETRY, "screenshot publishes outer and child geometry together")
    return {"beforeCapture": before, "afterCapture": after}


async def _iframe_screencast(probe: Probe) -> dict[str, Any]:
    previous = await _capture_screencast_frame(probe)
    assert_equal(await probe.evaluate(FRAME_GEOMETRY), INITIAL_FRAME_GEOMETRY, "first screencast frame geometry")
    await probe.evaluate("probe.setAttribute('data-wide', '')")
    assert_equal(await probe.evaluate(FRAME_GEOMETRY),
                 INITIAL_FRAME_GEOMETRY if is_moli_browser() else WIDE_FRAME_GEOMETRY,
                 "geometry between screencast frames")

    await _capture_screencast_frame(probe, previous)
    published = await probe.evaluate(FRAME_GEOMETRY)
    assert_equal(published, WIDE_FRAME_GEOMETRY, "new screencast frame updates both nested viewport sizes")
    return {"published": published}


async def _iframe_print(probe: Probe) -> dict[str, Any]:
    assert_equal(await probe.evaluate(FRAME_GEOMETRY), INITIAL_FRAME_GEOMETRY, "initial screen geometry before print")
    await probe.evaluate("probe.style.width = '300px'; probe.style.height = '180px'")
    resized = {
        "outerWidth": 300, "viewport": [240, 120], "viewportUnitWidth": 120,
        "nestedWidth": 120, "nestedViewportUnitWidth": 60, "wide": True, "siblingHit": True,
    }
    await _pdf(probe, 0)
    after_print = await probe.evaluate(FRAME_GEOMETRY)
    assert_equal(after_print, INITIAL_FRAME_GEOMETRY if is_moli_browser() else resized,
                 "print leaves the screen iframe viewport intact")
    await _capture(probe.client, probe.session)
    after_capture = await probe.evaluate(FRAME_GEOMETRY)
    assert_equal(after_capture, resized, "screen capture after printing publishes the new viewport")
    return {"afterPrint": after_print, "afterCapture": after_capture}


async def _iframe_captured_touch(probe: Probe) -> dict[str, Any]:
    await probe.evaluate("""
        window.frameTouches = [];
        const child = probe.contentDocument;
        for (const type of ['touchstart', 'touchmove', 'touchend']) {
          child.addEventListener(type, event => {
            const touch = event.changedTouches[0];
            frameTouches.push([type, event.target.id, touch.clientX, touch.clientY]);
          });
        }
    """)
    method = "Input.dispatchTouchEvent"
    await probe.call(method, {"type": "touchStart", "touchPoints": [{"x": 140, "y": 100, "id": 1}]})
    await probe.evaluate("probe.parentElement.style.marginLeft = '120px'")
    await probe.call(method, {"type": "touchMove", "touchPoints": [{"x": 160, "y": 110, "id": 1}]})
    await _capture(probe.client, probe.session)
    # This point is outside the moved child viewport, so it also tests routing
    # to the captured target after publishing the new frame transform.
    await probe.call(method, {"type": "touchMove", "touchPoints": [{"x": 142, "y": 120, "id": 1}]})
    await probe.call(method, {"type": "touchEnd", "touchPoints": []})
    events = await probe.evaluate("frameTouches")
    assert_equal(events, [
        ["touchstart", "child", 10, 10],
        ["touchmove", "child", 30 if is_moli_browser() else 10, 20],
        ["touchmove", "child", -8, 30],
        ["touchend", "child", -8, 30],
    ], "captured touch uses the current screen snapshot")
    return {"events": events}


async def _iframe_browsing_context(probe: Probe) -> dict[str, Any]:
    assert_equal(await probe.evaluate(FRAME_GEOMETRY), INITIAL_FRAME_GEOMETRY, "old browsing context geometry")
    await probe.set_width(280)
    navigated = await probe.evaluate("""
        new Promise(resolve => {
          window.oldFrame = document.getElementById('probe');
          window.oldChild = oldFrame.contentWindow;
          const oldDocument = oldFrame.contentDocument;
          oldFrame.addEventListener('load', () => resolve([
            oldFrame.contentWindow === oldChild,
            oldFrame.contentDocument !== oldDocument,
            !!oldFrame.contentDocument.getElementById('replacement'),
            oldFrame.contentWindow.innerWidth,
          ]), { once: true });
          oldFrame.srcdoc = '<p id="replacement">New child document</p>';
        })
    """, await_promise=True)
    assert_equal(navigated, [True, True, True, 140 if is_moli_browser() else 220],
                 "child navigation keeps its browsing context and viewport")

    assert_equal(await probe.evaluate("oldFrame.remove(); oldFrame.contentWindow === null"), True,
                 "detached iframe has no Window")
    reattached = await probe.evaluate("""(() => {
      document.querySelector('section').prepend(oldFrame);
      return [oldFrame.contentWindow === oldChild, oldFrame.contentWindow.innerWidth];
    })()""")
    # Lazy infers the new context's viewport from the frozen owner width until
    # screen capture measures the new content box.
    assert_equal(reattached, [False, 200 if is_moli_browser() else 220],
                 "reattaching the owner creates a new browsing context without the old content viewport")
    await _capture(probe.client, probe.session)
    assert_equal(await probe.evaluate("oldFrame.contentWindow.innerWidth"), 220,
                 "first new screen layout measures the replacement viewport")
    return {"sameContextViewport": navigated[-1], "reattachedViewport": reattached[-1], "publishedViewport": 220}


async def run_layout_policy_group(endpoint: str, fixture: str, results: list[dict[str, Any]]) -> None:
    object_params = lambda p, phase: {"objectId": p.object_id}
    initializers: list[tuple[str, Operation]] = [
        ("Page.getLayoutMetrics", _command("Page.getLayoutMetrics")),
        ("DOM.getBoxModel", _command("DOM.getBoxModel", object_params)),
        ("DOM.getContentQuads", _command("DOM.getContentQuads", object_params)),
        ("DOM.getNodeForLocation", _command("DOM.getNodeForLocation", lambda p, phase: {"x": 40, "y": 30})),
        ("DOM.scrollIntoViewIfNeeded", _command("DOM.scrollIntoViewIfNeeded", object_params)),
        ("DOM.focus", _command("DOM.focus", object_params)),
        ("Runtime.evaluate/clientWidth", _script("document.getElementById('probe').clientWidth")),
        ("Runtime.evaluate/rect", _script("document.getElementById('probe').getBoundingClientRect().width")),
        ("Runtime.evaluate/range", _script("(() => { const r = document.createRange(); r.selectNode(document.getElementById('probe')); return r.getBoundingClientRect().width; })()")),
        ("Runtime.evaluate/hit", _script("document.elementFromPoint(40,30).id")),
        ("Runtime.callFunctionOn/geometry", _command("Runtime.callFunctionOn", lambda p, phase: {
            "objectId": p.object_id, "functionDeclaration": "function(){ return this.getClientRects().length; }", "returnByValue": True,
        })),
        ("Input.dispatchMouseEvent/move", _input("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": 40, "y": 30}, "mousemove")),
        ("Input.dispatchMouseEvent/wheel", _input("Input.dispatchMouseEvent", {"type": "mouseWheel", "x": 40, "y": 30, "deltaX": 0, "deltaY": 10}, "wheel")),
        ("Input.dispatchTouchEvent/start", _input("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": 40, "y": 30, "id": 1}]}, "touchstart")),
        ("Input.dispatchDragEvent/enter", _input("Input.dispatchDragEvent", {"type": "dragEnter", "x": 40, "y": 30, "data": {"items": [{"mimeType": "text/plain", "data": "layout-probe"}], "dragOperationsMask": 1}}, "dragenter")),
        ("Input.synthesizeTapGesture", _input("Input.synthesizeTapGesture", {"x": 40, "y": 30, "gestureSourceType": "mouse"}, "click")),
    ]
    non_publishers: list[tuple[str, Operation]] = [
        ("Runtime.evaluate/plain", _script("[document.getElementById('probe').textContent, innerWidth, innerHeight, scrollX, scrollY]")),
        ("Runtime.callFunctionOn/plain", _command("Runtime.callFunctionOn", lambda p, phase: {
            "objectId": p.object_id, "functionDeclaration": "function(){ return this.id; }", "returnByValue": True,
        })),
        ("DOM/reads", _dom_reads),
        ("CSS/computed-style-and-grid", _style_reads),
        ("CSS/stylesheet-read-write", _style_sheet),
        ("DOM.setAttributeValue", _attribute_width),
        ("Emulation/viewport-and-media", _viewport),
        ("Page.captureSnapshot", _command("Page.captureSnapshot", lambda p, phase: {"format": "mhtml"})),
        ("DOMSnapshot.captureSnapshot", _command("DOMSnapshot.captureSnapshot", lambda p, phase: {"computedStyles": ["display", "width"], "includeDOMRects": True})),
        ("Runtime.focus/preventScroll", _script("document.getElementById('probe').focus({preventScroll:true})")),
        ("Input.dispatchTouchEvent/idle-end", _idle_touch_end),
        ("Page.printToPDF", _pdf),
    ]
    checks: list[tuple[str, Callable[[Probe], Awaitable[dict[str, Any]]]]] = [
        *[("initializes/" + name, lambda p, op=operation: _initializes_once(p, op)) for name, operation in initializers],
        *[("does-not-publish/" + name, lambda p, op=operation: _does_not_publish(p, op)) for name, operation in non_publishers],
        ("publishes/screenshot", _screenshot),
        ("publishes/screencast-frame", _screencast),
        ("validation/empty-clip", _invalid_clip),
        ("state/live-viewport-frozen-content", _live_metrics),
        ("state/missing-box", _missing_box),
        ("state/svg-shares-frozen-publication", _svg_publication),
        *[("replacement/" + kind, _replacement(kind)) for kind in ["Page.navigate", "Page.reload", "Page.setDocumentContent", "document.open"]],
    ]
    iframe_checks = [
        ("iframe/screenshot-publishes-geometry", _iframe_screenshot),
        ("iframe/screencast-publishes-nested-viewports", _iframe_screencast),
        ("iframe/print-preserves-screen-geometry", _iframe_print),
        ("iframe/captured-touch-uses-published-coordinates", _iframe_captured_touch),
        ("iframe/viewport-follows-browsing-context", _iframe_browsing_context),
    ]
    client = await connect_raw_cdp(endpoint)
    failures = []
    try:
        for name, run, url in [
            *((name, run, DATA_URL) for name, run in checks),
            *((name, run, FRAME_DATA_URL) for name, run in iframe_checks),
        ]:
            try:
                async with _fresh_probe(client, url) as probe:
                    observed = await run(probe)
                record(results, "layout_policy/" + name, {"policy": "frozen" if is_moli_browser() else "live", **observed})
            except Exception as error:
                detail = f"{type(error).__name__}: {error}"
                failures.append(f"{name}: {detail}")
                results.append({"name": "layout_policy/" + name, "ok": False,
                                "error": detail, "traceback": traceback.format_exc()})
    finally:
        await client.websocket.close()
    if failures:
        raise SmokeError("layout policy failures:\n" + "\n".join(failures))
