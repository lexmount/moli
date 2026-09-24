from __future__ import annotations

import asyncio
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..raw_cdp import RawCdpClient, connect_raw_cdp, discover_websocket_url


async def _response_for_id(
    client: RawCdpClient,
    message_id: int,
    *,
    timeout: float = 5.0,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out waiting for raw CDP response id={message_id}: {seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == message_id:
            return message, seen


async def _success(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    message_id = await client.send(method, params, session_id=session_id)
    response, seen = await _response_for_id(client, message_id)
    if "error" in response:
        raise SmokeError(f"{method} failed: {response['error']!r}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise SmokeError(f"{method} returned no result object: {response!r}")
    return result, seen


async def _wait_for_load(
    client: RawCdpClient,
    session_id: str,
    seen: list[dict[str, Any]],
) -> None:
    if any(
        message.get("sessionId") == session_id
        and message.get("method") == "Page.loadEventFired"
        for message in seen
    ):
        return
    deadline = asyncio.get_running_loop().time() + 5.0
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError("timed out waiting for raw Page.loadEventFired")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        if (
            message.get("sessionId") == session_id
            and message.get("method") == "Page.loadEventFired"
        ):
            return


def _attribute_map(node: dict[str, Any]) -> dict[str, str]:
    attributes = node.get("attributes") or []
    return {
        str(attributes[index]): str(attributes[index + 1])
        for index in range(0, len(attributes) - 1, 2)
    }


async def run_dom_hit_test_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    websocket_url = await discover_websocket_url(endpoint)
    is_moli = websocket_url.endswith("/devtools/browser/moli-browser")
    client = await connect_raw_cdp(endpoint)
    target_id: str | None = None
    try:
        created, _ = await _success(client, "Target.createTarget", {"url": "about:blank"})
        target_id = created.get("targetId")
        if not isinstance(target_id, str) or not target_id:
            raise SmokeError(f"Target.createTarget returned no targetId: {created!r}")

        attached, _ = await _success(
            client,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        session_id = attached.get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            raise SmokeError(f"Target.attachToTarget returned no sessionId: {attached!r}")

        await _success(client, "Page.enable", session_id=session_id)
        navigation, seen = await _success(
            client,
            "Page.navigate",
            {"url": f"{fixture}/chromium-cdp-hit-test-page"},
            session_id=session_id,
        )
        if navigation.get("errorText"):
            raise SmokeError(f"Page.navigate failed: {navigation!r}")
        await _wait_for_load(client, session_id, seen)
        await _success(client, "Page.captureScreenshot", session_id=session_id)

        command_id = await client.send(
            "DOM.getNodeForLocation",
            {
                "x": 10,
                "y": 10,
                "includeUserAgentShadowDOM": True,
                "ignorePointerEventsNone": True,
            },
            session_id=session_id,
        )
        response, _ = await _response_for_id(client, command_id)
        assert_equal(response.get("sessionId"), session_id, "hit-test response session")

        if "error" in response:
            raise SmokeError(f"DOM.getNodeForLocation failed: {response!r}")
        hit = response.get("result")
        if not isinstance(hit, dict):
            raise SmokeError(f"DOM.getNodeForLocation returned no hit-test result: {response!r}")
        backend_node_id = hit.get("backendNodeId")
        frame_id = hit.get("frameId")
        if not isinstance(backend_node_id, int) or backend_node_id <= 0:
            raise SmokeError(f"DOM.getNodeForLocation returned an invalid backendNodeId: {hit!r}")
        if not isinstance(frame_id, str) or not frame_id:
            raise SmokeError(f"DOM.getNodeForLocation returned an invalid frameId: {hit!r}")

        described, _ = await _success(
            client,
            "DOM.describeNode",
            {"backendNodeId": backend_node_id},
            session_id=session_id,
        )
        node = described.get("node")
        if not isinstance(node, dict):
            raise SmokeError(f"could not describe the hit node: {described!r}")
        hit_node_id = _attribute_map(node).get("id")
        assert_equal(hit_node_id, "hit-overlay", "option-aware hit node")
        record(
            results,
            "dom_get_node_for_location_layout_hit_test",
            {
                "supported": True,
                "engine": "moli" if is_moli else "chromium",
                "backendNodeIdPresent": True,
                "frameIdPresent": True,
                "nodeName": node.get("nodeName"),
                "nodeIdAttribute": hit_node_id,
            },
        )
    finally:
        if target_id is not None:
            try:
                await _success(client, "Target.closeTarget", {"targetId": target_id})
            except Exception:
                pass
        await client.websocket.close()
