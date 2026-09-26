"""Moli's first-demand / frozen-screen-layout contract, through public CDP."""

from __future__ import annotations

import asyncio
import base64
import json
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any, AsyncIterator, Awaitable, Callable
from urllib.parse import quote

from ..assertions import SmokeError, assert_equal, record
from ..pdf_document import assert_pdf_envelope
from ..png_image import decode_png
from ..raw_cdp import RawCdpClient, connect_raw_cdp, discover_websocket_url
from .layout_screenshot import _capture, _open_target, _response_for_id, _success, _wait_for_load


HTML = """<!doctype html><style>
html { scrollbar-width: none; background: white }
body { margin: 0; height: 1200px }
#probe { position: absolute; left: 20px; top: 20px; width: 80px; height: 40px;
         display: grid; grid-template-columns: 1fr 1fr; background: rgb(0,255,0) }
</style><div id="probe" tabindex="0"></div>
<script>
window.inputEvents = [];
for (const name of ['mousemove','wheel','touchstart','dragenter','click']) {
  document.addEventListener(name, event => {
    inputEvents.push(name);
    if (name === 'wheel' || name === 'dragenter') event.preventDefault();
  }, {passive: false});
}
</script>"""
DATA_URL = "data:text/html," + quote(HTML)


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

    async def evaluate(self, expression: str) -> Any:
        result = await self.call("Runtime.evaluate", {
            "expression": expression, "returnByValue": True,
        })
        return result["result"].get("value")

    async def width(self) -> Any:
        return await self.evaluate("document.getElementById('probe').getBoundingClientRect().width")

    async def set_width(self, width: int) -> None:
        await self.evaluate(f"document.getElementById('probe').style.width = '{width}px'")


@asynccontextmanager
async def _fresh_probe(client: RawCdpClient) -> AsyncIterator[Probe]:
    # Only DOM/Runtime metadata is read here. No Playwright preflight, geometry
    # query, screenshot, or screencast may warm a case before its first command.
    target, session = await _open_target(client, DATA_URL)
    try:
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
    assert_equal(await probe.width(), 80, "first demand published the 80px box")
    warm = await operation(probe, 1)
    assert_equal(warm, initial, "warm command returned the original geometry")
    assert_equal(await probe.width(), 80, "warm demand retained the 80px box")
    return {"firstPublishedWidth": 80, "warmWidthAfterMutation": 80}


async def _does_not_publish(probe: Probe, operation: Operation) -> dict[str, Any]:
    await operation(probe, 0)
    await probe.set_width(120)
    assert_equal(await probe.width(), 120, "non-layout command left the page cold")
    await probe.set_width(160)
    await operation(probe, 1)
    assert_equal(await probe.width(), 120, "non-layout command did not refresh a warm page")
    return {"firstGeometryWidth": 120, "warmWidthAfterMutation": 120}


def _command(method: str, params: Callable[[Probe, int], dict[str, Any]] | None = None) -> Operation:
    async def run(probe: Probe, phase: int) -> Any:
        return await probe.call(method, params(probe, phase) if params else None)
    return run


def _script(expression: str) -> Operation:
    async def run(probe: Probe, phase: int) -> Any:
        return await probe.evaluate(expression)
    return run


def _input(method: str, params: dict[str, Any], event: str) -> Operation:
    async def run(probe: Probe, phase: int) -> None:
        await probe.evaluate("inputEvents.length = 0")
        actual = dict(params)
        if "x" in actual:
            actual["x"] += phase
        await probe.call(method, actual)
        deadline = asyncio.get_running_loop().time() + 4
        while event not in await probe.evaluate("inputEvents"):
            if asyncio.get_running_loop().time() >= deadline:
                raise SmokeError(f"{method} did not dispatch {event}")
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
                          and message["params"]["header"]["isInline"]), None)
            if sheet is not None:
                break
            seen = [await probe.client.recv()]
    params = {"styleSheetId": sheet["styleSheetId"]}
    original = await probe.call("CSS.getStyleSheet", params)
    text = original["styleSheet"]["text"] + "\nbody { color: rgb(1,2,3) }"
    await probe.call("CSS.setStyleSheetText", {**params, "text": text})
    updated = await probe.call("CSS.getStyleSheet", params)
    assert_equal(updated["styleSheet"]["text"], text, "stylesheet mutation applied")


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
    assert_equal(await probe.width(), 80, "cold screenshot published screen layout")
    second, _ = await _capture(probe.client, probe.session)
    assert_equal(decode_png(second).pixel(120, 30), (0, 255, 0, 255), "warm screenshot updated pixels")
    await probe.set_width(160)
    assert_equal(await probe.width(), 120, "warm screenshot published refreshed layout")
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
        _, _, seen = await _success(probe.client, "Page.startScreencast", {
            "format": "jpeg", "maxWidth": 400, "maxHeight": 300,
        }, session_id=probe.session)
        event = await _frame(probe, seen, previous)
        previous = event["params"]["sessionId"]
        if not base64.b64decode(event["params"]["data"], validate=True).startswith(b"\xff\xd8"):
            raise SmokeError("screencast frame did not contain JPEG data")
        await probe.call("Page.stopScreencast")
        await probe.set_width(mutation)
        assert_equal(await probe.width(), published, "actual screencast frame published layout")
    return {"firstFrameWidth": 80, "newFrameWidth": 120}


async def _invalid_clip(probe: Probe) -> dict[str, Any]:
    async def reject(phase: int) -> None:
        message_id = await probe.client.send("Page.captureScreenshot", {
            "format": "png", "clip": {"x": 0, "y": 0, "width": 0, "height": 0, "scale": 1},
        }, session_id=probe.session)
        response, _ = await _response_for_id(probe.client, message_id)
        assert_equal(response.get("error", {}).get("code"), -32602, "empty clip parameter error")
    return await _does_not_publish(probe, lambda _, phase: reject(phase))


async def _live_metrics(probe: Probe) -> dict[str, Any]:
    initial = await probe.call("Page.getLayoutMetrics")
    assert_equal(initial["cssContentSize"]["height"], 1200, "cold content size is real layout")
    await probe.evaluate("document.body.style.height = '1600px'; window.scrollTo(0,100)")
    await probe.call("Emulation.setDeviceMetricsOverride", {
        "width": 900, "height": 500, "deviceScaleFactor": 1, "mobile": False,
    })
    warm = await probe.call("Page.getLayoutMetrics")
    assert_equal(warm["cssContentSize"], initial["cssContentSize"], "content size remains frozen")
    assert_equal(warm["cssLayoutViewport"]["clientWidth"], 900, "viewport remains live")
    assert_equal(warm["cssLayoutViewport"]["pageY"], 100, "scroll remains live")
    await _capture(probe.client, probe.session)
    refreshed = await probe.call("Page.getLayoutMetrics")
    assert_equal(refreshed["cssContentSize"]["height"], 1600, "capture updates content extent")
    assert_equal(refreshed["cssContentSize"]["width"], 900, "capture updates content width")
    return {"coldContentHeight": 1200, "warmContentHeight": 1200, "refreshedContentHeight": 1600}


async def _missing_box(probe: Probe) -> dict[str, Any]:
    assert_equal(await probe.width(), 80, "initial screen layout")
    await probe.evaluate("const late = document.createElement('div'); late.id = 'late'; late.style.width = '200px'; document.body.append(late)")
    expression = "document.getElementById('late').getBoundingClientRect().width"
    assert_equal(await probe.evaluate(expression), 0, "new node does not refresh snapshot")
    await probe.call("Page.getLayoutMetrics")
    assert_equal(await probe.evaluate(expression), 0, "metrics do not refresh a missing box")
    await _capture(probe.client, probe.session)
    assert_equal(await probe.evaluate(expression), 200, "capture publishes the new node")
    return {"missingBoxWidth": 0, "afterCaptureWidth": 200}


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
            _, _, seen = await _success(probe.client, kind, params, session_id=probe.session)
            await _wait_for_load(probe.client, probe.session, seen)
        await probe.set_width(160)
        assert_equal(await probe.width(), 160, "replacement Document gets a fresh initial layout")
        await probe.set_width(200)
        assert_equal(await probe.width(), 160, "replacement Document also freezes after initialization")
        return {"oldWidth": 80, "newDocumentWidth": 160}
    return run


async def run_layout_policy_group(endpoint: str, fixture: str, results: list[dict[str, Any]]) -> None:
    if not (await discover_websocket_url(endpoint)).endswith("/devtools/browser/moli-browser"):
        record(results, "layout_policy_not_applicable", {"reason": "Moli's frozen screen-layout contract"})
        return

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
        ("Input.dispatchDragEvent/enter", _input("Input.dispatchDragEvent", {"type": "dragEnter", "x": 40, "y": 30, "data": {"items": [], "dragOperationsMask": 1}}, "dragenter")),
        ("Input.synthesizeTapGesture", _input("Input.synthesizeTapGesture", {"x": 40, "y": 30}, "click")),
    ]
    non_publishers: list[tuple[str, Operation]] = [
        ("Runtime.evaluate/plain", _script("[document.getElementById('probe').textContent, innerWidth, innerHeight, scrollX, scrollY]")),
        ("Runtime.callFunctionOn/plain", _command("Runtime.callFunctionOn", lambda p, phase: {
            "objectId": p.object_id, "functionDeclaration": "function(){ return this.id; }", "returnByValue": True,
        })),
        ("DOM/reads", _dom_reads),
        ("CSS/computed-style-and-grid", _style_reads),
        ("CSS/stylesheet-read-write", _style_sheet),
        ("DOM.setAttributeValue", _command("DOM.setAttributeValue", lambda p, phase: {"nodeId": p.node_id, "name": "style", "value": "width:100px"})),
        ("Emulation/viewport-and-media", _viewport),
        ("Page.captureSnapshot", _command("Page.captureSnapshot", lambda p, phase: {"format": "mhtml"})),
        ("DOMSnapshot.captureSnapshot", _command("DOMSnapshot.captureSnapshot", lambda p, phase: {"computedStyles": ["display", "width"], "includeDOMRects": True})),
        ("Runtime.focus/preventScroll", _script("document.getElementById('probe').focus({preventScroll:true})")),
        ("Input.dispatchTouchEvent/idle-end", _command("Input.dispatchTouchEvent", lambda p, phase: {"type": "touchEnd", "touchPoints": []})),
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
        *[("replacement/" + kind, _replacement(kind)) for kind in ["Page.navigate", "Page.reload", "Page.setDocumentContent", "document.open"]],
    ]
    client = await connect_raw_cdp(endpoint)
    failures = []
    try:
        for name, run in checks:
            try:
                async with _fresh_probe(client) as probe:
                    observed = await run(probe)
                record(results, "layout_policy/" + name, observed)
            except Exception as error:
                failures.append(f"{name}: {error}")
                results.append({"name": "layout_policy/" + name, "ok": False, "error": str(error)})
    finally:
        await client.websocket.close()
    if failures:
        raise SmokeError("layout policy failures:\n" + "\n".join(failures))
