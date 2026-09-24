from __future__ import annotations

import base64
import json
from pathlib import Path
from typing import Any, Awaitable

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..helpers import attach_cdp_event_collector, evaluate_xhr


BODY_TAKEN_CONTINUE_RESPONSE_ERROR = "Unable to continue request as is after body is taken"


async def run_page_network_group(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events

    await page.route(
        "**/document-fulfill",
        lambda route: route.fulfill(status=200, content_type="text/html; charset=utf-8", body="<!doctype html><main>document fulfilled body</main>"),
    )
    fulfilled_response = await page.goto(f"{fixture}/document-fulfill", wait_until="load", timeout=10_000)
    assert_equal(fulfilled_response.status if fulfilled_response else None, 200, "document route fulfill response status")
    assert_equal(await page.text_content("main"), "document fulfilled body", "document route fulfill body")
    state.record("route_fulfill_document")
    await page.unroute("**/document-fulfill")

    async def continue_document(route: Any) -> None:
        headers = dict(route.request.headers)
        headers["x-smoke-nav-route"] = "continued-document"
        await route.continue_(headers=headers)

    await page.route("**/document-continue", continue_document)
    document_continue_start = len(subresource_events)
    continued_response = await page.goto(f"{fixture}/document-continue", wait_until="load", timeout=10_000)
    assert_equal(continued_response.status if continued_response else None, 200, "document route continue response status")
    assert_equal(await page.text_content("main"), "continued-document", "document route continue body")

    def saw_document_continue_events() -> bool:
        events = subresource_events[document_continue_start:]
        request = _find_request(events, "Document", f"{fixture}/document-continue")
        request_id = request and request["params"].get("requestId")
        return bool(
            request_id
            and any(event["method"] == "Network.responseReceived" and event["params"].get("requestId") == request_id for event in events)
            and any(event["method"] == "Network.loadingFinished" and event["params"].get("requestId") == request_id for event in events)
        )

    await wait_until(saw_document_continue_events, "Document route continue Network events")
    state.record("route_continue_document")
    await page.unroute("**/document-continue")

    await page.route("**/document-abort", lambda route: route.abort("blockedbyclient"))
    document_abort_start = len(subresource_events)
    aborted_document_error = ""
    try:
        await page.goto(f"{fixture}/document-abort", wait_until="load", timeout=10_000)
    except Exception as error:
        aborted_document_error = str(error)
    if "ERR_BLOCKED_BY_CLIENT" not in aborted_document_error:
        raise SmokeError(f"document route abort should reject navigation, got {aborted_document_error}")

    def saw_document_abort_events() -> bool:
        events = subresource_events[document_abort_start:]
        request = _find_request(events, "Document", f"{fixture}/document-abort")
        request_id = request and request["params"].get("requestId")
        return bool(
            request_id
            and any(
                event["method"] == "Network.loadingFailed"
                and event["params"].get("requestId") == request_id
                and event["params"].get("errorText") == "net::ERR_BLOCKED_BY_CLIENT"
                for event in events
            )
        )

    await wait_until(saw_document_abort_events, "Document route abort Network.loadingFailed")
    state.record("route_abort_document")
    await page.unroute("**/document-abort")

    await page.route(
        "**/api",
        lambda route: route.fulfill(status=200, content_type="text/plain; charset=utf-8", body="fulfilled body"),
    )
    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    fetched = await page.evaluate("async () => await fetch('/api').then(response => response.text())")
    assert_equal(fetched, "fulfilled body", "route fulfilled fetch body")
    state.record("route_fulfill_fetch")

    xhr_fulfilled = await evaluate_xhr(page, "/api")
    assert_equal(xhr_fulfilled.get("phase"), "load", "route fulfilled xhr phase")
    assert_equal(xhr_fulfilled.get("status"), 200, "route fulfilled xhr status")
    assert_equal(xhr_fulfilled.get("text"), "fulfilled body", "route fulfilled xhr body")
    state.record("route_fulfill_xhr")

    async def continue_api(route: Any) -> None:
        headers = dict(route.request.headers)
        headers["x-smoke-route"] = "continued"
        await route.continue_(headers=headers)

    await page.route("**/api-continue", continue_api)
    continued = await page.evaluate("async () => await fetch('/api-continue').then(response => response.json())")
    assert_equal(continued.get("routeHeader"), "continued", "route continue request header")
    state.record("route_continue_fetch")

    xhr_continued = await evaluate_xhr(page, "/api-continue", "POST", "payload")
    assert_equal(xhr_continued.get("phase"), "load", "route continue xhr phase")
    assert_equal(xhr_continued.get("status"), 200, "route continue xhr status")
    assert_equal(xhr_continued.get("text"), json.dumps({"method": "POST", "routeHeader": "continued"}, separators=(",", ":")), "route continue xhr body")
    state.record("route_continue_xhr")

    await page.route("**/api-abort", lambda route: route.abort("blockedbyclient"))
    aborted_fetch = await page.evaluate(
        """async () => {
          try { await fetch('/api-abort'); return 'resolved'; }
          catch (error) { return `${error?.constructor?.name || 'Error'}:${error?.message || String(error)}`; }
        }"""
    )
    if not str(aborted_fetch).startswith("TypeError:"):
        raise SmokeError(f"route abort should reject fetch with TypeError, got {aborted_fetch}")
    state.record("route_abort_fetch")

    xhr_aborted = await evaluate_xhr(page, "/api-abort")
    if "error" not in xhr_aborted.get("events", []):
        raise SmokeError(f"route abort xhr should emit error, got {xhr_aborted}")
    if "load" in xhr_aborted.get("events", []):
        raise SmokeError(f"route abort xhr should not emit load, got {xhr_aborted}")
    assert_equal(xhr_aborted.get("status"), 0, "route abort xhr status")
    state.record("route_abort_xhr")
    await page.unroute("**/api")
    await page.unroute("**/api-continue")
    await page.unroute("**/api-abort")

    await _verify_fetch_network_events(state)
    await _verify_xhr_network_events(state)
    await _verify_request_payload_network_events(state)
    await _verify_response_header_network_events(state)
    await _verify_fetch_interception_resource_type_matrix(state)
    await _verify_response_stage_fetch_interception(state)
    await _verify_response_stage_xhr_get_response_body(state)
    await _verify_response_stage_fetch_body_stream(state)
    await _verify_response_stage_fetch_binary_body(state)
    await _verify_response_stage_fetch_binary_body_stream(state)
    await _verify_fetch_auth_challenge(state)
    await _verify_fetch_auth_cancel(state)
    await _verify_fetch_auth_response_stage(state)
    await _verify_redirect_chain_network_events(state)
    await _verify_parser_script_network_events(state)
    await _verify_parser_stylesheet_network_events(state)
    await _verify_parser_stylesheet_network_events_without_script_gate(state)
    await _verify_chromium_resource_type_network_matrix(state)
    await _verify_blocked_websocket_events(state)


async def run_websocket_group(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    websocket_events = state.websocket_events
    event_start = len(websocket_events)

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    websocket_result = await page.evaluate(
        """async () => await new Promise((resolve, reject) => {
          const url = new URL('/ws-echo', location.href);
          url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
          const socket = new WebSocket(url.href, 'smoke');
          const timer = setTimeout(() => { socket.close(); reject(new Error(`websocket timed out at readyState=${socket.readyState}`)); }, 5000);
          socket.onopen = () => socket.send('websocket ping');
          socket.onmessage = event => {
            clearTimeout(timer);
            const result = { data: event.data, protocol: socket.protocol, readyStateBeforeClose: socket.readyState };
            socket.close(1000, 'done');
            resolve(result);
          };
          socket.onerror = () => { clearTimeout(timer); reject(new Error(`websocket error at readyState=${socket.readyState}`)); };
        })"""
    )
    assert_equal(websocket_result.get("data"), "echo:websocket ping", "websocket echoed message")
    assert_equal(websocket_result.get("protocol"), "smoke", "websocket selected protocol")
    assert_equal(websocket_result.get("readyStateBeforeClose"), 1, "websocket ready state before close")
    state.record("websocket_echo_round_trip")

    expected_ws_url = fixture.replace("http:", "ws:", 1) + "/ws-echo"
    await wait_until(
        lambda: any(
            event["method"] == "Network.webSocketCreated"
            and event["params"].get("url") == expected_ws_url
            for event in websocket_events[event_start:]
        ),
        "Network.webSocketCreated",
    )
    current_events = websocket_events[event_start:]
    ws_created = next(
        (
            event
            for event in current_events
            if event["method"] == "Network.webSocketCreated"
            and event["params"].get("url") == expected_ws_url
        ),
        None,
    )
    request_id = ws_created and ws_created["params"].get("requestId")
    if not request_id:
        raise SmokeError(f"missing Network.webSocketCreated event: {current_events}")
    await wait_until(
        lambda: any(
            event["method"] == "Network.webSocketFrameReceived"
            and event["params"].get("requestId") == request_id
            for event in websocket_events[event_start:]
        ),
        "Network.webSocketFrameReceived",
    )
    current_events = websocket_events[event_start:]
    ws_handshake = next(
        (
            event
            for event in current_events
            if event["method"] == "Network.webSocketHandshakeResponseReceived"
            and event["params"].get("requestId") == request_id
        ),
        None,
    )
    assert_equal(
        ws_handshake.get("params", {}).get("response", {}).get("status")
        if ws_handshake
        else None,
        101,
        "websocket CDP handshake status",
    )
    ws_sent = next(
        (
            event
            for event in current_events
            if event["method"] == "Network.webSocketFrameSent"
            and event["params"].get("requestId") == request_id
        ),
        None,
    )
    assert_equal(
        ws_sent.get("params", {}).get("response", {}).get("opcode")
        if ws_sent
        else None,
        1,
        "websocket CDP sent opcode",
    )
    ws_recv = next(
        (
            event
            for event in current_events
            if event["method"] == "Network.webSocketFrameReceived"
            and event["params"].get("requestId") == request_id
        ),
        None,
    )
    assert_equal(
        ws_recv.get("params", {}).get("response", {}).get("opcode")
        if ws_recv
        else None,
        1,
        "websocket CDP received opcode",
    )
    await wait_until(
        lambda: any(
            event["method"] == "Network.webSocketClosed"
            and event["params"].get("requestId") == request_id
            for event in websocket_events[event_start:]
        ),
        "Network.webSocketClosed",
    )
    request_methods = [
        event["method"]
        for event in websocket_events[event_start:]
        if event["params"].get("requestId") == request_id
    ]
    assert_equal(
        request_methods[-1],
        "Network.webSocketClosed",
        "websocket terminal event",
    )
    state.record(
        "websocket_network_events",
        {
            "requestId": request_id,
            "eventMethods": request_methods,
        },
    )


async def run_download_group(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture

    await page.goto(f"{fixture}/download-page", wait_until="load", timeout=10_000)
    async with page.expect_download(timeout=10_000) as download_info:
        await page.locator("#download").evaluate("anchor => anchor.click()")
    download = await download_info.value
    assert_equal(download.suggested_filename, "smoke-download.txt", "download suggested filename")
    download_path = await download.path()
    assert_equal(Path(download_path).read_text(encoding="utf-8"), "download contents", "download artifact contents")
    state.record("download_event_and_artifact")

    await page.goto(f"{fixture}/download-page", wait_until="load", timeout=10_000)
    async with page.expect_download(timeout=10_000) as slow_download_info:
        await page.locator("#slow-download").evaluate("anchor => anchor.click()")
    slow_download = await slow_download_info.value
    assert_equal(slow_download.suggested_filename, "slow-smoke-download.txt", "slow download suggested filename")
    await slow_download.cancel()
    assert_equal(await slow_download.failure(), "canceled", "slow download cancel failure state")
    state.record("download_cancel")


async def _verify_fetch_network_events(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    fetch_network_start = len(subresource_events)
    observed_fetch = await page.evaluate("async () => await fetch('/api-continue').then(response => response.json())")
    assert_equal(observed_fetch.get("routeHeader"), None, "unrouted fetch should reach fixture server")

    def saw_fetch_events() -> bool:
        events = subresource_events[fetch_network_start:]
        request = _find_request(events, "Fetch", f"{fixture}/api-continue")
        request_id = request and request["params"].get("requestId")
        if not request_id:
            return False
        matching_indices = {
            method: next(
                (
                    index
                    for index, event in enumerate(events)
                    if event["method"] == method and event["params"].get("requestId") == request_id
                ),
                None,
            )
            for method in ("Network.responseReceived", "Network.dataReceived", "Network.loadingFinished")
        }
        response_index = matching_indices["Network.responseReceived"]
        data_index = matching_indices["Network.dataReceived"]
        finished_index = matching_indices["Network.loadingFinished"]
        if response_index is None or data_index is None or finished_index is None:
            return False
        data_params = events[data_index]["params"]
        return bool(
            response_index < data_index < finished_index
            and data_params.get("dataLength", 0) > 0
            and data_params.get("encodedDataLength", 0) > 0
        )

    await wait_until(saw_fetch_events, "Fetch Network events")
    state.record("fetch_network_events")


async def _verify_xhr_network_events(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    xhr_network_start = len(subresource_events)
    observed_xhr = await page.evaluate(
        """async () => await new Promise(resolve => {
          const xhr = new XMLHttpRequest();
          xhr.open('GET', '/api-continue', true);
          xhr.onload = () => resolve(JSON.parse(xhr.responseText));
          xhr.send();
        })"""
    )
    assert_equal(observed_xhr.get("routeHeader"), None, "unrouted xhr should reach fixture server")
    assert_equal(observed_xhr.get("method"), "GET", "unrouted xhr should keep original method")

    def saw_xhr_events() -> bool:
        events = subresource_events[xhr_network_start:]
        request = _find_request(events, "XHR", f"{fixture}/api-continue")
        request_id = request and request["params"].get("requestId")
        return bool(
            request_id
            and any(event["method"] == "Network.responseReceived" and event["params"].get("requestId") == request_id for event in events)
            and any(event["method"] == "Network.loadingFinished" and event["params"].get("requestId") == request_id for event in events)
        )

    await wait_until(saw_xhr_events, "XHR Network events")
    state.record("xhr_network_events")


async def _verify_request_payload_network_events(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    payload_start = len(subresource_events)

    fetch_payload = "fetch-payload=alpha"
    fetch_echo = await page.evaluate(
        """async ({ body }) => await fetch('/api-echo', {
          method: 'POST',
          headers: {
            'content-type': 'text/plain;charset=UTF-8',
            'x-smoke-post': 'fetch-custom'
          },
          body
        }).then(response => response.json())""",
        {"body": fetch_payload},
    )
    assert_equal(fetch_echo.get("method"), "POST", "fetch echo method")
    assert_equal(fetch_echo.get("body"), fetch_payload, "fetch echo body")
    assert_equal(fetch_echo.get("customHeader"), "fetch-custom", "fetch echo custom header")

    xhr_payload = "xhr-payload=beta"
    xhr_echo = await page.evaluate(
        """async ({ body }) => await new Promise(resolve => {
          const xhr = new XMLHttpRequest();
          xhr.open('POST', '/api-echo', true);
          xhr.setRequestHeader('content-type', 'text/plain;charset=UTF-8');
          xhr.setRequestHeader('x-smoke-post', 'xhr-custom');
          xhr.onload = () => resolve(JSON.parse(xhr.responseText));
          xhr.send(body);
        })""",
        {"body": xhr_payload},
    )
    assert_equal(xhr_echo.get("method"), "POST", "xhr echo method")
    assert_equal(xhr_echo.get("body"), xhr_payload, "xhr echo body")
    assert_equal(xhr_echo.get("customHeader"), "xhr-custom", "xhr echo custom header")

    fetch_request: dict[str, Any] | None = None
    xhr_request: dict[str, Any] | None = None

    def saw_payload_events() -> bool:
        nonlocal fetch_request, xhr_request
        events = subresource_events[payload_start:]
        fetch_request_id = _completed_request_id(events, "Fetch", f"{fixture}/api-echo")
        xhr_request_id = _completed_request_id(events, "XHR", f"{fixture}/api-echo")
        fetch_request = _request_by_id(events, fetch_request_id) if fetch_request_id else None
        xhr_request = _request_by_id(events, xhr_request_id) if xhr_request_id else None
        return bool(fetch_request and xhr_request)

    await wait_until(saw_payload_events, "Fetch/XHR request payload Network events")
    _assert_request_payload(
        fetch_request,
        method="POST",
        post_data=fetch_payload,
        custom_header="fetch-custom",
        label="fetch",
    )
    _assert_request_payload(
        xhr_request,
        method="POST",
        post_data=xhr_payload,
        custom_header="xhr-custom",
        label="xhr",
    )
    fetch_request_id = fetch_request["params"]["requestId"]
    xhr_request_id = xhr_request["params"]["requestId"]
    fetch_post_data = await state.cdp.send(
        "Network.getRequestPostData",
        {"requestId": fetch_request_id},
    )
    xhr_post_data = await state.cdp.send(
        "Network.getRequestPostData",
        {"requestId": xhr_request_id},
    )
    assert_equal(
        fetch_post_data.get("postData"),
        fetch_payload,
        "fetch Network.getRequestPostData",
    )
    assert_equal(
        xhr_post_data.get("postData"),
        xhr_payload,
        "xhr Network.getRequestPostData",
    )
    state.record(
        "request_payload_network_events",
        {
            "fetchRequestId": fetch_request_id,
            "xhrRequestId": xhr_request_id,
            "getRequestPostData": {
                "fetch": fetch_post_data["postData"],
                "xhr": xhr_post_data["postData"],
            },
        },
    )


async def _verify_response_header_network_events(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    response_header_start = len(subresource_events)

    fetch_result = await page.evaluate(
        """async () => await fetch('/api-response-headers', {
          headers: { 'x-smoke-response-kind': 'fetch' }
        }).then(response => response.json())"""
    )
    assert_equal(fetch_result.get("kind"), "fetch", "fetch response header fixture kind")

    xhr_result = await page.evaluate(
        """async () => await new Promise(resolve => {
          const xhr = new XMLHttpRequest();
          xhr.open('GET', '/api-response-headers', true);
          xhr.setRequestHeader('x-smoke-response-kind', 'xhr');
          xhr.onload = () => resolve(JSON.parse(xhr.responseText));
          xhr.send();
        })"""
    )
    assert_equal(xhr_result.get("kind"), "xhr", "xhr response header fixture kind")

    fetch_response: dict[str, Any] | None = None
    xhr_response: dict[str, Any] | None = None

    def saw_response_header_events() -> bool:
        nonlocal fetch_response, xhr_response
        events = subresource_events[response_header_start:]
        fetch_request_id = _completed_request_id(events, "Fetch", f"{fixture}/api-response-headers")
        xhr_request_id = _completed_request_id(events, "XHR", f"{fixture}/api-response-headers")
        fetch_response = _response_by_id(events, fetch_request_id) if fetch_request_id else None
        xhr_response = _response_by_id(events, xhr_request_id) if xhr_request_id else None
        return bool(fetch_response and xhr_response)

    await wait_until(saw_response_header_events, "Fetch/XHR response header Network events")
    _assert_response_headers(fetch_response, expected_kind="fetch", label="fetch")
    _assert_response_headers(xhr_response, expected_kind="xhr", label="xhr")
    state.record("response_header_network_events")


async def _verify_fetch_interception_resource_type_matrix(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    fetch_events = attach_cdp_event_collector(state.cdp, ["Fetch.requestPaused"])

    for filter_type in ("Fetch", "XHR", "EventSource"):
        token = filter_type.lower()
        fetch_url = f"{fixture}/api?fetch-interception-filter={token}&kind=fetch"
        xhr_url = f"{fixture}/api?fetch-interception-filter={token}&kind=xhr"
        event_source_url = (
            f"{fixture}/semantic-event-source?fetch-interception-filter={token}"
        )
        event_start = len(fetch_events)
        network_start = len(state.subresource_events)
        await state.cdp.send(
            "Fetch.enable",
            {
                "patterns": [
                    {
                        "urlPattern": f"*fetch-interception-filter={token}*",
                        "requestStage": "Request",
                        "resourceType": filter_type,
                    }
                ]
            },
        )
        try:
            await page.evaluate(
                """({ fetchUrl, xhrUrl, eventSourceUrl }) => {
                  globalThis.__smokeFetchInterceptionMatrix = {
                    fetch: 'pending',
                    xhr: 'pending',
                    eventSource: 'pending'
                  };
                  fetch(fetchUrl).then(
                    () => { globalThis.__smokeFetchInterceptionMatrix.fetch = 'done'; },
                    error => { globalThis.__smokeFetchInterceptionMatrix.fetch = `error:${error}`; }
                  );
                  const xhr = new XMLHttpRequest();
                  xhr.open('GET', xhrUrl, true);
                  xhr.onload = () => { globalThis.__smokeFetchInterceptionMatrix.xhr = 'done'; };
                  xhr.onerror = () => { globalThis.__smokeFetchInterceptionMatrix.xhr = 'error'; };
                  xhr.send();
                  const source = new EventSource(eventSourceUrl);
                  source.addEventListener('semantic', () => {
                    globalThis.__smokeFetchInterceptionMatrix.eventSource = 'done';
                    source.close();
                  });
                  source.onerror = () => {
                    if (globalThis.__smokeFetchInterceptionMatrix.eventSource === 'pending') {
                      globalThis.__smokeFetchInterceptionMatrix.eventSource = 'error';
                    }
                  };
                }""",
                {
                    "fetchUrl": fetch_url,
                    "xhrUrl": xhr_url,
                    "eventSourceUrl": event_source_url,
                },
            )

            def matching_pauses() -> list[dict[str, Any]]:
                return [
                    event
                    for event in fetch_events[event_start:]
                    if event.get("method") == "Fetch.requestPaused"
                    and event.get("params", {}).get("request", {}).get("url")
                    in (fetch_url, xhr_url, event_source_url)
                    and "responseStatusCode" not in event.get("params", {})
                ]

            await wait_until(
                lambda: len(matching_pauses()) == 3,
                f"{filter_type} filter shared XHR interception pauses",
            )
            pauses = matching_pauses()
            assert_equal(len(pauses), 3, f"{filter_type} filter pause count")
            for url, network_type in (
                (fetch_url, "Fetch"),
                (xhr_url, "XHR"),
                (event_source_url, "EventSource"),
            ):
                paused = next(
                    event
                    for event in pauses
                    if event["params"].get("request", {}).get("url") == url
                )
                assert_equal(
                    paused["params"].get("resourceType"),
                    "XHR",
                    f"{filter_type} filter Fetch-domain resource type for {network_type}",
                )
                network_id = paused["params"].get("networkId")
                if not isinstance(network_id, str) or not network_id:
                    raise SmokeError(f"{filter_type} filter pause missed networkId: {paused}")
                try:
                    await wait_until(
                        lambda network_id=network_id, network_type=network_type: any(
                            event.get("method") == "Network.requestWillBeSent"
                            and event.get("params", {}).get("requestId") == network_id
                            and event.get("params", {}).get("type") == network_type
                            for event in state.subresource_events[network_start:]
                        ),
                        f"{filter_type} filter preserved Network {network_type} type",
                    )
                except SmokeError as error:
                    correlated = [
                        event
                        for event in state.subresource_events[network_start:]
                        if event.get("params", {}).get("requestId") == network_id
                    ]
                    requests = [
                        event
                        for event in state.subresource_events[network_start:]
                        if event.get("method") == "Network.requestWillBeSent"
                    ]
                    raise SmokeError(
                        f"{error}; pause={paused!r}; correlated Network events={correlated!r}; "
                        f"request events since matrix start={requests!r}"
                    ) from error

            for paused in pauses:
                await state.cdp.send(
                    "Fetch.continueRequest",
                    {"requestId": paused["params"]["requestId"]},
                )
            await wait_until(
                lambda: page.evaluate(
                    "() => globalThis.__smokeFetchInterceptionMatrix?.fetch === 'done'"
                    " && globalThis.__smokeFetchInterceptionMatrix?.xhr === 'done'"
                    " && globalThis.__smokeFetchInterceptionMatrix?.eventSource === 'done'"
                ),
                f"{filter_type} filter continued fetch, XHR, and EventSource",
            )
            state.record(f"fetch_interception_{token}_filter_shared_xhr_type")
        finally:
            await state.cdp.send("Fetch.disable")


async def _verify_response_stage_fetch_interception(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    response_stage_events = attach_cdp_event_collector(state.cdp, ["Fetch.requestPaused"])
    response_stage_start = len(response_stage_events)
    network_start = len(state.subresource_events)
    await state.cdp.send(
        "Fetch.enable",
        {
            "patterns": [
                {
                    "urlPattern": "*/api-response-stage",
                    "requestStage": "Response",
                    "resourceType": "Fetch",
                }
            ]
        },
    )
    try:
        await page.evaluate(
            """() => {
              globalThis.__smokeResponseStageResult = 'pending';
              fetch('/api-response-stage')
                .then(response => response.text())
                .then(
                  text => { globalThis.__smokeResponseStageResult = text; },
                  error => { globalThis.__smokeResponseStageResult = `error:${error?.message || String(error)}`; }
                );
              return 'scheduled';
            }"""
        )

        paused: dict[str, Any] | None = None

        def saw_response_stage_pause() -> bool:
            nonlocal paused
            paused = next(
                (
                    event
                    for event in response_stage_events[response_stage_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("request", {}).get("url")
                    == f"{fixture}/api-response-stage"
                    and event["params"].get("resourceType") == "XHR"
                    and event["params"].get("responseStatusCode") == 200
                ),
                None,
            )
            return paused is not None

        await wait_until(saw_response_stage_pause, "Fetch response-stage requestPaused")
        assert paused is not None
        response_headers = paused["params"].get("responseHeaders") or []
        if not _header_list_contains(response_headers, "x-smoke-response-stage", "paused"):
            raise SmokeError(f"missing response-stage pause header: {paused}")
        assert_equal(
            await page.evaluate("() => globalThis.__smokeResponseStageResult"),
            "pending",
            "fetch response-stage should pause before body is delivered",
        )

        request_id = paused["params"].get("requestId")
        network_id = paused["params"].get("networkId")
        if not isinstance(request_id, str) or not request_id:
            raise SmokeError(f"missing response-stage Fetch requestId: {paused}")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing response-stage Fetch networkId: {paused}")
        await state.cdp.send("Fetch.continueResponse", {"requestId": request_id})

        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in state.subresource_events[network_start:]
            ),
            "Fetch response-stage Network.loadingFinished",
        )
        await wait_until(
            lambda: page.evaluate("() => globalThis.__smokeResponseStageResult === 'response-stage body'"),
            "Fetch response-stage continued body",
        )
        state.record("response_stage_fetch_interception")
    finally:
        await state.cdp.send("Fetch.disable")


async def _verify_response_stage_xhr_get_response_body(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    response_stage_events = attach_cdp_event_collector(state.cdp, ["Fetch.requestPaused"])
    response_stage_start = len(response_stage_events)
    network_start = len(state.subresource_events)
    await state.cdp.send(
        "Fetch.enable",
        {
            "patterns": [
                {
                    "urlPattern": "*/api-response-stage",
                    "requestStage": "Response",
                    "resourceType": "XHR",
                }
            ]
        },
    )
    try:
        await page.evaluate(
            """() => {
              globalThis.__smokeXhrResponseStageResult = { phase: 'pending' };
              const xhr = new XMLHttpRequest();
              xhr.open('GET', '/api-response-stage', true);
              xhr.onload = () => {
                globalThis.__smokeXhrResponseStageResult = {
                  phase: 'load',
                  status: xhr.status,
                  text: xhr.responseText
                };
              };
              xhr.onerror = () => {
                globalThis.__smokeXhrResponseStageResult = {
                  phase: 'error',
                  status: xhr.status,
                  text: xhr.responseText
                };
              };
              xhr.send();
              return 'scheduled';
            }"""
        )

        paused: dict[str, Any] | None = None

        def saw_response_stage_pause() -> bool:
            nonlocal paused
            paused = next(
                (
                    event
                    for event in response_stage_events[response_stage_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("request", {}).get("url")
                    == f"{fixture}/api-response-stage"
                    and event["params"].get("resourceType") == "XHR"
                    and event["params"].get("responseStatusCode") == 200
                ),
                None,
            )
            return paused is not None

        await wait_until(saw_response_stage_pause, "XHR response-stage requestPaused")
        assert paused is not None
        response_headers = paused["params"].get("responseHeaders") or []
        if not _header_list_contains(response_headers, "x-smoke-response-stage", "paused"):
            raise SmokeError(f"missing XHR response-stage pause header: {paused}")
        assert_equal(
            (await page.evaluate("() => globalThis.__smokeXhrResponseStageResult")).get("phase"),
            "pending",
            "xhr response-stage should pause before load",
        )

        request_id = paused["params"].get("requestId")
        network_id = paused["params"].get("networkId")
        if not isinstance(request_id, str) or not request_id:
            raise SmokeError(f"missing response-stage XHR requestId: {paused}")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing response-stage XHR networkId: {paused}")

        body = await state.cdp.send("Fetch.getResponseBody", {"requestId": request_id})
        assert_equal(body.get("body"), "response-stage body", "XHR response-stage Fetch.getResponseBody body")
        assert_equal(body.get("base64Encoded"), False, "XHR response-stage Fetch.getResponseBody encoding")

        await state.cdp.send("Fetch.continueResponse", {"requestId": request_id})
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in state.subresource_events[network_start:]
            ),
            "XHR response-stage Network.loadingFinished",
        )
        await wait_until(
            lambda: page.evaluate(
                "() => globalThis.__smokeXhrResponseStageResult?.phase === 'load'"
                " && globalThis.__smokeXhrResponseStageResult?.text === 'response-stage body'"
            ),
            "XHR response-stage continued body",
        )
        result = await page.evaluate("() => globalThis.__smokeXhrResponseStageResult")
        assert_equal(result.get("status"), 200, "XHR response-stage status after continue")
        state.record("response_stage_xhr_get_response_body")
    finally:
        await state.cdp.send("Fetch.disable")


async def _verify_response_stage_fetch_body_stream(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    response_stage_events = attach_cdp_event_collector(state.cdp, ["Fetch.requestPaused"])
    response_stage_start = len(response_stage_events)
    network_start = len(state.subresource_events)
    stream_handle: str | None = None
    await state.cdp.send(
        "Fetch.enable",
        {
            "patterns": [
                {
                    "urlPattern": "*/api-response-stage",
                    "requestStage": "Response",
                    "resourceType": "Fetch",
                }
            ]
        },
    )
    try:
        await page.evaluate(
            """() => {
              globalThis.__smokeFetchStreamResult = 'pending';
              fetch('/api-response-stage')
                .then(response => response.text())
                .then(
                  text => { globalThis.__smokeFetchStreamResult = text; },
                  error => { globalThis.__smokeFetchStreamResult = `error:${error?.message || String(error)}`; }
                );
              return 'scheduled';
            }"""
        )

        paused: dict[str, Any] | None = None

        def saw_response_stage_pause() -> bool:
            nonlocal paused
            paused = next(
                (
                    event
                    for event in response_stage_events[response_stage_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("request", {}).get("url")
                    == f"{fixture}/api-response-stage"
                    and event["params"].get("resourceType") == "XHR"
                    and event["params"].get("responseStatusCode") == 200
                ),
                None,
            )
            return paused is not None

        await wait_until(saw_response_stage_pause, "Fetch response-stage stream requestPaused")
        assert paused is not None
        request_id = paused["params"].get("requestId")
        network_id = paused["params"].get("networkId")
        if not isinstance(request_id, str) or not request_id:
            raise SmokeError(f"missing response-stage stream requestId: {paused}")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing response-stage stream networkId: {paused}")
        assert_equal(
            await page.evaluate("() => globalThis.__smokeFetchStreamResult"),
            "pending",
            "fetch response-stage stream should pause before body is delivered",
        )

        stream = await state.cdp.send("Fetch.takeResponseBodyAsStream", {"requestId": request_id})
        stream_handle = stream.get("stream")
        if not isinstance(stream_handle, str) or not stream_handle:
            raise SmokeError(f"missing Fetch.takeResponseBodyAsStream handle: {stream}")
        # Chromium rejects every explicit offset for Fetch response streams,
        # including zero; a rejected read must leave the sequential cursor alone.
        for offset in (9, 0):
            await _expect_cdp_error(
                state.cdp.send("IO.read", {"handle": stream_handle, "offset": offset, "size": 5}),
                "OffsetNotSupportedForStream",
                "Fetch response-stage stream rejects random access",
            )
        first_chunk = await state.cdp.send(
            "IO.read", {"handle": stream_handle, "size": 8}
        )
        assert_equal(first_chunk.get("base64Encoded"), False, "Fetch response-stage stream first chunk encoding")
        assert_equal(first_chunk.get("data"), "response", "Fetch response-stage stream first chunk")
        assert_equal(first_chunk.get("eof"), False, "Fetch response-stage stream first chunk eof")
        tail_chunk = await state.cdp.send("IO.read", {"handle": stream_handle})
        assert_equal(tail_chunk.get("base64Encoded"), False, "Fetch response-stage stream tail encoding")
        assert_equal(tail_chunk.get("data"), "-stage body", "Fetch response-stage stream tail chunk")
        assert_equal(tail_chunk.get("eof"), True, "Fetch response-stage stream tail eof")

        await _expect_cdp_error(
            state.cdp.send("Fetch.continueResponse", {"requestId": request_id}),
            BODY_TAKEN_CONTINUE_RESPONSE_ERROR,
            "Fetch response-stage stream continueResponse after body taken",
        )
        await state.cdp.send(
            "Fetch.fulfillRequest",
            {
                "requestId": request_id,
                "responseCode": 200,
                "responseHeaders": [
                    {"name": "content-type", "value": "text/plain; charset=utf-8"},
                ],
                "body": base64.b64encode(b"response-stage body").decode("ascii"),
            },
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in state.subresource_events[network_start:]
            ),
            "Fetch response-stage stream fulfilled Network.loadingFinished",
        )
        await wait_until(
            lambda: page.evaluate("() => globalThis.__smokeFetchStreamResult === 'response-stage body'"),
            "Fetch response-stage stream fulfilled body",
        )
        await state.cdp.send("IO.close", {"handle": stream_handle})
        await _expect_cdp_error(
            state.cdp.send("IO.read", {"handle": stream_handle}),
            "StreamHandleNotFound",
            "Fetch response-stage stream read after close",
        )
        stream_handle = None
        state.record("response_stage_fetch_body_stream")
    finally:
        if stream_handle:
            await state.cdp.send("IO.close", {"handle": stream_handle})
        await state.cdp.send("Fetch.disable")


async def _verify_response_stage_fetch_binary_body(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    response_stage_events = attach_cdp_event_collector(state.cdp, ["Fetch.requestPaused"])
    response_stage_start = len(response_stage_events)
    network_start = len(state.subresource_events)
    await state.cdp.send(
        "Fetch.enable",
        {
            "patterns": [
                {
                    "urlPattern": "*/api-binary",
                    "requestStage": "Response",
                    "resourceType": "Fetch",
                }
            ]
        },
    )
    try:
        await page.evaluate(
            """() => {
              globalThis.__smokeBinaryResponseStageResult = 'pending';
              fetch('/api-binary')
                .then(async response => Array.from(new Uint8Array(await response.arrayBuffer())))
                .then(
                  bytes => { globalThis.__smokeBinaryResponseStageResult = bytes; },
                  error => { globalThis.__smokeBinaryResponseStageResult = `error:${error?.message || String(error)}`; }
                );
              return 'scheduled';
            }"""
        )

        paused: dict[str, Any] | None = None

        def saw_response_stage_pause() -> bool:
            nonlocal paused
            paused = next(
                (
                    event
                    for event in response_stage_events[response_stage_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("request", {}).get("url") == f"{fixture}/api-binary"
                    and event["params"].get("resourceType") == "XHR"
                    and event["params"].get("responseStatusCode") == 200
                ),
                None,
            )
            return paused is not None

        await wait_until(saw_response_stage_pause, "Fetch binary response-stage requestPaused")
        assert paused is not None
        request_id = paused["params"].get("requestId")
        network_id = paused["params"].get("networkId")
        if not isinstance(request_id, str) or not request_id:
            raise SmokeError(f"missing binary response-stage requestId: {paused}")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing binary response-stage networkId: {paused}")
        response_headers = paused["params"].get("responseHeaders") or []
        if not _header_list_contains(response_headers, "x-smoke-binary", "ok"):
            raise SmokeError(f"missing binary response-stage header: {paused}")
        assert_equal(
            await page.evaluate("() => globalThis.__smokeBinaryResponseStageResult"),
            "pending",
            "binary fetch response-stage should pause before body is delivered",
        )

        body = await state.cdp.send("Fetch.getResponseBody", {"requestId": request_id})
        assert_equal(body.get("base64Encoded"), True, "Fetch binary response-stage body encoding")
        assert_equal(body.get("body"), "AP9h", "Fetch binary response-stage base64 body")

        await state.cdp.send("Fetch.continueResponse", {"requestId": request_id})
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in state.subresource_events[network_start:]
            ),
            "Fetch binary response-stage Network.loadingFinished",
        )
        await wait_until(
            lambda: page.evaluate("() => JSON.stringify(globalThis.__smokeBinaryResponseStageResult) === '[0,255,97]'"),
            "Fetch binary response-stage continued bytes",
        )
        state.record("response_stage_fetch_binary_body")
    finally:
        await state.cdp.send("Fetch.disable")


async def _verify_response_stage_fetch_binary_body_stream(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    response_stage_events = attach_cdp_event_collector(state.cdp, ["Fetch.requestPaused"])
    response_stage_start = len(response_stage_events)
    network_start = len(state.subresource_events)
    stream_handle: str | None = None
    await state.cdp.send(
        "Fetch.enable",
        {
            "patterns": [
                {
                    "urlPattern": "*/api-binary",
                    "requestStage": "Response",
                    "resourceType": "Fetch",
                }
            ]
        },
    )
    try:
        await page.evaluate(
            """() => {
              globalThis.__smokeBinaryStreamResult = 'pending';
              fetch('/api-binary')
                .then(async response => Array.from(new Uint8Array(await response.arrayBuffer())))
                .then(
                  bytes => { globalThis.__smokeBinaryStreamResult = bytes; },
                  error => { globalThis.__smokeBinaryStreamResult = `error:${error?.message || String(error)}`; }
                );
              return 'scheduled';
            }"""
        )

        paused: dict[str, Any] | None = None

        def saw_response_stage_pause() -> bool:
            nonlocal paused
            paused = next(
                (
                    event
                    for event in response_stage_events[response_stage_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("request", {}).get("url") == f"{fixture}/api-binary"
                    and event["params"].get("resourceType") == "XHR"
                    and event["params"].get("responseStatusCode") == 200
                ),
                None,
            )
            return paused is not None

        await wait_until(saw_response_stage_pause, "Fetch binary response-stage stream requestPaused")
        assert paused is not None
        request_id = paused["params"].get("requestId")
        network_id = paused["params"].get("networkId")
        if not isinstance(request_id, str) or not request_id:
            raise SmokeError(f"missing binary response-stage stream requestId: {paused}")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing binary response-stage stream networkId: {paused}")
        assert_equal(
            await page.evaluate("() => globalThis.__smokeBinaryStreamResult"),
            "pending",
            "binary stream response-stage should pause before body is delivered",
        )

        stream = await state.cdp.send("Fetch.takeResponseBodyAsStream", {"requestId": request_id})
        stream_handle = stream.get("stream")
        if not isinstance(stream_handle, str) or not stream_handle:
            raise SmokeError(f"missing binary Fetch.takeResponseBodyAsStream handle: {stream}")
        chunk = await state.cdp.send("IO.read", {"handle": stream_handle})
        assert_equal(chunk.get("base64Encoded"), True, "Fetch binary response-stage stream encoding")
        assert_equal(chunk.get("data"), "AP9h", "Fetch binary response-stage stream base64 body")
        assert_equal(chunk.get("eof"), True, "Fetch binary response-stage stream eof")

        await _expect_cdp_error(
            state.cdp.send("Fetch.continueResponse", {"requestId": request_id}),
            BODY_TAKEN_CONTINUE_RESPONSE_ERROR,
            "Fetch binary response-stage stream continueResponse after body taken",
        )
        await state.cdp.send(
            "Fetch.fulfillRequest",
            {
                "requestId": request_id,
                "responseCode": 200,
                "responseHeaders": [
                    {"name": "content-type", "value": "application/octet-stream"},
                ],
                "body": "AP9h",
            },
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in state.subresource_events[network_start:]
            ),
            "Fetch binary response-stage stream fulfilled Network.loadingFinished",
        )
        await wait_until(
            lambda: page.evaluate("() => JSON.stringify(globalThis.__smokeBinaryStreamResult) === '[0,255,97]'"),
            "Fetch binary response-stage stream fulfilled bytes",
        )
        await state.cdp.send("IO.close", {"handle": stream_handle})
        stream_handle = None
        state.record("response_stage_fetch_binary_body_stream")
    finally:
        if stream_handle:
            await state.cdp.send("IO.close", {"handle": stream_handle})
        await state.cdp.send("Fetch.disable")


async def _verify_fetch_auth_challenge(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?realm=page-fetch-auth"
    fetch_events = attach_cdp_event_collector(state.cdp, ["Fetch.requestPaused", "Fetch.authRequired"])
    fetch_start = len(fetch_events)
    network_start = len(state.subresource_events)
    await state.cdp.send(
        "Fetch.enable",
        {
            "handleAuthRequests": True,
            "patterns": [
                {
                    "urlPattern": "*/api-auth*",
                    "requestStage": "Request",
                    "resourceType": "Fetch",
                }
            ],
        },
    )
    try:
        await page.evaluate(
            """() => {
              globalThis.__smokeFetchAuthResult = 'pending';
              fetch('/api-auth?realm=page-fetch-auth')
                .then(response => response.text())
                .then(
                  text => { globalThis.__smokeFetchAuthResult = text; },
                  error => { globalThis.__smokeFetchAuthResult = `error:${error?.message || String(error)}`; }
                );
              return 'scheduled';
            }"""
        )

        request_paused: dict[str, Any] | None = None

        def saw_request_pause() -> bool:
            nonlocal request_paused
            request_paused = next(
                (
                    event
                    for event in fetch_events[fetch_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("request", {}).get("url") == auth_url
                    and event["params"].get("resourceType") == "XHR"
                    and "responseStatusCode" not in event["params"]
                ),
                None,
            )
            return request_paused is not None

        await wait_until(saw_request_pause, "Fetch auth request-stage pause")
        assert request_paused is not None
        request_id = request_paused["params"].get("requestId")
        network_id = request_paused["params"].get("networkId")
        if not isinstance(request_id, str) or not request_id:
            raise SmokeError(f"missing auth Fetch requestId: {request_paused}")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing auth Fetch networkId: {request_paused}")
        assert_equal(
            await page.evaluate("() => globalThis.__smokeFetchAuthResult"),
            "pending",
            "fetch auth should pause before request continues",
        )

        await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})
        auth_required: dict[str, Any] | None = None

        def saw_auth_required() -> bool:
            nonlocal auth_required
            auth_required = next(
                (
                    event
                    for event in fetch_events[fetch_start:]
                    if event["method"] == "Fetch.authRequired"
                    and event["params"].get("requestId") == request_id
                ),
                None,
            )
            return auth_required is not None

        await wait_until(saw_auth_required, "Fetch.authRequired")
        assert auth_required is not None
        assert_equal(
            auth_required["params"].get("resourceType"),
            "XHR",
            "Fetch.authRequired resource type",
        )
        if "networkId" in auth_required["params"]:
            raise SmokeError(f"Fetch.authRequired must not expose networkId: {auth_required}")
        challenge = auth_required["params"].get("authChallenge") or {}
        assert_equal(challenge.get("source"), "Server", "Fetch auth challenge source")
        assert_equal(challenge.get("scheme"), "basic", "Fetch auth challenge scheme")
        assert_equal(challenge.get("realm"), "page-fetch-auth", "Fetch auth challenge realm")
        assert_equal(
            await page.evaluate("() => globalThis.__smokeFetchAuthResult"),
            "pending",
            "fetch auth should pause before credentials are provided",
        )

        await state.cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": request_id,
                "authChallengeResponse": {
                    "response": "ProvideCredentials",
                    "username": "user",
                    "password": "pass",
                },
            },
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.responseReceived"
                and event["params"].get("requestId") == network_id
                and event["params"].get("response", {}).get("status") == 200
                for event in state.subresource_events[network_start:]
            ),
            "authenticated Fetch Network.responseReceived",
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in state.subresource_events[network_start:]
            ),
            "authenticated Fetch Network.loadingFinished",
        )
        await wait_until(
            lambda: page.evaluate("() => globalThis.__smokeFetchAuthResult === 'authenticated fetch'"),
            "authenticated fetch result",
        )
        state.record("fetch_auth_challenge_continue")
    finally:
        await state.cdp.send("Fetch.disable")


async def _verify_fetch_auth_cancel(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?cancel=1&realm=page-fetch-cancel"
    fetch_events = attach_cdp_event_collector(
        state.cdp,
        ["Fetch.requestPaused", "Fetch.authRequired"],
    )
    fetch_start = len(fetch_events)
    network_start = len(state.subresource_events)
    await state.cdp.send(
        "Fetch.enable",
        {
            "handleAuthRequests": True,
            "patterns": [
                {
                    "urlPattern": "*/api-auth*",
                    "requestStage": "Request",
                    "resourceType": "Fetch",
                }
            ],
        },
    )
    try:
        await page.evaluate(
            """() => {
              globalThis.__smokeFetchAuthCancelResult = 'pending';
              fetch('/api-auth?cancel=1&realm=page-fetch-cancel')
                .then(async response =>
                  `resolved:${response.ok}:${response.status}:${await response.text()}`)
                .then(
                  result => { globalThis.__smokeFetchAuthCancelResult = result; },
                  error => {
                    globalThis.__smokeFetchAuthCancelResult =
                      `error:${error?.constructor?.name || 'Error'}:${error?.message || String(error)}`;
                  }
                );
              return 'scheduled';
            }"""
        )

        request_paused: dict[str, Any] | None = None

        def saw_request_pause() -> bool:
            nonlocal request_paused
            request_paused = next(
                (
                    event
                    for event in fetch_events[fetch_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("request", {}).get("url") == auth_url
                    and event["params"].get("resourceType") == "XHR"
                    and "responseStatusCode" not in event["params"]
                ),
                None,
            )
            return request_paused is not None

        await wait_until(saw_request_pause, "Fetch auth cancel request-stage pause")
        assert request_paused is not None
        request_id = request_paused["params"].get("requestId")
        network_id = request_paused["params"].get("networkId")
        if not isinstance(request_id, str) or not request_id:
            raise SmokeError(f"missing auth-cancel Fetch requestId: {request_paused}")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing auth-cancel Fetch networkId: {request_paused}")

        await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})
        auth_required: dict[str, Any] | None = None

        def saw_auth_required() -> bool:
            nonlocal auth_required
            auth_required = next(
                (
                    event
                    for event in fetch_events[fetch_start:]
                    if event["method"] == "Fetch.authRequired"
                    and event["params"].get("requestId") == request_id
                ),
                None,
            )
            return auth_required is not None

        await wait_until(saw_auth_required, "Fetch auth cancel authRequired")
        assert auth_required is not None
        assert_equal(
            auth_required["params"].get("resourceType"),
            "XHR",
            "Fetch auth cancel authRequired resource type",
        )
        if "networkId" in auth_required["params"]:
            raise SmokeError(f"Fetch.authRequired must not expose networkId: {auth_required}")
        await state.cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": request_id,
                "authChallengeResponse": {"response": "CancelAuth"},
            },
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.responseReceived"
                and event["params"].get("requestId") == network_id
                and event["params"].get("response", {}).get("status") == 401
                for event in state.subresource_events[network_start:]
            ),
            "Fetch auth cancel challenged Network.responseReceived",
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in state.subresource_events[network_start:]
            ),
            "Fetch auth cancel Network.loadingFinished",
        )
        await wait_until(
            lambda: page.evaluate(
                "() => globalThis.__smokeFetchAuthCancelResult === "
                "'resolved:false:401:auth required'"
            ),
            "Fetch auth cancel challenged response",
        )
        if any(
            event["method"] == "Network.loadingFailed"
            and event["params"].get("requestId") == network_id
            for event in state.subresource_events[network_start:]
        ):
            raise SmokeError("Fetch auth cancel must not emit Network.loadingFailed")
        state.record("fetch_auth_challenge_cancel")
    finally:
        await state.cdp.send("Fetch.disable")


async def _verify_fetch_auth_response_stage(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?response-stage=1&realm=page-fetch-response-stage"
    fetch_events = attach_cdp_event_collector(
        state.cdp,
        ["Fetch.requestPaused", "Fetch.authRequired"],
    )
    fetch_start = len(fetch_events)
    network_start = len(state.subresource_events)
    await state.cdp.send(
        "Fetch.enable",
        {
            "handleAuthRequests": True,
            "patterns": [
                {
                    "urlPattern": "*/api-auth*",
                    "requestStage": "Request",
                    "resourceType": "Fetch",
                }
            ],
        },
    )
    try:
        await page.evaluate(
            """() => {
              globalThis.__smokeFetchAuthResponseStageResult = 'pending';
              fetch('/api-auth?response-stage=1&realm=page-fetch-response-stage')
                .then(response => response.text())
                .then(
                  text => { globalThis.__smokeFetchAuthResponseStageResult = text; },
                  error => {
                    globalThis.__smokeFetchAuthResponseStageResult =
                      `error:${error?.constructor?.name || 'Error'}:${error?.message || String(error)}`;
                  }
                );
              return 'scheduled';
            }"""
        )

        request_paused: dict[str, Any] | None = None

        def saw_request_pause() -> bool:
            nonlocal request_paused
            request_paused = next(
                (
                    event
                    for event in fetch_events[fetch_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("request", {}).get("url") == auth_url
                    and event["params"].get("resourceType") == "XHR"
                    and "responseStatusCode" not in event["params"]
                ),
                None,
            )
            return request_paused is not None

        await wait_until(saw_request_pause, "Fetch auth response-stage request pause")
        assert request_paused is not None
        request_id = request_paused["params"].get("requestId")
        network_id = request_paused["params"].get("networkId")
        if not isinstance(request_id, str) or not request_id:
            raise SmokeError(f"missing auth response-stage Fetch requestId: {request_paused}")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing auth response-stage Fetch networkId: {request_paused}")
        assert_equal(
            await page.evaluate("() => globalThis.__smokeFetchAuthResponseStageResult"),
            "pending",
            "fetch auth response-stage should pause before request continues",
        )

        await state.cdp.send(
            "Fetch.continueRequest",
            {"requestId": request_id, "interceptResponse": True},
        )
        auth_required: dict[str, Any] | None = None

        def saw_auth_required() -> bool:
            nonlocal auth_required
            auth_required = next(
                (
                    event
                    for event in fetch_events[fetch_start:]
                    if event["method"] == "Fetch.authRequired"
                    and event["params"].get("requestId") == request_id
                ),
                None,
            )
            return auth_required is not None

        await wait_until(saw_auth_required, "Fetch auth response-stage authRequired")
        assert auth_required is not None
        assert_equal(
            auth_required["params"].get("resourceType"),
            "XHR",
            "Fetch auth response-stage authRequired resource type",
        )
        if "networkId" in auth_required["params"]:
            raise SmokeError(f"Fetch.authRequired must not expose networkId: {auth_required}")
        challenge = auth_required["params"].get("authChallenge") or {}
        assert_equal(challenge.get("source"), "Server", "Fetch auth response-stage challenge source")
        assert_equal(challenge.get("scheme"), "basic", "Fetch auth response-stage challenge scheme")
        assert_equal(
            challenge.get("realm"),
            "page-fetch-response-stage",
            "Fetch auth response-stage challenge realm",
        )
        assert_equal(
            await page.evaluate("() => globalThis.__smokeFetchAuthResponseStageResult"),
            "pending",
            "fetch auth response-stage should pause before credentials are provided",
        )

        await state.cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": request_id,
                "authChallengeResponse": {
                    "response": "ProvideCredentials",
                    "username": "user",
                    "password": "pass",
                },
            },
        )
        response_paused: dict[str, Any] | None = None

        def saw_response_pause() -> bool:
            nonlocal response_paused
            response_paused = next(
                (
                    event
                    for event in fetch_events[fetch_start:]
                    if event["method"] == "Fetch.requestPaused"
                    and event["params"].get("requestId") == request_id
                    and event["params"].get("networkId") == network_id
                    and event["params"].get("responseStatusCode") == 200
                ),
                None,
            )
            return response_paused is not None

        await wait_until(saw_response_pause, "Fetch auth response-stage response pause")
        assert response_paused is not None
        assert_equal(
            response_paused["params"].get("resourceType"),
            "XHR",
            "Fetch auth response-stage response resource type",
        )
        response_headers = response_paused["params"].get("responseHeaders") or []
        if not _header_list_contains(response_headers, "x-smoke-auth-stage", "ok"):
            raise SmokeError(f"Fetch auth response-stage missed authenticated header: {response_paused}")
        assert_equal(
            await page.evaluate("() => globalThis.__smokeFetchAuthResponseStageResult"),
            "pending",
            "fetch auth response-stage should remain pending before response continue",
        )

        body_result = await state.cdp.send("Fetch.getResponseBody", {"requestId": request_id})
        assert_equal(body_result.get("base64Encoded"), False, "Fetch auth response-stage body encoding")
        assert_equal(body_result.get("body"), "authenticated fetch", "Fetch auth response-stage paused body")

        await state.cdp.send("Fetch.continueResponse", {"requestId": request_id})
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in state.subresource_events[network_start:]
            ),
            "Fetch auth response-stage Network.loadingFinished",
        )
        await wait_until(
            lambda: page.evaluate("() => globalThis.__smokeFetchAuthResponseStageResult === 'authenticated fetch'"),
            "Fetch auth response-stage continued body",
        )
        state.record("fetch_auth_response_stage")
    finally:
        await state.cdp.send("Fetch.disable")


async def _verify_redirect_chain_network_events(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    redirect_start = len(subresource_events)

    fetch_result = await page.evaluate(
        """async () => await fetch('/api-redirect-start').then(response => response.json())"""
    )
    assert_equal(fetch_result.get("redirected"), True, "fetch redirect final body")
    assert_equal(fetch_result.get("method"), "GET", "fetch redirect method")

    xhr_result = await page.evaluate(
        """async () => await new Promise(resolve => {
          const xhr = new XMLHttpRequest();
          xhr.open('GET', '/api-redirect-start', true);
          xhr.onload = () => resolve(JSON.parse(xhr.responseText));
          xhr.send();
        })"""
    )
    assert_equal(xhr_result.get("redirected"), True, "xhr redirect final body")
    assert_equal(xhr_result.get("method"), "GET", "xhr redirect method")

    def saw_redirect_events() -> bool:
        events = subresource_events[redirect_start:]
        return _redirect_chain_complete(events, "Fetch", fixture) and _redirect_chain_complete(
            events, "XHR", fixture
        )

    await wait_until(saw_redirect_events, "Fetch/XHR redirect chain Network events")
    events = subresource_events[redirect_start:]
    _assert_redirect_chain_events(events, "Fetch", fixture, "fetch")
    _assert_redirect_chain_events(events, "XHR", fixture, "xhr")
    state.record("redirect_chain_network_events")


async def _verify_parser_script_network_events(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    parser_script_start = len(subresource_events)
    await page.goto(f"{fixture}/parser-script-page", wait_until="load", timeout=10_000)
    assert_equal(await page.evaluate("() => globalThis.__smokeParserScriptValue"), "parser script loaded", "parser script executed")
    parser_script_request_id: str | None = None

    def saw_parser_script_events() -> bool:
        nonlocal parser_script_request_id
        events = subresource_events[parser_script_start:]
        request = _find_request(events, "Script", f"{fixture}/parser-script.js")
        response = next(
            (
                event
                for event in events
                if event["method"] == "Network.responseReceived"
                and event["params"].get("type") == "Script"
                and event["params"].get("response", {}).get("url") == f"{fixture}/parser-script.js"
            ),
            None,
        )
        request_id = request and request["params"].get("requestId")
        if not request_id or not response:
            return False
        if not any(event["method"] == "Network.loadingFinished" and event["params"].get("requestId") == request_id for event in events):
            return False
        parser_script_request_id = request_id
        return True

    await wait_until(saw_parser_script_events, "parser Script Network events")
    body = await state.cdp.send("Network.getResponseBody", {"requestId": parser_script_request_id})
    assert_equal(body.get("body"), 'globalThis.__smokeParserScriptValue = "parser script loaded";', "parser script response body")
    state.record("parser_script_network_events")


async def _verify_parser_stylesheet_network_events(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    stylesheet_start = len(subresource_events)
    await page.goto(f"{fixture}/stylesheet-resource-page", wait_until="load", timeout=10_000)
    assert_equal(await page.text_content("#styled"), "stylesheet resource page", "stylesheet resource page text")

    link_request_id: str | None = None
    import_request_id: str | None = None

    def saw_stylesheet_events() -> bool:
        nonlocal link_request_id, import_request_id
        events = subresource_events[stylesheet_start:]
        link_request_id = _completed_request_id(events, "Stylesheet", f"{fixture}/resource-link.css")
        import_request_id = _completed_request_id(events, "Stylesheet", f"{fixture}/resource-import.css")
        return bool(link_request_id and import_request_id)

    await wait_until(saw_stylesheet_events, "parser Stylesheet Network events")
    link_body = await state.cdp.send("Network.getResponseBody", {"requestId": link_request_id})
    import_body = await state.cdp.send("Network.getResponseBody", {"requestId": import_request_id})
    assert_equal(link_body.get("body"), "main { color: rgb(12, 34, 56); }", "link stylesheet response body")
    assert_equal(import_body.get("body"), "main { background-color: rgb(210, 220, 230); }", "import stylesheet response body")
    state.record("parser_stylesheet_network_events")


async def _verify_parser_stylesheet_network_events_without_script_gate(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    stylesheet_start = len(subresource_events)
    await page.goto(f"{fixture}/stylesheet-resource-no-script-page", wait_until="load", timeout=10_000)
    assert_equal(await page.text_content("#styled"), "stylesheet resource page", "stylesheet no-script page text")

    link_request_id: str | None = None
    import_request_id: str | None = None

    def saw_stylesheet_events() -> bool:
        nonlocal link_request_id, import_request_id
        events = subresource_events[stylesheet_start:]
        link_request_id = _completed_request_id(events, "Stylesheet", f"{fixture}/resource-link.css")
        import_request_id = _completed_request_id(events, "Stylesheet", f"{fixture}/resource-import.css")
        return bool(link_request_id and import_request_id)

    await wait_until(saw_stylesheet_events, "parser Stylesheet Network events without script gate")
    link_body = await state.cdp.send("Network.getResponseBody", {"requestId": link_request_id})
    import_body = await state.cdp.send("Network.getResponseBody", {"requestId": import_request_id})
    assert_equal(link_body.get("body"), "main { color: rgb(12, 34, 56); }", "link stylesheet no-script response body")
    assert_equal(import_body.get("body"), "main { background-color: rgb(210, 220, 230); }", "import stylesheet no-script response body")
    state.record("parser_stylesheet_network_events_without_script_gate")


async def _verify_chromium_resource_type_network_matrix(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    resource_start = len(subresource_events)
    await page.goto(f"{fixture}/chromium-resource-type-page", wait_until="load", timeout=10_000)
    assert_equal(
        await page.evaluate("() => globalThis.__smokeChromiumResourceScript === true"),
        True,
        "chromium-derived resource script executed",
    )
    xhr_result = await page.evaluate("() => globalThis.__smokeResourceXhrDone")
    assert_equal(xhr_result.get("status"), 200, "chromium-derived resource XHR status")

    expected_types = {
        "/chromium-resource-type-page": "Document",
        "/chromium-resource-style.css": "Stylesheet",
        "/chromium-resource-script.js": "Script",
        "/chromium-resource-image.png": "Image",
        "/chromium-resource-audio.wav": "Media",
        "/chromium-resource-video.ogv": "Media",
        "/chromium-resource-captions.vtt": "TextTrack",
        "/chromium-resource-xhr.bin": "XHR",
    }
    request_ids: dict[str, str] = {}

    def saw_resource_type_matrix() -> bool:
        events = subresource_events[resource_start:]
        for path, resource_type in expected_types.items():
            request_id = _completed_request_id(events, resource_type, f"{fixture}{path}")
            if not request_id:
                return False
            request_ids[path] = request_id
        return True

    try:
        await wait_until(saw_resource_type_matrix, "Chromium-derived resource type Network matrix")
    except SmokeError as error:
        events = subresource_events[resource_start:]
        seen = [
            (
                event["method"],
                event["params"].get("type"),
                event["params"].get("request", {}).get("url")
                or event["params"].get("response", {}).get("url")
                or event["params"].get("requestId"),
            )
            for event in events
            if event["method"].startswith("Network.")
        ]
        missing = [
            (path, resource_type)
            for path, resource_type in expected_types.items()
            if not _completed_request_id(events, resource_type, f"{fixture}{path}")
        ]
        raise SmokeError(
            f"{error}; missing={missing!r}; seen={seen!r}"
        ) from error

    expected_bodies: dict[str, tuple[str, bool]] = {
        "/chromium-resource-style.css": ("main { color: rgb(31, 41, 59); }", False),
        "/chromium-resource-script.js": ("globalThis.__smokeChromiumResourceScript = true;", False),
        "/chromium-resource-image.png": (base64.b64encode(_transparent_png_bytes()).decode("ascii"), True),
        "/chromium-resource-audio.wav": (base64.b64encode(b"\x00\xffmoli-media").decode("ascii"), True),
        "/chromium-resource-video.ogv": (base64.b64encode(b"\x00\xffmoli-media").decode("ascii"), True),
        "/chromium-resource-captions.vtt": ("WEBVTT\n\n00:00.000 --> 00:01.000\ncaption\n", False),
        "/chromium-resource-xhr.bin": (base64.b64encode(b"\x00\xffmoli-xhr").decode("ascii"), True),
    }
    for path, (expected_body, expected_base64) in expected_bodies.items():
        body = await state.cdp.send("Network.getResponseBody", {"requestId": request_ids[path]})
        assert_equal(body.get("base64Encoded"), expected_base64, f"{path} Network.getResponseBody encoding")
        assert_equal(body.get("body"), expected_body, f"{path} Network.getResponseBody body")
    state.record("chromium_resource_type_network_matrix")


async def run_network_body_cache_group(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    small_url = f"{fixture}/api-response-body-budget-small"
    oversize_url = f"{fixture}/api-response-body-budget-oversize"

    await state.cdp.send(
        "Network.enable",
        {
            "maxTotalBufferSize": 20_000_000,
            "maxResourceBufferSize": 2_000_000,
        },
    )
    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    event_start = len(subresource_events)
    fetched = await page.evaluate(
        """async () => {
          const small = await fetch('/api-response-body-budget-small')
            .then(response => response.text());
          const oversize = await fetch('/api-response-body-budget-oversize')
            .then(response => response.arrayBuffer());
          return { small, oversizeLength: oversize.byteLength };
        }"""
    )
    assert_equal(
        fetched.get("small"),
        "bounded response body remains readable",
        "bounded response-body small page consumer",
    )
    assert_equal(
        fetched.get("oversizeLength"),
        2_000_001,
        "bounded response-body oversize page consumer",
    )

    request_ids: dict[str, str] = {}

    def saw_completed_requests() -> bool:
        events = subresource_events[event_start:]
        small_request_id = _completed_request_id(events, "Fetch", small_url)
        oversize_request_id = _completed_request_id(events, "Fetch", oversize_url)
        if not small_request_id or not oversize_request_id:
            return False
        request_ids["small"] = small_request_id
        request_ids["oversize"] = oversize_request_id
        return True

    await wait_until(
        saw_completed_requests,
        "bounded response-body Network terminal events",
    )
    small_body = await state.cdp.send(
        "Network.getResponseBody",
        {"requestId": request_ids["small"]},
    )
    assert_equal(
        small_body.get("body"),
        "bounded response body remains readable",
        "bounded response-body retained small body",
    )
    assert_equal(
        small_body.get("base64Encoded"),
        False,
        "bounded response-body retained small encoding",
    )

    eviction_error = "Request content was evicted from inspector cache"
    for attempt in range(2):
        await _expect_cdp_error(
            state.cdp.send(
                "Network.getResponseBody",
                {"requestId": request_ids["oversize"]},
            ),
            eviction_error,
            f"bounded response-body oversize eviction attempt {attempt + 1}",
        )
    state.record(
        "network_response_body_budget",
        {
            "maxTotalBufferSize": 20_000_000,
            "maxResourceBufferSize": 2_000_000,
        },
    )


def _transparent_png_bytes() -> bytes:
    return base64.b64decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII="
    )


async def _verify_blocked_websocket_events(state: SmokeState) -> None:
    page = state.page
    fixture = state.fixture
    subresource_events = state.subresource_events
    blocked_ws_url = fixture.replace("http:", "ws:", 1) + "/ws-blocked"
    blocked_ws_start = len(subresource_events)
    await state.cdp.send("Network.setBlockedURLs", {"urls": [blocked_ws_url + "*"]})
    blocked_ws_result = await page.evaluate(
        """async url => await new Promise(resolve => {
          const socket = new WebSocket(url);
          const timer = setTimeout(() => { socket.close(); resolve(`timeout:${socket.readyState}`); }, 5000);
          socket.onopen = () => { clearTimeout(timer); resolve('open'); };
          socket.onerror = () => { clearTimeout(timer); resolve(`error:${socket.readyState}`); };
          socket.onclose = event => { clearTimeout(timer); resolve(`close:${event.code}:${event.wasClean}`); };
        })""",
        blocked_ws_url,
    )
    if blocked_ws_result == "open" or str(blocked_ws_result).startswith("timeout:"):
        raise SmokeError(f"blocked WebSocket should fail, got {blocked_ws_result}")

    def saw_blocked_ws_events() -> bool:
        events = subresource_events[blocked_ws_start:]
        request = _find_request(events, "WebSocket", blocked_ws_url)
        request_id = request and request["params"].get("requestId")
        return bool(
            request_id
            and any(
                event["method"] == "Network.loadingFailed"
                and event["params"].get("requestId") == request_id
                and event["params"].get("errorText") == "net::ERR_BLOCKED_BY_CLIENT"
                for event in events
            )
        )

    await wait_until(saw_blocked_ws_events, "blocked WebSocket Network.loadingFailed")
    await state.cdp.send("Network.setBlockedURLs", {"urls": []})
    state.record("blocked_websocket_network_events")


def _find_request(events: list[dict[str, Any]], resource_type: str, url: str) -> dict[str, Any] | None:
    return next(
        (
            event
            for event in events
            if event["method"] == "Network.requestWillBeSent"
            and event["params"].get("type") == resource_type
            and event["params"].get("request", {}).get("url") == url
        ),
        None,
    )


def _completed_request_id(events: list[dict[str, Any]], resource_type: str, url: str) -> str | None:
    request = _find_request(events, resource_type, url)
    request_id = request and request["params"].get("requestId")
    if not request_id:
        return None
    response = next(
        (
            event
            for event in events
            if event["method"] == "Network.responseReceived"
            and event["params"].get("requestId") == request_id
            and event["params"].get("type") == resource_type
            and event["params"].get("response", {}).get("url") == url
        ),
        None,
    )
    if response is None:
        return None
    finished = any(
        event["method"] == "Network.loadingFinished" and event["params"].get("requestId") == request_id
        for event in events
    )
    return request_id if finished else None


def _redirect_chain_complete(events: list[dict[str, Any]], resource_type: str, fixture: str) -> bool:
    final_request = _find_request(events, resource_type, f"{fixture}/api-redirect-final")
    request_id = final_request and final_request["params"].get("requestId")
    return bool(
        request_id
        and _find_request(events, resource_type, f"{fixture}/api-redirect-start")
        and final_request["params"].get("redirectResponse")
        and _response_by_id(events, request_id)
        and any(
            event["method"] == "Network.loadingFinished"
            and event["params"].get("requestId") == request_id
            for event in events
        )
    )


def _assert_redirect_chain_events(events: list[dict[str, Any]], resource_type: str, fixture: str, label: str) -> None:
    start_request = _find_request(events, resource_type, f"{fixture}/api-redirect-start")
    final_request = _find_request(events, resource_type, f"{fixture}/api-redirect-final")
    if start_request is None or final_request is None:
        raise SmokeError(f"missing {label} redirect requestWillBeSent events")
    start_index = _event_index(events, start_request)
    final_index = _event_index(events, final_request)
    if not start_index < final_index:
        raise SmokeError(f"{label} redirect requestWillBeSent ordering regressed")
    request_id = final_request["params"].get("requestId")
    assert_equal(start_request["params"].get("requestId"), request_id, f"{label} redirect requestId continuity")
    redirect_response = final_request["params"].get("redirectResponse") or {}
    assert_equal(redirect_response.get("status"), 302, f"{label} redirect response status")
    assert_equal(redirect_response.get("url"), f"{fixture}/api-redirect-start", f"{label} redirect response url")
    assert_equal(_header_value(redirect_response.get("headers") or {}, "location"), "/api-redirect-final", f"{label} redirect location header")
    assert_equal(_header_value(redirect_response.get("headers") or {}, "x-smoke-redirect"), "start", f"{label} redirect custom header")

    response = _response_by_id(events, request_id)
    if response is None:
        raise SmokeError(f"missing {label} redirect final responseReceived")
    response_index = _event_index(events, response)
    if not final_index < response_index:
        raise SmokeError(f"{label} redirect final responseReceived ordering regressed")
    assert_equal(response["params"].get("type"), resource_type, f"{label} redirect final response type")
    assert_equal(response["params"].get("response", {}).get("url"), f"{fixture}/api-redirect-final", f"{label} redirect final response url")
    assert_equal(response["params"].get("response", {}).get("status"), 200, f"{label} redirect final response status")

    finished = next(
        (
            event
            for event in events
            if event["method"] == "Network.loadingFinished"
            and event["params"].get("requestId") == request_id
        ),
        None,
    )
    if finished is None:
        raise SmokeError(f"missing {label} redirect loadingFinished")
    if not response_index < _event_index(events, finished):
        raise SmokeError(f"{label} redirect loadingFinished ordering regressed")


def _event_index(events: list[dict[str, Any]], needle: dict[str, Any]) -> int:
    for index, event in enumerate(events):
        if event is needle:
            return index
    raise SmokeError("event was not collected from the supplied event list")


def _request_by_id(events: list[dict[str, Any]], request_id: str) -> dict[str, Any] | None:
    return next(
        (
            event
            for event in events
            if event["method"] == "Network.requestWillBeSent"
            and event["params"].get("requestId") == request_id
        ),
        None,
    )


def _response_by_id(events: list[dict[str, Any]], request_id: str) -> dict[str, Any] | None:
    return next(
        (
            event
            for event in events
            if event["method"] == "Network.responseReceived"
            and event["params"].get("requestId") == request_id
        ),
        None,
    )


def _assert_request_payload(
    event: dict[str, Any] | None,
    *,
    method: str,
    post_data: str,
    custom_header: str,
    label: str,
) -> None:
    request = (event or {}).get("params", {}).get("request", {})
    assert_equal(request.get("method"), method, f"{label} Network request method")
    assert_equal(request.get("hasPostData"), True, f"{label} Network request hasPostData")
    assert_equal(request.get("postData"), post_data, f"{label} Network request postData")
    headers = request.get("headers") or {}
    assert_equal(_header_value(headers, "x-smoke-post"), custom_header, f"{label} Network request custom header")
    assert_equal(_header_value(headers, "content-type"), "text/plain;charset=UTF-8", f"{label} Network request content-type")


def _assert_response_headers(event: dict[str, Any] | None, *, expected_kind: str, label: str) -> None:
    response = (event or {}).get("params", {}).get("response", {})
    headers = response.get("headers") or {}
    assert_equal(response.get("status"), 200, f"{label} Network response status")
    assert_equal(_header_value(headers, "x-smoke-response"), "header-visible", f"{label} Network response custom header")
    assert_equal(_header_value(headers, "x-smoke-request-kind"), expected_kind, f"{label} Network response request-derived header")
    content_type = _header_value(headers, "content-type")
    if content_type != "application/json; charset=utf-8":
        raise SmokeError(f"{label} Network response content-type mismatch: {content_type!r}")


def _header_value(headers: dict[str, Any], name: str) -> Any:
    expected = name.lower()
    for key, value in headers.items():
        if key.lower() == expected:
            return value
    return None


def _header_list_contains(headers: list[dict[str, Any]], name: str, value: str) -> bool:
    expected = name.lower()
    return any(
        str(header.get("name", "")).lower() == expected and header.get("value") == value
        for header in headers
    )


async def _expect_cdp_error(awaitable: Awaitable[Any], expected: str, label: str) -> None:
    try:
        await awaitable
    except Exception as error:
        message = str(error)
        if expected in message:
            return
        raise SmokeError(
            f"{label}: expected CDP error containing {expected!r}, got {message!r}"
        ) from error
    raise SmokeError(f"{label}: expected CDP error containing {expected!r}")
