from __future__ import annotations
import asyncio
from typing import Any
from urllib.parse import urlsplit
from . import SmokeState
from ..assertions import SmokeError


_NAVIGATION_TIMEOUT_SECONDS = 10.0


def _alternate_loopback_origin(url: str) -> str:
    parsed = urlsplit(url)
    if parsed.hostname not in {"127.0.0.1", "localhost"} or parsed.port is None:
        raise SmokeError(f"expected loopback fixture origin, got {url}")
    hostname = "localhost" if parsed.hostname == "127.0.0.1" else "127.0.0.1"
    return f"{parsed.scheme}://{hostname}:{parsed.port}"

async def _performance_metrics(cdp: Any) -> dict[str, float]:
    result = await cdp.send("Performance.getMetrics")
    metrics = result.get("metrics") or []
    return {metric.get("name"): metric.get("value") for metric in metrics if metric.get("name")}

async def _navigate_with_cdp_until_dom_ready(state: SmokeState, url: str) -> None:
    ready: set[tuple[str, Any, Any]] = set()
    changed = asyncio.Event()

    def lifecycle(params: dict[str, Any]) -> None:
        if params.get("name") == "DOMContentLoaded":
            ready.add(("lifecycle", params.get("frameId"), params.get("loaderId")))
            changed.set()

    def within_document(params: dict[str, Any]) -> None:
        ready.add(("within-document", params.get("frameId"), params.get("url")))
        changed.set()

    state.cdp.on("Page.lifecycleEvent", lifecycle)
    state.cdp.on("Page.navigatedWithinDocument", within_document)
    phase = "enabling Page lifecycle events"
    try:
        async with asyncio.timeout(_NAVIGATION_TIMEOUT_SECONDS):
            await state.cdp.send("Page.enable")
            await state.cdp.send("Page.setLifecycleEventsEnabled", {"enabled": True})
            phase = "Page.navigate"
            result = await state.cdp.send("Page.navigate", {"url": url})
            if result.get("errorText"):
                raise SmokeError(f"Page.navigate failed for {url}: {result}")
            frame, loader = result["frameId"], result.get("loaderId")
            phase = f"DOM ready for frame={frame}, loader={loader}"

            # A lifecycle event can precede the navigate reply. Keep those
            # events, but reject old loaders and child-frame notifications.
            expected = (("lifecycle", frame, loader) if loader is not None
                        else ("within-document", frame, url))
            while expected not in ready:
                changed.clear()
                await changed.wait()
            phase = "Runtime.evaluate after DOM ready"
            snapshot = await state.cdp.send("Runtime.evaluate", {
                "expression": "({url: location.href, readyState: document.readyState})",
                "returnByValue": True,
            })
            value = snapshot.get("result", {}).get("value")
            if (not isinstance(value, dict) or value.get("url") != url
                    or value.get("readyState") not in ("interactive", "complete")):
                raise SmokeError(f"Unexpected document after CDP navigation to {url}: {snapshot}")
    except TimeoutError as error:
        raise SmokeError(f"CDP navigation timed out during {phase}: {url}") from error
    finally:
        state.cdp.remove_listener("Page.lifecycleEvent", lifecycle)
        state.cdp.remove_listener("Page.navigatedWithinDocument", within_document)

async def _send_cdp_expect_optional_error(cdp: Any, method: str, params: dict[str, Any]) -> dict[str, Any] | None:
    try:
        await cdp.send(method, params)
    except Exception as error:
        return {"message": str(error)}
    return None

def _has_event(events: list[dict[str, Any]], method: str) -> bool:
    return any(event["method"] == method for event in events)

def _events_with_method(events: list[dict[str, Any]], method: str) -> list[dict[str, Any]]:
    return [event for event in events if event["method"] == method]

def _frame_tree_ids(frame_tree: dict[str, Any]) -> set[str]:
    frame_ids: set[str] = set()
    frame_id = frame_tree.get("frame", {}).get("id")
    if isinstance(frame_id, str) and frame_id:
        frame_ids.add(frame_id)
    for child in frame_tree.get("childFrames") or []:
        if isinstance(child, dict):
            frame_ids.update(_frame_tree_ids(child))
    return frame_ids

def _assert_script_coverage_array(result: dict[str, Any], label: str) -> None:
    scripts = result.get("result")
    if not isinstance(scripts, list):
        raise SmokeError(f"{label} should return script coverage array: {result}")
    for script in scripts:
        if "scriptId" not in script or "functions" not in script:
            raise SmokeError(f"{label} script coverage entry missing fields: {script}")

def _find_script_coverage_by_url(result: dict[str, Any], url_suffix: str) -> dict[str, Any] | None:
    scripts = result.get("result")
    if not isinstance(scripts, list):
        return None
    for script in scripts:
        if isinstance(script, dict) and str(script.get("url") or "").endswith(url_suffix):
            return script
    return None

def _find_coverage_function(script: dict[str, Any], function_name: str) -> dict[str, Any] | None:
    for function in script.get("functions") or []:
        if isinstance(function, dict) and function.get("functionName") == function_name:
            return function
    return None

def _assert_coverage_contains_function(
    result: dict[str, Any],
    url_suffix: str,
    function_name: str,
    label: str,
) -> None:
    script = _find_script_coverage_by_url(result, url_suffix)
    if not script:
        raise SmokeError(f"{label} should include sourceURL script {url_suffix}: {result}")
    function = _find_coverage_function(script, function_name)
    if not function:
        raise SmokeError(f"{label} should include target function {function_name}: {script}")
    if not function.get("ranges"):
        raise SmokeError(f"{label} should include function ranges: {function}")

def _coverage_function_total_count(function: dict[str, Any]) -> int:
    total = 0
    for range_ in function.get("ranges") or []:
        count = range_.get("count") if isinstance(range_, dict) else None
        if isinstance(count, int):
            total += count
    return total

def _profile_function_names(profile: dict[str, Any]) -> set[str]:
    names: set[str] = set()
    for node in profile.get("nodes") or []:
        call_frame = node.get("callFrame") if isinstance(node, dict) else None
        function_name = call_frame.get("functionName") if isinstance(call_frame, dict) else None
        if isinstance(function_name, str) and function_name:
            names.add(function_name)
    return names

def _assert_profile_tree_shape(profile: dict[str, Any], label: str) -> None:
    nodes = profile.get("nodes")
    if not isinstance(nodes, list) or not nodes:
        raise SmokeError(f"{label} should include non-empty nodes: {profile}")
    root_id = nodes[0].get("id") if isinstance(nodes[0], dict) else None
    if not isinstance(root_id, int):
        raise SmokeError(f"{label} first node should be the root node with an integer id: {profile}")

    children_by_id: dict[int, list[int]] = {}
    known_ids: set[int] = set()
    for node in nodes:
        if not isinstance(node, dict):
            raise SmokeError(f"{label} node should be an object: {node}")
        node_id = node.get("id")
        if not isinstance(node_id, int):
            raise SmokeError(f"{label} node should include an integer id: {node}")
        if node_id in known_ids:
            raise SmokeError(f"{label} node ids should be unique: {profile}")
        known_ids.add(node_id)
        raw_children = node.get("children", [])
        if raw_children is None:
            raw_children = []
        if not isinstance(raw_children, list):
            raise SmokeError(f"{label} children should be an array when present: {node}")
        children: list[int] = []
        for child in raw_children:
            if not isinstance(child, int):
                raise SmokeError(f"{label} child ids should be integers: {node}")
            children.append(child)
        children_by_id[node_id] = children

    for node_id, children in children_by_id.items():
        for child in children:
            if child not in known_ids:
                raise SmokeError(f"{label} child id {child} is missing from nodes for parent {node_id}: {profile}")

    reachable: set[int] = set()
    stack = [root_id]
    while stack:
        node_id = stack.pop()
        if node_id in reachable:
            continue
        reachable.add(node_id)
        stack.extend(children_by_id.get(node_id, []))
    if reachable != known_ids:
        unreachable = sorted(known_ids - reachable)
        raise SmokeError(f"{label} all nodes should be reachable from the first root node; unreachable={unreachable}")

    samples = profile.get("samples")
    if isinstance(samples, list):
        for sample in samples:
            if not isinstance(sample, int):
                raise SmokeError(f"{label} samples should be integer node ids: {profile}")
            if sample not in reachable:
                raise SmokeError(f"{label} sample id {sample} should be reachable from the root: {profile}")
        time_deltas = profile.get("timeDeltas")
        if isinstance(time_deltas, list) and len(time_deltas) != len(samples):
            raise SmokeError(f"{label} timeDeltas length should match samples length: {profile}")

def _find_dom_node(node: dict[str, Any], predicate: Any) -> dict[str, Any] | None:
    if predicate(node):
        return node
    for child in node.get("children") or []:
        found = _find_dom_node(child, predicate)
        if found:
            return found
    return None

def _attribute_list_to_dict(attributes: list[Any]) -> dict[str, str]:
    result: dict[str, str] = {}
    for index in range(0, len(attributes), 2):
        if index + 1 < len(attributes):
            result[str(attributes[index])] = str(attributes[index + 1])
    return result
