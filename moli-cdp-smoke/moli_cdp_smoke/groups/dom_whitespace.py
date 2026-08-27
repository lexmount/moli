from __future__ import annotations

import asyncio
from typing import Any

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until


def _attributes(node: dict[str, Any]) -> dict[str, str]:
    values = node.get("attributes") or []
    return {
        str(values[index]): str(values[index + 1])
        for index in range(0, len(values) - 1, 2)
    }


def _find_node(node: dict[str, Any], predicate: Any) -> dict[str, Any] | None:
    if predicate(node):
        return node
    descendants = list(node.get("children") or [])
    descendants.extend(node.get("shadowRoots") or [])
    descendants.extend(node.get("pseudoElements") or [])
    content_document = node.get("contentDocument")
    if isinstance(content_document, dict):
        descendants.append(content_document)
    for child in descendants:
        found = _find_node(child, predicate)
        if found is not None:
            return found
    return None


def _node_by_id(document: dict[str, Any], element_id: str) -> dict[str, Any]:
    node = _find_node(
        document.get("root") or document,
        lambda candidate: _attributes(candidate).get("id") == element_id,
    )
    if node is None:
        raise SmokeError(f"DOM snapshot is missing #{element_id}: {document}")
    return node


def _required_positive(value: Any, label: str) -> int:
    if not isinstance(value, int) or value <= 0:
        raise SmokeError(f"{label} must be a positive integer: {value!r}")
    return value


def _whitespace_text_node_count(node: dict[str, Any]) -> int:
    current = int(
        node.get("nodeType") == 3
        and isinstance(node.get("nodeValue"), str)
        and not node["nodeValue"].strip()
    )
    descendants = list(node.get("children") or [])
    descendants.extend(node.get("shadowRoots") or [])
    content_document = node.get("contentDocument")
    if isinstance(content_document, dict):
        descendants.append(content_document)
    return current + sum(_whitespace_text_node_count(child) for child in descendants)


async def _search(
    session: Any,
    query: str,
    *,
    include_user_agent_shadow_dom: bool = False,
) -> tuple[int, list[int]]:
    performed = await session.send(
        "DOM.performSearch",
        {
            "query": query,
            "includeUserAgentShadowDOM": include_user_agent_shadow_dom,
        },
    )
    result_count = performed.get("resultCount")
    search_id = performed.get("searchId")
    if not isinstance(result_count, int) or not isinstance(search_id, str):
        raise SmokeError(f"DOM.performSearch returned an invalid result: {performed}")
    node_ids: list[int] = []
    if result_count:
        results = await session.send(
            "DOM.getSearchResults",
            {
                "searchId": search_id,
                "fromIndex": 0,
                "toIndex": result_count,
            },
        )
        raw_node_ids = results.get("nodeIds")
        if not isinstance(raw_node_ids, list) or not all(
            isinstance(node_id, int) for node_id in raw_node_ids
        ):
            raise SmokeError(f"DOM.getSearchResults returned invalid nodeIds: {results}")
        node_ids = raw_node_ids
    await session.send("DOM.discardSearchResults", {"searchId": search_id})
    return result_count, node_ids


async def _node_stack(session: Any, node_id: int) -> dict[str, Any]:
    return await session.send("DOM.getNodeStackTraces", {"nodeId": node_id})


def _blog_node(document: dict[str, Any]) -> dict[str, Any]:
    root = document.get("root") or {}
    blog = _find_node(
        root,
        lambda node: "blog" in _attributes(node).get("class", "").split(),
    )
    if blog is None:
        raise SmokeError(f"ldm0.top fixture DOM.getDocument missing .blog: {document}")
    return blog


def _blog_projection_counts(blog: dict[str, Any]) -> dict[str, int]:
    children = blog.get("children") or []
    chunks = [
        child
        for child in children
        if "blog_chunk" in _attributes(child).get("class", "").split()
    ]
    comments = [child for child in children if child.get("nodeType") == 8]
    whitespace = [
        child
        for child in children
        if child.get("nodeType") == 3
        and isinstance(child.get("nodeValue"), str)
        and not child["nodeValue"].strip()
    ]
    return {
        "children": len(children),
        "chunks": len(chunks),
        "comments": len(comments),
        "whitespace": len(whitespace),
    }


async def run_dom_whitespace_group(state: SmokeState) -> None:
    page = await state.context.new_page()
    default_session = None
    all_session = None
    config_session = None
    ua_session = None
    try:
        await page.goto(
            f"{state.fixture}/ldm0-top-dom-whitespace",
            wait_until="domcontentloaded",
            timeout=10_000,
        )
        default_session = await state.context.new_cdp_session(page)
        all_session = await state.context.new_cdp_session(page)
        config_session = await state.context.new_cdp_session(page)
        ua_session = await state.context.new_cdp_session(page)

        await default_session.send("DOM.enable")
        await all_session.send("DOM.enable", {"includeWhitespace": "all"})
        await ua_session.send("DOM.enable")
        await config_session.send("DOM.enable", {"includeWhitespace": "none"})
        await config_session.send("DOM.enable", {"includeWhitespace": "all"})
        locked_document = await config_session.send("DOM.getDocument", {"depth": -1})
        locked_counts = _blog_projection_counts(_blog_node(locked_document))
        assert_equal(
            locked_counts["whitespace"],
            0,
            "repeated DOM.enable does not change the first whitespace mode",
        )
        await config_session.send("DOM.disable")
        await config_session.send("DOM.enable", {"includeWhitespace": "all"})
        reset_document = await config_session.send("DOM.getDocument", {"depth": -1})
        reset_counts = _blog_projection_counts(_blog_node(reset_document))
        assert_equal(
            reset_counts["whitespace"],
            11,
            "DOM.disable clears the first-enable whitespace mode",
        )

        default_document = await default_session.send("DOM.getDocument", {"depth": -1})
        default_blog = _blog_node(default_document)
        default_counts = _blog_projection_counts(default_blog)
        assert_equal(default_counts["children"], 10, "ldm0.top default blog child count")
        assert_equal(default_counts["chunks"], 8, "ldm0.top default blog chunk count")
        assert_equal(default_counts["comments"], 2, "ldm0.top default blog comment count")
        assert_equal(
            default_counts["whitespace"],
            0,
            "default DOM projection omits ldm0.top indentation text nodes",
        )
        assert_equal(
            default_blog.get("childNodeCount"),
            default_counts["children"],
            "default DOM projection childNodeCount",
        )

        all_document = await all_session.send("DOM.getDocument", {"depth": -1})
        all_blog = _blog_node(all_document)
        all_counts = _blog_projection_counts(all_blog)
        assert_equal(all_counts["children"], 21, "ldm0.top all-mode blog child count")
        assert_equal(all_counts["chunks"], 8, "ldm0.top all-mode blog chunk count")
        assert_equal(all_counts["comments"], 2, "ldm0.top all-mode blog comment count")
        assert_equal(all_counts["whitespace"], 11, "ldm0.top all-mode whitespace count")
        if all_counts["children"] <= default_counts["children"]:
            raise SmokeError(
                "includeWhitespace=all did not expand the ldm0.top .blog projection: "
                f"default={default_counts}, all={all_counts}"
            )
        assert_equal(
            all_blog.get("childNodeCount"),
            all_counts["children"],
            "all-mode DOM projection childNodeCount",
        )

        whitespace_nodes = [
            child
            for child in all_blog.get("children") or []
            if child.get("nodeType") == 3
            and isinstance(child.get("nodeValue"), str)
            and not child["nodeValue"].strip()
        ]
        whitespace_node = whitespace_nodes[0]
        backend_node_id = whitespace_node.get("backendNodeId")
        all_frontend_node_id = whitespace_node.get("nodeId")
        if not isinstance(backend_node_id, int) or backend_node_id <= 0:
            raise SmokeError(f"all-mode whitespace node has no backend identity: {whitespace_node}")
        if not isinstance(all_frontend_node_id, int) or all_frontend_node_id <= 0:
            raise SmokeError(f"all-mode whitespace node has no frontend identity: {whitespace_node}")

        default_push = await default_session.send(
            "DOM.pushNodesByBackendIdsToFrontend",
            {"backendNodeIds": [backend_node_id]},
        )
        all_push = await all_session.send(
            "DOM.pushNodesByBackendIdsToFrontend",
            {"backendNodeIds": [backend_node_id]},
        )
        assert_equal(
            default_push.get("nodeIds"),
            [0],
            "default pushNodes keeps hidden whitespace unbound",
        )
        assert_equal(
            all_push.get("nodeIds"),
            [all_frontend_node_id],
            "all-mode pushNodes keeps whitespace binding",
        )

        evaluated = await default_session.send(
            "Runtime.evaluate",
            {
                "expression": "Array.from(document.querySelector('.blog').childNodes).find(node => node.nodeType === Node.TEXT_NODE && !node.data.trim())"
            },
        )
        object_id = (evaluated.get("result") or {}).get("objectId")
        if not isinstance(object_id, str) or not object_id:
            raise SmokeError(f"default session did not resolve whitespace object: {evaluated}")
        requested = await default_session.send("DOM.requestNode", {"objectId": object_id})
        described = await default_session.send(
            "DOM.describeNode", {"objectId": object_id, "depth": 0}
        )
        assert_equal(requested.get("nodeId"), 0, "default requestNode whitespace nodeId")
        described_node = described.get("node") or {}
        assert_equal(described_node.get("nodeId"), 0, "default describeNode whitespace nodeId")
        assert_equal(
            described_node.get("backendNodeId"),
            backend_node_id,
            "default describeNode preserves whitespace backend identity",
        )

        search_query = "//*[@class='blog']/text()"
        search_projection: dict[str, list[int]] = {}
        for label, search_session in (
            ("default", default_session),
            ("all", all_session),
        ):
            result_count, node_ids = await _search(search_session, search_query)
            assert_equal(
                result_count,
                all_counts["whitespace"],
                f"{label} whitespace search result count",
            )
            search_projection[label] = node_ids
        if any(node_id != 0 for node_id in search_projection["default"]):
            raise SmokeError(
                "default whitespace search published hidden frontend ids: "
                f"{search_projection['default']}"
            )
        if any(not isinstance(node_id, int) or node_id <= 0 for node_id in search_projection["all"]):
            raise SmokeError(
                "all-mode whitespace search omitted frontend ids: "
                f"{search_projection['all']}"
            )

        ua_search_counts: dict[str, int] = {}
        ua_search_node_ids: dict[str, list[int]] = {}
        for label, query, include_ua, expected_count in (
            ("css-default", "#editing-view-port", False, 0),
            ("css-enabled", "#editing-view-port", True, 1),
            ("text-default", "needle", False, 1),
            ("text-enabled", "needle", True, 2),
            ("xpath-enabled", "//*[@id='editing-view-port']", True, 0),
        ):
            result_count, node_ids = await _search(
                ua_session,
                query,
                include_user_agent_shadow_dom=include_ua,
            )
            assert_equal(result_count, expected_count, f"UA shadow search {label}")
            ua_search_counts[label] = result_count
            ua_search_node_ids[label] = node_ids
        assert_equal(
            ua_search_node_ids["css-enabled"],
            [0],
            "unpublished generated UA search result uses Chromium nodeId 0",
        )
        assert_equal(
            ua_search_node_ids["text-enabled"],
            [0, 0],
            "search results remain unbound before DOM.getDocument publishes the session tree",
        )

        default_pierced = await default_session.send(
            "DOM.getDocument", {"depth": -1, "pierce": True}
        )
        all_pierced = await all_session.send(
            "DOM.getDocument", {"depth": -1, "pierce": True}
        )
        await ua_session.send("DOM.getDocument", {"depth": -1, "pierce": True})
        default_frame = _node_by_id(default_pierced, "whitespace-frame")
        all_frame = _node_by_id(all_pierced, "whitespace-frame")
        default_child_document = default_frame.get("contentDocument")
        all_child_document = all_frame.get("contentDocument")
        if not isinstance(default_child_document, dict) or not isinstance(
            all_child_document, dict
        ):
            raise SmokeError(
                "pierced DOM snapshots did not expose the child document: "
                f"default={default_frame}, all={all_frame}"
            )
        assert_equal(
            _whitespace_text_node_count(default_child_document),
            0,
            "default child-frame projection omits indentation text",
        )
        if _whitespace_text_node_count(all_child_document) <= 0:
            raise SmokeError(
                "includeWhitespace=all did not reach the child-frame document: "
                f"{all_child_document}"
            )
        child_document_backend_id = _required_positive(
            default_child_document.get("backendNodeId"),
            "default child document backendNodeId",
        )
        assert_equal(
            all_child_document.get("backendNodeId"),
            child_document_backend_id,
            "child document backend identity is session independent",
        )
        default_child_description = (
            await default_session.send(
                "DOM.describeNode",
                {
                    "backendNodeId": child_document_backend_id,
                    "depth": -1,
                    "pierce": True,
                },
            )
        )["node"]
        all_child_description = (
            await all_session.send(
                "DOM.describeNode",
                {
                    "backendNodeId": child_document_backend_id,
                    "depth": -1,
                    "pierce": True,
                },
            )
        )["node"]
        assert_equal(
            _whitespace_text_node_count(default_child_description),
            0,
            "default child-frame describeNode projection",
        )
        if _whitespace_text_node_count(all_child_description) <= 0:
            raise SmokeError(
                "includeWhitespace=all was lost by child-frame describeNode: "
                f"{all_child_description}"
            )

        pierced_ua_count, pierced_ua_node_ids = await _search(
            ua_session,
            "#editing-view-port",
            include_user_agent_shadow_dom=True,
        )
        assert_equal(pierced_ua_count, 1, "pierced UA shadow search count")
        pierced_ua_node_id = _required_positive(
            pierced_ua_node_ids[0], "pierced UA shadow search nodeId"
        )
        pierced_ua_description = (
            await ua_session.send(
                "DOM.describeNode", {"nodeId": pierced_ua_node_id, "depth": 1}
            )
        )["node"]
        assert_equal(
            _attributes(pierced_ua_description).get("id"),
            "editing-view-port",
            "pierced UA search result identity",
        )
        repeated_ua_count, repeated_ua_node_ids = await _search(
            ua_session,
            "#editing-view-port",
            include_user_agent_shadow_dom=True,
        )
        assert_equal(repeated_ua_count, 1, "repeated UA shadow search count")
        assert_equal(
            repeated_ua_node_ids,
            [pierced_ua_node_id],
            "repeated UA search reuses published Inspector identity",
        )

        default_mutation_host = _node_by_id(default_pierced, "whitespace-mutation")
        all_mutation_host = _node_by_id(all_pierced, "whitespace-mutation")
        default_mutation_host_id = _required_positive(
            default_mutation_host.get("nodeId"), "default mutation host nodeId"
        )
        all_mutation_children = all_mutation_host.get("children") or []
        if len(all_mutation_children) != 1:
            raise SmokeError(
                "all-mode mutation host must expose one whitespace child: "
                f"{all_mutation_host}"
            )
        all_mutation_text_id = _required_positive(
            all_mutation_children[0].get("nodeId"), "all-mode mutation text nodeId"
        )
        default_mutation_events: list[dict[str, Any]] = []
        all_mutation_events: list[dict[str, Any]] = []
        for method in (
            "DOM.childNodeInserted",
            "DOM.childNodeRemoved",
            "DOM.characterDataModified",
        ):
            default_session.on(
                method,
                lambda params, method=method: default_mutation_events.append(
                    {"method": method, "params": params}
                ),
            )
            all_session.on(
                method,
                lambda params, method=method: all_mutation_events.append(
                    {"method": method, "params": params}
                ),
            )

        await default_session.send(
            "Runtime.evaluate",
            {
                "expression": "document.getElementById('whitespace-mutation').firstChild.data='visible'"
            },
        )
        await wait_until(
            lambda: any(
                event["method"] == "DOM.childNodeInserted"
                and event["params"].get("parentNodeId") == default_mutation_host_id
                for event in default_mutation_events
            )
            and any(
                event["method"] == "DOM.characterDataModified"
                and event["params"].get("nodeId") == all_mutation_text_id
                for event in all_mutation_events
            ),
            "whitespace-to-visible DOM projection events",
        )
        default_visible_events = [
            event
            for event in default_mutation_events
            if event["params"].get("parentNodeId") == default_mutation_host_id
        ]
        all_visible_events = [
            event
            for event in all_mutation_events
            if event["params"].get("nodeId") == all_mutation_text_id
        ]
        assert_equal(
            [event["method"] for event in default_visible_events],
            ["DOM.childNodeInserted"],
            "default whitespace-to-visible event",
        )
        assert_equal(
            [event["method"] for event in all_visible_events],
            ["DOM.characterDataModified"],
            "all-mode whitespace-to-visible event",
        )
        inserted_node_id = _required_positive(
            (default_visible_events[0]["params"].get("node") or {}).get("nodeId"),
            "default inserted text nodeId",
        )
        assert_equal(
            all_visible_events[0]["params"].get("characterData"),
            "visible",
            "all-mode visible character data",
        )

        default_mutation_events.clear()
        all_mutation_events.clear()
        await default_session.send(
            "Runtime.evaluate",
            {
                "expression": "document.getElementById('whitespace-mutation').firstChild.data='  '"
            },
        )
        await wait_until(
            lambda: any(
                event["method"] == "DOM.childNodeRemoved"
                and event["params"].get("nodeId") == inserted_node_id
                for event in default_mutation_events
            )
            and any(
                event["method"] == "DOM.characterDataModified"
                and event["params"].get("nodeId") == all_mutation_text_id
                for event in all_mutation_events
            ),
            "visible-to-whitespace DOM projection events",
        )
        default_hidden_events = [
            event
            for event in default_mutation_events
            if event["params"].get("nodeId") == inserted_node_id
        ]
        all_hidden_events = [
            event
            for event in all_mutation_events
            if event["params"].get("nodeId") == all_mutation_text_id
        ]
        assert_equal(
            [event["method"] for event in default_hidden_events],
            ["DOM.childNodeRemoved"],
            "default visible-to-whitespace event",
        )
        assert_equal(
            [event["method"] for event in all_hidden_events],
            ["DOM.characterDataModified"],
            "all-mode visible-to-whitespace event",
        )
        assert_equal(
            all_hidden_events[0]["params"].get("characterData"),
            "  ",
            "all-mode hidden character data",
        )

        tail_node_id = _required_positive(
            _node_by_id(default_pierced, "tail").get("nodeId"),
            "pre-capture tail nodeId",
        )
        assert_equal(
            await _node_stack(default_session, tail_node_id),
            {},
            "nodes created before stack capture have no creation trace",
        )

        await default_session.send(
            "DOM.setNodeStackTracesEnabled", {"enable": True}
        )
        await default_session.send(
            "Runtime.evaluate",
            {
                "expression": "function stackOuter(){function stackInner(){const node=document.createElement('section');node.id='stack-smoke-node';document.body.append(node)}stackInner()}stackOuter()\n//# sourceURL=dom-stack-smoke.js"
            },
        )
        stack_count, stack_node_ids = await _search(default_session, "#stack-smoke-node")
        assert_equal(stack_count, 1, "creation-stack node search")
        stack_node_id = _required_positive(stack_node_ids[0], "creation-stack nodeId")
        stack_trace = await _node_stack(default_session, stack_node_id)
        stack_frames = (stack_trace.get("creation") or {}).get("callFrames") or []
        if len(stack_frames) < 3:
            raise SmokeError(f"DOM creation stack is incomplete: {stack_trace}")
        assert_equal(
            [stack_frames[0].get("functionName"), stack_frames[1].get("functionName")],
            ["stackInner", "stackOuter"],
            "DOM creation stack function order",
        )
        assert_equal(
            stack_frames[0].get("url"),
            "dom-stack-smoke.js",
            "DOM creation stack source URL",
        )
        if not stack_frames[0].get("scriptId"):
            raise SmokeError(f"DOM creation stack has no scriptId: {stack_trace}")
        assert_equal(stack_frames[0].get("lineNumber"), 0, "DOM stack zero-based line")

        peer_stack_count, peer_stack_node_ids = await _search(
            all_session, "#stack-smoke-node"
        )
        assert_equal(peer_stack_count, 1, "peer creation-stack node search")
        peer_stack_node_id = _required_positive(
            peer_stack_node_ids[0], "peer creation-stack nodeId"
        )
        peer_stack_trace = await _node_stack(all_session, peer_stack_node_id)
        assert_equal(
            peer_stack_trace,
            {},
            "node stack capture switch is session local",
        )

        await default_session.send(
            "DOM.setNodeStackTracesEnabled", {"enable": False}
        )
        await default_session.send(
            "Runtime.evaluate",
            {
                "expression": "const node=document.createElement('aside');node.id='stack-after-disable';document.body.append(node)\n//# sourceURL=dom-stack-smoke.js"
            },
        )
        after_count, after_node_ids = await _search(
            default_session, "#stack-after-disable"
        )
        assert_equal(after_count, 1, "post-disable stack node search")
        after_node_id = _required_positive(after_node_ids[0], "post-disable stack nodeId")
        after_disable_trace = await _node_stack(default_session, after_node_id)
        assert_equal(
            after_disable_trace,
            {},
            "nodes created after disabling stack capture have no trace",
        )
        assert_equal(
            await _node_stack(default_session, stack_node_id),
            stack_trace,
            "disabling capture preserves previously captured traces",
        )

        await default_session.send(
            "DOM.setNodeStackTracesEnabled", {"enable": True}
        )
        await default_session.send(
            "Runtime.evaluate",
            {
                "expression": "function stackFragment(){const container=document.createElement('div');container.innerHTML='<span id=stack-fragment-node>parsed</span>';document.body.append(container)}stackFragment()\n//# sourceURL=dom-stack-fragment-smoke.js"
            },
        )
        fragment_count, fragment_node_ids = await _search(
            default_session, "#stack-fragment-node"
        )
        assert_equal(fragment_count, 1, "fragment-created stack node search")
        fragment_node_id = _required_positive(
            fragment_node_ids[0], "fragment-created stack nodeId"
        )
        fragment_trace = await _node_stack(default_session, fragment_node_id)
        fragment_frames = (fragment_trace.get("creation") or {}).get("callFrames") or []
        if not fragment_frames:
            raise SmokeError(f"innerHTML-created node has no creation stack: {fragment_trace}")
        assert_equal(
            fragment_frames[0].get("functionName"),
            "stackFragment",
            "fragment creation stack function",
        )
        assert_equal(
            fragment_frames[0].get("url"),
            "dom-stack-fragment-smoke.js",
            "fragment creation stack URL",
        )

        document_updated = asyncio.Event()

        def on_document_updated(_: dict[str, Any]) -> None:
            document_updated.set()

        # Runtime.evaluate may resolve before the replacement Document reaches
        # the Inspector DOM binding. Listen first so an early event is retained,
        # then refresh frontend node ids only after the invalidation barrier.
        default_session.on("DOM.documentUpdated", on_document_updated)
        try:
            await default_session.send(
                "Runtime.evaluate",
                {
                    "expression": "document.open();document.write('<!doctype html><html><body></body></html>');document.close();function stackAfterOpen(){const node=document.createElement('article');node.id='stack-after-open';document.body.append(node)}stackAfterOpen()\n//# sourceURL=dom-stack-open-smoke.js"
                },
            )
            try:
                await asyncio.wait_for(document_updated.wait(), timeout=10)
            except TimeoutError as error:
                raise SmokeError(
                    "document.open did not publish DOM.documentUpdated"
                ) from error
        finally:
            default_session.remove_listener(
                "DOM.documentUpdated", on_document_updated
            )
        await default_session.send("DOM.getDocument", {"depth": 1})
        open_count, open_node_ids = await _search(
            default_session, "#stack-after-open"
        )
        assert_equal(open_count, 1, "document.open creation-stack node search")
        open_node_id = _required_positive(
            open_node_ids[0], "document.open creation-stack nodeId"
        )
        open_trace = await _node_stack(default_session, open_node_id)
        open_frames = (open_trace.get("creation") or {}).get("callFrames") or []
        if not open_frames:
            raise SmokeError(
                "document.open replacement node has no creation stack: "
                f"{open_trace}"
            )
        assert_equal(
            open_frames[0].get("functionName"),
            "stackAfterOpen",
            "document.open replacement creation stack function",
        )
        assert_equal(
            open_frames[0].get("url"),
            "dom-stack-open-smoke.js",
            "document.open replacement creation stack URL",
        )
        await default_session.send(
            "DOM.setNodeStackTracesEnabled", {"enable": False}
        )

        state.record(
            "ldm0_top_dom_whitespace_projection",
            {
                "default": default_counts,
                "all": all_counts,
                "configurationLifecycle": {
                    "repeatedEnableWhitespace": locked_counts["whitespace"],
                    "afterDisableWhitespace": reset_counts["whitespace"],
                },
                "sessionIsolation": {
                    "backendNodeId": backend_node_id,
                    "defaultPushNodeId": default_push["nodeIds"][0],
                    "allPushNodeId": all_push["nodeIds"][0],
                    "defaultRequestNodeId": requested["nodeId"],
                    "defaultSearchNodeIds": search_projection["default"],
                    "allSearchNodeIds": search_projection["all"],
                },
                "childFrame": {
                    "backendNodeId": child_document_backend_id,
                    "defaultWhitespace": _whitespace_text_node_count(
                        default_child_description
                    ),
                    "allWhitespace": _whitespace_text_node_count(all_child_description),
                },
                "whitespaceMutation": {
                    "defaultTransitions": [
                        default_visible_events[0]["method"],
                        default_hidden_events[0]["method"],
                    ],
                    "allTransitions": [
                        all_visible_events[0]["method"],
                        all_hidden_events[0]["method"],
                    ],
                },
                "userAgentShadowSearch": {
                    "counts": ua_search_counts,
                    "beforePierceNodeIds": ua_search_node_ids,
                    "afterPierceNodeId": pierced_ua_node_id,
                    "reusedNodeId": repeated_ua_node_ids[0],
                },
                "nodeCreationStack": {
                    "nodeId": stack_node_id,
                    "frameFunctions": [
                        stack_frames[0]["functionName"],
                        stack_frames[1]["functionName"],
                    ],
                    "url": stack_frames[0]["url"],
                    "peerHasCreation": bool(peer_stack_trace.get("creation")),
                    "afterDisableHasCreation": bool(
                        after_disable_trace.get("creation")
                    ),
                    "fragmentFunction": fragment_frames[0]["functionName"],
                    "documentOpenFunction": open_frames[0]["functionName"],
                },
            },
        )
    finally:
        if ua_session is not None:
            await ua_session.detach()
        if config_session is not None:
            await config_session.detach()
        if all_session is not None:
            await all_session.detach()
        if default_session is not None:
            await default_session.detach()
        await page.close()
