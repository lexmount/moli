from __future__ import annotations

import asyncio
import json
import urllib.request
from typing import Any, Callable

from ..assertions import SmokeError, assert_equal, record
from ..navigation import navigate as navigate_document
from ..raw_cdp import RawCdpClient, connect_raw_cdp


def _read_fixture_json(url: str) -> dict[str, Any]:
    request = urllib.request.Request(url, method="POST")
    with urllib.request.urlopen(request, timeout=2) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if not isinstance(payload, dict):
        raise SmokeError(f"fixture control returned a non-object: {payload!r}")
    return payload


async def _fixture_control(fixture: str, action: str) -> dict[str, Any]:
    return await asyncio.to_thread(
        _read_fixture_json,
        f"{fixture}/parser-dom-mutation-gate/{action}",
    )


async def _wait_for_fixture_request(fixture: str) -> None:
    deadline = asyncio.get_running_loop().time() + 5.0
    while True:
        status = await _fixture_control(fixture, "status")
        if status.get("requestSeen") is True:
            return
        if asyncio.get_running_loop().time() >= deadline:
            raise SmokeError(
                f"parser-blocking script did not reach the fixture gate: {status!r}"
            )
        # smoke-timing: poll the actual fixture request within the five-second deadline.
        await asyncio.sleep(0.01)


async def _receive_until(
    client: RawCdpClient,
    messages: list[dict[str, Any]],
    predicate: Callable[[dict[str, Any]], bool],
    label: str,
    *,
    timeout: float = 5.0,
) -> dict[str, Any]:
    existing = next((message for message in messages if predicate(message)), None)
    if existing is not None:
        return existing
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for {label}: {messages[-30:]!r}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        messages.append(message)
        if predicate(message):
            return message


async def _response(
    client: RawCdpClient,
    messages: list[dict[str, Any]],
    message_id: int,
    label: str,
) -> dict[str, Any]:
    return await _receive_until(
        client,
        messages,
        lambda message: message.get("id") == message_id,
        label,
    )


async def _success(
    client: RawCdpClient,
    messages: list[dict[str, Any]],
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
) -> dict[str, Any]:
    message_id = await client.send(method, params, session_id=session_id)
    response = await _response(client, messages, message_id, f"{method} response")
    if "error" in response:
        raise SmokeError(f"{method} failed: {response['error']!r}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise SmokeError(f"{method} returned no result object: {response!r}")
    return result


def _find_node(
    node: dict[str, Any],
    predicate: Callable[[dict[str, Any]], bool],
) -> dict[str, Any] | None:
    if predicate(node):
        return node
    for child in node.get("children") or []:
        found = _find_node(child, predicate)
        if found is not None:
            return found
    return None


def _attribute_map(node: dict[str, Any]) -> dict[str, str]:
    attributes = node.get("attributes") or []
    return {
        str(attributes[index]): str(attributes[index + 1])
        for index in range(0, len(attributes) - 1, 2)
    }


def _session_event(
    message: dict[str, Any],
    session_id: str,
    method: str,
) -> bool:
    return message.get("sessionId") == session_id and message.get("method") == method


async def run_dom_parser_mutations_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _fixture_control(fixture, "reset")
    client = await connect_raw_cdp(endpoint)
    target_id: str | None = None
    messages: list[dict[str, Any]] = []
    try:
        created = await _success(
            client,
            messages,
            "Target.createTarget",
            {"url": "about:blank"},
        )
        target_id = created.get("targetId")
        if not isinstance(target_id, str) or not target_id:
            raise SmokeError(f"Target.createTarget returned no targetId: {created!r}")

        attached = await _success(
            client,
            messages,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        session_id = attached.get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            raise SmokeError(f"Target.attachToTarget returned no sessionId: {attached!r}")

        await _success(client, messages, "Page.enable", session_id=session_id)
        await _success(client, messages, "DOM.enable", session_id=session_id)
        await navigate_document(client, session_id, {"url": f"{fixture}/plain?dom-parser-baseline"},
                                on_message=messages.append)
        messages.clear()
        await _fixture_control(fixture, "reset")

        navigation = await _success(
            client,
            messages,
            "Page.navigate",
            {"url": f"{fixture}/parser-dom-mutation-page"},
            session_id=session_id,
        )
        if navigation.get("errorText"):
            raise SmokeError(f"Page.navigate failed: {navigation!r}")
        await _receive_until(
            client,
            messages,
            lambda message: _session_event(
                message, session_id, "Page.frameNavigated"
            ),
            "held parser Page.frameNavigated",
        )
        await _wait_for_fixture_request(fixture)

        early_document = await _success(
            client,
            messages,
            "DOM.getDocument",
            {"depth": -1},
            session_id=session_id,
        )
        early_root = early_document.get("root")
        if not isinstance(early_root, dict):
            raise SmokeError(f"early DOM.getDocument returned no root: {early_document!r}")
        if _find_node(early_root, lambda node: node.get("localName") == "body"):
            raise SmokeError(f"held parser unexpectedly exposed BODY: {early_root!r}")
        early_root_node_id = early_root.get("nodeId")
        if not isinstance(early_root_node_id, int) or early_root_node_id <= 0:
            raise SmokeError(f"early document returned invalid nodeId: {early_root!r}")
        commit_document_updated_count = sum(
            _session_event(message, session_id, "DOM.documentUpdated")
            for message in messages
        )
        assert_equal(
            commit_document_updated_count,
            1,
            "pre-parser commit DOM.documentUpdated count",
        )
        if any(
            _session_event(message, session_id, "Page.domContentEventFired")
            for message in messages
        ):
            raise SmokeError(
                f"DOMContentLoaded crossed the parser-blocking script gate: {messages!r}"
            )

        await _fixture_control(fixture, "release")
        await _receive_until(
            client,
            messages,
            lambda _message: sum(
                _session_event(candidate, session_id, "DOM.documentUpdated")
                for candidate in messages
            )
            == 2
            and any(
                _session_event(candidate, session_id, "Page.domContentEventFired")
                for candidate in messages
            ),
            "parser-tail DOM refresh and DOMContentLoaded",
        )

        body_inserted_index = next(
            (
                index
                for index, message in enumerate(messages)
                if _session_event(message, session_id, "DOM.childNodeInserted")
                and message.get("params", {}).get("node", {}).get("localName")
                == "body"
            ),
            None,
        )
        if body_inserted_index is None:
            raise SmokeError(f"missing parser-tail BODY insertion: {messages!r}")
        document_updated_indices = [
            index
            for index, message in enumerate(messages)
            if _session_event(message, session_id, "DOM.documentUpdated")
        ]
        assert_equal(
            len(document_updated_indices),
            2,
            "commit and DCL DOM.documentUpdated barriers",
        )
        dcl_index = next(
            index
            for index, message in enumerate(messages)
            if _session_event(message, session_id, "Page.domContentEventFired")
        )
        commit_document_updated_index, dcl_document_updated_index = (
            document_updated_indices
        )
        if not (
            commit_document_updated_index
            < body_inserted_index
            < dcl_document_updated_index
            < dcl_index
        ):
            raise SmokeError(
                "expected commit documentUpdated -> BODY insertion -> DCL documentUpdated "
                "-> DOMContentLoaded, got "
                f"{[message.get('method') for message in messages]!r}"
            )

        stale_id = await client.send(
            "DOM.describeNode",
            {"nodeId": early_root_node_id},
            session_id=session_id,
        )
        stale_response = await _response(
            client, messages, stale_id, "stale DOM.describeNode response"
        )
        assert_equal(
            stale_response.get("error", {}).get("code"),
            -32000,
            "pre-DCL frontend node id invalidation",
        )

        refreshed_document = await _success(
            client,
            messages,
            "DOM.getDocument",
            {"depth": -1},
            session_id=session_id,
        )
        refreshed_root = refreshed_document.get("root")
        if not isinstance(refreshed_root, dict):
            raise SmokeError(
                f"refreshed DOM.getDocument returned no root: {refreshed_document!r}"
            )
        body = _find_node(
            refreshed_root,
            lambda node: node.get("localName") == "body",
        )
        if body is None or _attribute_map(body).get("id") != "late-body":
            raise SmokeError(f"refreshed DOM snapshot is missing #late-body: {refreshed_root!r}")
        if _find_node(body, lambda node: node.get("localName") == "main") is None:
            raise SmokeError(f"refreshed BODY is missing its parser tail: {body!r}")

        event_sequence = [
            message["method"]
            for message in messages
            if message.get("sessionId") == session_id
            and message.get("method")
            in {
                "Page.frameNavigated",
                "DOM.childNodeInserted",
                "DOM.documentUpdated",
                "Page.domContentEventFired",
            }
        ]
        record(
            results,
            "dom_parser_tail_mutation_binding_refresh",
            {
                "earlyBodyPresent": False,
                "eventSequence": event_sequence,
                "documentUpdatedCount": 2,
                "staleNodeErrorCode": -32000,
                "refreshedBodyId": "late-body",
            },
        )
    finally:
        try:
            await _fixture_control(fixture, "release")
        except Exception:
            pass
        if target_id is not None:
            try:
                await _success(
                    client,
                    messages,
                    "Target.closeTarget",
                    {"targetId": target_id},
                )
            except Exception:
                pass
        await client.websocket.close()
