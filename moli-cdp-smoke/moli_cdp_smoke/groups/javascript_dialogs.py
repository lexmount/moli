from __future__ import annotations

import asyncio
import json
from collections.abc import Callable
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..raw_cdp import RawCdpClient


async def _receive_until(
    client: RawCdpClient,
    seen: list[dict[str, Any]],
    ready: Callable[[], bool],
    label: str,
) -> None:
    try:
        async with asyncio.timeout(5):
            while not ready():
                seen.append(await client.recv())
    except TimeoutError as error:
        raise SmokeError(f"{label}: timed out; seen={seen[-20:]!r}") from error


async def javascript_dialog_subscriptions(
    client: RawCdpClient,
    target_id: str,
    opening_session: str,
    results: list[dict[str, Any]],
) -> None:
    attach = await client.send("Target.attachToTarget", {"targetId": target_id, "flatten": True})
    attached, _ = await client.recv_until_id(attach, timeout=5)
    peer_session = attached["result"]["sessionId"]
    assert_equal(peer_session != opening_session, True, "dialog sessions are distinct")
    enabled = await client.send("Page.enable", session_id=peer_session)
    await client.recv_until_id(enabled, timeout=5)
    sessions = {opening_session, peer_session}

    for index, handler in enumerate([peer_session, opening_session]):
        message = f"shared dialog {index}"
        reply_text = f"shared reply {index}"
        evaluate = await client.send("Runtime.evaluate", {
            "expression": f"prompt({json.dumps(message)}, 'shared default')",
            "returnByValue": True,
        }, session_id=opening_session)
        seen: list[dict[str, Any]] = []

        def response(command_id: int) -> dict[str, Any] | None:
            return next((item for item in seen if item.get("id") == command_id), None)

        def dialog_events(method: str) -> list[dict[str, Any]]:
            return [item for item in seen if item.get("method") == method and item.get("sessionId") in sessions]

        def notified_sessions(method: str) -> set[str]:
            return {item["sessionId"] for item in dialog_events(method)}

        await _receive_until(client, seen, lambda: (
            response(evaluate) is not None
            or notified_sessions("Page.javascriptDialogOpening") == sessions
        ), "shared dialog opening")
        assert_equal(response(evaluate), None, "prompt must wait for either subscribed handler")
        for opening in dialog_events("Page.javascriptDialogOpening"):
            assert_equal(opening["params"]["message"], message, "shared opening message")
            assert_equal(opening["params"]["defaultPrompt"], "shared default", "shared opening default")

        handle = await client.send("Page.handleJavaScriptDialog", {
            "accept": True, "promptText": reply_text,
        }, session_id=handler)
        await _receive_until(client, seen, lambda: (
            response(evaluate) is not None and response(handle) is not None
            and notified_sessions("Page.javascriptDialogClosed") == sessions
        ), "shared dialog handling")
        assert_equal(response(handle).get("error"), None, "shared handle response")
        assert_equal(response(evaluate).get("error"), None, "shared evaluation response")
        assert_equal(response(evaluate)["result"]["result"]["value"], reply_text, "shared prompt return value")

        # Page-scoped evaluation replies fence the dialog output before its
        # final multiplicity is checked; no quiet-period sleep is needed.
        for session in sessions:
            barrier = await client.send("Runtime.evaluate", {
                "expression": "true", "returnByValue": True,
            }, session_id=session)
            _, trailing = await client.recv_until_id(barrier, timeout=5)
            seen.extend(trailing)
        for method in ["Page.javascriptDialogOpening", "Page.javascriptDialogClosed"]:
            for session in sessions:
                matching = [item for item in dialog_events(method) if item["sessionId"] == session]
                assert_equal(len(matching), 1, f"one {method} per subscribed session")
                if method == "Page.javascriptDialogClosed":
                    assert_equal(matching[0]["params"]["result"], True, "shared closed acceptance")
                    assert_equal(matching[0]["params"]["userInput"], reply_text, "shared closed prompt text")

    detach = await client.send("Target.detachFromTarget", {"sessionId": peer_session})
    await client.recv_until_id(detach, timeout=5)
    record(results, "raw_cdp_shared_javascript_dialog_subscriptions", {"sessions": 2, "dialogs": 2})
