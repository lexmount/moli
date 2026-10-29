from __future__ import annotations

import asyncio
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..config import is_moli_browser
from ..raw_cdp import RawCdpClient, connect_raw_cdp


FILE_URL = "file:///moli-policy-must-not-open"
BAD_PORT_URL = "http://example.test:1/"
NAVIGATION_POLICY_MESSAGE = (
    "Navigation to a local file URL requires an explicitly granted browser capability."
)
NAVIGATION_METHODS = {
    "Page.frameStartedNavigating",
    "Page.frameStartedLoading",
    "Page.frameNavigated",
    "Page.domContentEventFired",
    "Page.loadEventFired",
    "Network.requestWillBeSent",
    "Network.loadingFailed",
}


async def run_url_policy_group(
    endpoint: str,
    _fixture: str,
    results: list[dict[str, Any]],
) -> None:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_id: str | None = None
    session_id: str | None = None
    try:
        browser_context_id = await _create_browser_context(client)
        target_id, session_id = await _create_page_session(client, browser_context_id)
        await _enable_url_policy_observers(client, session_id)

        navigate_id = await client.send(
            "Page.navigate",
            {"url": FILE_URL},
            session_id=session_id,
        )
        navigate, navigation_messages = await _recv_until_response(client, navigate_id)
        if is_moli_browser():
            assert_equal(
                navigate,
                {"id": navigate_id, "sessionId": session_id,
                 "error": {"code": -32000, "message": NAVIGATION_POLICY_MESSAGE}},
                "CDP rejected file navigation response shape",
            )
        else:
            assert_equal(navigate.get("result", {}).get("errorText"), "net::ERR_FILE_NOT_FOUND", "Chromium missing file navigation")
            assert_equal(any(m.get("method") == "Fetch.requestPaused" for m in navigation_messages), True, "Chromium intercepts file navigation")
            reset = await client.send("Page.navigate", {"url": "about:blank"}, session_id=session_id)
            await _recv_until_response(client, reset)

        location, location_messages = await _evaluate(
            client,
            session_id,
            "location.href",
        )
        assert_equal(location, "about:blank", "CDP rejected file navigation current URL")
        if is_moli_browser():
            _assert_no_navigation_or_transport_events(
                navigation_messages + location_messages,
                "CDP rejected file navigation",
            )
        record(
            results,
            "cdp_file_navigation_policy",
            {
                "policy": "moli-hosted-file-capability" if is_moli_browser() else "chromium-local-file-navigation",
                "sessionId": session_id,
                "urlAfterFailure": location,
            },
        )

        fetch_result, fetch_messages = await _evaluate(
            client,
            session_id,
            """
            (async () => {
              try {
                await fetch('file:///moli-policy-must-not-open');
                return { resolved: true };
              } catch (error) {
                return {
                  resolved: false,
                  name: error && error.name,
                  message: error && error.message,
                };
              }
            })()
            """,
            await_promise=True,
        )
        assert_equal(
            fetch_result,
            {
                "resolved": False,
                "name": "TypeError",
                "message": 'URL scheme "file" is not supported.' if is_moli_browser() else "Failed to fetch",
            },
            "CDP Runtime.evaluate fetch(file:) rejection",
        )
        fetch_network = _assert_early_network_failure(
            fetch_messages,
            "Fetch",
            "CDP fetch(file:) rejection",
        )
        record(
            results,
            "cdp_file_fetch_policy",
            {
                "resolved": fetch_result["resolved"],
                "exceptionName": fetch_result["name"],
                "message": fetch_result["message"],
                "networkEvents": fetch_network,
            },
        )

        async_xhr_result, async_xhr_messages = await _evaluate(
            client,
            session_id,
            """
            new Promise(resolve => {
              const events = [];
              const xhr = new XMLHttpRequest();
              xhr.onreadystatechange = () => events.push('readystatechange:' + xhr.readyState);
              xhr.onloadstart = () => events.push('loadstart');
              xhr.onerror = () => events.push('error');
              xhr.onloadend = () => {
                events.push('loadend');
                resolve({
                  events,
                  readyState: xhr.readyState,
                  status: xhr.status,
                  responseURL: xhr.responseURL,
                  responseText: xhr.responseText,
                });
              };
              xhr.open('GET', 'file:///moli-policy-must-not-open');
              xhr.send();
            })
            """,
            await_promise=True,
        )
        assert_equal(
            async_xhr_result,
            {
                "events": [
                    "readystatechange:1",
                    "loadstart",
                    "readystatechange:4",
                    "error",
                    "loadend",
                ],
                "readyState": 4,
                "status": 0,
                "responseURL": "",
                "responseText": "",
            },
            "CDP Runtime.evaluate async XHR(file:) rejection",
        )
        async_xhr_network = _assert_early_network_failure(
            async_xhr_messages,
            "XHR",
            "CDP async XHR(file:) rejection",
        )
        record(
            results,
            "cdp_file_async_xhr_policy",
            {**async_xhr_result, "networkEvents": async_xhr_network},
        )

        sync_xhr_result, sync_xhr_messages = await _evaluate(
            client,
            session_id,
            """
            (() => {
              const events = [];
              const xhr = new XMLHttpRequest();
              xhr.onreadystatechange = () => events.push('readystatechange:' + xhr.readyState);
              xhr.onloadstart = () => events.push('loadstart');
              xhr.onerror = () => events.push('error');
              xhr.onloadend = () => events.push('loadend');
              xhr.open('GET', 'file:///moli-policy-must-not-open', false);
              let error = null;
              try {
                xhr.send();
              } catch (caught) {
                error = {
                  name: caught && caught.name,
                  message: caught && caught.message,
                  isDomException: caught instanceof DOMException,
                };
              }
              return { error, events, readyState: xhr.readyState, status: xhr.status };
            })()
            """,
        )
        assert_equal(
            sync_xhr_result,
            {
                "error": {
                    "name": "NetworkError",
                    "message": "Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'file:///moli-policy-must-not-open'.",
                    "isDomException": True,
                },
                "events": ["readystatechange:1"],
                "readyState": 4,
                "status": 0,
            },
            "CDP Runtime.evaluate synchronous XHR(file:) rejection",
        )
        sync_xhr_network = _assert_early_network_failure(
            sync_xhr_messages,
            "XHR",
            "CDP synchronous XHR(file:) rejection",
        )
        record(
            results,
            "cdp_file_sync_xhr_policy",
            {**sync_xhr_result, "networkEvents": sync_xhr_network},
        )

        bad_port_result, bad_port_messages = await _evaluate(
            client,
            session_id,
            """
            (() => {
              const events = [];
              const uploadEvents = [];
              const xhr = new XMLHttpRequest();
              xhr.onreadystatechange = () => events.push('readystatechange:' + xhr.readyState);
              xhr.onloadstart = () => events.push('loadstart');
              xhr.onerror = () => events.push('error');
              xhr.onloadend = () => events.push('loadend');
              xhr.upload.onloadstart = () => uploadEvents.push('loadstart');
              xhr.upload.onerror = () => uploadEvents.push('error');
              xhr.upload.onloadend = () => uploadEvents.push('loadend');
              xhr.open('POST', 'http://example.test:1/', false);
              let error = null;
              try {
                xhr.send('payload');
              } catch (caught) {
                error = {
                  name: caught && caught.name,
                  message: caught && caught.message,
                  isDomException: caught instanceof DOMException,
                };
              }
              return {
                error,
                events,
                uploadEvents,
                readyState: xhr.readyState,
                status: xhr.status,
                responseURL: xhr.responseURL,
                responseText: xhr.responseText,
              };
            })()
            """,
        )
        assert_equal(
            bad_port_result,
            {
                "error": {
                    "name": "NetworkError",
                    "message": "Failed to execute 'send' on 'XMLHttpRequest': Failed to load 'http://example.test:1/'.",
                    "isDomException": True,
                },
                "events": ["readystatechange:1"],
                "uploadEvents": [],
                "readyState": 4,
                "status": 0,
                "responseURL": "",
                "responseText": "",
            },
            "CDP Runtime.evaluate synchronous XHR bad-port rejection",
        )
        bad_port_network = _assert_early_network_failure(
            bad_port_messages,
            "XHR",
            "CDP synchronous XHR bad-port rejection",
            request_url=BAD_PORT_URL,
            error_text="xhr: blocked bad port for `http://example.test:1/`",
        )
        record(
            results,
            "cdp_sync_xhr_bad_port_semantics",
            {**bad_port_result, "networkEvents": bad_port_network},
        )
    finally:
        if target_id is not None:
            close_id = await client.send("Target.closeTarget", {"targetId": target_id})
            await client.recv_until_id(close_id)
        if browser_context_id is not None:
            dispose_id = await client.send(
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
            await client.recv_until_id(dispose_id)
        await client.websocket.close()


async def _create_browser_context(client: RawCdpClient) -> str:
    command_id = await client.send("Target.createBrowserContext")
    response, _ = await client.recv_until_id(command_id)
    browser_context_id = response.get("result", {}).get("browserContextId")
    if not isinstance(browser_context_id, str) or not browser_context_id:
        raise SmokeError(f"missing browserContextId: {response!r}")
    return browser_context_id


async def _create_page_session(
    client: RawCdpClient,
    browser_context_id: str,
) -> tuple[str, str]:
    create_id = await client.send(
        "Target.createTarget",
        {"browserContextId": browser_context_id, "url": "about:blank"},
    )
    create, _ = await client.recv_until_id(create_id)
    target_id = create.get("result", {}).get("targetId")
    if not isinstance(target_id, str) or not target_id:
        raise SmokeError(f"missing targetId: {create!r}")

    attach_id = await client.send(
        "Target.attachToTarget",
        {"targetId": target_id, "flatten": True},
    )
    attach, _ = await client.recv_until_id(attach_id)
    session_id = attach.get("result", {}).get("sessionId")
    if not isinstance(session_id, str) or not session_id:
        raise SmokeError(f"missing sessionId: {attach!r}")
    return target_id, session_id


async def _enable_url_policy_observers(client: RawCdpClient, session_id: str) -> None:
    for method, params in (
        ("Runtime.enable", {}),
        ("Page.enable", {}),
        ("Network.enable", {}),
        (
            "Fetch.enable",
            {
                "patterns": [
                    {
                        "urlPattern": "*",
                        "requestStage": "Request",
                    }
                ]
            },
        ),
    ):
        command_id = await client.send(method, params, session_id=session_id)
        await client.recv_until_id(command_id)


async def _recv_until_response(
    client: RawCdpClient,
    message_id: int,
    *,
    timeout: float = 10.0,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for CDP response id={message_id}: {seen[-20:]!r}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if not is_moli_browser() and message.get("method") == "Fetch.requestPaused":
            await client.send("Fetch.continueRequest", {"requestId": message["params"]["requestId"]}, session_id=message["sessionId"])
        if message.get("id") == message_id:
            return message, seen


async def _evaluate(
    client: RawCdpClient,
    session_id: str,
    expression: str,
    *,
    await_promise: bool = False,
) -> tuple[Any, list[dict[str, Any]]]:
    command_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": expression,
            "awaitPromise": await_promise,
            "returnByValue": True,
        },
        session_id=session_id,
    )
    response, messages = await _recv_until_response(client, command_id)
    result = response.get("result", {}).get("result", {})
    if "exceptionDetails" in response.get("result", {}):
        raise SmokeError(f"Runtime.evaluate raised unexpectedly: {response!r}")
    if not isinstance(result, dict) or "value" not in result:
        raise SmokeError(f"Runtime.evaluate returned no by-value result: {response!r}")
    return result["value"], messages


def _assert_no_navigation_or_transport_events(
    messages: list[dict[str, Any]],
    label: str,
) -> None:
    unexpected = [message for message in messages if message.get("method") in NAVIGATION_METHODS]
    if unexpected:
        raise SmokeError(f"{label} emitted navigation/network events: {unexpected!r}")


def _assert_early_network_failure(
    messages: list[dict[str, Any]],
    resource_type: str,
    label: str,
    *,
    request_url: str = FILE_URL,
    error_text: str = 'URL scheme "file" is not supported.',
) -> list[str]:
    if not is_moli_browser():
        error_text = "net::ERR_UNSAFE_PORT" if request_url == BAD_PORT_URL else ""
    forbidden_methods = {
        "Network.responseReceived",
        "Network.loadingFinished",
    }
    if is_moli_browser():
        forbidden_methods.add("Fetch.requestPaused")
    elif request_url == BAD_PORT_URL:
        assert_equal(any(m.get("method") == "Fetch.requestPaused" for m in messages), True,
                     "Chromium intercepts synchronous bad-port XHR before rejecting it")
    unexpected = [
        message for message in messages if message.get("method") in forbidden_methods
    ]
    if unexpected:
        raise SmokeError(f"{label} reached interception or transport: {unexpected!r}")

    network_events = [
        message
        for message in messages
        if message.get("method")
        in {"Network.requestWillBeSent", "Network.loadingFailed"}
    ]
    assert_equal(
        [event["method"] for event in network_events],
        ["Network.requestWillBeSent", "Network.loadingFailed"],
        f"{label} early network failure event order",
    )
    request = network_events[0]["params"]
    failure = network_events[1]["params"]
    assert_equal(request["request"]["url"], request_url, f"{label} observed URL")
    assert_equal(request["type"], resource_type, f"{label} request resource type")
    assert_equal(failure["type"], resource_type, f"{label} failure resource type")
    assert_equal(
        failure["requestId"],
        request["requestId"],
        f"{label} request/failure identity",
    )
    assert_equal(
        failure["errorText"],
        error_text,
        f"{label} failure reason",
    )
    assert_equal(failure["canceled"], False, f"{label} canceled flag")
    return [event["method"] for event in network_events]
