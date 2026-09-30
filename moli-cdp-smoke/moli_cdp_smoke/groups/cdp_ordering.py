"""Renderer publication contracts calibrated against Chromium 145.0.7632.116.

Use the actual WebSocket order, retaining replies received while waiting for
another id. Timeouts only bound liveness; no sleep or retry proves ordering.
The four mutation cases explicitly distinguish Moli's current owner-only entry
from Chromium's ordinary-pause capability. They are not CDP-wide requirements.
"""

from __future__ import annotations

import asyncio
import contextlib
import sys
from dataclasses import dataclass, field
from typing import Any, Callable

from ..assertions import SmokeError, assert_equal, record_contract
from ..raw_cdp import RawCdpClient, connect_raw_cdp, discover_websocket_url


CHROMIUM_SOURCE = "Chromium 145.0.7632.116 raw CDP calibration, 2026-10-01"


@dataclass
class _Wire:
    client: RawCdpClient
    scenario: str
    requests: dict[int, tuple[str, str | None]] = field(default_factory=dict)
    messages: list[dict[str, Any]] = field(default_factory=list)
    trace: list[dict[str, Any]] = field(default_factory=list)

    async def send(
        self, method: str, params: dict[str, Any] | None = None, session: str | None = None
    ) -> int:
        print(f"[moli-cdp-smoke] CHECKPOINT cdp-ordering/{self.scenario}/{method}",
              file=sys.stderr, flush=True)
        rid = await self.client.send(method, params, session_id=session)
        self.requests[rid] = (method, session)
        self.trace.append({"direction": "send", "id": rid, "method": method,
                           "params": params, "sessionId": session})
        return rid

    async def until(
        self, predicate: Callable[[dict[str, Any]], bool], label: str, *, after: int = 0
    ) -> dict[str, Any]:
        try:
            async with asyncio.timeout(10):
                for message in self.messages[after:]:
                    if predicate(message):
                        return message
                while True:
                    message = await self.client.recv()
                    # Retain rejected frames too, so routing/duplicate failures
                    # preserve the actual wire evidence in the artifact.
                    self.trace.append({"direction": "receive", "message": message})
                    rid = message.get("id")
                    if isinstance(rid, int):
                        if rid not in self.requests:
                            raise SmokeError(f"unrecognized response id: {message!r}")
                        assert_equal(message.get("sessionId"), self.requests[rid][1],
                                     f"{self.requests[rid][0]} original response session")
                        if self.has_response(rid):
                            raise SmokeError(f"duplicate response id={rid}: {message!r}")
                    self.messages.append(message)
                    if predicate(message):
                        return message
        except TimeoutError as error:
            raise SmokeError(f"timed out waiting for {label}; seen={self.messages[-20:]!r}") from error

    async def response(self, rid: int, *, allow_error: bool = False) -> dict[str, Any]:
        message = await self.until(lambda m: m.get("id") == rid, self.requests[rid][0])
        if "error" in message and not allow_error:
            raise SmokeError(f"{self.requests[rid][0]} failed: {message!r}")
        return message

    async def call(
        self, method: str, params: dict[str, Any] | None = None, session: str | None = None
    ) -> dict[str, Any]:
        message = await self.response(await self.send(method, params, session))
        result = message.get("result")
        if not isinstance(result, dict) or result.get("exceptionDetails") is not None:
            raise SmokeError(f"{method} returned an invalid result: {message!r}")
        return result

    async def evaluate(self, session: str, expression: str) -> Any:
        result = await self.call("Runtime.evaluate", {
            "expression": expression, "returnByValue": True
        }, session)
        return result.get("result", {}).get("value")

    def has_response(self, rid: int) -> bool:
        return any(m.get("id") == rid for m in self.messages)

    def index(self, rid: int) -> int:
        return next(i for i, m in enumerate(self.messages) if m.get("id") == rid)

    def assert_all_replied(self) -> None:
        assert_equal(sorted(m["id"] for m in self.messages if isinstance(m.get("id"), int)),
                     sorted(self.requests), "each admitted command has exactly one terminal")


@dataclass(frozen=True)
class _Page:
    session: str
    url: str
    frame: str
    node: int
    sheet: str
    css_event_index: int
    css_reply_index: int


async def _fixture(wire: _Wire, session: str, url: str) -> _Page:
    await wire.call("Page.enable", session=session)
    await wire.call("Page.setLifecycleEventsEnabled", {"enabled": True}, session)
    start = len(wire.messages)
    navigation = await wire.call("Page.navigate", {"url": url}, session)
    if navigation.get("errorText") or not navigation.get("loaderId"):
        raise SmokeError(f"local fixture navigation failed: {navigation!r}")
    # A navigate response does not promise a parsed Document. Match this
    # navigation's loader so initial-document replay cannot release the gate.
    await wire.until(lambda m: m.get("sessionId") == session
                     and m.get("method") == "Page.lifecycleEvent"
                     and m["params"].get("loaderId") == navigation["loaderId"]
                     and m["params"].get("name") == "DOMContentLoaded",
                     "fixture Document parsed", after=start)
    await wire.evaluate(session,
        "document.head.innerHTML = '<style>input {color: red}</style>';"
        "document.body.innerHTML = '<input id=probe>'; void 0")
    await wire.call("DOM.enable", session=session)
    document = await wire.call("DOM.getDocument", session=session)
    node = await wire.call("DOM.querySelector", {
        "nodeId": document["root"]["nodeId"], "selector": "#probe"
    }, session)
    if not node.get("nodeId"):
        raise SmokeError(f"inline fixture node is missing: {node!r}")
    tree = await wire.call("Page.getFrameTree", session=session)
    frame = tree["frameTree"]["frame"]["id"]
    start = len(wire.messages)
    enable = await wire.send("CSS.enable", session=session)
    await wire.response(enable)
    event = await wire.until(lambda m: m.get("sessionId") == session
                            and m.get("method") == "CSS.styleSheetAdded"
                            and m.get("params", {}).get("header", {}).get("frameId") == frame,
                            "fixture stylesheet event", after=start)
    event_index = wire.messages.index(event)
    if event_index >= wire.index(enable):
        raise SmokeError("CSS.styleSheetAdded must precede CSS.enable response")
    return _Page(session, url, frame, node["nodeId"], event["params"]["header"]["styleSheetId"],
                 event_index, wire.index(enable))


async def _native_prefix(wire: _Wire, page: _Page, _is_moli: bool) -> dict[str, Any]:
    session = page.session
    start = len(wire.messages)
    replacement = await wire.send("Runtime.evaluate", {
        "expression": "history.replaceState(null, '', '#ordering'); void 0"
    }, session)
    commands = [
        ("Page.getFrameTree", {}),
        ("Page.getResourceTree", {}),
        ("Page.getLayoutMetrics", {}),
        ("DOMSnapshot.captureSnapshot", {"computedStyles": ["display"]}),
        ("DOM.describeNode", {"nodeId": page.node}),
        ("CSS.getComputedStyleForNode", {"nodeId": page.node}),
        ("Accessibility.getFullAXTree", {}),
        ("DOM.resolveNode", {"nodeId": page.node, "objectGroup": "cdp-ordering"}),
        ("DOM.getAttributes", {"nodeId": page.node}),
        ("DOM.describeNode", {"backendNodeId": 2147483647}),
    ]
    ids = [await wire.send(method, params, session) for method, params in commands]
    enable = await wire.send("Runtime.enable", session=session)
    # Reading replies in reverse must never dictate their wire publication order.
    replies = {rid: await wire.response(rid, allow_error=rid == ids[-1]) for rid in reversed(ids)}
    replaced = await wire.response(replacement)
    if replaced["result"].get("exceptionDetails") is not None:
        raise SmokeError(f"same-document fixture mutation failed: {replaced!r}")
    await wire.response(enable)
    context = await wire.until(lambda m: m.get("sessionId") == session
                              and m.get("method") == "Runtime.executionContextCreated"
                              and m["params"]["context"].get("auxData", {}).get("isDefault") is True,
                              "Runtime default context replay", after=start)
    indexes = [wire.index(rid) for rid in ids]
    assert_equal(indexes, sorted(indexes), "ready native terminals retain Main publication order")
    if wire.index(replacement) >= min(indexes):
        raise SmokeError("native snapshots overtook the preceding synchronous history mutation")
    context_index = wire.messages.index(context)
    if not max(indexes) < context_index < wire.index(enable):
        raise SmokeError(f"native terminals/context/enable order: {indexes}, {context_index}, {wire.index(enable)}")
    assert_equal(replies[ids[-1]].get("error", {}).get("code"), -32000,
                 "native backend failure retains its ordered terminal")
    for rid in ids[:2]:
        frame = replies[rid]["result"]["frameTree"]["frame"]
        # Chromium represents the fragment separately; Moli currently includes
        # it in url. Check the effective URL without assuming identical fields.
        assert_equal(frame["url"] + frame.get("urlFragment", ""),
                     f"{page.url}#ordering", "tree reports live renderer URL")
    if not replies[ids[3]]["result"]["documents"] or not replies[ids[6]]["result"]["nodes"]:
        raise SmokeError("native DOM/AX snapshot unexpectedly empty")
    assert_equal(replies[ids[4]]["result"]["node"]["nodeName"], "INPUT", "native node snapshot")
    color = next(p["value"] for p in replies[ids[5]]["result"]["computedStyle"] if p["name"] == "color")
    assert_equal(color, "rgb(255, 0, 0)", "native computed style")
    assert_equal(replies[ids[8]]["result"]["attributes"], ["id", "probe"],
                 "native attributes share the renderer query and its ordered terminal")
    object_id = replies[ids[7]]["result"]["object"]["objectId"]
    resolved = await wire.call("Runtime.callFunctionOn", {
        "objectId": object_id, "functionDeclaration": "function() { return this.id; }",
        "returnByValue": True
    }, session)
    assert_equal(resolved["result"].get("value"), "probe", "object is usable immediately after native reply")
    await wire.call("Runtime.releaseObjectGroup", {"objectGroup": "cdp-ordering"}, session)
    return {"historyReplyIndex": wire.index(replacement),
            "nativeReplyIndexes": indexes, "contextEventIndex": context_index,
            "enableReplyIndex": wire.index(enable), "cssEventIndex": page.css_event_index,
            "cssEnableReplyIndex": page.css_reply_index}


async def _world_prefix(wire: _Wire, page: _Page, _is_moli: bool) -> dict[str, Any]:
    await wire.call("Runtime.enable", session=page.session)
    start = len(wire.messages)
    rid = await wire.send("Page.createIsolatedWorld", {
        "frameId": page.frame, "worldName": "cdp-ordering-world"
    }, page.session)
    reply = await wire.response(rid)
    context = reply["result"]["executionContextId"]
    event = await wire.until(lambda m: m.get("sessionId") == page.session
                            and m.get("method") == "Runtime.executionContextCreated"
                            and m["params"]["context"]["id"] == context,
                            "isolated world context", after=start)
    if wire.messages.index(event) >= wire.index(rid):
        raise SmokeError("isolated context notification must precede world response")
    return {"eventIndex": wire.messages.index(event), "replyIndex": wire.index(rid)}


async def _deferred_reply(wire: _Wire, page: _Page, _is_moli: bool) -> dict[str, Any]:
    await wire.evaluate(page.session, "globalThis.__gate = new Promise(r => globalThis.__resolve = r); void 0")
    held = await wire.send("Runtime.evaluate", {
        "expression": "__gate", "awaitPromise": True, "returnByValue": True
    }, page.session)
    later = await wire.send("Runtime.evaluate", {"expression": "6 * 7", "returnByValue": True}, page.session)
    reply = await wire.response(later)
    assert_equal(reply["result"]["result"].get("value"), 42, "later command proceeds while Promise pending")
    if wire.has_response(held):
        raise SmokeError("unresolved Promise replied before its resolver was sent")
    resolver = await wire.send("Runtime.evaluate", {"expression": "__resolve(42)"}, page.session)
    await wire.response(resolver)
    held_reply = await wire.response(held)
    assert_equal(held_reply["result"]["result"].get("value"), 42, "deferred command resolves")
    return {"laterReplyIndex": wire.index(later), "heldReplyIndex": wire.index(held)}


async def _pause(wire: _Wire, page: _Page, *, instrumentation: bool = False) -> int:
    await wire.call("Debugger.enable", session=page.session)
    if instrumentation:
        await wire.call("Debugger.setInstrumentationBreakpoint", {
            "instrumentation": "beforeScriptExecution"
        }, page.session)
    start = len(wire.messages)
    outer = await wire.send("Runtime.evaluate", {
        "expression": "21 * 2" if instrumentation else "(() => { debugger; return 42; })()",
        "returnByValue": True
    }, page.session)
    paused = await wire.until(lambda m: m.get("sessionId") == page.session
                             and m.get("method") == "Debugger.paused",
                             "Debugger pause", after=start)
    if instrumentation:
        assert_equal(paused["params"].get("reason"), "instrumentation", "instrumentation pause reason")
    if wire.has_response(outer):
        raise SmokeError("outer evaluation completed while paused")
    return outer


async def _normal_pause(wire: _Wire, page: _Page, _is_moli: bool) -> dict[str, Any]:
    outer = await _pause(wire, page)
    commands = [("Page.getFrameTree", {}), ("DOM.describeNode", {"nodeId": page.node}),
                ("CSS.getComputedStyleForNode", {"nodeId": page.node}),
                ("DOM.getAttributes", {"nodeId": page.node})]
    ids = [await wire.send(method, params, page.session) for method, params in commands]
    replies = {rid: await wire.response(rid) for rid in reversed(ids)}
    assert_equal(replies[ids[-1]]["result"]["attributes"], ["id", "probe"],
                 "native attributes resolve the frontend node while paused")
    if wire.has_response(outer):
        raise SmokeError("native queries must leave the outer evaluation paused")
    await wire.call("Debugger.resume", session=page.session)
    reply = await wire.response(outer)
    assert_equal(reply["result"]["result"].get("value"), 42, "outer evaluation resumes")
    return {"nestedReplyIndexes": [wire.index(rid) for rid in ids], "outerReplyIndex": wire.index(outer)}


async def _paused_dom_agent(wire: _Wire, page: _Page, _is_moli: bool) -> dict[str, Any]:
    point = await wire.evaluate(page.session,
        "(() => { const r = document.getElementById('probe').getBoundingClientRect();"
        "return {x: Math.floor(r.left + r.width / 2), y: Math.floor(r.top + r.height / 2)}; })()")
    node = await wire.call("DOM.describeNode", {"nodeId": page.node}, page.session)
    outer = await _pause(wire, page)
    commands = [
        ("DOM.getNodeForLocation", point),
        ("DOM.getNodeStackTraces", {"nodeId": page.node}),
        ("DOM.setNodeStackTracesEnabled", {"enable": True}),
        ("DOM.disable", {}),
    ]
    ids = [await wire.send(method, params, page.session) for method, params in commands]
    replies = [await wire.response(rid) for rid in ids]
    assert_equal(replies[0]["result"]["backendNodeId"], node["node"]["backendNodeId"],
                 "paused hit test finds the fixture input")
    assert_equal(replies[0]["result"]["frameId"], page.frame, "paused hit test frame")
    for reply, (method, _params) in zip(replies[1:], commands[1:]):
        assert_equal(reply["result"], {}, f"{method} completes during ordinary pause")
    # Disabling must clear the old frontend bindings before acknowledging it.
    stale = await wire.send("DOM.getAttributes", {"nodeId": page.node}, page.session)
    stale_reply = await wire.response(stale, allow_error=True)
    assert_equal(stale_reply.get("error", {}).get("code"), -32000,
                 "DOM.disable invalidates the original frontend node while paused")
    if wire.has_response(outer):
        raise SmokeError("DOM agent inspection must leave the outer evaluation paused")
    # All four actual SessionSink replies must arrive before the client even
    # sends resume. A typed backend query would miss the native-wrapper bug.
    resume_send_index = len(wire.trace)
    await wire.call("Debugger.resume", session=page.session)
    reply = await wire.response(outer)
    assert_equal(reply["result"]["result"].get("value"), 42, "outer evaluation resumes once")
    return {"pausedReplyIndexes": [wire.index(rid) for rid in ids],
            "resumeSendTraceIndex": resume_send_index, "outerReplyIndex": wire.index(outer)}


async def _instrumentation_pause(wire: _Wire, page: _Page, _is_moli: bool) -> dict[str, Any]:
    outer = await _pause(wire, page, instrumentation=True)
    query = await wire.send("Page.getFrameTree", session=page.session)
    io = await wire.send("Performance.getMetrics", session=page.session)
    await wire.response(io)
    # IO is a liveness observation, not a general Main/IO ordering barrier.
    if wire.has_response(query) or wire.has_response(outer):
        raise SmokeError("instrumentation pause admitted ordinary Main work at the IO observation")
    resume = await wire.send("Debugger.resume", session=page.session)
    await wire.response(resume)
    await wire.response(query)
    reply = await wire.response(outer)
    assert_equal(reply["result"]["result"].get("value"), 42, "instrumented evaluation resumes")
    if wire.index(query) < wire.index(io):
        raise SmokeError("ordinary Main query overtook the instrumentation IO observation")
    return {"ioReplyIndex": wire.index(io), "queryReplyIndex": wire.index(query),
            "outerReplyIndex": wire.index(outer)}


async def _focus_reentry(wire: _Wire, page: _Page, _is_moli: bool) -> dict[str, Any]:
    await wire.evaluate(page.session, "document.getElementById('probe').addEventListener('focus', () => {"
                        "debugger; globalThis.__focusFinished = true; }); void 0")
    await wire.call("Debugger.enable", session=page.session)
    start = len(wire.messages)
    focus = await wire.send("DOM.focus", {"nodeId": page.node}, page.session)
    await wire.until(lambda m: m.get("sessionId") == page.session and m.get("method") == "Debugger.paused",
                     "focus callback pause", after=start)
    query = await wire.send("Page.getFrameTree", session=page.session)
    await wire.response(query)
    if wire.has_response(focus):
        raise SmokeError("DOM.focus must remain pending during its paused callback")
    await wire.call("Debugger.resume", session=page.session)
    await wire.response(focus)
    assert_equal(await wire.evaluate(page.session, "__focusFinished"), True, "focus callback finished")
    return {"nestedReplyIndex": wire.index(query), "focusReplyIndex": wire.index(focus)}


async def _paused_mutation(wire: _Wire, page: _Page, is_moli: bool) -> dict[str, Any]:
    method, params, expression, expected = {
        "paused_dom_attribute": ("DOM.setAttributeValue", {
            "nodeId": page.node, "name": "data-probe", "value": "done"
        }, "document.getElementById('probe').getAttribute('data-probe')", "done"),
        "paused_stylesheet_edit": ("CSS.setStyleSheetText", {
            "styleSheetId": page.sheet, "text": "input {color: blue}"
        }, "getComputedStyle(document.getElementById('probe')).color", "rgb(0, 0, 255)"),
        "paused_document_content": ("Page.setDocumentContent", {
            "frameId": page.frame, "html": "<!doctype html><body>replacement</body>"
        }, "document.body.textContent", "replacement"),
        "paused_navigator_configuration": ("Emulation.setHardwareConcurrencyOverride", {
            "hardwareConcurrency": 4
        }, "navigator.hardwareConcurrency", 4),
    }[wire.scenario]
    outer = await _pause(wire, page)
    mutation = await wire.send(method, params, page.session)
    if not is_moli:
        # Chromium must actually finish the mutation before we send resume.
        await wire.response(mutation)
    io = await wire.send("Performance.getMetrics", session=page.session)
    await wire.response(io)
    assert_equal(wire.has_response(mutation), not is_moli,
                 f"{method} {'Moli owner-only' if is_moli else 'Chromium nested'} entry at IO observation")
    if wire.has_response(outer):
        raise SmokeError(f"{method} unexpectedly resumed the outer evaluation")
    resume_send_index = len(wire.trace)
    resume = await wire.send("Debugger.resume", session=page.session)
    await wire.response(resume)
    await wire.response(mutation)
    reply = await wire.response(outer)
    assert_equal(reply["result"]["result"].get("value"), 42, "paused evaluation resumes exactly once")
    assert_equal(await wire.evaluate(page.session, expression), expected, f"{method} mutation takes effect")
    return {"mutation": method, "engine": "moli" if is_moli else "chromium",
            "replyDuringPause": not is_moli, "ioReplyIndex": wire.index(io),
            "resumeSendTraceIndex": resume_send_index, "mutationReplyIndex": wire.index(mutation),
            "outerReplyIndex": wire.index(outer), "value": expected}


async def run_cdp_ordering_group(endpoint: str, fixture_url: str, results: list[dict[str, Any]]) -> None:
    is_moli = (await discover_websocket_url(endpoint)).endswith("/devtools/browser/moli-browser")
    scenarios = [
        ("native_terminal_notification_prefix", _native_prefix),
        ("isolated_world_notification_prefix", _world_prefix),
        ("deferred_reply_later_resolver", _deferred_reply),
        ("normal_pause_native_queries", _normal_pause),
        ("normal_pause_dom_agent_commands", _paused_dom_agent),
        ("instrumentation_pause_io_only", _instrumentation_pause),
        ("native_focus_callback_reentry", _focus_reentry),
        ("paused_dom_attribute", _paused_mutation),
        ("paused_stylesheet_edit", _paused_mutation),
        ("paused_document_content", _paused_mutation),
        ("paused_navigator_configuration", _paused_mutation),
    ]
    failures = []
    for name, scenario in scenarios:
        client = await connect_raw_cdp(endpoint)
        wire = _Wire(client, name)
        context_id = None
        session = None
        try:
            context = await wire.call("Target.createBrowserContext")
            context_id = context["browserContextId"]
            target = await wire.call("Target.createTarget", {"url": "about:blank", "browserContextId": context_id})
            attached = await wire.call("Target.attachToTarget", {"targetId": target["targetId"], "flatten": True})
            session = attached["sessionId"]
            page = await _fixture(wire, session, f"{fixture_url}/plain")
            observed = await scenario(wire, page, is_moli)
            # A same-Main renderer follower drains earlier ready terminals; a
            # Browser-only response would not be such a publication boundary.
            await wire.call("Page.getFrameTree", session=session)
            await wire.call("Target.disposeBrowserContext", {"browserContextId": context_id})
            context_id = None
            wire.assert_all_replied()
            record_contract(results, f"cdp_ordering_{name}",
                contract=("Moli owner-only mutations wait for resume; Chromium completes them in ordinary pause."
                          if scenario is _paused_mutation else "Renderer ready replies and notifications retain causal order without blocking deferred work."),
                source=(f"{CHROMIUM_SOURCE}; Moli owner-only VM entry limitation" if scenario is _paused_mutation else CHROMIUM_SOURCE),
                commands=list(dict.fromkeys(method for method, _session in wire.requests.values())),
                observed={**observed, "wire": wire.trace})
        except Exception as error:
            failures.append(f"{name}: {type(error).__name__}: {error}")
            results.append({"name": f"cdp_ordering_{name}", "ok": False,
                            "error": str(error), "wire": wire.trace})
        finally:
            if context_id is not None:
                with contextlib.suppress(Exception):
                    async with asyncio.timeout(3):
                        if session:
                            await wire.call("Debugger.resume", session=session)
                        await wire.call("Target.disposeBrowserContext", {"browserContextId": context_id})
            await client.websocket.close()
    if failures:
        raise SmokeError("CDP ordering contracts failed: " + "; ".join(failures))
