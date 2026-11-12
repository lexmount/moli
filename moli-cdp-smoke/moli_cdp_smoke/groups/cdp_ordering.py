"""Renderer publication contracts calibrated against Chromium 145.0.7632.116.

Use the actual WebSocket order, retaining replies received while waiting for
another id. Timeouts only bound liveness; no sleep or retry proves ordering.
The pending-evaluation frame and download cases are also calibrated against Chromium
156.0.8078.73 (2026-11-12). The six mutation cases explicitly distinguish Moli's current owner-only entry
from Chromium's ordinary-pause capability. They are not CDP-wide requirements.
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import sys
import tempfile
from dataclasses import dataclass, field
from typing import Any, Callable

from ..assertions import SmokeError, assert_equal, record_contract
from ..raw_cdp import RawCdpClient, connect_raw_cdp, discover_browser_version


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
    root: int
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
        "document.body.innerHTML = '<input id=probe><section id=children>"
        "<div><span>nested</span></div></section>'; void 0")
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
    return _Page(session, url, frame, node["nodeId"], document["root"]["nodeId"],
                 event["params"]["header"]["styleSheetId"],
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
        ("CSS.getInlineStylesForNode", {"nodeId": page.node}),
        ("CSS.getMatchedStylesForNode", {"nodeId": page.node}),
        ("DOM.getOuterHTML", {"nodeId": page.node}),
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
    html = replies[ids[11]]["result"]["outerHTML"]
    if '<input' not in html or 'id="probe"' not in html:
        raise SmokeError(f"native outer HTML did not serialize the selected node: {html!r}")
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


async def _frame_facts_during_pending_evaluation(
    wire: _Wire, page: _Page, _is_moli: bool
) -> dict[str, Any]:
    # A second command proves the first evaluation reached its unresolved
    # Promise. Event receipt and exact frame identity establish order; no timer
    # or assumption about an otherwise empty protocol queue is involved.
    checks = []
    frame_id = None
    for method, mutation in [
        ("Page.frameAttached", "globalThis.__dependencyFrame = document.createElement('iframe');"
         "document.body.append(globalThis.__dependencyFrame);"),
        ("Page.frameDetached", "globalThis.__dependencyFrame.remove();"),
    ]:
        start = len(wire.messages)
        pending = await wire.send("Runtime.evaluate", {
            "expression": "(() => {" + mutation
                + "return new Promise(resolve => { globalThis.__dependencyRelease = resolve; }); })()",
            "awaitPromise": True, "returnByValue": True,
        }, page.session)
        assert_equal(await wire.evaluate(page.session,
            "typeof globalThis.__dependencyRelease === 'function'"), True,
            "mutation reached its unresolved Promise")
        event = await wire.until(
            lambda message: message.get("sessionId") == page.session
                and message.get("method") == method
                and (message.get("params", {}).get("parentFrameId") == page.frame
                     if frame_id is None else message.get("params", {}).get("frameId") == frame_id),
            method + " while the invoking evaluation is pending", after=start)
        if frame_id is None:
            frame_id = event["params"]["frameId"]
        assert_equal(wire.has_response(pending), False,
                     "frame fact must not depend on the original Runtime reply")
        await wire.evaluate(page.session,
            "globalThis.__dependencyRelease('released'); globalThis.__dependencyRelease = null; 1")
        reply = await wire.response(pending)
        assert_equal(reply["result"]["result"].get("value"), "released",
                     "the original evaluation resumes with its real result")
        matches = [message for message in wire.messages[start:]
                   if message.get("sessionId") == page.session and message.get("method") == method
                   and message.get("params", {}).get("frameId") == frame_id]
        assert_equal(len(matches), 1, "each frame lifecycle fact is delivered once")
        event_index = wire.messages.index(event)
        if event_index >= wire.index(pending):
            raise SmokeError(method + " overtook its recorded reply position")
        checks.append({"method": method, "frame": frame_id,
                       "eventIndex": event_index, "replyIndex": wire.index(pending)})
    return {"checks": checks}


async def _navigation_during_pending_evaluation(
    wire: _Wire, page: _Page, _is_moli: bool
) -> dict[str, Any]:
    await wire.call("Runtime.enable", session=page.session)
    await wire.call("Target.setDiscoverTargets", {"discover": True})
    opening = len(wire.messages)
    result = await wire.call("Runtime.evaluate", {
        "expression": "globalThis.__navigationChild = window.open('about:blank');"
                      "!!globalThis.__navigationChild",
        "userGesture": True, "returnByValue": True,
    }, page.session)
    assert_equal(result["result"].get("value"), True, "navigation fixture popup opened")
    created = await wire.until(
        lambda message: message.get("method") == "Target.targetCreated"
            and message.get("params", {}).get("targetInfo", {}).get("type") == "page"
            and message["params"]["targetInfo"].get("openerFrameId") == page.frame,
        "exact popup target", after=opening)
    child_target = created["params"]["targetInfo"]["targetId"]
    child_session = (await wire.call("Target.attachToTarget", {
        "targetId": child_target, "flatten": True,
    }))["sessionId"]
    await wire.call("Page.enable", session=child_session)
    await wire.call("Page.setLifecycleEventsEnabled", {"enabled": True}, child_session)
    await wire.call("Network.enable", session=child_session)
    destination = page.url + "?navigation-owner-progress"
    start = len(wire.messages)
    pending = await wire.send("Runtime.evaluate", {
        "expression": "(() => { console.log('owner-before-navigation');"
                      "__navigationChild.location.href = " + json.dumps(destination) + ";"
                      "console.log('owner-after-navigation');"
                      "return new Promise(resolve => { globalThis.__navigationRelease = resolve; }); })()",
        "awaitPromise": True, "returnByValue": True,
    }, page.session)
    assert_equal(await wire.evaluate(page.session,
        "typeof globalThis.__navigationRelease === 'function'"), True,
        "navigation source reached its unresolved Promise")
    source_events = []
    for marker in ["owner-before-navigation", "owner-after-navigation"]:
        source_events.append(await wire.until(
            lambda message: message.get("sessionId") == page.session
                and message.get("method") == "Runtime.consoleAPICalled"
                and message.get("params", {}).get("args", [{}])[0].get("value") == marker,
            marker, after=start))
    committed = await wire.until(
        lambda message: message.get("sessionId") == child_session
            and message.get("method") == "Page.frameNavigated"
            and message.get("params", {}).get("frame", {}).get("url") == destination,
        "child navigation while its source evaluation waits", after=start)
    frame = committed["params"]["frame"]
    metadata = await wire.until(
        lambda message: message.get("sessionId") == child_session
            and message.get("method") == "Network.responseReceived"
            and message.get("params", {}).get("type") == "Document"
            and message["params"].get("loaderId") == frame["loaderId"]
            and message["params"].get("response", {}).get("url") == destination,
        "exact navigation response metadata", after=start)
    loaded = await wire.until(
        lambda message: message.get("sessionId") == child_session
            and message.get("method") == "Page.lifecycleEvent"
            and message.get("params", {}).get("name") == "load"
            and message["params"].get("loaderId") == frame["loaderId"]
            and message["params"].get("frameId") == frame["id"],
        "exact child navigation load", after=start)
    assert_equal(wire.has_response(pending), False,
                 "navigation must not wait for its source evaluation response")
    before_index, after_index = [wire.messages.index(event) for event in source_events]
    metadata_index, commit_index, load_index = [
        wire.messages.index(event) for event in [metadata, committed, loaded]
    ]
    if before_index >= after_index or not metadata_index < commit_index < load_index:
        raise SmokeError("source facts and navigation metadata/commit/load lost their causal order")
    await wire.evaluate(page.session,
        "__navigationRelease('navigation-complete'); __navigationRelease = null; 1")
    reply = await wire.response(pending)
    assert_equal(reply["result"]["result"].get("value"), "navigation-complete",
                 "source evaluation preserves its real result")
    for event in [*source_events, metadata, committed, loaded]:
        assert_equal(sum(message == event for message in wire.messages[start:]), 1,
                     "each source and navigation fact is delivered once")
    await wire.call("Target.closeTarget", {"targetId": child_target})
    return {"beforeIndex": before_index, "afterIndex": after_index,
            "metadataIndex": metadata_index, "commitIndex": commit_index,
            "loadIndex": load_index, "sourceReplyIndex": wire.index(pending)}


async def _download_during_pending_evaluation(
    wire: _Wire, page: _Page, _is_moli: bool
) -> dict[str, Any]:
    info = await wire.call("Target.getTargetInfo", session=page.session)
    context = info["targetInfo"]["browserContextId"]
    with tempfile.TemporaryDirectory(prefix="moli-ordering-download-") as download_path:
        await wire.call("Browser.setDownloadBehavior", {
            "behavior": "allow", "downloadPath": download_path,
            "browserContextId": context, "eventsEnabled": True,
        })
        start = len(wire.messages)
        pending = await wire.send("Runtime.evaluate", {
            "expression": "(() => { const a = document.createElement('a');"
                "a.href='data:text/plain,download-probe'; a.download='probe.txt';"
                "document.body.append(a); a.click();"
                "return new Promise(resolve => { globalThis.__downloadRelease = resolve; }); })()",
            "awaitPromise": True, "returnByValue": True, "userGesture": True,
        }, page.session)
        assert_equal(await wire.evaluate(page.session,
            "typeof globalThis.__downloadRelease === 'function'"), True,
            "download activation reached its unresolved Promise")
        event = await wire.until(lambda message: message.get("method") == "Browser.downloadWillBegin",
                                 "download start while Runtime reply remains pending", after=start)
        assert_equal(wire.has_response(pending), False,
                     "download activation must not depend on its invoking Runtime reply")
        guid = event["params"]["guid"]
        await wire.until(lambda message: message.get("method") == "Browser.downloadProgress"
                         and message["params"].get("guid") == guid
                         and message["params"].get("state") == "completed",
                         "exact download completion while evaluation remains pending", after=start)
        assert_equal(wire.has_response(pending), False,
                     "download completion must remain independent of the unresolved Promise")
        await wire.evaluate(page.session,
            "globalThis.__downloadRelease('released'); globalThis.__downloadRelease = null; 1")
        reply = await wire.response(pending)
        assert_equal(reply["result"]["result"].get("value"), "released",
                     "download does not alter the eventual JavaScript result")
        return {"guid": guid, "eventIndex": wire.messages.index(event),
                "replyIndex": wire.index(pending)}


async def _normal_pause(wire: _Wire, page: _Page, _is_moli: bool) -> dict[str, Any]:
    await wire.evaluate(page.session,
        "document.getElementById('probe').setAttribute('style', 'outline-width: 3px'); void 0")
    children = await wire.call("DOM.querySelector", {
        "nodeId": page.root, "selector": "#children"
    }, page.session)
    outer = await _pause(wire, page)
    start = len(wire.messages)
    commands = [("Page.getFrameTree", {}), ("DOM.describeNode", {"nodeId": page.node}),
                ("CSS.getComputedStyleForNode", {"nodeId": page.node}),
                ("DOM.getAttributes", {"nodeId": page.node}),
                ("CSS.getInlineStylesForNode", {"nodeId": page.node}),
                ("CSS.getMatchedStylesForNode", {"nodeId": page.node}),
                ("DOM.getOuterHTML", {"nodeId": page.node}),
                ("DOM.querySelector", {"nodeId": page.root, "selector": "#probe"}),
                ("DOM.querySelectorAll", {"nodeId": page.root, "selector": "#probe"}),
                ("DOM.requestChildNodes", {"nodeId": children["nodeId"], "depth": 2}),
                ("Accessibility.getPartialAXTree", {"nodeId": page.node}),
                ("DOM.getBoxModel", {"nodeId": page.node})]
    ids = [await wire.send(method, params, page.session) for method, params in commands]
    ids_by_method = {method: rid for (method, _), rid in zip(commands, ids)}
    replies = {method: await wire.response(rid)
               for (method, _), rid in zip(reversed(commands), reversed(ids))}
    assert_equal(replies["DOM.getAttributes"]["result"]["attributes"],
                 ["id", "probe", "style", "outline-width: 3px"],
                 "native attributes resolve the frontend node while paused")
    for method in ("CSS.getInlineStylesForNode", "CSS.getMatchedStylesForNode"):
        properties = replies[method]["result"]["inlineStyle"]["cssProperties"]
        width = next(p["value"] for p in properties if p["name"] == "outline-width")
        assert_equal(width, "3px", "inline style resolves the frontend node while paused")
    html = replies["DOM.getOuterHTML"]["result"]["outerHTML"]
    if 'id="probe"' not in html or 'outline-width: 3px' not in html:
        raise SmokeError(f"paused outer HTML did not serialize the live node: {html!r}")
    assert_equal(replies["DOM.querySelector"]["result"]["nodeId"], page.node,
                 "paused selector preserves the frontend node binding")
    assert_equal(replies["DOM.querySelectorAll"]["result"]["nodeIds"], [page.node],
                 "paused selector-all returns the live input")
    child_event = await wire.until(
        lambda m: m.get("sessionId") == page.session
        and m.get("method") == "DOM.setChildNodes"
        and m["params"].get("parentId") == children["nodeId"],
        "paused child-node notification", after=start)
    if wire.messages.index(child_event) >= wire.index(ids_by_method["DOM.requestChildNodes"]):
        raise SmokeError("child snapshots must be published before requestChildNodes response")
    nodes = child_event["params"]["nodes"]
    assert_equal(nodes[0]["nodeName"], "DIV", "paused child capture reads the live subtree")
    assert_equal(nodes[0]["children"][0]["nodeName"], "SPAN", "paused child capture retains depth")
    backend = replies["DOM.describeNode"]["result"]["node"]["backendNodeId"]
    if not any(node.get("backendDOMNodeId") == backend
               for node in replies["Accessibility.getPartialAXTree"]["result"]["nodes"]):
        raise SmokeError("paused AX query did not resolve the fixture node")
    model = replies["DOM.getBoxModel"]["result"]["model"]
    assert_equal(len(model["content"]), 8, "paused geometry returns a content quad")
    if model["width"] <= 0 or model["height"] <= 0:
        raise SmokeError(f"paused geometry has no input layout: {model!r}")
    indexes = [wire.index(rid) for rid in ids]
    assert_equal(indexes, sorted(indexes), "nested ready replies retain Main publication order")
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
    if wire.scenario == "paused_dom_scroll":
        await wire.evaluate(page.session,
            "document.getElementById('probe').style.marginTop = '3000px'; void 0")
    method, params, expression, expected = {
        "paused_dom_attribute": ("DOM.setAttributeValue", {
            "nodeId": page.node, "name": "data-probe", "value": "done"
        }, "document.getElementById('probe').getAttribute('data-probe')", "done"),
        "paused_dom_remove": ("DOM.removeNode", {"nodeId": page.node},
            "document.getElementById('probe') === null", True),
        "paused_dom_scroll": ("DOM.scrollIntoViewIfNeeded", {"nodeId": page.node},
            "window.scrollY > 0", True),
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
    is_moli = isinstance((await discover_browser_version(endpoint)).get("Moli-Version"), str)
    scenarios = [
        ("native_terminal_notification_prefix", _native_prefix),
        ("isolated_world_notification_prefix", _world_prefix),
        ("deferred_reply_later_resolver", _deferred_reply),
        ("frame_facts_during_pending_evaluation", _frame_facts_during_pending_evaluation),
        ("navigation_during_pending_evaluation", _navigation_during_pending_evaluation),
        ("download_during_pending_evaluation", _download_during_pending_evaluation),
        ("normal_pause_native_queries", _normal_pause),
        ("normal_pause_dom_agent_commands", _paused_dom_agent),
        ("instrumentation_pause_io_only", _instrumentation_pause),
        ("native_focus_callback_reentry", _focus_reentry),
        ("paused_dom_attribute", _paused_mutation),
        ("paused_dom_remove", _paused_mutation),
        ("paused_dom_scroll", _paused_mutation),
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
