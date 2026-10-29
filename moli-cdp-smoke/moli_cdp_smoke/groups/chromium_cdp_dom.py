from __future__ import annotations

import asyncio
import base64
from typing import Any
from ..diagnostics import new_cdp_session
from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..helpers import capture_layout, attach_cdp_event_collector

from .chromium_cdp_support import _attribute_list_to_dict, _find_dom_node, _navigate_with_cdp_until_dom_ready, _send_cdp_expect_optional_error


async def _verify_chromium_dom_get_attributes_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/chromium-cdp-dom-page")
    document = await state.cdp.send("DOM.getDocument", {"depth": -1})
    root = document.get("root")
    if not root:
        raise SmokeError(f"DOM.getDocument missing root: {document}")
    target = _find_dom_node(root, lambda node: node.get("nodeName") == "P")
    if not target:
        raise SmokeError(f"DOM.getDocument did not expose target paragraph: {document}")
    attributes_result = await state.cdp.send("DOM.getAttributes", {"nodeId": target["nodeId"]})
    attributes = _attribute_list_to_dict(attributes_result.get("attributes") or [])
    assert_equal(attributes.get("class"), "class1", "Chromium DOM.getAttributes class sample")
    assert_equal(attributes.get("attr1"), "attr1", "Chromium DOM.getAttributes attr1 sample")

    document_attribute_error = await _send_cdp_expect_optional_error(
        state.cdp,
        "DOM.getAttributes",
        {"nodeId": root["nodeId"]},
    )
    if not document_attribute_error:
        raise SmokeError("DOM.getAttributes on the document node should return an error")
    state.record("chromium_dom_get_attributes_sample")

async def _verify_chromium_css_computed_style_breadth_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-cdp-computed-style-breadth"
    )
    assert_equal(await state.cdp.send("DOM.enable"), {}, "computed style DOM.enable result")
    assert_equal(await state.cdp.send("CSS.enable"), {}, "computed style CSS.enable result")
    setup = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": """
              (() => {
                const sheet = document.createElement('style');
                sheet.textContent = `#computed-style-target {
                  animation-timeline: auto;
                  animation-range-start: entry 10%;
                  animation-range-end: exit 20%;
                  background-position-x: 25%;
                  column-span: all;
                  column-width: 12px;
                  font-variant-alternates: historical-forms;
                  font-variant-emoji: emoji;
                  font-variant-position: super;
                  grid-auto-columns: 17px;
                  object-fit: cover;
                  overflow-wrap: anywhere;
                  pointer-events: none;
                  white-space-collapse: preserve;
                  zoom: 125%;
                }`;
                document.head.appendChild(sheet);
                const target = document.createElement('div');
                target.id = 'computed-style-target';
                target.style.setProperty('--smoke-token', 'present');
                document.body.appendChild(target);
                const style = getComputedStyle(target);
                const names = Array.from(style);
                return {
                  count: names.length,
                  unique: new Set(names).size === names.length,
                  hasPointerEvents: names.includes('pointer-events'),
                  hasGridAutoColumns: names.includes('grid-auto-columns'),
                  hasCustomProperty: names.includes('--smoke-token'),
                  excludesMarginShorthand: !names.includes('margin'),
                  pointerEvents: style.getPropertyValue('pointer-events'),
                  gridAutoColumns: style.getPropertyValue('grid-auto-columns'),
                  customProperty: style.getPropertyValue('--smoke-token'),
                  extendedValues: Object.fromEntries([
                    'animation-timeline',
                    'animation-range-start',
                    'animation-range-end',
                    'column-span',
                    'column-width',
                    'font-variant-alternates',
                    'font-variant-emoji',
                    'font-variant-position',
                    'zoom',
                  ].map(name => [name, style.getPropertyValue(name)])),
                };
              })()
            """,
            "returnByValue": True,
        },
    )
    js_summary = setup.get("result", {}).get("value")
    if not isinstance(js_summary, dict):
        raise SmokeError(f"computed style JavaScript summary is missing: {setup}")
    if not isinstance(js_summary.get("count"), int) or js_summary["count"] < 200:
        raise SmokeError(f"computed style JavaScript property set is too narrow: {js_summary}")
    for key in [
        "unique",
        "hasPointerEvents",
        "hasGridAutoColumns",
        "hasCustomProperty",
        "excludesMarginShorthand",
    ]:
        assert_equal(js_summary.get(key), True, f"computed style JavaScript {key}")
    assert_equal(js_summary.get("pointerEvents"), "none", "computed style JS pointer-events")
    assert_equal(js_summary.get("gridAutoColumns"), "17px", "computed style JS grid-auto-columns")
    assert_equal(js_summary.get("customProperty"), "present", "computed style JS custom property")
    expected_extended_values = {
        "animation-timeline": "auto",
        "animation-range-start": "entry 10%",
        "animation-range-end": "exit 20%",
        "column-span": "all",
        "column-width": "12px",
        "font-variant-alternates": "historical-forms",
        "font-variant-emoji": "emoji",
        "font-variant-position": "super",
        "zoom": "1.25",
    }
    assert_equal(
        js_summary.get("extendedValues"),
        expected_extended_values,
        "computed style JS Stylo-owned extended longhands",
    )

    document = await state.cdp.send("DOM.getDocument", {"depth": -1})
    target = _find_dom_node(
        document.get("root") or {},
        lambda node: _attribute_list_to_dict(node.get("attributes") or []).get("id")
        == "computed-style-target",
    )
    if not target:
        raise SmokeError(f"computed style target is missing from DOM.getDocument: {document}")
    async def read_computed_style() -> tuple[list[str], dict[str, object]]:
        result = await state.cdp.send(
            "CSS.getComputedStyleForNode", {"nodeId": target["nodeId"]}
        )
        properties = result.get("computedStyle")
        if not isinstance(properties, list) or len(properties) < 200:
            raise SmokeError(f"CDP computed style property set is too narrow: {result}")
        names = [
            property.get("name")
            for property in properties
            if isinstance(property, dict) and isinstance(property.get("name"), str)
        ]
        values = {
            property.get("name"): property.get("value")
            for property in properties
            if isinstance(property, dict) and isinstance(property.get("name"), str)
        }
        assert_equal(len(names), len(properties), "every CDP computed style entry has a name")
        assert_equal(len(values), len(properties), "CDP computed style names must be unique")
        return names, values

    names, values = await read_computed_style()
    for name, expected in {
        **expected_extended_values,
        "background-position-x": "25%",
        "grid-auto-columns": "17px",
        "object-fit": "cover",
        "overflow-wrap": "anywhere",
        "pointer-events": "none",
        "white-space-collapse": "preserve",
        "--smoke-token": "present",
    }.items():
        assert_equal(values.get(name), expected, f"CDP computed style {name}")
    for shorthand in ["margin", "mask", "padding-block"]:
        assert_equal(values.get(shorthand), None, f"CDP must not enumerate {shorthand}")

    # The first complete read may itself request Grid geometry in Moli. Sizes
    # sampled before that request can still be computed values, whereas a new
    # observation can use the newly published layout. Establish a geometry
    # boundary before asserting whole-declaration stability, retaining the
    # initial read above to cover cold enumeration and layout-independent data.
    # Calibrated with Chromium 145.0.7632.116 and Moli 4567db9e (three runs each):
    # only Moli's four size values change; both are stable after getBoxModel.
    size_names = ("width", "height", "inline-size", "block-size")
    initial_sizes = {name: values[name] for name in size_names}
    await capture_layout(state.page)
    await state.cdp.send("DOM.getBoxModel", {"nodeId": target["nodeId"]})
    sampled_names, sampled_values = await read_computed_style()
    assert_equal(sampled_names, names, "sampled CDP computed style names")
    assert_equal(
        {name: value for name, value in sampled_values.items() if name not in size_names},
        {name: value for name, value in values.items() if name not in size_names},
        "geometry sampling preserves this fixture's non-size properties",
    )
    repeated_names, repeated_values = await read_computed_style()
    assert_equal(repeated_names, names, "repeated CDP computed style names")
    assert_equal(repeated_values, sampled_values, "sampled CDP computed style values stay stable")

    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": """
              (() => {
                const target = document.getElementById('computed-style-target');
                target.style.pointerEvents = 'auto';
                target.style.setProperty('--smoke-token', 'updated');
              })()
            """,
        },
    )
    mutated_names, mutated_values = await read_computed_style()
    assert_equal(mutated_names, names, "mutated CDP computed style names")
    assert_equal(mutated_values.get("pointer-events"), "auto", "mutated pointer-events")
    assert_equal(mutated_values.get("--smoke-token"), "updated", "mutated custom property")
    state.record(
        "chromium_css_computed_style_breadth_sample",
        {
            "javascriptPropertyCount": js_summary["count"],
            "cdpPropertyCount": len(names),
            "initialSizes": initial_sizes,
            "sampledSizes": {name: sampled_values[name] for name in size_names},
        },
    )

async def _verify_sampled_computed_sizes(state: SmokeState) -> None:
    """Compare used sizes only after an explicit fresh visual observation.

    Cold CSSOM values need not match between engines. Moli's no-layout cold
    reads and reuse across mutation are tested with layout counters in Rust;
    this shared client contract checks the geometry both engines sampled.
    """
    size_names = ("width", "height", "inline-size", "block-size")
    for name, tag, style, expected in [
        (
            "content-box",
            "div",
            "width:500px;height:400px;max-width:120.5px;max-height:80.25px;"
            "padding:5px;border:2px solid;zoom:2;transform:scale(1.5)",
            ("120.5px", "80.25px", "120.5px", "80.25px"),
        ),
        (
            "vertical-border-box",
            "div",
            "width:500px;height:400px;max-width:160px;max-height:90px;"
            "padding:5px;border:2px solid;box-sizing:border-box;writing-mode:vertical-rl",
            ("160px", "90px", "90px", "160px"),
        ),
        (
            "grid",
            "div",
            "display:grid;width:180px;height:90px;grid-template-columns:1fr 2fr",
            ("180px", "90px", "180px", "90px"),
        ),
        (
            "vertical-canvas",
            "canvas",
            "writing-mode:vertical-rl",
            ("300px", "150px", "150px", "300px"),
        ),
    ]:
        await state.page.set_content(
            f'<!doctype html><style>html,body{{margin:0}}</style>'
            f'<{tag} id="sampled-size-target" style="{style}"></{tag}>'
        )

        await state.page.evaluate(
            "globalThis.heldSizeStyle = getComputedStyle(document.getElementById('sampled-size-target'))"
        )

        async def read_held_sizes() -> dict[str, str]:
            return await state.page.evaluate(
                "names => Object.fromEntries(names.map(name => [name, heldSizeStyle.getPropertyValue(name)]))",
                list(size_names),
            )

        initial = await read_held_sizes()
        document = await state.cdp.send("DOM.getDocument", {"depth": -1})
        target = _find_dom_node(
            document["root"],
            lambda node: _attribute_list_to_dict(node.get("attributes") or []).get("id")
            == "sampled-size-target",
        )
        if target is None:
            raise SmokeError(f"missing sampled size target: {name}")
        observations = []
        for mutated in (False, True):
            if mutated:
                await state.page.evaluate("""() => {
                    const target = document.getElementById('sampled-size-target');
                    target.style.width = '40px';
                    target.style.height = '30px';
                }""")
                expected = (
                    ("40px", "30px", "30px", "40px")
                    if name.startswith("vertical-")
                    else ("40px", "30px", "40px", "30px")
                )
            screenshot = await state.cdp.send("Page.captureScreenshot", {"format": "png"})
            if not base64.b64decode(screenshot.get("data", "")).startswith(b"\x89PNG\r\n\x1a\n"):
                raise SmokeError(f"sampled size screenshot is missing: {name}, mutated={mutated}")
            expected_values = dict(zip(size_names, expected, strict=True))
            previous_values = None
            for _ in range(2):
                result = await state.cdp.send(
                    "CSS.getComputedStyleForNode", {"nodeId": target["nodeId"]}
                )
                values = {entry["name"]: entry["value"] for entry in result["computedStyle"]}
                assert_equal(
                    {key: values.get(key) for key in size_names},
                    expected_values,
                    f"sampled CDP sizes: {name}, mutated={mutated}",
                )
                assert_equal(
                    await read_held_sizes(),
                    expected_values,
                    f"held CSSOM sizes: {name}, mutated={mutated}",
                )
                if previous_values is not None:
                    assert_equal(values, previous_values, f"sampled declaration stability: {name}")
                previous_values = values
            observations.append(expected_values)
        state.record(
            f"computed_size_sampling_{name}",
            {"initial": initial, "sampled": observations[0], "refreshed": observations[1]},
        )

async def _verify_chromium_dom_query_selector_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/chromium-cdp-dom-query-page")
    wire_order: list[dict[str, Any]] = []

    def on_set_child_nodes(params: dict[str, Any]) -> None:
        wire_order.append({"kind": "event", "params": params})

    def find_event_node(event: dict[str, Any], predicate: Any) -> dict[str, Any] | None:
        for node in event.get("params", {}).get("nodes") or []:
            found = _find_dom_node(node, predicate)
            if found:
                return found
        return None

    async def query(method: str, params: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        start = len(wire_order)
        result = await state.cdp.send(method, params)
        wire_order.append({"kind": "response", "method": method})
        return result, wire_order[start:]

    state.cdp.on("DOM.setChildNodes", on_set_child_nodes)
    try:
        # Chromium's default depth exposes BODY but not its children. A
        # query must synchronously push the missing node path before replying;
        # chromedp NodeReady relies on this exact frontend-node-map contract.
        document = await state.cdp.send("DOM.getDocument")
        root = document.get("root")
        body = _find_dom_node(root or {}, lambda node: node.get("nodeName") == "BODY")
        if not body:
            raise SmokeError(f"DOM.getDocument did not expose body for query sample: {document}")

        first_div, first_order = await query(
            "DOM.querySelector", {"nodeId": body["nodeId"], "selector": "div"}
        )
        assert_equal(
            [item["kind"] for item in first_order],
            ["event", "response"],
            "DOM.querySelector child-path event order",
        )
        assert_equal(
            first_order[0]["params"].get("parentId"),
            body["nodeId"],
            "DOM.querySelector child-path parent",
        )
        first_div_node = find_event_node(
            first_order[0], lambda node: node.get("nodeId") == first_div.get("nodeId")
        )
        first_attrs = _attribute_list_to_dict((first_div_node or {}).get("attributes") or [])
        assert_equal(first_attrs.get("id"), "firstDiv", "Chromium DOM.querySelector first div sample")

        second_div_node = find_event_node(
            first_order[0],
            lambda node: _attribute_list_to_dict(node.get("attributes") or []).get("id")
            == "secondDiv",
        )
        if not second_div_node:
            raise SmokeError(f"first body expansion did not expose secondDiv: {first_order}")
        second_div, second_order = await query(
            "DOM.querySelector",
            {"nodeId": body["nodeId"], "selector": "div#secondDiv"},
        )
        assert_equal(
            second_order,
            [{"kind": "response", "method": "DOM.querySelector"}],
            "repeated child-path suppression",
        )
        assert_equal(
            second_div.get("nodeId"),
            second_div_node.get("nodeId"),
            "Chromium DOM.querySelector id sample",
        )

        all_test_divs, query_all_order = await query(
            "DOM.querySelectorAll",
            {"nodeId": body["nodeId"], "selector": "div.testClass"},
        )
        assert_equal(
            query_all_order,
            [{"kind": "response", "method": "DOM.querySelectorAll"}],
            "DOM.querySelectorAll already-published path",
        )
        assert_equal(len(all_test_divs.get("nodeIds") or []), 5, "Chromium DOM.querySelectorAll class sample")

        depth_1 = find_event_node(
            first_order[0],
            lambda node: _attribute_list_to_dict(node.get("attributes") or []).get("id")
            == "depth-1",
        )
        if not depth_1:
            raise SmokeError(f"body expansion did not expose depth-1: {first_order}")
        deep_div, deep_order = await query(
            "DOM.querySelector",
            {"nodeId": body["nodeId"], "selector": "div#targetDiv"},
        )
        assert_equal(
            [item["kind"] for item in deep_order],
            ["event", "event", "response"],
            "deep DOM.querySelector path order",
        )
        assert_equal(
            deep_order[0]["params"].get("parentId"),
            depth_1.get("nodeId"),
            "deep DOM.querySelector first path parent",
        )
        depth_2 = find_event_node(
            deep_order[0],
            lambda node: _attribute_list_to_dict(node.get("attributes") or []).get("id")
            == "depth-2",
        )
        if not depth_2:
            raise SmokeError(f"first deep path event did not expose depth-2: {deep_order}")
        assert_equal(
            deep_order[1]["params"].get("parentId"),
            depth_2.get("nodeId"),
            "deep DOM.querySelector second path parent",
        )
        deep_node = find_event_node(
            deep_order[1], lambda node: node.get("nodeId") == deep_div.get("nodeId")
        )
        deep_attrs = _attribute_list_to_dict((deep_node or {}).get("attributes") or [])
        assert_equal(deep_attrs.get("id"), "targetDiv", "Chromium DOM.querySelector deep sample")
        state.record("chromium_dom_query_selector_sample")
        state.record("chromedp_node_ready_query_path_contract")
    finally:
        state.cdp.remove_listener("DOM.setChildNodes", on_set_child_nodes)

async def _verify_chromium_dom_single_text_child_projection_sample(
    state: SmokeState,
) -> None:
    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-cdp-dom-query-page"
    )

    def assert_only_text_child(
        node: dict[str, Any] | None, expected: str, label: str
    ) -> None:
        if not node:
            raise SmokeError(f"{label} node is missing")
        assert_equal(node.get("childNodeCount"), 1, f"{label} childNodeCount")
        children = node.get("children")
        if not isinstance(children, list) or len(children) != 1:
            raise SmokeError(f"{label} must publish its only text child: {node}")
        assert_equal(children[0].get("nodeName"), "#text", f"{label} child nodeName")
        assert_equal(children[0].get("nodeValue"), expected, f"{label} child nodeValue")

    depth_three = await state.cdp.send("DOM.getDocument", {"depth": 3})
    root = depth_three.get("root") or {}
    title = _find_dom_node(root, lambda node: node.get("nodeName") == "TITLE")
    single = _find_dom_node(
        root,
        lambda node: _attribute_list_to_dict(node.get("attributes") or []).get("id")
        == "singleTextChild",
    )
    multiple = _find_dom_node(
        root,
        lambda node: _attribute_list_to_dict(node.get("attributes") or []).get("id")
        == "multipleChildren",
    )
    assert_only_text_child(title, "Example Domain", "depth-three TITLE")
    assert_only_text_child(single, "Only child", "depth-three DIV")
    if not multiple:
        raise SmokeError("depth-three multiple-child DIV is missing")
    assert_equal(multiple.get("childNodeCount"), 2, "multiple-child DIV childNodeCount")
    if "children" in multiple:
        raise SmokeError(
            f"depth boundary must not expand a container with multiple children: {multiple}"
        )

    pushed: list[dict[str, Any]] = []

    def on_set_child_nodes(params: dict[str, Any]) -> None:
        pushed.append(params)

    state.cdp.on("DOM.setChildNodes", on_set_child_nodes)
    try:
        default_document = await state.cdp.send("DOM.getDocument")
        default_root = default_document.get("root") or {}
        head = _find_dom_node(default_root, lambda node: node.get("nodeName") == "HEAD")
        body = _find_dom_node(default_root, lambda node: node.get("nodeName") == "BODY")
        if not head or not body:
            raise SmokeError(
                f"default DOM projection is missing HEAD/BODY: {default_document}"
            )
        await state.cdp.send(
            "DOM.requestChildNodes", {"nodeId": head["nodeId"], "depth": 1}
        )
        await state.cdp.send(
            "DOM.requestChildNodes", {"nodeId": body["nodeId"], "depth": 1}
        )
        head_event = next(
            (event for event in pushed if event.get("parentId") == head["nodeId"]), None
        )
        body_event = next(
            (event for event in pushed if event.get("parentId") == body["nodeId"]), None
        )
        event_title = _find_dom_node(
            {"children": (head_event or {}).get("nodes") or []},
            lambda node: node.get("nodeName") == "TITLE",
        )
        event_single = _find_dom_node(
            {"children": (body_event or {}).get("nodes") or []},
            lambda node: _attribute_list_to_dict(node.get("attributes") or []).get("id")
            == "singleTextChild",
        )
        assert_only_text_child(event_title, "Example Domain", "requested TITLE")
        assert_only_text_child(event_single, "Only child", "requested DIV")
    finally:
        state.cdp.remove_listener("DOM.setChildNodes", on_set_child_nodes)

    state.record("chromium_dom_single_text_child_projection_sample")

async def _verify_chromium_dom_debugger_event_listeners_sample(state: SmokeState) -> None:
    page = await state.context.new_page()
    primary = None
    peer = None
    try:
        await page.goto(f"{state.fixture}/plain?dom-debugger-listeners")

        primary = await new_cdp_session(state.context, page)
        peer = await new_cdp_session(state.context, page)
        evaluated = await primary.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        document.body.innerHTML = `
                            <main id="listener-root">
                                <button id="listener-child">
                                    <span id="listener-grand"></span>
                                </button>
                            </main>`;
                        const root = document.querySelector('#listener-root');
                        const child = document.querySelector('#listener-child');
                        const grand = document.querySelector('#listener-grand');
                        function removed() {}
                        function duplicate() {}
                        root.addEventListener('removed', removed);
                        root.removeEventListener('removed', removed);
                        root.addEventListener('duplicate', duplicate);
                        root.addEventListener('duplicate', duplicate);
                        root.addEventListener('root-bubble', function rootBubble() {});
                        root.addEventListener(
                            'root-capture',
                            function rootCapture() {},
                            {capture: true, passive: true, once: true}
                        );
                        root.onclick = function rootProperty() {};
                        globalThis.__domDebuggerObjectListener = {
                            handleEvent: function domDebuggerObjectHandler() {}
                        };
                        root.addEventListener('object-event', __domDebuggerObjectListener);
                        root.addEventListener('group-a', function groupAFirst() {});
                        root.addEventListener('group-b', function groupB() {});
                        root.addEventListener('group-a', function groupASecond() {});
                        child.addEventListener('child-listener', function childListener() {});
                        grand.addEventListener('grand-listener', function grandListener() {});
                        const shadowHost = document.createElement('section');
                        root.append(shadowHost);
                        const shadowChild = shadowHost.attachShadow({mode: 'open'}).appendChild(
                            document.createElement('i')
                        );
                        shadowChild.addEventListener(
                            'shadow-listener',
                            function shadowListener() {}
                        );
                        return root;
                    })()
                """,
                "objectGroup": "dom-debugger-smoke",
            },
        )
        object_id = evaluated.get("result", {}).get("objectId")
        if not object_id:
            raise SmokeError(f"Runtime.evaluate should return listener root handle: {evaluated}")

        frame_tree = await primary.send("Page.getFrameTree")
        frame_id = frame_tree.get("frameTree", {}).get("frame", {}).get("id")
        if not frame_id:
            raise SmokeError(f"Page.getFrameTree should return a root frame: {frame_tree}")
        isolated_world = await primary.send(
            "Page.createIsolatedWorld",
            {"frameId": frame_id, "worldName": "dom-debugger-listener-world"},
        )
        isolated_context_id = isolated_world.get("executionContextId")
        if not isinstance(isolated_context_id, int):
            raise SmokeError(
                f"Page.createIsolatedWorld should return a context id: {isolated_world}"
            )
        await primary.send(
            "Runtime.evaluate",
            {
                "contextId": isolated_context_id,
                "expression": """
                    document.querySelector('#listener-root').addEventListener(
                        'isolated-listener',
                        function isolatedWorldHandler() {}
                    )
                """,
            },
        )

        default_result = await primary.send(
            "DOMDebugger.getEventListeners", {"objectId": object_id}
        )
        default_listeners = default_result.get("listeners") or []
        default_types = [listener.get("type") for listener in default_listeners]
        if default_types != [
            "root-capture",
            "duplicate",
            "root-bubble",
            "click",
            "object-event",
            "group-a",
            "group-a",
            "group-b",
        ]:
            raise SmokeError(
                "DOMDebugger default depth should report capture listeners first, suppress "
                f"removed/duplicate entries, and stay on the root node: {default_result}"
            )
        backend_ids = {listener.get("backendNodeId") for listener in default_listeners}
        if len(backend_ids) != 1 or not all(
            isinstance(backend_id, int) and backend_id > 0 for backend_id in backend_ids
        ):
            raise SmokeError(
                f"DOMDebugger node listeners should share a positive backendNodeId: {default_result}"
            )
        for listener in default_listeners:
            if not isinstance(listener.get("scriptId"), str):
                raise SmokeError(f"DOMDebugger listener should include scriptId: {listener}")
            if not isinstance(listener.get("lineNumber"), int) or not isinstance(
                listener.get("columnNumber"), int
            ):
                raise SmokeError(f"DOMDebugger listener should include source location: {listener}")
            if not listener.get("handler", {}).get("objectId") or not listener.get(
                "originalHandler", {}
            ).get("objectId"):
                raise SmokeError(
                    "object-group-backed DOMDebugger listeners should include live handler "
                    f"RemoteObjects: {listener}"
                )

        object_listener = next(
            listener for listener in default_listeners if listener.get("type") == "object-event"
        )
        assert_equal(
            object_listener.get("handler", {}).get("type"),
            "function",
            "DOMDebugger effective object listener handler",
        )
        assert_equal(
            object_listener.get("originalHandler", {}).get("type"),
            "object",
            "DOMDebugger original object listener handler",
        )
        for label, remote_object, expression in (
            (
                "effective handler",
                object_listener["handler"],
                "function() { return this === __domDebuggerObjectListener.handleEvent; }",
            ),
            (
                "original handler",
                object_listener["originalHandler"],
                "function() { return this === __domDebuggerObjectListener; }",
            ),
        ):
            identity = await primary.send(
                "Runtime.callFunctionOn",
                {
                    "objectId": remote_object["objectId"],
                    "functionDeclaration": expression,
                    "returnByValue": True,
                },
            )
            assert_equal(
                identity.get("result", {}).get("value"),
                True,
                f"DOMDebugger {label} RemoteObject identity",
            )

        depth_two = await primary.send(
            "DOMDebugger.getEventListeners", {"objectId": object_id, "depth": 2}
        )
        depth_two_types = {listener.get("type") for listener in depth_two.get("listeners") or []}
        if "child-listener" not in depth_two_types or "grand-listener" in depth_two_types:
            raise SmokeError(f"DOMDebugger depth=2 traversal mismatch: {depth_two}")

        full_subtree = await primary.send(
            "DOMDebugger.getEventListeners", {"objectId": object_id, "depth": -1}
        )
        full_types = {listener.get("type") for listener in full_subtree.get("listeners") or []}
        if "grand-listener" not in full_types or "shadow-listener" in full_types:
            raise SmokeError(f"DOMDebugger non-piercing subtree traversal mismatch: {full_subtree}")

        pierced = await primary.send(
            "DOMDebugger.getEventListeners",
            {"objectId": object_id, "depth": -1, "pierce": True},
        )
        pierced_types = {listener.get("type") for listener in pierced.get("listeners") or []}
        if not {"shadow-listener", "isolated-listener"}.issubset(pierced_types):
            raise SmokeError(
                "DOMDebugger pierce should traverse author shadow roots and include listeners "
                f"from other worlds: {pierced}"
            )
        isolated_listener = next(
            listener
            for listener in pierced.get("listeners") or []
            if listener.get("type") == "isolated-listener"
        )
        isolated_handler_id = isolated_listener.get("handler", {}).get("objectId")
        if not isolated_handler_id:
            raise SmokeError(
                "DOMDebugger should wrap isolated-world handlers in the source object group: "
                f"{isolated_listener}"
            )
        isolated_handler_name = await primary.send(
            "Runtime.callFunctionOn",
            {
                "objectId": isolated_handler_id,
                "functionDeclaration": "function() { return this.name; }",
                "returnByValue": True,
            },
        )
        assert_equal(
            isolated_handler_name.get("result", {}).get("value"),
            "isolatedWorldHandler",
            "DOMDebugger isolated-world handler RemoteObject",
        )

        ungrouped = await primary.send(
            "Runtime.evaluate",
            {"expression": "document.querySelector('#listener-root')"},
        )
        ungrouped_id = ungrouped.get("result", {}).get("objectId")
        ungrouped_listeners = await primary.send(
            "DOMDebugger.getEventListeners", {"objectId": ungrouped_id}
        )
        if any(
            "handler" in listener or "originalHandler" in listener
            for listener in ungrouped_listeners.get("listeners") or []
        ):
            raise SmokeError(
                "DOMDebugger should omit handler RemoteObjects when the source object has no "
                f"object group: {ungrouped_listeners}"
            )

        plain = await primary.send("Runtime.evaluate", {"expression": "({answer: 42})"})
        plain_listeners = await primary.send(
            "DOMDebugger.getEventListeners",
            {"objectId": plain.get("result", {}).get("objectId")},
        )
        assert_equal(
            plain_listeners.get("listeners"),
            [],
            "DOMDebugger plain object listener result",
        )

        ordered_target = await primary.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        const target = new EventTarget();
                        function removedNumericTwo() {}
                        target.addEventListener('2', removedNumericTwo);
                        target.addEventListener('1', function numericOne() {});
                        target.removeEventListener('2', removedNumericTwo);
                        target.addEventListener('2', function numericTwo() {});
                        target.addEventListener('plain', function plainType() {});
                        return target;
                    })()
                """,
                "objectGroup": "dom-debugger-smoke",
            },
        )
        ordered_listeners = await primary.send(
            "DOMDebugger.getEventListeners",
            {"objectId": ordered_target.get("result", {}).get("objectId")},
        )
        assert_equal(
            [listener.get("type") for listener in ordered_listeners.get("listeners") or []],
            ["1", "2", "plain"],
            "DOMDebugger EventTarget numeric-name and remove/re-add ordering",
        )

        peer_error = await _send_cdp_expect_optional_error(
            peer,
            "DOMDebugger.getEventListeners",
            {"objectId": object_id},
        )
        if not peer_error or "Could not find object with given id" not in str(peer_error):
            raise SmokeError(
                "DOMDebugger object handles must remain Inspector-session-local: "
                f"{peer_error}"
            )

        await primary.send(
            "Runtime.releaseObjectGroup", {"objectGroup": "dom-debugger-smoke"}
        )
        released_error = await _send_cdp_expect_optional_error(
            primary,
            "DOMDebugger.getEventListeners",
            {"objectId": object_id},
        )
        if not released_error or "Could not find object with given id" not in str(released_error):
            raise SmokeError(
                f"DOMDebugger should reject released object handles: {released_error}"
            )

        state.record(
            "chromium_dom_debugger_event_listeners_sample",
            {
                "defaultListenerTypes": default_types,
                "depthTwoListenerTypes": sorted(str(value) for value in depth_two_types),
                "piercedListenerTypes": sorted(str(value) for value in pierced_types),
            },
        )
    finally:
        if primary is not None:
            await primary.detach()
        if peer is not None:
            await peer.detach()
        await page.close()

async def _verify_chromium_dom_debugger_event_listener_breakpoint_sample(
    state: SmokeState,
) -> None:
    page = await state.context.new_page()
    owner = None
    peer = None
    try:
        await page.goto(f"{state.fixture}/plain?dom-debugger-event-breakpoint")

        owner = await new_cdp_session(state.context, page)
        peer = await new_cdp_session(state.context, page)
        owner_events = attach_cdp_event_collector(owner, ["Debugger.paused"])
        peer_events = attach_cdp_event_collector(peer, ["Debugger.paused"])

        for method in (
            "DOMDebugger.setEventListenerBreakpoint",
            "DOMDebugger.removeEventListenerBreakpoint",
        ):
            error = await _send_cdp_expect_optional_error(
                owner,
                method,
                {"eventName": ""},
            )
            if not error or "Event name is empty" not in str(error):
                raise SmokeError(f"{method} should reject an empty event name: {error}")

        await peer.send("Debugger.enable")
        await owner.send(
            "DOMDebugger.setEventListenerBreakpoint",
            {"eventName": "custom", "targetName": "EventTargetImpl"},
        )
        setup = await peer.send(
            "Runtime.evaluate",
            {
                "expression": """
                    globalThis.__breakpointTarget = new EventTarget();
                    globalThis.__breakpointCount = 0;
                    __breakpointTarget.addEventListener(
                        'custom', () => ++__breakpointCount
                    );
                    __breakpointTarget.addEventListener(
                        'custom', () => ++__breakpointCount
                    );
                    true
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            setup.get("result", {}).get("value"),
            True,
            "DOMDebugger event breakpoint setup",
        )
        disabled_owner_dispatch = await peer.send(
            "Runtime.evaluate",
            {
                "expression": """
                    __breakpointTarget.dispatchEvent(new Event('custom'));
                    __breakpointCount
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            disabled_owner_dispatch.get("result", {}).get("value"),
            2,
            "a peer Debugger must not activate a disabled owner's DOMDebugger breakpoint",
        )
        if owner_events or peer_events:
            raise SmokeError(
                "a DOMDebugger breakpoint owned by a Debugger-disabled session must not pause: "
                f"owner={owner_events}, peer={peer_events}"
            )

        await owner.send("Debugger.enable")
        dispatch = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        __breakpointTarget.dispatchEvent(new Event('custom'));
                        __breakpointCount
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: len(owner_events) == 1 and len(peer_events) == 1,
            "first DOMDebugger event-listener pause in both sessions",
        )
        first_owner_pause = owner_events[0]["params"]
        first_peer_pause = peer_events[0]["params"]
        assert_equal(
            first_owner_pause.get("reason"),
            "EventListener",
            "DOMDebugger breakpoint owner pause reason",
        )
        assert_equal(
            first_owner_pause.get("data"),
            {"eventName": "listener:custom", "targetName": "EventTargetImpl"},
            "DOMDebugger breakpoint owner pause data",
        )
        assert_equal(
            first_peer_pause.get("reason"),
            "other",
            "DOMDebugger breakpoint peer pause reason",
        )
        if "data" in first_peer_pause:
            raise SmokeError(
                f"non-owner Debugger pause should omit DOMDebugger data: {first_peer_pause}"
            )

        # Chromium completes a resume sent by any enabled peer before the next
        # listener's immediate pause becomes observable.
        await peer.send("Debugger.resume")
        await wait_until(
            lambda: len(owner_events) == 2 and len(peer_events) == 2,
            "second DOMDebugger event-listener pause in both sessions",
        )
        assert_equal(
            owner_events[1]["params"].get("data"),
            {"eventName": "listener:custom", "targetName": "EventTargetImpl"},
            "second DOMDebugger listener pause data",
        )
        assert_equal(
            peer_events[1]["params"].get("reason"),
            "other",
            "second DOMDebugger peer pause reason",
        )
        await owner.send("Debugger.resume")
        dispatched = await asyncio.wait_for(dispatch, timeout=5)
        assert_equal(
            dispatched.get("result", {}).get("value"),
            4,
            "resumed DOMDebugger event dispatch result",
        )

        await owner.send(
            "DOMDebugger.removeEventListenerBreakpoint",
            {"eventName": "custom", "targetName": "eventtargetimpl"},
        )
        removed_dispatch = await peer.send(
            "Runtime.evaluate",
            {
                "expression": """
                    __breakpointTarget.dispatchEvent(new Event('custom'));
                    __breakpointCount
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            removed_dispatch.get("result", {}).get("value"),
            6,
            "DOMDebugger targetName removal is ASCII case-insensitive",
        )
        if len(owner_events) != 2 or len(peer_events) != 2:
            raise SmokeError(
                "removed DOMDebugger event breakpoint must not pause: "
                f"owner={owner_events}, peer={peer_events}"
            )

        await owner.send(
            "DOMDebugger.setEventListenerBreakpoint",
            {"eventName": "click"},
        )
        await page.goto(f"{state.fixture}/plain?dom-debugger-event-breakpoint-after-navigation")

        navigation_dispatch = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        document.body.innerHTML = '<button id="after">after</button>';
                        after.addEventListener('click', () => 42);
                        after.click();
                        true
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: len(owner_events) == 3 and len(peer_events) == 3,
            "restored DOMDebugger event-listener pause after navigation",
        )
        assert_equal(
            owner_events[2]["params"].get("data"),
            {"eventName": "listener:click", "targetName": "BUTTON"},
            "restored DOMDebugger event breakpoint pause data",
        )
        await owner.send("Debugger.resume")
        navigation_dispatched = await asyncio.wait_for(navigation_dispatch, timeout=5)
        assert_equal(
            navigation_dispatched.get("result", {}).get("value"),
            True,
            "resumed DOMDebugger event dispatch after navigation",
        )

        await owner.detach()
        owner = None
        detached_owner_dispatch = await peer.send(
            "Runtime.evaluate",
            {
                "expression": "after.click(); 1",
                "returnByValue": True,
            },
        )
        assert_equal(
            detached_owner_dispatch.get("result", {}).get("value"),
            1,
            "detached DOMDebugger breakpoint owner cleanup",
        )
        if len(peer_events) != 3:
            raise SmokeError(
                "detaching the DOMDebugger breakpoint owner must remove renderer state: "
                f"{peer_events}"
            )

        state.record(
            "chromium_dom_debugger_event_listener_breakpoint_sample",
            {
                "ownerPauseReasons": [
                    event["params"].get("reason") for event in owner_events
                ],
                "peerPauseReasons": [
                    event["params"].get("reason") for event in peer_events
                ],
            },
        )
    finally:
        if owner is not None:
            await owner.detach()
        if peer is not None:
            await peer.detach()
        await page.close()

async def _verify_chromium_dom_debugger_dom_breakpoint_sample(
    state: SmokeState,
) -> None:
    page = await state.context.new_page()
    owner = None
    peer = None
    pending_task: asyncio.Task[Any] | None = None
    pause_promise_task: asyncio.Task[Any] | None = None
    try:
        await page.goto(f"{state.fixture}/plain?dom-debugger-dom-breakpoint")

        await page.evaluate(
            """
            document.body.innerHTML = `
                <main id="root">
                    <section id="middle"><span>old</span></section>
                </main>
            `;
            true
            """
        )
        owner = await new_cdp_session(state.context, page)
        peer = await new_cdp_session(state.context, page)
        owner_events = attach_cdp_event_collector(
            owner,
            ["DOM.setChildNodes", "DOM.childNodeRemoved", "Debugger.paused"],
        )
        peer_events = attach_cdp_event_collector(peer, ["Debugger.paused"])
        await owner.send("DOM.enable")
        await owner.send("Debugger.enable")
        await peer.send("Debugger.enable")
        document_node_id: int | None = None

        async def query_node(selector: str, depth: int = 1) -> int:
            nonlocal document_node_id
            if document_node_id is None:
                document = await owner.send("DOM.getDocument", {"depth": depth})
                document_node_id = document.get("root", {}).get("nodeId")
                if not isinstance(document_node_id, int) or document_node_id <= 0:
                    raise SmokeError(
                        f"DOM.getDocument missing root for {selector}: {document}"
                    )
            result = await owner.send(
                "DOM.querySelector",
                {"nodeId": document_node_id, "selector": selector},
            )
            node_id = result.get("nodeId")
            if not isinstance(node_id, int) or node_id <= 0:
                raise SmokeError(f"DOM.querySelector did not find {selector}: {result}")
            return node_id

        root_node_id = await query_node("#root")
        missing_node = await _send_cdp_expect_optional_error(
            owner,
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": 2_147_483_647, "type": "bogus"},
        )
        if not missing_node or "Could not find node with given id" not in str(
            missing_node
        ):
            raise SmokeError(
                "DOM breakpoint node validation must precede type validation: "
                f"{missing_node}"
            )
        unknown_type = await _send_cdp_expect_optional_error(
            owner,
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": root_node_id, "type": "bogus"},
        )
        if not unknown_type or "Unknown DOM breakpoint type: bogus" not in str(
            unknown_type
        ):
            raise SmokeError(
                f"DOM breakpoint should reject an unknown type: {unknown_type}"
            )

        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": root_node_id, "type": "subtree-modified"},
        )
        subtree_event_start = len(owner_events)
        pending_task = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        document.querySelector('#middle').appendChild(
                            document.createElement('b')
                        );
                        true
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused"
                for event in owner_events[subtree_event_start:]
            )
            == 1
            and len(peer_events) == 1,
            "DOM subtree-modified pause in owner and peer sessions",
        )
        subtree_events = owner_events[subtree_event_start:]
        subtree_methods = [event["method"] for event in subtree_events]
        if "DOM.setChildNodes" not in subtree_methods:
            raise SmokeError(
                "an unbound DOM mutation target must be pushed before pause: "
                f"{subtree_events}"
            )
        if subtree_methods.index("DOM.setChildNodes") > subtree_methods.index(
            "Debugger.paused"
        ):
            raise SmokeError(
                "DOM.setChildNodes must precede Debugger.paused for an unbound target: "
                f"{subtree_events}"
            )
        subtree_pause = next(
            event["params"]
            for event in subtree_events
            if event["method"] == "Debugger.paused"
        )
        assert_equal(
            subtree_pause.get("reason"),
            "DOM",
            "DOM subtree breakpoint owner reason",
        )
        assert_equal(
            subtree_pause.get("data", {}).get("nodeId"),
            root_node_id,
            "DOM subtree breakpoint owner nodeId",
        )
        assert_equal(
            subtree_pause.get("data", {}).get("type"),
            "subtree-modified",
            "DOM subtree breakpoint type",
        )
        assert_equal(
            subtree_pause.get("data", {}).get("insertion"),
            True,
            "DOM subtree insertion marker",
        )
        target_node_id = subtree_pause.get("data", {}).get("targetNodeId")
        if not isinstance(target_node_id, int) or target_node_id <= 0:
            raise SmokeError(
                f"DOM subtree pause missing targetNodeId: {subtree_pause}"
            )
        assert_equal(
            peer_events[0]["params"].get("reason"),
            "other",
            "non-owner DOM breakpoint peer reason",
        )
        if "data" in peer_events[0]["params"]:
            raise SmokeError(
                f"non-owner DOM breakpoint peer must omit data: {peer_events[0]}"
            )
        await peer.send("Debugger.resume")
        subtree_result = await asyncio.wait_for(pending_task, timeout=5)
        pending_task = None
        assert_equal(
            subtree_result.get("result", {}).get("value"),
            True,
            "resumed DOM subtree mutation result",
        )

        fragment_owner_start = sum(
            event["method"] == "Debugger.paused" for event in owner_events
        )
        fragment_peer_start = len(peer_events)
        pending_task = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        (() => {
                            const fragment = document.createDocumentFragment();
                            fragment.append(
                                document.createElement('i'),
                                document.createElement('u')
                            );
                            document.querySelector('#middle').appendChild(fragment);
                            return true;
                        })()
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused" for event in owner_events
            )
            == fragment_owner_start + 1
            and len(peer_events) == fragment_peer_start + 1,
            "single DOM pause for a DocumentFragment insertion batch",
        )
        await owner.send("Debugger.resume")
        fragment_result = await asyncio.wait_for(pending_task, timeout=5)
        pending_task = None
        assert_equal(
            fragment_result.get("result", {}).get("value"),
            True,
            "resumed DocumentFragment insertion result",
        )
        assert_equal(
            sum(event["method"] == "Debugger.paused" for event in owner_events),
            fragment_owner_start + 1,
            "DocumentFragment insertion batch owner pause count",
        )
        assert_equal(
            len(peer_events),
            fragment_peer_start + 1,
            "DocumentFragment insertion batch peer pause count",
        )

        for _ in range(2):
            await owner.send(
                "DOMDebugger.removeDOMBreakpoint",
                {"nodeId": root_node_id, "type": "subtree-modified"},
            )

        await peer.send(
            "Runtime.evaluate",
            {
                "expression": """
                    document.querySelector('#root').insertAdjacentHTML(
                        'beforeend',
                        '<div id="move-old"><em id="moving-child"></em></div>' +
                        '<div id="move-new"></div>'
                    );
                    true
                """,
                "returnByValue": True,
            },
        )
        moving_node_id = await query_node("#moving-child")
        move_old_node_id = await query_node("#move-old")
        move_new_node_id = await query_node("#move-new")
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": moving_node_id, "type": "node-removed"},
        )
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": move_new_node_id, "type": "subtree-modified"},
        )
        move_owner_start = sum(
            event["method"] == "Debugger.paused" for event in owner_events
        )
        move_wire_start = len(owner_events)
        move_peer_start = len(peer_events)
        pending_task = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        globalThis.__movingChild =
                            document.querySelector('#moving-child');
                        document.querySelector('#move-new').appendChild(__movingChild);
                        true
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused" for event in owner_events
            )
            == move_owner_start + 1
            and len(peer_events) == move_peer_start + 1,
            "DOM moved-node removal pause",
        )
        first_move_pause = next(
            event["params"]
            for event in reversed(owner_events)
            if event["method"] == "Debugger.paused"
        )
        assert_equal(
            first_move_pause.get("data"),
            {"nodeId": moving_node_id, "type": "node-removed"},
            "moved-node removal breakpoint data",
        )
        first_move_events = owner_events[move_wire_start:]
        first_move_methods = [event["method"] for event in first_move_events]
        if "DOM.childNodeRemoved" not in first_move_methods:
            raise SmokeError(
                "WillRemoveDOMNode must publish DOM.childNodeRemoved before pausing: "
                f"{first_move_events}"
            )
        if first_move_methods.index("DOM.childNodeRemoved") > first_move_methods.index(
            "Debugger.paused"
        ):
            raise SmokeError(
                "DOM.childNodeRemoved must precede Debugger.paused: "
                f"{first_move_events}"
            )
        first_move_removed = next(
            event
            for event in first_move_events
            if event["method"] == "DOM.childNodeRemoved"
        )
        assert_equal(
            first_move_removed.get("params", {}).get("nodeId"),
            moving_node_id,
            "moved-node pre-pause removal nodeId",
        )
        assert_equal(
            first_move_removed.get("params", {}).get("parentNodeId"),
            move_old_node_id,
            "moved-node pre-pause removal parentNodeId",
        )
        first_move_state = await owner.send(
            "Runtime.evaluate",
            {
                "expression": "__movingChild.parentNode && __movingChild.parentNode.id",
                "returnByValue": True,
            },
        )
        assert_equal(
            first_move_state.get("result", {}).get("value"),
            "move-old",
            "moved node remains attached during WillRemoveDOMNode",
        )
        await owner.send("Debugger.resume")
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused" for event in owner_events
            )
            == move_owner_start + 2
            and len(peer_events) == move_peer_start + 2,
            "DOM moved-node insertion pause",
        )
        second_move_pause = next(
            event["params"]
            for event in reversed(owner_events)
            if event["method"] == "Debugger.paused"
        )
        assert_equal(
            second_move_pause.get("data"),
            {
                "nodeId": move_new_node_id,
                "targetNodeId": move_new_node_id,
                "type": "subtree-modified",
                "insertion": True,
            },
            "moved-node insertion breakpoint data",
        )
        detached_move_state = await owner.send(
            "Runtime.evaluate",
            {
                "expression": "__movingChild.parentNode",
                "returnByValue": True,
            },
        )
        assert_equal(
            detached_move_state.get("result", {}).get("subtype"),
            "null",
            "moved node is detached before WillInsertDOMNode",
        )
        await owner.send("Debugger.resume")
        move_result = await asyncio.wait_for(pending_task, timeout=5)
        pending_task = None
        assert_equal(
            move_result.get("result", {}).get("value"),
            True,
            "resumed moved-node insertion result",
        )
        final_move_state = await owner.send(
            "Runtime.evaluate",
            {
                "expression": "__movingChild.parentNode.id",
                "returnByValue": True,
            },
        )
        assert_equal(
            final_move_state.get("result", {}).get("value"),
            "move-new",
            "moved node final parent",
        )
        move_removal_events = [
            event
            for event in owner_events[move_wire_start:]
            if event["method"] == "DOM.childNodeRemoved"
        ]
        assert_equal(
            len(move_removal_events),
            1,
            "moved-node removal event must not be projected twice",
        )
        await owner.send(
            "DOMDebugger.removeDOMBreakpoint",
            {"nodeId": move_new_node_id, "type": "subtree-modified"},
        )

        await peer.send(
            "Runtime.evaluate",
            {
                "expression": "document.querySelector('#root').setAttribute('data-v', 'same')",
            },
        )
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": root_node_id, "type": "attribute-modified"},
        )
        attribute_owner_start = sum(
            event["method"] == "Debugger.paused" for event in owner_events
        )
        attribute_peer_start = len(peer_events)
        pending_task = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        document.querySelector('#root').setAttribute('data-v', 'same');
                        true
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused" for event in owner_events
            )
            == attribute_owner_start + 1
            and len(peer_events) == attribute_peer_start + 1,
            "DOM attribute-modified pause for an unchanged value",
        )
        attribute_pause = next(
            event["params"]
            for event in reversed(owner_events)
            if event["method"] == "Debugger.paused"
        )
        assert_equal(
            attribute_pause.get("data"),
            {"nodeId": root_node_id, "type": "attribute-modified"},
            "DOM attribute breakpoint data",
        )
        await owner.send("Debugger.resume")
        attribute_result = await asyncio.wait_for(pending_task, timeout=5)
        pending_task = None
        assert_equal(
            attribute_result.get("result", {}).get("value"),
            True,
            "resumed DOM attribute mutation result",
        )
        await owner.send(
            "DOMDebugger.removeDOMBreakpoint",
            {"nodeId": root_node_id, "type": "attribute-modified"},
        )

        middle_node_id = await query_node("#middle")
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": root_node_id, "type": "subtree-modified"},
        )
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": middle_node_id, "type": "node-removed"},
        )
        removal_owner_start = sum(
            event["method"] == "Debugger.paused" for event in owner_events
        )
        removal_peer_start = len(peer_events)
        pending_task = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        globalThis.__removedMiddle = document.querySelector('#middle');
                        __removedMiddle.remove();
                        true
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused" for event in owner_events
            )
            == removal_owner_start + 1
            and len(peer_events) == removal_peer_start + 1,
            "DOM node-removed direct breakpoint pause",
        )
        removal_pause = next(
            event["params"]
            for event in reversed(owner_events)
            if event["method"] == "Debugger.paused"
        )
        assert_equal(
            removal_pause.get("data"),
            {"nodeId": middle_node_id, "type": "node-removed"},
            "DOM node-removed direct breakpoint data",
        )
        await peer.send("Debugger.resume")
        removal_result = await asyncio.wait_for(pending_task, timeout=5)
        pending_task = None
        assert_equal(
            removal_result.get("result", {}).get("value"),
            True,
            "resumed DOM node removal result",
        )
        await owner.send(
            "DOMDebugger.removeDOMBreakpoint",
            {"nodeId": root_node_id, "type": "subtree-modified"},
        )
        stale_breakpoint_result = await asyncio.wait_for(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        document.querySelector('#root').appendChild(__removedMiddle);
                        __removedMiddle.remove();
                        true
                    """,
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            stale_breakpoint_result.get("result", {}).get("value"),
            True,
            "detached subtree must not retain a node-removed breakpoint",
        )

        await peer.send(
            "Runtime.evaluate",
            {
                "expression": """
                    document.querySelector('#root').appendChild(
                        document.createTextNode('old')
                    );
                    true
                """,
                "returnByValue": True,
            },
        )
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": root_node_id, "type": "subtree-modified"},
        )
        character_owner_start = sum(
            event["method"] == "Debugger.paused" for event in owner_events
        )
        character_peer_start = len(peer_events)
        pending_task = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        document.querySelector('#root').lastChild.data = 'new';
                        true
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused" for event in owner_events
            )
            == character_owner_start + 1
            and len(peer_events) == character_peer_start + 1,
            "DOM character-data subtree breakpoint pause",
        )
        character_pause = next(
            event["params"]
            for event in reversed(owner_events)
            if event["method"] == "Debugger.paused"
        )
        assert_equal(
            character_pause.get("data", {}).get("insertion"),
            False,
            "DOM character-data subtree marker",
        )
        pause_runtime_probe = await asyncio.wait_for(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": "21 * 2",
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            pause_runtime_probe.get("result", {}).get("value"),
            42,
            "paused Runtime.evaluate liveness",
        )
        pause_object_probe = await asyncio.wait_for(
            owner.send(
                "Runtime.evaluate",
                {"expression": "({ answer: 42 })"},
            ),
            timeout=5,
        )
        pause_remote_object = pause_object_probe.get("result", {})
        assert_equal(
            pause_remote_object.get("type"),
            "object",
            "paused object-valued Runtime.evaluate type",
        )
        if not pause_remote_object.get("objectId"):
            raise SmokeError(
                "paused object-valued Runtime.evaluate must return an objectId: "
                f"{pause_object_probe}"
            )
        character_state = await asyncio.wait_for(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": "document.querySelector('#root').lastChild.data",
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            character_state.get("result", {}).get("value"),
            "new",
            "character-data breakpoint observes the committed value",
        )
        pause_promise_task = asyncio.create_task(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": "Promise.resolve(43)",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            )
        )
        await peer.send("Debugger.resume")
        pause_promise_probe = await asyncio.wait_for(pause_promise_task, timeout=5)
        pause_promise_task = None
        assert_equal(
            pause_promise_probe.get("result", {}).get("value"),
            43,
            "paused awaitPromise Runtime.evaluate completes after resume",
        )
        character_result = await asyncio.wait_for(pending_task, timeout=5)
        pending_task = None
        assert_equal(
            character_result.get("result", {}).get("value"),
            True,
            "resumed DOM character-data result",
        )
        await owner.send(
            "DOMDebugger.removeDOMBreakpoint",
            {"nodeId": root_node_id, "type": "subtree-modified"},
        )

        await peer.send(
            "Runtime.evaluate",
            {
                "expression": """
                    document.querySelector('#root').replaceChildren(
                        document.createTextNode('a'),
                        document.createTextNode('b')
                    );
                    true
                """,
                "returnByValue": True,
            },
        )
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": root_node_id, "type": "subtree-modified"},
        )
        normalize_owner_start = sum(
            event["method"] == "Debugger.paused" for event in owner_events
        )
        normalize_peer_start = len(peer_events)
        pending_task = asyncio.create_task(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        document.querySelector('#root').normalize();
                        true
                    """,
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused" for event in owner_events
            )
            == normalize_owner_start + 1
            and len(peer_events) == normalize_peer_start + 1,
            "first DOM normalize breakpoint pause",
        )
        normalize_intermediate = await asyncio.wait_for(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        (() => {
                            const root = document.querySelector('#root');
                            return {
                                count: root.childNodes.length,
                                first: root.firstChild.data,
                                second: root.lastChild.data,
                                text: root.textContent,
                            };
                        })()
                    """,
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            normalize_intermediate.get("result", {}).get("value"),
            {"count": 2, "first": "ab", "second": "b", "text": "abb"},
            "normalize character-data pause state",
        )
        await peer.send("Debugger.resume")
        await wait_until(
            lambda: sum(
                event["method"] == "Debugger.paused" for event in owner_events
            )
            == normalize_owner_start + 2
            and len(peer_events) == normalize_peer_start + 2,
            "second DOM normalize breakpoint pause",
        )
        normalize_removal_pause = next(
            event["params"]
            for event in reversed(owner_events)
            if event["method"] == "Debugger.paused"
        )
        assert_equal(
            normalize_removal_pause.get("data", {}).get("insertion"),
            False,
            "normalize sibling-removal subtree marker",
        )
        await owner.send("Debugger.resume")
        normalize_result = await asyncio.wait_for(pending_task, timeout=5)
        pending_task = None
        assert_equal(
            normalize_result.get("result", {}).get("value"),
            True,
            "resumed DOM normalize result",
        )
        normalized_state = await peer.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        const root = document.querySelector('#root');
                        return {count: root.childNodes.length, text: root.textContent};
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            normalized_state.get("result", {}).get("value"),
            {"count": 1, "text": "ab"},
            "completed DOM normalize state",
        )
        await owner.send(
            "DOMDebugger.removeDOMBreakpoint",
            {"nodeId": root_node_id, "type": "subtree-modified"},
        )

        current_root_node_id = await query_node("#root")
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": current_root_node_id, "type": "attribute-modified"},
        )
        await owner.send("DOM.disable")
        document_node_id = None
        disabled_result = await asyncio.wait_for(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        document.querySelector('#root').setAttribute('data-disabled', '1');
                        true
                    """,
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            disabled_result.get("result", {}).get("value"),
            True,
            "DOM.disable clears DOM mutation breakpoints",
        )
        expected_peer_pause_count = normalize_peer_start + 2
        if len(peer_events) != expected_peer_pause_count:
            raise SmokeError(
                f"DOM.disable must suppress future DOM breakpoint pauses: {peer_events}"
            )

        await owner.send("DOM.enable")
        navigation_root_node_id = await query_node("#root")
        await owner.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": navigation_root_node_id, "type": "attribute-modified"},
        )
        await page.goto(
            f"{state.fixture}/plain?dom-debugger-dom-breakpoint-after-navigation"
        )

        navigation_result = await asyncio.wait_for(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        document.body.setAttribute('data-after-navigation', '1');
                        true
                    """,
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            navigation_result.get("result", {}).get("value"),
            True,
            "navigation clears document-local DOM mutation breakpoints",
        )
        if len(peer_events) != expected_peer_pause_count:
            raise SmokeError(
                "navigation must not restore DOM mutation breakpoints: "
                f"{peer_events}"
            )

        state.record(
            "chromium_dom_debugger_dom_breakpoint_sample",
            {
                "ownerPauseData": [
                    event["params"].get("data")
                    for event in owner_events
                    if event["method"] == "Debugger.paused"
                ],
                "peerPauseReasons": [
                    event["params"].get("reason") for event in peer_events
                ],
            },
        )
    finally:
        # A failed assertion can leave the mutation command inside V8's nested
        # pause loop. Resume before detaching so smoke cleanup cannot mask the
        # original failure by waiting forever on the blocked Page owner.
        for session in (owner, peer):
            if session is None:
                continue
            try:
                await asyncio.wait_for(session.send("Debugger.resume"), timeout=1)
            except Exception:
                pass
        if pending_task is not None:
            pending_task.cancel()
            await asyncio.gather(pending_task, return_exceptions=True)
        if pause_promise_task is not None:
            pause_promise_task.cancel()
            await asyncio.gather(pause_promise_task, return_exceptions=True)
        for session in (owner, peer):
            if session is None:
                continue
            try:
                await asyncio.wait_for(session.detach(), timeout=2)
            except Exception:
                pass
        try:
            await asyncio.wait_for(page.close(), timeout=2)
        except Exception:
            pass

async def _verify_chromium_dom_debugger_parser_mutation_no_pause_sample(
    state: SmokeState,
) -> None:
    page = await state.context.new_page()
    session = None
    set_content_task: asyncio.Task[Any] | None = None
    try:
        await page.goto(f"{state.fixture}/plain?dom-debugger-parser-mutation")

        session = await new_cdp_session(state.context, page)
        pauses = attach_cdp_event_collector(session, ["Debugger.paused"])
        await session.send("DOM.enable")
        await session.send("Debugger.enable")
        document = await session.send("DOM.getDocument", {"depth": 1})
        document_node_id = document.get("root", {}).get("nodeId")
        if not isinstance(document_node_id, int) or document_node_id <= 0:
            raise SmokeError(f"DOM.getDocument missing parser root: {document}")
        await session.send(
            "DOMDebugger.setDOMBreakpoint",
            {"nodeId": document_node_id, "type": "subtree-modified"},
        )
        frame_tree = await session.send("Page.getFrameTree")
        frame_id = frame_tree.get("frameTree", {}).get("frame", {}).get("id")
        if not frame_id:
            raise SmokeError(f"Page.getFrameTree missing parser frame: {frame_tree}")

        set_content_task = asyncio.create_task(
            session.send(
                "Page.setDocumentContent",
                {
                    "frameId": frame_id,
                    "html": "<html><body><main id='parser-new'>new</main></body></html>",
                },
            )
        )
        await wait_until(
            lambda: set_content_task.done() or bool(pauses),
            "parser mutation completion without a DOM breakpoint pause",
        )
        if pauses:
            await session.send("Debugger.resume")
            raise SmokeError(
                "Blink parser insertion/removal paths must not trigger DOM breakpoints: "
                f"{pauses}"
            )
        await asyncio.wait_for(set_content_task, timeout=5)
        set_content_task = None
        state.record("chromium_dom_debugger_parser_mutation_no_pause_sample")
    finally:
        if set_content_task is not None:
            set_content_task.cancel()
            await asyncio.gather(set_content_task, return_exceptions=True)
        if session is not None:
            await asyncio.wait_for(session.detach(), timeout=2)
        await asyncio.wait_for(page.close(), timeout=2)

async def _verify_chromium_dom_debugger_xhr_breakpoint_sample(
    state: SmokeState,
) -> None:
    page = await state.context.new_page()
    owner = None
    peer = None
    second_owner = None
    pending_tasks: list[asyncio.Task[Any]] = []
    try:
        await page.goto(f"{state.fixture}/plain?dom-debugger-xhr-breakpoint")

        owner = await new_cdp_session(state.context, page)
        peer = await new_cdp_session(state.context, page)
        owner_events = attach_cdp_event_collector(owner, ["Debugger.paused"])
        peer_events = attach_cdp_event_collector(peer, ["Debugger.paused"])

        missing_url = await _send_cdp_expect_optional_error(
            owner,
            "DOMDebugger.setXHRBreakpoint",
            {},
        )
        if not missing_url or "Invalid parameters" not in str(missing_url):
            raise SmokeError(
                "DOMDebugger.setXHRBreakpoint should require url: "
                f"{missing_url}"
            )

        await peer.send("Debugger.enable")
        for pattern in ("xhr-breakpoint-specific", "xhr-breakpoint"):
            await owner.send("DOMDebugger.setXHRBreakpoint", {"url": pattern})

        disabled_url = f"{state.fixture}/plain?xhr-breakpoint=disabled"
        disabled_owner = await asyncio.wait_for(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": f"fetch({disabled_url!r}).then(response => response.status)",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            disabled_owner.get("result", {}).get("value"),
            200,
            "a peer Debugger must not activate a disabled owner's XHR breakpoint",
        )
        if owner_events or peer_events:
            raise SmokeError(
                "a Debugger-disabled XHR breakpoint owner must not pause: "
                f"owner={owner_events}, peer={peer_events}"
            )

        await owner.send("Debugger.enable")
        fetch_url = f"{state.fixture}/plain?xhr-breakpoint=xhr-breakpoint-specific"
        fetch_task = asyncio.create_task(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": f"fetch({fetch_url!r}).then(response => response.status)",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            )
        )
        pending_tasks.append(fetch_task)
        await wait_until(
            lambda: len(owner_events) == 1 and len(peer_events) == 1,
            "DOMDebugger fetch breakpoint pause in both sessions",
        )
        assert_equal(
            owner_events[0]["params"].get("reason"),
            "XHR",
            "DOMDebugger fetch breakpoint owner reason",
        )
        assert_equal(
            owner_events[0]["params"].get("data"),
            {"breakpointURL": "xhr-breakpoint", "url": fetch_url},
            "DOMDebugger fetch breakpoint owner data",
        )
        assert_equal(
            peer_events[0]["params"].get("reason"),
            "other",
            "DOMDebugger fetch breakpoint peer reason",
        )
        if "data" in peer_events[0]["params"]:
            raise SmokeError(
                "a non-owner Debugger session must omit XHR pause data: "
                f"{peer_events[0]}"
            )
        await peer.send("Debugger.resume")
        fetch_result = await asyncio.wait_for(fetch_task, timeout=5)
        pending_tasks.remove(fetch_task)
        assert_equal(
            fetch_result.get("result", {}).get("value"),
            200,
            "resumed DOMDebugger fetch result",
        )

        xhr_url = f"{state.fixture}/plain?xhr-breakpoint=xhr"
        xhr_task = asyncio.create_task(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": f"""
                        new Promise((resolve, reject) => {{
                            const xhr = new XMLHttpRequest();
                            xhr.onload = () => resolve(xhr.status);
                            xhr.onerror = () => reject(new Error('XHR failed'));
                            xhr.open('GET', {xhr_url!r});
                            xhr.send();
                        }})
                    """,
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            )
        )
        pending_tasks.append(xhr_task)
        await wait_until(
            lambda: len(owner_events) == 2 and len(peer_events) == 2,
            "DOMDebugger XMLHttpRequest breakpoint pause in both sessions",
        )
        assert_equal(
            owner_events[1]["params"].get("data"),
            {"breakpointURL": "xhr-breakpoint", "url": xhr_url},
            "DOMDebugger XMLHttpRequest breakpoint owner data",
        )
        assert_equal(
            peer_events[1]["params"].get("reason"),
            "other",
            "DOMDebugger XMLHttpRequest breakpoint peer reason",
        )
        await owner.send("Debugger.resume")
        xhr_result = await asyncio.wait_for(xhr_task, timeout=5)
        pending_tasks.remove(xhr_task)
        assert_equal(
            xhr_result.get("result", {}).get("value"),
            200,
            "resumed DOMDebugger XMLHttpRequest result",
        )

        for pattern in ("xhr-breakpoint", "xhr-breakpoint-specific"):
            await owner.send("DOMDebugger.removeXHRBreakpoint", {"url": pattern})
            await owner.send("DOMDebugger.removeXHRBreakpoint", {"url": pattern})
        removed = await asyncio.wait_for(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": f"fetch({fetch_url!r}).then(response => response.status)",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            removed.get("result", {}).get("value"),
            200,
            "removing an XHR breakpoint is idempotent and suppresses future pauses",
        )
        if len(owner_events) != 2 or len(peer_events) != 2:
            raise SmokeError(
                "removed DOMDebugger XHR breakpoints must not pause: "
                f"owner={owner_events}, peer={peer_events}"
            )

        await owner.send("DOMDebugger.setXHRBreakpoint", {"url": ""})
        invalid_state_task = asyncio.create_task(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        (() => {
                            const xhr = new XMLHttpRequest();
                            try { xhr.send(); } catch (error) { return error.name; }
                        })()
                    """,
                    "returnByValue": True,
                },
            )
        )
        pending_tasks.append(invalid_state_task)
        await wait_until(
            lambda: len(owner_events) == 3 and len(peer_events) == 3,
            "DOMDebugger match-all invalid-state XHR pause",
        )
        assert_equal(
            owner_events[2]["params"].get("data"),
            {"breakpointURL": "", "url": ""},
            "DOMDebugger match-all pauses before XMLHttpRequest state validation",
        )
        await peer.send("Debugger.resume")
        invalid_state = await asyncio.wait_for(invalid_state_task, timeout=5)
        pending_tasks.remove(invalid_state_task)
        assert_equal(
            invalid_state.get("result", {}).get("value"),
            "InvalidStateError",
            "resumed invalid-state XMLHttpRequest result",
        )

        await page.goto(f"{state.fixture}/semantic-frames?dom-debugger-xhr-navigation")

        child = next(
            (
                frame
                for frame in page.frames
                if frame != page.main_frame and "/semantic-frame-child" in frame.url
            ),
            None,
        )
        if child is None:
            raise SmokeError("DOMDebugger XHR smoke should load a child frame")
        frame_url = f"{state.fixture}/plain?frame-xhr-breakpoint"
        frame_task = asyncio.create_task(
            child.evaluate(f"fetch({frame_url!r}).then(response => response.status)")
        )
        pending_tasks.append(frame_task)
        await wait_until(
            lambda: len(owner_events) == 4 and len(peer_events) == 4,
            "restored child-frame DOMDebugger XHR breakpoint pause",
        )
        assert_equal(
            owner_events[3]["params"].get("data"),
            {"breakpointURL": "", "url": frame_url},
            "DOMDebugger XHR breakpoint navigation restore and child-frame scope",
        )
        await owner.send("Debugger.resume")
        assert_equal(
            await asyncio.wait_for(frame_task, timeout=5),
            200,
            "resumed child-frame fetch result",
        )
        pending_tasks.remove(frame_task)

        await owner.send("DOMDebugger.removeXHRBreakpoint", {"url": ""})
        second_owner = await new_cdp_session(state.context, page)
        second_owner_events = attach_cdp_event_collector(
            second_owner,
            ["Debugger.paused"],
        )
        await second_owner.send("Debugger.enable")
        await owner.send("DOMDebugger.setXHRBreakpoint", {"url": "multi-owner"})
        await second_owner.send(
            "DOMDebugger.setXHRBreakpoint",
            {"url": "owner-specific"},
        )
        multi_owner_url = f"{state.fixture}/plain?multi-owner=owner-specific"
        multi_owner_task = asyncio.create_task(
            owner.send(
                "Runtime.evaluate",
                {
                    "expression": f"fetch({multi_owner_url!r}).then(response => response.status)",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            )
        )
        pending_tasks.append(multi_owner_task)
        await wait_until(
            lambda: len(owner_events) == 5
            and len(peer_events) == 5
            and len(second_owner_events) == 1,
            "first multi-owner DOMDebugger XHR breakpoint pause",
        )
        await peer.send("Debugger.resume")
        await wait_until(
            lambda: len(owner_events) == 6
            and len(peer_events) == 6
            and len(second_owner_events) == 2,
            "second multi-owner DOMDebugger XHR breakpoint pause",
        )
        multi_owner_pauses = owner_events[4:6] + second_owner_events
        xhr_owner_pauses = [
            event
            for event in multi_owner_pauses
            if event["params"].get("reason") == "XHR"
        ]
        assert_equal(
            len(xhr_owner_pauses),
            2,
            "one XHR pause owner per matching DOMDebugger session",
        )
        multi_owner_breakpoint_urls = {
            event["params"].get("data", {}).get("breakpointURL")
            for event in xhr_owner_pauses
        }
        assert_equal(
            multi_owner_breakpoint_urls,
            {"multi-owner", "owner-specific"},
            "sequential DOMDebugger XHR pauses preserve each owner's breakpoint data",
        )
        for label, events in (
            ("first owner", owner_events[4:6]),
            ("second owner", second_owner_events),
        ):
            assert_equal(
                sum(event["params"].get("reason") == "XHR" for event in events),
                1,
                f"{label} should own exactly one of two XHR pauses",
            )
        if any(
            event["params"].get("reason") != "other" or "data" in event["params"]
            for event in peer_events[4:6]
        ):
            raise SmokeError(
                "a non-owner peer must receive two data-less multi-owner pauses: "
                f"{peer_events[4:6]}"
            )
        await owner.send("Debugger.resume")
        multi_owner_result = await asyncio.wait_for(multi_owner_task, timeout=5)
        pending_tasks.remove(multi_owner_task)
        assert_equal(
            multi_owner_result.get("result", {}).get("value"),
            200,
            "resumed multi-owner DOMDebugger fetch result",
        )
        await owner.send("DOMDebugger.removeXHRBreakpoint", {"url": "multi-owner"})
        await second_owner.send(
            "DOMDebugger.removeXHRBreakpoint",
            {"url": "owner-specific"},
        )
        await second_owner.detach()
        second_owner = None

        await owner.send("DOMDebugger.setXHRBreakpoint", {"url": "worker-xhr-breakpoint"})
        worker_task = asyncio.create_task(
            page.evaluate(
                f"""
                    new Promise((resolve, reject) => {{
                        const worker = new Worker({f'{state.fixture}/worker.js'!r});
                        worker.onmessage = event => resolve(event.data);
                        worker.onerror = event => reject(new Error(event.message));
                        worker.postMessage({{
                            kind: 'fetch',
                            url: {f'{state.fixture}/plain?worker-xhr-breakpoint'!r}
                        }});
                    }})
                """
            )
        )
        pending_tasks.append(worker_task)
        worker_result = await asyncio.wait_for(worker_task, timeout=5)
        pending_tasks.remove(worker_task)
        assert_equal(
            worker_result.get("status"),
            200,
            "a page-target XHR breakpoint must not instrument a dedicated worker target",
        )
        if len(owner_events) != 6 or len(peer_events) != 6:
            raise SmokeError(
                "a page-target XHR breakpoint must exclude dedicated workers: "
                f"owner={owner_events}, peer={peer_events}"
            )

        await owner.detach()
        owner = None
        detached_url = f"{state.fixture}/plain?worker-xhr-breakpoint=detached"
        detached_owner = await asyncio.wait_for(
            peer.send(
                "Runtime.evaluate",
                {
                    "expression": f"fetch({detached_url!r}).then(response => response.status)",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            ),
            timeout=5,
        )
        assert_equal(
            detached_owner.get("result", {}).get("value"),
            200,
            "detached XHR breakpoint owner cleanup",
        )
        if len(peer_events) != 6:
            raise SmokeError(
                "detaching the XHR breakpoint owner must remove renderer state: "
                f"{peer_events}"
            )

        state.record(
            "chromium_dom_debugger_xhr_breakpoint_sample",
            {
                "ownerPauseReasons": [
                    event["params"].get("reason") for event in owner_events
                ],
                "peerPauseReasons": [
                    event["params"].get("reason") for event in peer_events
                ],
                "multiOwnerBreakpointURLs": sorted(multi_owner_breakpoint_urls),
            },
        )
    finally:
        for task in pending_tasks:
            task.cancel()
        if owner is not None:
            await owner.detach()
        if peer is not None:
            await peer.detach()
        if second_owner is not None:
            await second_owner.detach()
        await page.close()


async def run_dom_group(state: SmokeState) -> None:
    await _verify_chromium_dom_get_attributes_sample(state)
    await _verify_chromium_dom_query_selector_sample(state)
    await _verify_chromium_dom_single_text_child_projection_sample(state)
    await _verify_chromium_dom_debugger_event_listeners_sample(state)
    await _verify_chromium_dom_debugger_event_listener_breakpoint_sample(state)
    await _verify_chromium_dom_debugger_dom_breakpoint_sample(state)
    await _verify_chromium_dom_debugger_parser_mutation_no_pause_sample(state)
    await _verify_chromium_dom_debugger_xhr_breakpoint_sample(state)


async def run_computed_style_samples(state: SmokeState) -> None:
    await _verify_chromium_css_computed_style_breadth_sample(state)
    await _verify_sampled_computed_sizes(state)
