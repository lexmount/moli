from __future__ import annotations

import asyncio
import json
from typing import Any

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..helpers import attach_cdp_event_collector, collect_worker_network_events, run_worker_command


async def run_workers_group(state: SmokeState) -> None:
    page = state.page
    context = state.context
    fixture = state.fixture

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    worker_result = await run_worker_command(page, "worker ping")
    assert_equal(worker_result.get("echoed"), "worker ping", "worker echoed message")
    assert_equal(worker_result.get("pathname"), "/worker.js", "worker location pathname")
    assert_equal(worker_result.get("selfEqualsGlobal"), True, "worker global self identity")
    state.record("worker_postmessage_round_trip")

    await _verify_shared_worker_postmessage_reuse(state)

    await context.route(
        "**/worker-route-fulfill",
        lambda route: route.fulfill(status=200, content_type="text/plain; charset=utf-8", body="worker fulfilled body"),
    )
    worker_fetch_fulfill = await run_worker_command(page, {"kind": "fetch", "url": "/worker-route-fulfill"})
    assert_equal(worker_fetch_fulfill.get("ok"), True, "worker fetch route fulfill ok")
    assert_equal(worker_fetch_fulfill.get("status"), 200, "worker fetch route fulfill status")
    assert_equal(worker_fetch_fulfill.get("text"), "worker fulfilled body", "worker fetch route fulfill body")
    state.record("worker_route_fulfill_fetch")

    async def continue_worker_xhr(route: Any) -> None:
        headers = dict(route.request.headers)
        headers["x-smoke-worker-route"] = "continued-from-worker"
        await route.continue_(headers=headers)

    await context.route("**/worker-route-continue", continue_worker_xhr)
    worker_xhr_continue = await run_worker_command(page, {"kind": "xhr", "url": "/worker-route-continue"})
    assert_equal(worker_xhr_continue.get("ok"), True, "worker xhr route continue ok")
    assert_equal(worker_xhr_continue.get("status"), 200, "worker xhr route continue status")
    assert_equal(
        worker_xhr_continue.get("text"),
        json.dumps({"method": "GET", "routeHeader": "continued-from-worker"}, separators=(",", ":")),
        "worker xhr route continue body",
    )
    state.record("worker_route_continue_xhr")

    await context.route("**/worker-route-abort", lambda route: route.abort("blockedbyclient"))
    worker_fetch_abort = await run_worker_command(page, {"kind": "fetch", "url": "/worker-route-abort"})
    assert_equal(worker_fetch_abort.get("ok"), False, "worker fetch route abort should reject")
    if not str(worker_fetch_abort.get("error", "")).startswith("TypeError:"):
        raise SmokeError(f"worker fetch route abort should reject with TypeError, got {worker_fetch_abort}")
    state.record("worker_route_abort_fetch")
    await context.unroute("**/worker-route-fulfill")
    await context.unroute("**/worker-route-continue")
    await context.unroute("**/worker-route-abort")

    async with collect_worker_network_events(state.endpoint, state.cdp) as network_events:
        await _verify_worker_fetch_auth_challenge(state, network_events)
        await _verify_worker_fetch_auth_cancel(state, network_events)
        await _verify_worker_fetch_auth_response_stage(state, network_events)
        await _verify_worker_xhr_auth_challenge(state, network_events)
        await _verify_worker_xhr_auth_cancel(state, network_events)
        await _verify_worker_xhr_auth_response_stage(state, network_events)


async def _verify_shared_worker_postmessage_reuse(state: SmokeState) -> None:
    result = await state.page.evaluate(
        """
        async ({ timeout }) => {
          const connect = label => new Promise((resolve, reject) => {
            const worker = new SharedWorker('/shared-worker.js?cdp-page-shared', 'cdp-page-shared-worker-smoke');
            globalThis.__cdpPageSharedWorkers = globalThis.__cdpPageSharedWorkers || [];
            globalThis.__cdpPageSharedWorkers.push(worker);
            const timer = setTimeout(() => reject(new Error(`shared worker ${label} timeout`)), timeout);
            worker.port.onmessage = event => {
              clearTimeout(timer);
              resolve({ label, data: event.data });
            };
            worker.port.start();
            worker.port.postMessage({ kind: 'probe', value: label });
          });
          const ports = await Promise.all([connect('first'), connect('second')]);
          return { ports };
        }
        """,
        {"timeout": 5_000},
    )
    ports = result.get("ports")
    if not isinstance(ports, list) or len(ports) != 2:
        raise SmokeError(f"shared worker page probe should return two ports: {result}")
    first = ports[0].get("data") if isinstance(ports[0], dict) else None
    second = ports[1].get("data") if isinstance(ports[1], dict) else None
    assert_equal(
        first.get("kind") if isinstance(first, dict) else None,
        "probe-result",
        "shared worker first probe kind",
    )
    assert_equal(
        second.get("kind") if isinstance(second, dict) else None,
        "probe-result",
        "shared worker second probe kind",
    )
    assert_equal(
        first.get("echoed") if isinstance(first, dict) else None,
        "first",
        "shared worker first probe echo",
    )
    assert_equal(
        second.get("echoed") if isinstance(second, dict) else None,
        "second",
        "shared worker second probe echo",
    )
    assert_equal(
        second.get("connectionCount") if isinstance(second, dict) else None,
        2,
        "shared worker named instance reuse",
    )
    assert_equal(
        second.get("isSharedWorker") if isinstance(second, dict) else None,
        True,
        "shared worker global scope",
    )
    state.record("shared_worker_postmessage_reuse")


async def _enable_worker_auth_interception(state: SmokeState, resource_type: str) -> None:
    await state.cdp.send(
        "Fetch.enable",
        {
            "handleAuthRequests": True,
            "patterns": [
                {
                    "urlPattern": "*/api-auth*",
                    "requestStage": "Request",
                    "resourceType": resource_type,
                }
            ],
        },
    )


def _assert_worker_task_pending(worker_task: asyncio.Task[Any], label: str) -> None:
    if worker_task.done():
        raise SmokeError(f"{label} settled too early: {worker_task.result()!r}")


async def _wait_worker_auth_request_pause(
    fetch_events: list[dict[str, Any]],
    fetch_start: int,
    auth_url: str,
    resource_type: str,
    label: str,
) -> tuple[str, str]:
    request_paused: dict[str, Any] | None = None

    def saw_request_pause() -> bool:
        nonlocal request_paused
        request_paused = next(
            (
                event
                for event in fetch_events[fetch_start:]
                if event["method"] == "Fetch.requestPaused"
                and event["params"].get("request", {}).get("url") == auth_url
                and event["params"].get("resourceType") == resource_type
                and "responseStatusCode" not in event["params"]
            ),
            None,
        )
        return request_paused is not None

    await wait_until(saw_request_pause, f"{label} request-stage pause")
    assert request_paused is not None
    request_id = request_paused["params"].get("requestId")
    network_id = request_paused["params"].get("networkId")
    if not isinstance(request_id, str) or not request_id:
        raise SmokeError(f"missing {label} Fetch requestId: {request_paused}")
    if not isinstance(network_id, str) or not network_id:
        raise SmokeError(f"missing {label} Fetch networkId: {request_paused}")
    return request_id, network_id


async def _wait_worker_auth_required(
    fetch_events: list[dict[str, Any]],
    fetch_start: int,
    request_id: str,
    label: str,
) -> dict[str, Any]:
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

    await wait_until(saw_auth_required, f"{label} authRequired")
    assert auth_required is not None
    assert_equal(
        auth_required["params"].get("resourceType"),
        "XHR",
        f"{label} Fetch.authRequired resource type",
    )
    if "networkId" in auth_required["params"]:
        raise SmokeError(f"{label} Fetch.authRequired must not expose networkId: {auth_required}")
    return auth_required


async def _wait_worker_auth_response_pause(
    fetch_events: list[dict[str, Any]],
    fetch_start: int,
    request_id: str,
    network_id: str,
    label: str,
) -> dict[str, Any]:
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

    await wait_until(saw_response_pause, f"{label} response pause")
    assert response_paused is not None
    assert_equal(
        response_paused["params"].get("resourceType"),
        "XHR",
        f"{label} response-stage Fetch resource type",
    )
    response_headers = response_paused["params"].get("responseHeaders") or []
    if not any(
        str(header.get("name", "")).lower() == "x-smoke-auth-stage" and header.get("value") == "ok"
        for header in response_headers
    ):
        raise SmokeError(f"{label} missed authenticated response header: {response_paused}")
    return response_paused


async def _provide_worker_auth_credentials(state: SmokeState, request_id: str) -> None:
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


async def _cancel_worker_auth(state: SmokeState, request_id: str) -> None:
    await state.cdp.send(
        "Fetch.continueWithAuth",
        {
            "requestId": request_id,
            "authChallengeResponse": {"response": "CancelAuth"},
        },
    )


async def _verify_worker_fetch_auth_challenge(
    state: SmokeState, network_events: list[dict[str, Any]],
) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?worker=1&realm=worker-fetch-auth"
    fetch_events = attach_cdp_event_collector(
        state.cdp,
        ["Fetch.requestPaused", "Fetch.authRequired"],
    )
    fetch_start = len(fetch_events)
    network_start = len(network_events)
    await _enable_worker_auth_interception(state, "Fetch")
    worker_task = asyncio.create_task(
        run_worker_command(
            page,
            {"kind": "fetch", "url": "/api-auth?worker=1&realm=worker-fetch-auth"},
            timeout_ms=20_000,
        )
    )
    try:
        request_id, network_id = await _wait_worker_auth_request_pause(
            fetch_events, fetch_start, auth_url, "XHR", "worker Fetch auth"
        )
        _assert_worker_task_pending(worker_task, "worker auth fetch before CDP continue")

        await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})
        auth_required = await _wait_worker_auth_required(
            fetch_events, fetch_start, request_id, "worker Fetch"
        )
        challenge = auth_required["params"].get("authChallenge") or {}
        assert_equal(challenge.get("source"), "Server", "worker Fetch auth challenge source")
        assert_equal(challenge.get("scheme"), "basic", "worker Fetch auth challenge scheme")
        assert_equal(challenge.get("realm"), "worker-fetch-auth", "worker Fetch auth challenge realm")
        _assert_worker_task_pending(worker_task, "worker auth fetch before credentials were provided")

        await _provide_worker_auth_credentials(state, request_id)
        result = await asyncio.wait_for(worker_task, timeout=10)
        assert_equal(result.get("ok"), True, "worker auth fetch ok")
        assert_equal(result.get("status"), 200, "worker auth fetch status")
        assert_equal(result.get("text"), "authenticated fetch", "worker auth fetch body")
        await wait_until(
            lambda: any(
                event["method"] == "Network.responseReceived"
                and event["params"].get("requestId") == network_id
                and event["params"].get("response", {}).get("status") == 200
                for event in network_events[network_start:]
            ),
            "worker authenticated Fetch Network.responseReceived",
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in network_events[network_start:]
            ),
            "worker authenticated Fetch Network.loadingFinished",
        )
        state.record("worker_fetch_auth_challenge_continue")
    finally:
        if not worker_task.done():
            worker_task.cancel()
        await state.cdp.send("Fetch.disable")


async def _verify_worker_fetch_auth_cancel(
    state: SmokeState, network_events: list[dict[str, Any]],
) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?worker-cancel=1&realm=worker-fetch-cancel"
    fetch_events = attach_cdp_event_collector(
        state.cdp,
        ["Fetch.requestPaused", "Fetch.authRequired"],
    )
    fetch_start = len(fetch_events)
    network_start = len(network_events)
    await _enable_worker_auth_interception(state, "Fetch")
    worker_task = asyncio.create_task(
        run_worker_command(
            page,
            {"kind": "fetch", "url": "/api-auth?worker-cancel=1&realm=worker-fetch-cancel"},
            timeout_ms=20_000,
        )
    )
    try:
        request_id, network_id = await _wait_worker_auth_request_pause(
            fetch_events, fetch_start, auth_url, "XHR", "worker Fetch auth cancel"
        )
        _assert_worker_task_pending(worker_task, "worker auth-cancel fetch before CDP continue")

        await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})
        await _wait_worker_auth_required(
            fetch_events, fetch_start, request_id, "worker Fetch auth cancel"
        )
        await _cancel_worker_auth(state, request_id)
        result = await asyncio.wait_for(worker_task, timeout=10)
        assert_equal(result.get("ok"), False, "worker auth cancel fetch Response.ok")
        assert_equal(result.get("status"), 401, "worker auth cancel fetch status")
        assert_equal(result.get("text"), "auth required", "worker auth cancel fetch body")
        await wait_until(
            lambda: any(
                event["method"] == "Network.responseReceived"
                and event["params"].get("requestId") == network_id
                and event["params"].get("response", {}).get("status") == 401
                for event in network_events[network_start:]
            ),
            "worker auth cancel Network.responseReceived",
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in network_events[network_start:]
            ),
            "worker auth cancel Network.loadingFinished",
        )
        if any(
            event["method"] == "Network.loadingFailed"
            and event["params"].get("requestId") == network_id
            for event in network_events[network_start:]
        ):
            raise SmokeError("worker auth cancel Fetch must not emit Network.loadingFailed")
        state.record("worker_fetch_auth_challenge_cancel")
    finally:
        if not worker_task.done():
            worker_task.cancel()
        await state.cdp.send("Fetch.disable")


async def _verify_worker_fetch_auth_response_stage(
    state: SmokeState, network_events: list[dict[str, Any]],
) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?worker-response-stage=1&realm=worker-fetch-response-stage"
    fetch_events = attach_cdp_event_collector(
        state.cdp,
        ["Fetch.requestPaused", "Fetch.authRequired"],
    )
    fetch_start = len(fetch_events)
    network_start = len(network_events)
    await _enable_worker_auth_interception(state, "Fetch")
    worker_task = asyncio.create_task(
        run_worker_command(
            page,
            {"kind": "fetch", "url": "/api-auth?worker-response-stage=1&realm=worker-fetch-response-stage"},
            timeout_ms=20_000,
        )
    )
    try:
        request_id, network_id = await _wait_worker_auth_request_pause(
            fetch_events, fetch_start, auth_url, "XHR", "worker Fetch auth response-stage"
        )
        _assert_worker_task_pending(worker_task, "worker auth response-stage fetch before CDP continue")

        await state.cdp.send(
            "Fetch.continueRequest",
            {"requestId": request_id, "interceptResponse": True},
        )
        await _wait_worker_auth_required(
            fetch_events, fetch_start, request_id, "worker Fetch auth response-stage"
        )
        await _provide_worker_auth_credentials(state, request_id)
        await _wait_worker_auth_response_pause(
            fetch_events, fetch_start, request_id, network_id, "worker Fetch auth response-stage"
        )
        _assert_worker_task_pending(worker_task, "worker auth response-stage fetch before response continue")

        await state.cdp.send("Fetch.continueResponse", {"requestId": request_id})
        result = await asyncio.wait_for(worker_task, timeout=10)
        assert_equal(result.get("ok"), True, "worker auth response-stage fetch ok")
        assert_equal(result.get("status"), 200, "worker auth response-stage fetch status")
        assert_equal(result.get("text"), "authenticated fetch", "worker auth response-stage fetch body")
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in network_events[network_start:]
            ),
            "worker auth response-stage Network.loadingFinished",
        )
        state.record("worker_fetch_auth_response_stage")
    finally:
        if not worker_task.done():
            worker_task.cancel()
        await state.cdp.send("Fetch.disable")


async def _verify_worker_xhr_auth_challenge(
    state: SmokeState, network_events: list[dict[str, Any]],
) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?worker-xhr=1&realm=worker-xhr-auth"
    fetch_events = attach_cdp_event_collector(
        state.cdp,
        ["Fetch.requestPaused", "Fetch.authRequired"],
    )
    fetch_start = len(fetch_events)
    network_start = len(network_events)
    await _enable_worker_auth_interception(state, "XHR")
    worker_task = asyncio.create_task(
        run_worker_command(
            page,
            {"kind": "xhr", "url": "/api-auth?worker-xhr=1&realm=worker-xhr-auth"},
            timeout_ms=20_000,
        )
    )
    try:
        request_id, network_id = await _wait_worker_auth_request_pause(
            fetch_events, fetch_start, auth_url, "XHR", "worker XHR auth"
        )
        _assert_worker_task_pending(worker_task, "worker XHR auth before CDP continue")

        await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})
        auth_required = await _wait_worker_auth_required(
            fetch_events, fetch_start, request_id, "worker XHR"
        )
        challenge = auth_required["params"].get("authChallenge") or {}
        assert_equal(challenge.get("source"), "Server", "worker XHR auth challenge source")
        assert_equal(challenge.get("scheme"), "basic", "worker XHR auth challenge scheme")
        assert_equal(challenge.get("realm"), "worker-xhr-auth", "worker XHR auth challenge realm")
        _assert_worker_task_pending(worker_task, "worker XHR auth before credentials were provided")

        await _provide_worker_auth_credentials(state, request_id)
        result = await asyncio.wait_for(worker_task, timeout=10)
        assert_equal(result.get("ok"), True, "worker XHR auth ok")
        assert_equal(result.get("status"), 200, "worker XHR auth status")
        assert_equal(result.get("text"), "authenticated fetch", "worker XHR auth body")
        await wait_until(
            lambda: any(
                event["method"] == "Network.responseReceived"
                and event["params"].get("requestId") == network_id
                and event["params"].get("response", {}).get("status") == 200
                for event in network_events[network_start:]
            ),
            "worker authenticated XHR Network.responseReceived",
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in network_events[network_start:]
            ),
            "worker authenticated XHR Network.loadingFinished",
        )
        state.record("worker_xhr_auth_challenge_continue")
    finally:
        if not worker_task.done():
            worker_task.cancel()
        await state.cdp.send("Fetch.disable")


async def _verify_worker_xhr_auth_cancel(
    state: SmokeState, network_events: list[dict[str, Any]],
) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?worker-xhr-cancel=1&realm=worker-xhr-cancel"
    fetch_events = attach_cdp_event_collector(
        state.cdp,
        ["Fetch.requestPaused", "Fetch.authRequired"],
    )
    fetch_start = len(fetch_events)
    network_start = len(network_events)
    await _enable_worker_auth_interception(state, "XHR")
    worker_task = asyncio.create_task(
        run_worker_command(
            page,
            {"kind": "xhr", "url": "/api-auth?worker-xhr-cancel=1&realm=worker-xhr-cancel"},
            timeout_ms=20_000,
        )
    )
    try:
        request_id, network_id = await _wait_worker_auth_request_pause(
            fetch_events, fetch_start, auth_url, "XHR", "worker XHR auth cancel"
        )
        _assert_worker_task_pending(worker_task, "worker XHR auth-cancel before CDP continue")

        await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})
        await _wait_worker_auth_required(
            fetch_events, fetch_start, request_id, "worker XHR auth cancel"
        )
        await _cancel_worker_auth(state, request_id)
        result = await asyncio.wait_for(worker_task, timeout=10)
        assert_equal(result.get("ok"), True, "worker XHR auth cancel should load")
        assert_equal(result.get("status"), 401, "worker XHR auth cancel status")
        assert_equal(result.get("text"), "auth required", "worker XHR auth cancel body")
        await wait_until(
            lambda: any(
                event["method"] == "Network.responseReceived"
                and event["params"].get("requestId") == network_id
                and event["params"].get("response", {}).get("status") == 401
                for event in network_events[network_start:]
            ),
            "worker XHR auth cancel Network.responseReceived",
        )
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in network_events[network_start:]
            ),
            "worker XHR auth cancel Network.loadingFinished",
        )
        if any(
            event["method"] == "Network.loadingFailed"
            and event["params"].get("requestId") == network_id
            for event in network_events[network_start:]
        ):
            raise SmokeError("worker XHR auth cancel must not emit Network.loadingFailed")
        state.record("worker_xhr_auth_challenge_cancel")
    finally:
        if not worker_task.done():
            worker_task.cancel()
        await state.cdp.send("Fetch.disable")


async def _verify_worker_xhr_auth_response_stage(
    state: SmokeState, network_events: list[dict[str, Any]],
) -> None:
    page = state.page
    fixture = state.fixture
    auth_url = f"{fixture}/api-auth?worker-xhr-response-stage=1&realm=worker-xhr-response-stage"
    fetch_events = attach_cdp_event_collector(
        state.cdp,
        ["Fetch.requestPaused", "Fetch.authRequired"],
    )
    fetch_start = len(fetch_events)
    network_start = len(network_events)
    await _enable_worker_auth_interception(state, "XHR")
    worker_task = asyncio.create_task(
        run_worker_command(
            page,
            {"kind": "xhr", "url": "/api-auth?worker-xhr-response-stage=1&realm=worker-xhr-response-stage"},
            timeout_ms=20_000,
        )
    )
    try:
        request_id, network_id = await _wait_worker_auth_request_pause(
            fetch_events, fetch_start, auth_url, "XHR", "worker XHR auth response-stage"
        )
        _assert_worker_task_pending(worker_task, "worker XHR auth response-stage before CDP continue")

        await state.cdp.send(
            "Fetch.continueRequest",
            {"requestId": request_id, "interceptResponse": True},
        )
        await _wait_worker_auth_required(
            fetch_events, fetch_start, request_id, "worker XHR auth response-stage"
        )
        await _provide_worker_auth_credentials(state, request_id)
        await _wait_worker_auth_response_pause(
            fetch_events, fetch_start, request_id, network_id, "worker XHR auth response-stage"
        )
        _assert_worker_task_pending(worker_task, "worker XHR auth response-stage before response continue")

        await state.cdp.send("Fetch.continueResponse", {"requestId": request_id})
        result = await asyncio.wait_for(worker_task, timeout=10)
        assert_equal(result.get("ok"), True, "worker XHR auth response-stage ok")
        assert_equal(result.get("status"), 200, "worker XHR auth response-stage status")
        assert_equal(result.get("text"), "authenticated fetch", "worker XHR auth response-stage body")
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == network_id
                for event in network_events[network_start:]
            ),
            "worker XHR auth response-stage Network.loadingFinished",
        )
        state.record("worker_xhr_auth_response_stage")
    finally:
        if not worker_task.done():
            worker_task.cancel()
        await state.cdp.send("Fetch.disable")
