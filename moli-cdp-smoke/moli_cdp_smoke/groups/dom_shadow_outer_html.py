from __future__ import annotations

from typing import Any

from . import SmokeState
from ..assertions import SmokeError, assert_equal


def _attributes(node: dict[str, Any]) -> dict[str, str]:
    values = node.get("attributes") or []
    return {
        str(values[index]): str(values[index + 1])
        for index in range(0, len(values) - 1, 2)
    }


def _find_node_by_id(node: dict[str, Any], element_id: str) -> dict[str, Any] | None:
    if _attributes(node).get("id") == element_id:
        return node
    descendants = list(node.get("children") or [])
    descendants.extend(node.get("shadowRoots") or [])
    content_document = node.get("contentDocument")
    if isinstance(content_document, dict):
        descendants.append(content_document)
    for child in descendants:
        found = _find_node_by_id(child, element_id)
        if found is not None:
            return found
    return None


def _required_identity(node: dict[str, Any], field: str) -> int:
    value = node.get(field)
    if not isinstance(value, int) or value <= 0:
        raise SmokeError(f"DOM node has no {field}: {node}")
    return value


async def _outer_html(session: Any, params: dict[str, Any]) -> str:
    result = await session.send("DOM.getOuterHTML", params)
    outer_html = result.get("outerHTML")
    if not isinstance(outer_html, str):
        raise SmokeError(f"DOM.getOuterHTML returned no markup: {result}")
    return outer_html


async def run_dom_shadow_outer_html_group(state: SmokeState) -> None:
    page = await state.context.new_page()
    session = None
    try:
        await page.goto(
            f"{state.fixture}/dom-shadow-outer-html",
            wait_until="load",
            timeout=10_000,
        )
        session = await state.context.new_cdp_session(page)
        mutation_events: list[str] = []
        for method in (
            "DOM.childNodeInserted",
            "DOM.childNodeRemoved",
            "DOM.characterDataModified",
            "DOM.attributeModified",
            "DOM.attributeRemoved",
        ):
            session.on(method, lambda _params, method=method: mutation_events.append(method))

        await session.send("DOM.enable")
        document = await session.send(
            "DOM.getDocument", {"depth": -1, "pierce": True}
        )
        root = document.get("root") or {}
        host = _find_node_by_id(root, "host")
        declarative = _find_node_by_id(root, "declarative")
        control = _find_node_by_id(root, "control")
        child_frame = _find_node_by_id(root, "shadow-child")
        child_host = _find_node_by_id(root, "child-host")
        if (
            host is None
            or declarative is None
            or control is None
            or child_frame is None
            or child_host is None
        ):
            raise SmokeError(f"shadow outerHTML fixture nodes are missing: {document}")

        host_node_id = _required_identity(host, "nodeId")
        host_backend_node_id = _required_identity(host, "backendNodeId")
        resolved = await session.send("DOM.resolveNode", {"nodeId": host_node_id})
        host_object_id = (resolved.get("object") or {}).get("objectId")
        if not isinstance(host_object_id, str) or not host_object_id:
            raise SmokeError(f"shadow host did not resolve to an objectId: {resolved}")

        ordinary = '<x-host id="host">light</x-host>'
        including_shadow = (
            '<x-host id="host"><template shadowrootmode="closed" '
            'shadowrootdelegatesfocus="" shadowrootserializable="" '
            'shadowrootclonable=""><span data-x="&amp;">shadow &lt;</span>'
            '<x-inner><template shadowrootmode="open"><b>nested</b></template>'
            'inner-light</x-inner></template>light</x-host>'
        )
        omitted = await _outer_html(session, {"nodeId": host_node_id})
        explicit_false = await _outer_html(
            session, {"nodeId": host_node_id, "includeShadowDOM": False}
        )
        by_node = await _outer_html(
            session, {"nodeId": host_node_id, "includeShadowDOM": True}
        )
        by_backend = await _outer_html(
            session,
            {
                "backendNodeId": host_backend_node_id,
                "includeShadowDOM": True,
            },
        )
        by_object = await _outer_html(
            session, {"objectId": host_object_id, "includeShadowDOM": True}
        )
        false_after_true = await _outer_html(
            session, {"objectId": host_object_id, "includeShadowDOM": False}
        )
        assert_equal(omitted, ordinary, "omitted includeShadowDOM host markup")
        assert_equal(explicit_false, ordinary, "false includeShadowDOM host markup")
        assert_equal(false_after_true, ordinary, "includeShadowDOM does not leak command state")
        assert_equal(by_node, including_shadow, "nodeId shadow-inclusive markup")
        assert_equal(by_backend, including_shadow, "backendNodeId shadow-inclusive markup")
        assert_equal(by_object, including_shadow, "objectId shadow-inclusive markup")
        if "shadowrootslotassignment" in by_node:
            raise SmokeError(f"Inspector markup exposed Web API-only shadow attributes: {by_node}")

        declarative_markup = await _outer_html(
            session,
            {
                "nodeId": _required_identity(declarative, "nodeId"),
                "includeShadowDOM": True,
            },
        )
        assert_equal(
            declarative_markup,
            '<x-declarative id="declarative"><template shadowrootmode="open">'
            '<i>declarative</i></template>declarative-light</x-declarative>',
            "declarative author shadow markup",
        )

        control_markup = await _outer_html(
            session,
            {
                "nodeId": _required_identity(control, "nodeId"),
                "includeShadowDOM": True,
            },
        )
        assert_equal(control_markup, '<input id="control">', "UA shadow roots stay excluded")

        detached = await session.send(
            "Runtime.evaluate", {"expression": "globalThis.__outerHtmlDetached"}
        )
        detached_object_id = (detached.get("result") or {}).get("objectId")
        if not isinstance(detached_object_id, str) or not detached_object_id:
            raise SmokeError(f"detached shadow host has no objectId: {detached}")
        detached_markup = await _outer_html(
            session,
            {"objectId": detached_object_id, "includeShadowDOM": True},
        )
        assert_equal(
            detached_markup,
            '<x-detached><template shadowrootmode="open"><em>detached-shadow</em>'
            '</template>detached-light</x-detached>',
            "detached objectId shadow-inclusive markup",
        )

        child_document = child_frame.get("contentDocument")
        if not isinstance(child_document, dict):
            raise SmokeError(
                f"pierced shadow snapshot has no child document: {child_frame}"
            )
        child_ordinary = '<x-child id="child-host">child-light</x-child>'
        child_including_shadow = (
            '<x-child id="child-host"><template shadowrootmode="closed">'
            '<span>child-shadow</span></template>child-light</x-child>'
        )
        child_by_node = await _outer_html(
            session,
            {
                "nodeId": _required_identity(child_host, "nodeId"),
                "includeShadowDOM": True,
            },
        )
        child_by_backend = await _outer_html(
            session,
            {
                "backendNodeId": _required_identity(child_host, "backendNodeId"),
                "includeShadowDOM": True,
            },
        )
        child_false = await _outer_html(
            session,
            {
                "nodeId": _required_identity(child_host, "nodeId"),
                "includeShadowDOM": False,
            },
        )
        assert_equal(child_by_node, child_including_shadow, "child-frame nodeId markup")
        assert_equal(
            child_by_backend,
            child_including_shadow,
            "child-frame backendNodeId markup",
        )
        assert_equal(child_false, child_ordinary, "child-frame false markup")

        child_document_with_shadow = await _outer_html(
            session,
            {
                "nodeId": _required_identity(child_document, "nodeId"),
                "includeShadowDOM": True,
            },
        )
        child_document_without_shadow = await _outer_html(
            session,
            {
                "nodeId": _required_identity(child_document, "nodeId"),
                "includeShadowDOM": False,
            },
        )
        if child_including_shadow not in child_document_with_shadow:
            raise SmokeError(
                "child document includeShadowDOM did not serialize its author root: "
                f"{child_document_with_shadow}"
            )
        if child_ordinary not in child_document_without_shadow:
            raise SmokeError(
                "child document ordinary serialization lost the shadow host: "
                f"{child_document_without_shadow}"
            )
        if "shadowrootmode" in child_document_without_shadow:
            raise SmokeError(
                "child document includeShadowDOM=false leaked a shadow template: "
                f"{child_document_without_shadow}"
            )
        assert_equal(mutation_events, [], "outerHTML reads emit no DOM mutation events")

        state.record(
            "dom_shadow_outer_html",
            {
                "omittedEqualsFalse": omitted == explicit_false,
                "referenceMarkupEqual": by_node == by_backend == by_object,
                "authorTemplateCount": by_node.count("<template shadowrootmode="),
                "declarativeIncluded": "shadowrootmode=\"open\"" in declarative_markup,
                "userAgentExcluded": "shadowrootmode" not in control_markup,
                "detachedIncluded": "detached-shadow" in detached_markup,
                "childFrameReferencesEqual": child_by_node == child_by_backend,
                "childDocumentIncluded": child_including_shadow
                in child_document_with_shadow,
                "mutationEvents": mutation_events,
            },
        )
    finally:
        if session is not None:
            await session.detach()
        await page.close()
