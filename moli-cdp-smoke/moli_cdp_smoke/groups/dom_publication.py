"""Validate the client's DOM mirror in WebSocket order, before each reply."""

from __future__ import annotations

import asyncio
import contextlib
import json
import sys
from dataclasses import dataclass, field
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..navigation import command, navigate, receive_response
from ..raw_cdp import RawCdpClient, connect_raw_cdp


def _descendants(node: dict[str, Any]):
    for relation in ("children", "shadowRoots", "pseudoElements"):
        for child in node.get(relation, []):
            yield relation, child
    for relation in ("contentDocument", "templateContent"):
        if isinstance(node.get(relation), dict):
            yield relation, node[relation]


@dataclass
class _DomMirror:
    nodes: dict[int, dict[str, Any]] = field(default_factory=dict)
    parents: dict[int, int] = field(default_factory=dict)
    relations: dict[int, str] = field(default_factory=dict)

    def require(self, node_id: int, label: str, *, allow_zero: bool = False) -> None:
        if allow_zero and node_id == 0:
            return
        if not node_id or node_id not in self.nodes:
            raise SmokeError(f"{label} references unpublished nodeId={node_id}")

    def clear(self) -> None:
        self.nodes.clear()
        self.parents.clear()
        self.relations.clear()

    def forget(self, node_id: int) -> None:
        for child in [child for child, parent in self.parents.items() if parent == node_id]:
            self.forget(child)
        self.nodes.pop(node_id, None)
        self.parents.pop(node_id, None)
        self.relations.pop(node_id, None)

    def remember(self, node: dict[str, Any], parent: int = 0, relation: str = "children") -> None:
        node_id = node["nodeId"]
        if not node_id:
            raise SmokeError(f"DOM tree snapshot contains nodeId 0: {node!r}")
        self.require(parent, "snapshot parent", allow_zero=True)
        self.require(node.get("parentId", 0), "snapshot parentId", allow_zero=True)
        # A replacement snapshot retires its old descendants, including frame
        # documents and shadow roots. Merely remembering IDs would miss this.
        self.forget(node_id)
        self.nodes[node_id] = node
        self.parents[node_id] = parent
        self.relations[node_id] = relation
        for child_relation, child in _descendants(node):
            self.remember(child, node_id, child_relation)

    def describe(self, node: dict[str, Any]) -> None:
        for field_name in ("nodeId", "parentId"):
            self.require(node.get(field_name, 0), f"describeNode {field_name}", allow_zero=True)
        for _, child in _descendants(node):
            self.describe(child)

    def observe(self, message: dict[str, Any]) -> None:
        method, params = message.get("method"), message.get("params", {})
        if method == "DOM.documentUpdated":
            self.clear()
        elif method == "DOM.setChildNodes":
            parent = params["parentId"]
            self.require(parent, method, allow_zero=True)
            if parent:
                for child in [child for child, owner in self.parents.items()
                              if owner == parent and self.relations[child] == "children"]:
                    self.forget(child)
            for node in params["nodes"]:
                self.remember(node, parent)
        elif method == "DOM.childNodeInserted":
            parent, previous = params["parentNodeId"], params["previousNodeId"]
            self.require(parent, method)
            self.require(previous, "previousNodeId", allow_zero=True)
            if previous:
                assert_equal(self.parents[previous], parent, "previous sibling parent")
            self.remember(params["node"], parent)
        elif method == "DOM.childNodeRemoved":
            self.require(params["parentNodeId"], method)
            self.require(params["nodeId"], method)
            assert_equal(self.parents[params["nodeId"]], params["parentNodeId"], "removed node parent")
            self.forget(params["nodeId"])
        elif method in ("DOM.attributeModified", "DOM.attributeRemoved",
                        "DOM.characterDataModified", "DOM.childNodeCountUpdated"):
            self.require(params["nodeId"], method)
        elif method == "DOM.shadowRootPushed":
            self.require(params["hostId"], method)
            self.remember(params["root"], params["hostId"], "shadowRoots")
        elif method == "DOM.shadowRootPopped":
            self.require(params["hostId"], method)
            self.require(params["rootId"], method)
            self.forget(params["rootId"])


@dataclass
class _Probe:
    client: RawCdpClient
    mirrors: dict[str, _DomMirror] = field(default_factory=dict)
    messages: list[dict[str, Any]] = field(default_factory=list)

    def observe(self, message: dict[str, Any]) -> None:
        self.messages.append(message)
        if mirror := self.mirrors.get(message.get("sessionId", "")):
            mirror.observe(message)

    async def call(self, method: str, params: dict[str, Any] | None = None,
                   session: str | None = None) -> dict[str, Any]:
        result, _, _ = await command(self.client, method, params, session_id=session,
                                     on_message=self.observe)
        if result.get("exceptionDetails"):
            raise SmokeError(f"{method} threw: {result!r}")
        if mirror := self.mirrors.get(session or ""):
            if method == "DOM.getDocument":
                mirror.clear()
                mirror.remember(result["root"])
            elif method == "DOM.describeNode":
                mirror.describe(result["node"])
            elif method in ("DOM.getSearchResults", "DOM.pushNodesByBackendIdsToFrontend"):
                for node_id in result["nodeIds"]:
                    mirror.require(node_id, f"{method} response",
                                   allow_zero=method == "DOM.pushNodesByBackendIdsToFrontend")
            elif method == "DOM.requestNode":
                mirror.require(result["nodeId"], "requestNode response")
            elif method in ("DOM.getFrameOwner", "DOM.getNodeForLocation") and "nodeId" in result:
                mirror.require(result["nodeId"], f"{method} response")
        return result

    async def attach(self, target: str) -> str:
        session = (await self.call("Target.attachToTarget", {
            "targetId": target, "flatten": True,
        }))["sessionId"]
        self.mirrors[session] = _DomMirror()
        return session

    async def evaluate(self, session: str, expression: str) -> dict[str, Any]:
        return (await self.call("Runtime.evaluate", {
            "expression": expression, "awaitPromise": True,
        }, session))["result"]

    async def document(self, session: str, depth: int = 1) -> None:
        await self.call("DOM.getDocument", {"depth": depth, "pierce": False}, session)

    async def search(self, session: str, query: str, count: int, *, ua: bool = False) -> list[int]:
        search = await self.call("DOM.performSearch", {
            "query": query, "includeUserAgentShadowDOM": ua,
        }, session)
        assert_equal(search["resultCount"], count, f"{query} result count")
        result = await self.call("DOM.getSearchResults", {
            "searchId": search["searchId"], "fromIndex": 0, "toIndex": count,
        }, session)
        await self.call("DOM.discardSearchResults", {"searchId": search["searchId"]}, session)
        return result["nodeIds"]


_HTML = """<main><section id="branch"><a id="target" class="hit light-hit">before</a>
<a id="second" class="hit light-hit">second</a><a id="third" class="hit light-hit">third</a>
</section></main><div id="host"><em id="retained">retained</em></div>
<input id="control" type="search" value="needle">"""


async def _install(probe: _Probe, session: str) -> None:
    await probe.evaluate(session, "(async () => {document.body.innerHTML = " + json.dumps(_HTML) + ";"
        "const frame=document.createElement('iframe');frame.id='frame';"
        "const loaded=new Promise(resolve=>frame.onload=resolve);"
        "frame.srcdoc='<main><a id=frame-hit class=hit>frame</a></main>';"
        "document.body.append(frame);await loaded;return true;})()")


async def _describe_search(probe: _Probe, session: str, ancestor: bool, backend: bool) -> None:
    selector = "#branch" if ancestor else "#target"
    obj = await probe.evaluate(session, f"document.querySelector({json.dumps(selector)})")
    described = await probe.call("DOM.describeNode", {"objectId": obj["objectId"], "depth": 2}, session)
    assert_equal(described["node"]["nodeId"], 0, "cold describeNode ID")
    if backend:
        again = await probe.call("DOM.describeNode", {
            "backendNodeId": described["node"]["backendNodeId"], "depth": 2,
        }, session)
        assert_equal(again["node"]["nodeId"], 0, "cold backend describeNode ID")
    ids = await probe.search(session, "#target", 1)
    described = await probe.call("DOM.describeNode", {"nodeId": ids[0]}, session)
    assert_equal(described["node"]["nodeId"], ids[0], "published describeNode ID")
    assert_equal(await probe.search(session, "#target", 1), ids, "repeat search identity")


async def _request_search(probe: _Probe, session: str, ancestor: bool) -> None:
    selector = "#branch" if ancestor else "#target"
    obj = await probe.evaluate(session, f"document.querySelector({json.dumps(selector)})")
    requested = await probe.call("DOM.requestNode", {"objectId": obj["objectId"]}, session)
    assert_equal(await probe.call("DOM.requestNode", {"objectId": obj["objectId"]}, session),
                 requested, "repeat requestNode identity")
    await probe.search(session, "#target", 1)


async def _ranges(probe: _Probe, session: str) -> None:
    search = await probe.call("DOM.performSearch", {"query": ".light-hit"}, session)
    assert_equal(search["resultCount"], 3, "range result count")
    params = {"searchId": search["searchId"], "fromIndex": 1, "toIndex": 2}
    result = await probe.call("DOM.getSearchResults", params, session)
    attrs = (await probe.call("DOM.getAttributes", {"nodeId": result["nodeIds"][0]}, session))["attributes"]
    assert_equal(dict(zip(attrs[::2], attrs[1::2]))["id"], "second", "requested result range")
    before = len(probe.messages)
    assert_equal(await probe.call("DOM.getSearchResults", params, session), result, "repeat range")
    assert_equal([m for m in probe.messages[before:] if m.get("method") == "DOM.setChildNodes"], [],
                 "already published range needs no repeated path")
    await probe.call("DOM.getSearchResults", {**params, "fromIndex": 0, "toIndex": 3}, session)


async def _mixed(probe: _Probe, session: str) -> None:
    await probe.evaluate(session, "document.getElementById('host').attachShadow({mode:'closed'})"
                         ".innerHTML='<section><a id=shadow-hit class=hit>shadow</a></section>'")
    await probe.search(session, ".hit", 5)


async def _ua(probe: _Probe, session: str) -> None:
    hidden = await probe.call("DOM.performSearch", {"query": "#editing-view-port"}, session)
    assert_equal(hidden["resultCount"], 0, "UA nodes excluded by default")
    await probe.search(session, "#editing-view-port", 1, ua=True)


async def _detached(probe: _Probe, session: str) -> None:
    search = await probe.call("DOM.performSearch", {"query": "#target"}, session)
    assert_equal(search["resultCount"], 1, "retained search result")
    await probe.evaluate(session, "window.retained=document.getElementById('target');retained.remove()")
    result = await probe.call("DOM.getSearchResults", {
        "searchId": search["searchId"], "fromIndex": 0, "toIndex": 1,
    }, session)
    assert_equal(probe.mirrors[session].nodes[result["nodeIds"][0]]["localName"], "a", "detached node")


async def _associated(probe: _Probe, session: str, frame: bool, depth: int) -> None:
    await probe.document(session, depth)
    owner = "frame" if frame else "host"
    root = next(node_id for node_id, node in probe.mirrors[session].nodes.items()
                if node["nodeType"] == 9 and probe.mirrors[session].parents[node_id] == 0)
    await probe.call("DOM.querySelector", {"nodeId": root, "selector": f"#{owner}"}, session)
    if frame:
        await probe.evaluate(session, "new Promise(resolve => {const f=document.getElementById('frame');"
            "f.onload=()=>resolve(true);f.srcdoc='<section><a id=replacement>new</a></section>';})")
        query = "#replacement"
    else:
        await probe.evaluate(session, "document.getElementById('host').attachShadow({mode:'closed'})"
            ".innerHTML='<section><a id=replacement>shadow</a></section>'")
        query = "#replacement"
    await probe.search(session, query, 1)
    # Previously published light children remain usable after the owner gains
    # an associated tree; searching it must repair any replacement snapshot.
    if not frame and depth == -1:
        await probe.search(session, "#retained", 1)


async def _parent_refresh(probe: _Probe, session: str, whitespace: bool) -> None:
    await probe.call("DOM.enable", {"includeWhitespace": "all" if whitespace else "none"}, session)
    await probe.evaluate(session, "document.getElementById('host').attachShadow({mode:'closed'})"
                         ".innerHTML='<section><a id=shadow-hit>shadow</a></section>'")
    await probe.call("DOM.getDocument", {"depth": -1, "pierce": True}, session)
    await probe.search(session, "#frame-hit", 1)
    await probe.search(session, "#shadow-hit", 1)
    await probe.search(session, "#editing-view-port", 1, ua=True)
    mirror = probe.mirrors[session]
    body = next(node_id for node_id, node in mirror.nodes.items() if node["nodeName"] == "BODY")
    retained = set(mirror.nodes)
    for _ in range(2):
        # The same complete child-snapshot publisher services mutation resyncs.
        # A shallow replacement must retain every still-live published ID,
        # including frame documents and generated/author shadow descendants.
        await probe.call("DOM.requestChildNodes", {"nodeId": body, "depth": 1}, session)
        missing = retained - mirror.nodes.keys()
        if missing:
            raise SmokeError(f"parent replacement retired live published nodes: {sorted(missing)}")
        await probe.search(session, "#frame-hit", 1)
        await probe.search(session, "#shadow-hit", 1)
        await probe.search(session, "#editing-view-port", 1, ua=True)
    await probe.evaluate(session, "document.getElementById('frame').contentDocument"
                         ".getElementById('frame-hit').setAttribute('data-resync','retained')")
    await probe.search(session, "#frame-hit", 1)


async def _template_refresh(probe: _Probe, session: str, owner_refresh: bool) -> None:
    await probe.evaluate(session, "document.body.innerHTML='<div id=template-host><template id=tpl>"
                         "<article id=article><span>inside</span></article></template></div>'")
    await probe.document(session, -1)
    mirror = probe.mirrors[session]
    template = next(node for node in mirror.nodes.values() if node["nodeName"] == "TEMPLATE")
    fragment = template["templateContent"]
    assert_equal(fragment.get("children"), None, "template content stays shallow")
    await probe.call("DOM.requestChildNodes", {"nodeId": fragment["nodeId"], "depth": -1}, session)
    article = next(node for node in mirror.nodes.values() if node["nodeName"] == "ARTICLE")
    retained = set(mirror.nodes)
    body = next(node_id for node_id, node in mirror.nodes.items() if node["nodeName"] == "BODY")
    if owner_refresh:
        await probe.evaluate(session, "document.getElementById('template-host').attachShadow({mode:'closed'})"
                             ".innerHTML='<a id=template-shadow>shadow</a>'")
        await probe.search(session, "#template-shadow", 1)
    else:
        await probe.call("DOM.requestChildNodes", {"nodeId": body, "depth": 1}, session)
    missing = retained - mirror.nodes.keys()
    if missing:
        raise SmokeError(f"template parent request retired published nodes: {sorted(missing)}")
    result = await probe.call("DOM.pushNodesByBackendIdsToFrontend", {
        "backendNodeIds": [article["backendNodeId"]],
    }, session)
    # Chromium returns 0 for backend IDs owned by the inert template Document.
    # A nonzero result must still refer to the retained frontend node.
    if result["nodeIds"][0] not in (0, article["nodeId"]):
        raise SmokeError(f"template descendant changed identity: {result!r}")
    await probe.call("DOM.getAttributes", {"nodeId": article["nodeId"]}, session)


async def _deep_owner_refresh(probe: _Probe, session: str) -> None:
    await probe.evaluate(session, "(() => {document.body.innerHTML='<main id=deep-host style=display:none></main>';"
        "let cursor=document.getElementById('deep-host');"
        "for(let i=0;i<1100;++i){const child=document.createElement('div');"
        "child.id='deep-'+i;cursor.appendChild(child);cursor=child;}})()")
    await probe.document(session, 4)
    mirror = probe.mirrors[session]
    for index in range(0, 1100, 50):
        node = next(n for n in mirror.nodes.values()
                    if n.get("attributes") == ["id", f"deep-{index}"])
        await probe.call("DOM.requestChildNodes", {"nodeId": node["nodeId"], "depth": 50}, session)
    leaf = next(n for n in mirror.nodes.values() if n.get("attributes") == ["id", "deep-1099"])
    retained = set(mirror.nodes)
    await probe.evaluate(session, "document.getElementById('deep-host').attachShadow({mode:'closed'})"
                         ".innerHTML='<b id=deep-shadow>shadow</b>'")
    await probe.search(session, "#deep-shadow", 1)
    missing = retained - mirror.nodes.keys()
    if missing:
        raise SmokeError(f"1100-level owner refresh retired published nodes: {sorted(missing)}")
    result = await probe.call("DOM.pushNodesByBackendIdsToFrontend", {
        "backendNodeIds": [leaf["backendNodeId"]],
    }, session)
    assert_equal(result["nodeIds"], [leaf["nodeId"]], "deep descendant keeps its frontend identity")
    await probe.evaluate(session, "document.getElementById('deep-1099').setAttribute('data-state','retained')")


async def _request_depth(probe: _Probe, session: str) -> None:
    await probe.document(session, 3)
    mirror = probe.mirrors[session]
    body = next(node_id for node_id, node in mirror.nodes.items() if node["nodeName"] == "BODY")
    for depth in (2, 3):
        start = len(probe.messages)
        await probe.call("DOM.requestChildNodes", {"nodeId": body, "depth": depth}, session)
        events = [m["params"] for m in probe.messages[start:] if m.get("method") == "DOM.setChildNodes"]
        if any(event["parentId"] == body for event in events):
            raise SmokeError("deeper request replaced an already expanded parent")
        start = len(probe.messages)
        await probe.call("DOM.requestChildNodes", {"nodeId": body, "depth": depth}, session)
        assert_equal([m for m in probe.messages[start:] if m.get("method") == "DOM.setChildNodes"], [],
                     "repeated child expansion is idempotent")
    for selector in ("#target", "#retained"):
        obj = await probe.evaluate(session, f"document.querySelector({json.dumps(selector)})")
        node = (await probe.call("DOM.describeNode", {"objectId": obj["objectId"]}, session))["node"]
        mirror.require(node["nodeId"], "deeper request published all branches")


async def _frontend_inputs(probe: _Probe, target: str) -> None:
    peer = await probe.attach(target)
    await probe.document(peer, -1)
    obj = await probe.evaluate(peer, "document.getElementById('branch')")
    peer_node = (await probe.call("DOM.describeNode", {"objectId": obj["objectId"]}, peer))["node"]
    session = await probe.attach(target)
    guessed = peer_node["nodeId"]

    async def assert_ignored_child_request(node_id: int) -> None:
        start = len(probe.messages)
        result = await probe.call("DOM.requestChildNodes", {"nodeId": node_id, "depth": 1}, session)
        assert_equal(result, {}, "missing child request succeeds")
        assert_equal([m for m in probe.messages[start:] if m.get("method", "").startswith("DOM.")], [],
                     "missing child request must not publish nodes")

    await assert_ignored_child_request(guessed)
    await probe.document(session)
    if guessed in probe.mirrors[session].nodes:
        raise SmokeError("fixture ID must be unknown to the shallow session")
    commands = [("DOM.requestChildNodes", {"depth": 1}),
                ("DOM.querySelector", {"selector": "#target"}),
                ("DOM.querySelectorAll", {"selector": "#target"}),
                ("DOM.getAttributes", {}), ("DOM.getOuterHTML", {}),
                ("DOM.describeNode", {}), ("DOM.resolveNode", {})]
    for method, params in commands:
        start = len(probe.messages)
        rid = await probe.client.send(method, {"nodeId": guessed, **params}, session_id=session)
        reply, _ = await receive_response(probe.client, rid, on_message=probe.observe)
        if method == "DOM.requestChildNodes":
            assert_equal(reply.get("result"), {}, "unknown child request is a no-op")
        else:
            assert_equal(reply.get("error", {}).get("code"), -32000, "unknown frontend input")
        assert_equal([m for m in probe.messages[start:] if m.get("method", "").startswith("DOM.")], [],
                     "unknown input must not publish nodes")
    obj = await probe.evaluate(session, "document.getElementById('branch')")
    node_id = (await probe.call("DOM.requestNode", {"objectId": obj["objectId"]}, session))["nodeId"]
    for method, params in commands:
        await probe.call(method, {"nodeId": node_id, **params}, session)
    await probe.call("DOM.disable", session=session)
    await assert_ignored_child_request(node_id)


async def _id_exit(probe: _Probe, target: str, kind: str, state: int) -> None:
    session = await probe.attach(target)
    await probe.evaluate(session, "document.getElementById('target').style.cssText="
                         "'position:fixed;left:20px;top:20px;width:40px;height:40px;display:block;z-index:10000'")
    if state:
        await probe.document(session, 1 if state == 1 else -1)
    selector = "#frame" if kind == "frame_owner" else "#target"
    obj = await probe.evaluate(session, f"document.querySelector({json.dumps(selector)})")
    described = (await probe.call("DOM.describeNode", {"objectId": obj["objectId"]}, session))["node"]
    backend = described["backendNodeId"]
    assert_equal(bool(described["nodeId"]), state == 2, "initial frontend publication state")
    if kind == "push":
        params = {"backendNodeIds": [backend, 0, backend]}
        if not state:
            rid = await probe.client.send("DOM.pushNodesByBackendIdsToFrontend", params, session_id=session)
            reply, _ = await receive_response(probe.client, rid, on_message=probe.observe)
            if "Document needs to be requested first" not in reply.get("error", {}).get("message", ""):
                raise SmokeError(f"cold pushNodes must require a requested document: {reply!r}")
            return
        result = await probe.call("DOM.pushNodesByBackendIdsToFrontend", params, session)
        node_id = result["nodeIds"][0]
        assert_equal(result["nodeIds"], [node_id, 0, node_id], "batch positions and duplicate identities")
    elif kind == "frame_owner":
        tree = (await probe.call("Page.getFrameTree", session=session))["frameTree"]
        result = await probe.call("DOM.getFrameOwner", {
            "frameId": tree["childFrames"][0]["frame"]["id"],
        }, session)
        assert_equal(result["backendNodeId"], backend, "frame owner backend identity")
        node_id = result.get("nodeId")
    else:
        result = await probe.call("DOM.getNodeForLocation", {"x": 25, "y": 25}, session)
        assert_equal(result["backendNodeId"], backend, "hit-test backend identity")
        node_id = result.get("nodeId")
    if not state:
        assert_equal(node_id, None, "cold document exposes backend identity only")
        assert_equal(probe.mirrors[session].nodes, {}, "cold lookup does not publish a DOM tree")
        return
    probe.mirrors[session].require(node_id, "node ID outlet")
    if state == 2:
        assert_equal(node_id, described["nodeId"], "previously published identity stays stable")
    await probe.call("DOM.getAttributes", {"nodeId": node_id}, session)
    assert_equal(await probe.search(session, selector, 1), [node_id], "subsequent search reuses publication")


async def _push_inspector_trees(probe: _Probe, target: str) -> None:
    session = await probe.attach(target)
    await probe.evaluate(session, "window.pushShadow=document.getElementById('host').attachShadow({mode:'closed'});"
        "pushShadow.innerHTML='<a id=push-shadow>shadow</a>';"
        "window.pushDetached=document.createElement('a');pushDetached.textContent='detached';"
        "document.body.insertAdjacentHTML('beforeend','<li id=push-marker>marker</li>');"
        "document.getElementById('push-marker').getBoundingClientRect()")
    await probe.document(session)
    backend_ids = []
    for expression in ("document.getElementById('target')", "pushShadow.firstChild",
                       "document.getElementById('frame').contentDocument.getElementById('frame-hit')",
                       "pushDetached"):
        obj = await probe.evaluate(session, expression)
        node = (await probe.call("DOM.describeNode", {"objectId":obj["objectId"]}, session))["node"]
        backend_ids.append(node["backendNodeId"])
    for selector, field, wanted in (("#control", "shadowRoots", "editing-view-port"),
                                    ("#push-marker", "pseudoElements", None)):
        obj = await probe.evaluate(session, f"document.querySelector({json.dumps(selector)})")
        node = (await probe.call("DOM.describeNode", {
            "objectId":obj["objectId"], "depth":-1, "pierce":True,
        }, session))["node"]
        pending = list(node.get(field, []))
        match = None
        while pending:
            candidate = pending.pop()
            attributes = candidate.get("attributes", [])
            if wanted is None or dict(zip(attributes[::2],attributes[1::2])).get("id") == wanted:
                match = candidate
                break
            pending.extend(child for _, child in _descendants(candidate))
        if match is None:
            raise SmokeError(f"missing generated Inspector node for {selector}: {node!r}")
        backend_ids.append(match["backendNodeId"])
    result = await probe.call("DOM.pushNodesByBackendIdsToFrontend", {"backendNodeIds":backend_ids}, session)
    for backend, node_id in zip(backend_ids, result["nodeIds"], strict=True):
        probe.mirrors[session].require(node_id, "mixed Inspector push response")
        node = (await probe.call("DOM.describeNode", {"nodeId":node_id}, session))["node"]
        assert_equal(node["backendNodeId"], backend, "published backend identity")
    # Chromium can assign a fresh frontend ID when pushing a detached root
    # again. Connected Inspector nodes retain their published identities.
    connected = [index for index in range(len(backend_ids)) if index != 3]
    repeated = await probe.call("DOM.pushNodesByBackendIdsToFrontend", {
        "backendNodeIds":[backend_ids[index] for index in connected],
    }, session)
    assert_equal(repeated["nodeIds"], [result["nodeIds"][index] for index in connected],
                 "repeat connected push identity")


async def _sessions(probe: _Probe, session: str, target: str, mutations: bool) -> None:
    other = await probe.attach(target)
    await probe.document(other, -1 if mutations else 1)
    if not mutations:
        await probe.search(session, "#target", 1)
        obj = await probe.evaluate(other, "document.getElementById('target')")
        node = (await probe.call("DOM.describeNode", {"objectId": obj["objectId"]}, other))["node"]
        assert_equal(node["nodeId"], 0, "another session's search does not publish here")
        await probe.search(other, "#target", 1)
        return
    expression = """(() => {const n=document.getElementById('target');
        n.setAttribute('data-state','ready');n.removeAttribute('data-state');
        n.firstChild.data='changed';n.append(document.createElement('b'));n.lastChild.remove();
        n.firstChild.data=' ';n.firstChild.data='restored';})()"""
    for published in (False, True):
        if published:
            obj = await probe.evaluate(session, "document.getElementById('branch')")
            branch = await probe.call("DOM.requestNode", {"objectId": obj["objectId"]}, session)
            await probe.call("DOM.requestChildNodes", {"nodeId": branch["nodeId"], "depth": -1}, session)
        start = len(probe.messages)
        await probe.evaluate(session, expression)
        await probe.evaluate(other, "void 0")
        events = [m for m in probe.messages[start:] if m.get("method", "").startswith("DOM.")]
        for receiver in (session, other):
            methods = {m["method"] for m in events if m.get("sessionId") == receiver}
            if receiver == session and not published:
                assert_equal(methods, set(), "hidden nodes emit no mutation events")
            else:
                expected = {"DOM.attributeModified", "DOM.attributeRemoved", "DOM.characterDataModified",
                            "DOM.childNodeInserted", "DOM.childNodeRemoved"}
                if not expected <= methods:
                    raise SmokeError(f"missing published mutations: {expected - methods!r}")


async def _navigation(probe: _Probe, session: str, fixture: str) -> None:
    await probe.search(session, "#target", 1)
    search = await probe.call("DOM.performSearch", {"query": "#target"}, session)
    await navigate(probe.client, session, {"url": f"{fixture}/plain?dom-publication-replacement"},
                   on_message=probe.observe)
    assert_equal(probe.mirrors[session].nodes, {}, "documentUpdated retires the client mirror")
    rid = await probe.client.send("DOM.getSearchResults", {
        "searchId": search["searchId"], "fromIndex": 0, "toIndex": 1,
    }, session_id=session)
    response, _ = await receive_response(probe.client, rid, on_message=probe.observe)
    if "error" not in response:
        raise SmokeError(f"retired search survived navigation: {response!r}")
    await _install(probe, session)
    await probe.document(session)
    await probe.search(session, "#target", 1)


async def run_dom_publication_group(endpoint: str, fixture: str, results: list[dict[str, Any]]) -> None:
    # This isolated worker receives 1000-level snapshots; its JSON decoder,
    # DOM mirror, and persisted diagnostics must all be able to inspect them.
    sys.setrecursionlimit(max(sys.getrecursionlimit(), 8000))
    scenarios = [
        (f"describe_{reference}_{node}", lambda p, s, t, a=ancestor, b=backend: _describe_search(p, s, a, b))
        for node, ancestor in (("target", False), ("ancestor", True))
        for reference, backend in (("object", False), ("backend", True))
    ]
    scenarios += [(f"request_{node}", lambda p, s, t, a=ancestor: _request_search(p, s, a))
                  for node, ancestor in (("target", False), ("ancestor", True))]
    scenarios += [
        ("range_and_repeat", lambda p, s, t: _ranges(p, s)),
        ("mixed_light_shadow_frame", lambda p, s, t: _mixed(p, s)),
        ("generated_ua", lambda p, s, t: _ua(p, s)),
        ("retained_detached", lambda p, s, t: _detached(p, s)),
        ("session_isolation", lambda p, s, t: _sessions(p, s, t, False)),
        ("mutation_visibility", lambda p, s, t: _sessions(p, s, t, True)),
        ("navigation_reset", lambda p, s, t: _navigation(p, s, fixture)),
    ]
    scenarios += [
        (f"new_{kind}_{depth}", lambda p, s, t, f=frame, d=depth: _associated(p, s, f, d))
        for kind, frame in (("shadow", False), ("frame_document", True)) for depth in (1, -1)
    ]
    scenarios += [
        (f"parent_refresh_whitespace_{whitespace}", lambda p, s, t, w=whitespace: _parent_refresh(p, s, w))
        for whitespace in (False, True)
    ]
    scenarios += [
        (f"{kind}_publication_{state}", lambda p, s, t, k=kind, d=state: _id_exit(p, t, k, d))
        for kind in ("push", "frame_owner", "hit_test") for state in range(3)
    ]
    scenarios.append(("push_inspector_trees", lambda p, s, t: _push_inspector_trees(p, t)))
    scenarios.append(("frontend_input_visibility", lambda p, s, t: _frontend_inputs(p, t)))
    scenarios += [
        ("template_parent_refresh", lambda p, s, t: _template_refresh(p, s, False)),
        ("template_owner_refresh", lambda p, s, t: _template_refresh(p, s, True)),
        ("request_children_depth_and_repeat", lambda p, s, t: _request_depth(p, s)),
        ("deep_owner_refresh", lambda p, s, t: _deep_owner_refresh(p, s)),
    ]
    failures = []
    for name, scenario in scenarios:
        client = await connect_raw_cdp(endpoint)
        probe = _Probe(client)
        target = None
        try:
            target = (await probe.call("Target.createTarget", {"url": "about:blank"}))["targetId"]
            session = await probe.attach(target)
            await probe.call("Page.enable", session=session)
            await navigate(client, session, {"url": f"{fixture}/plain?dom-publication"},
                           on_message=probe.observe)
            await _install(probe, session)
            await probe.document(session)
            await scenario(probe, session, target)
            record(results, f"dom_publication_{name}", {"messages": len(probe.messages)})
        except Exception as error:
            failures.append(f"{name}: {error}")
            results.append({"name": f"dom_publication_{name}", "ok": False,
                            "error": str(error), "wire": probe.messages})
        finally:
            if target:
                with contextlib.suppress(Exception):
                    async with asyncio.timeout(3):
                        await probe.call("Target.closeTarget", {"targetId": target})
            await client.websocket.close()
    if failures:
        raise SmokeError("DOM publication contracts failed: " + "; ".join(failures))
