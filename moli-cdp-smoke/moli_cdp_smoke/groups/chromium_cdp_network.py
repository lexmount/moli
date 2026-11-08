from __future__ import annotations
import asyncio
import base64
from contextlib import suppress
from typing import Any

from urllib.parse import urlsplit
from ..diagnostics import new_cdp_session
from ..navigation import navigate_cdp
from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..config import is_moli_browser
from ..fixture import FixtureServer
from ..helpers import attach_cdp_event_collector, run_worker_command

from .chromium_cdp_support import _alternate_loopback_origin, _has_event


def _header_value(headers: dict[str, Any], name: str) -> Any:
    return next(
        (value for header_name, value in headers.items() if header_name.lower() == name.lower()),
        None,
    )

def _network_events_for_request(
    events: list[dict[str, Any]], method: str, request_id: str
) -> list[dict[str, Any]]:
    return [
        event
        for event in events
        if event.get("method") == method
        and event.get("params", {}).get("requestId") == request_id
    ]

def _assert_single_successful_transport_extra_info(
    events: list[dict[str, Any]],
    request_id: str,
    expected_host: str,
    label: str,
    *,
    expected_cookie_header: str | None = None,
) -> None:
    requests = _network_events_for_request(events, "Network.requestWillBeSent", request_id)
    assert_equal(len(requests), 1, f"{label} browser-visible request count")

    request_extra = _network_events_for_request(
        events, "Network.requestWillBeSentExtraInfo", request_id
    )
    assert_equal(len(request_extra), 1, f"{label} request ExtraInfo count")
    request_pause = next(
        (
            event
            for event in events
            if event.get("method") == "Fetch.requestPaused"
            and event.get("params", {}).get("networkId") == request_id
            and "responseStatusCode" not in event.get("params", {})
        ),
        None,
    )
    if request_pause is None:
        raise SmokeError(f"{label} missing request-stage Fetch.requestPaused")
    request_headers = request_extra[0].get("params", {}).get("headers") or {}
    assert_equal(
        _header_value(request_headers, "host"),
        expected_host,
        f"{label} raw Host header",
    )
    assert_equal(
        _header_value(request_headers, "authorization"),
        None,
        f"{label} must expose the initial unauthenticated request headers",
    )
    if expected_cookie_header is not None:
        browser_visible_headers = (
            requests[0].get("params", {}).get("request", {}).get("headers") or {}
        )
        assert_equal(
            _header_value(browser_visible_headers, "cookie"),
            None,
            f"{label} browser-visible request must omit transport Cookie header",
        )
        raw_cookie_header = _header_value(request_headers, "cookie")
        if not isinstance(raw_cookie_header, str) or expected_cookie_header not in raw_cookie_header:
            raise SmokeError(
                f"{label} raw Cookie header missing {expected_cookie_header!r}: "
                f"{raw_cookie_header!r}"
            )

    response_extra = _network_events_for_request(
        events, "Network.responseReceivedExtraInfo", request_id
    )
    assert_equal(len(response_extra), 1, f"{label} response ExtraInfo count")
    assert_equal(
        response_extra[0].get("params", {}).get("statusCode"),
        200,
        f"{label} raw response status",
    )

    responses = _network_events_for_request(events, "Network.responseReceived", request_id)
    assert_equal(len(responses), 1, f"{label} browser-visible response count")
    assert_equal(
        responses[0].get("params", {}).get("response", {}).get("status"),
        200,
        f"{label} browser-visible response status",
    )
    assert_equal(
        responses[0].get("params", {}).get("hasExtraInfo"),
        True,
        f"{label} response hasExtraInfo",
    )
    assert_equal(
        len(_network_events_for_request(events, "Network.loadingFinished", request_id)),
        1,
        f"{label} loadingFinished count",
    )
    assert_equal(
        len(_network_events_for_request(events, "Network.loadingFailed", request_id)),
        0,
        f"{label} loadingFailed count",
    )

async def _start_intercepted_fetch(
    state: SmokeState,
    events: list[dict[str, Any]],
    event_start: int,
    url: str,
    result_name: str,
    label: str,
) -> tuple[str, str]:
    await state.page.evaluate(
        """({ url, resultName }) => {
          globalThis[resultName] = 'pending';
          fetch(url)
            .then(response => response.text())
            .then(
              text => { globalThis[resultName] = text; },
              error => { globalThis[resultName] = `error:${String(error)}`; }
            );
          return 'scheduled';
        }""",
        {"url": url, "resultName": result_name},
    )

    request_pause: dict[str, Any] | None = None

    def saw_request_pause() -> bool:
        nonlocal request_pause
        request_pause = next(
            (
                event
                for event in events[event_start:]
                if event.get("method") == "Fetch.requestPaused"
                and event.get("params", {}).get("request", {}).get("url") == url
                and "responseStatusCode" not in event.get("params", {})
            ),
            None,
        )
        return request_pause is not None

    await wait_until(saw_request_pause, f"{label} request pause")
    assert request_pause is not None
    assert_equal(
        request_pause.get("params", {}).get("resourceType"),
        "XHR",
        f"{label} Fetch-domain resource type",
    )
    request_id = request_pause.get("params", {}).get("requestId")
    network_id = request_pause.get("params", {}).get("networkId")
    if not isinstance(request_id, str) or not request_id:
        raise SmokeError(f"{label} missing requestId: {request_pause}")
    if not isinstance(network_id, str) or not network_id:
        raise SmokeError(f"{label} missing networkId: {request_pause}")
    return request_id, network_id

async def _wait_for_successful_fetch_extra_info(
    state: SmokeState,
    events: list[dict[str, Any]],
    event_start: int,
    network_id: str,
    url: str,
    result_name: str,
    expected_body: str,
    label: str,
    *,
    expected_cookie_header: str | None = None,
) -> None:
    await wait_until(
        lambda: len(
            _network_events_for_request(
                events[event_start:], "Network.responseReceivedExtraInfo", network_id
            )
        )
        == 1
        and len(
            _network_events_for_request(
                events[event_start:], "Network.loadingFinished", network_id
            )
        )
        == 1,
        f"{label} network completion",
    )
    await wait_until(
        lambda: state.page.evaluate(
            "({ resultName, expectedBody }) => globalThis[resultName] === expectedBody",
            {"resultName": result_name, "expectedBody": expected_body},
        ),
        f"{label} body",
    )
    _assert_single_successful_transport_extra_info(
        events[event_start:],
        network_id,
        urlsplit(url).netloc,
        label,
        expected_cookie_header=expected_cookie_header,
    )

async def _verify_chromium_page_lifecycle_order(state: SmokeState) -> None:
    events = attach_cdp_event_collector(
        state.cdp,
        ["Page.frameNavigated", "Page.domContentEventFired", "Page.loadEventFired"],
    )
    await state.cdp.send("Page.enable")
    start = len(events)
    navigation = await navigate_cdp(state.cdp, f"{state.fixture}/chromium-cdp-lifecycle-page")
    committed = next(index for index, event in enumerate(events[start:], start)
                     if event["method"] == "Page.frameNavigated"
                     and event["params"].get("frame", {}).get("id") == navigation["frameId"]
                     and event["params"].get("frame", {}).get("loaderId") == navigation["loaderId"])
    await wait_until(
        lambda: _has_event(events[committed:], "Page.domContentEventFired")
        and _has_event(events[committed:], "Page.loadEventFired"),
        "requested Document Page.domContentEventFired/Page.loadEventFired sample",
    )
    methods = [event["method"] for event in events[committed:]]
    dom_index = methods.index("Page.domContentEventFired")
    load_index = methods.index("Page.loadEventFired")
    if dom_index > load_index:
        raise SmokeError(f"Page.domContentEventFired should precede Page.loadEventFired: {methods}")
    state.record("chromium_page_lifecycle_order")

async def _verify_chromium_main_document_network_extra_info_sample(state: SmokeState) -> None:
    observed_methods = [
        "Network.requestWillBeSent",
        "Network.requestWillBeSentExtraInfo",
        "Network.responseReceivedExtraInfo",
        "Network.responseReceived",
        "Network.loadingFinished",
        "Network.loadingFailed",
    ]
    events = attach_cdp_event_collector(state.cdp, observed_methods)
    await state.cdp.send("Network.enable")
    fixture = _alternate_loopback_origin(state.fixture)

    async def navigate_and_events(
        url: str, expected_exchange_count: int, *, reload: bool = False
    ) -> list[dict[str, Any]]:
        start = len(events)
        if reload:
            await state.cdp.send("Page.reload", {"ignoreCache": False})
        else:
            await state.cdp.send("Page.navigate", {"url": url})

        def completed() -> bool:
            initial_request = next(
                (
                    event
                    for event in events[start:]
                    if event.get("method") == "Network.requestWillBeSent"
                    and event.get("params", {}).get("type") == "Document"
                    and event.get("params", {}).get("request", {}).get("url") == url
                ),
                None,
            )
            if initial_request is None:
                return False
            request_id = initial_request.get("params", {}).get("requestId")
            request_events = [
                event
                for event in events[start:]
                if event.get("params", {}).get("requestId") == request_id
            ]
            terminal_arrived = any(
                event.get("method") in {"Network.loadingFinished", "Network.loadingFailed"}
                for event in request_events
            )
            request_extra_count = sum(
                event.get("method") == "Network.requestWillBeSentExtraInfo"
                for event in request_events
            )
            response_extra_count = sum(
                event.get("method") == "Network.responseReceivedExtraInfo"
                for event in request_events
            )
            return (
                terminal_arrived
                and request_extra_count >= expected_exchange_count
                and response_extra_count >= expected_exchange_count
            )

        await wait_until(completed, f"main-document Network ExtraInfo completion for {url}")
        return events[start:]

    def request_events(navigation_events: list[dict[str, Any]], initial_url: str) -> list[dict[str, Any]]:
        initial_request = next(
            (
                event
                for event in navigation_events
                if event.get("method") == "Network.requestWillBeSent"
                and event.get("params", {}).get("type") == "Document"
                and event.get("params", {}).get("request", {}).get("url") == initial_url
            ),
            None,
        )
        if initial_request is None:
            raise SmokeError(f"missing Document requestWillBeSent for {initial_url}: {navigation_events}")
        request_id = initial_request.get("params", {}).get("requestId")
        return [
            event
            for event in navigation_events
            if event.get("params", {}).get("requestId") == request_id
        ]

    plain_url = f"{fixture}/plain?chromium-main-document-extra-info"
    plain = request_events(await navigate_and_events(plain_url, 1), plain_url)
    assert_equal(
        [
            event.get("method")
            for event in plain
            if event.get("method") == "Network.requestWillBeSentExtraInfo"
        ],
        ["Network.requestWillBeSentExtraInfo"],
        "normal Document request ExtraInfo count",
    )
    assert_equal(
        [
            event.get("method")
            for event in plain
            if event.get("method") == "Network.responseReceivedExtraInfo"
        ],
        ["Network.responseReceivedExtraInfo"],
        "normal Document response ExtraInfo count",
    )
    plain_response = next(event for event in plain if event.get("method") == "Network.responseReceived")
    assert_equal(
        plain_response.get("params", {}).get("hasExtraInfo"),
        True,
        "normal Document response hasExtraInfo",
    )
    plain_request_extra = next(
        event
        for event in plain
        if event.get("method") == "Network.requestWillBeSentExtraInfo"
    )
    assert_equal(
        plain_request_extra.get("params", {}).get("associatedCookies"),
        [],
        "normal no-cookie Document request ExtraInfo",
    )
    plain_request_headers = plain_request_extra.get("params", {}).get("headers") or {}
    assert_equal(
        _header_value(plain_request_headers, "Host"),
        urlsplit(fixture).netloc,
        "normal Document transport-generated Host header",
    )
    accept_encoding = _header_value(plain_request_headers, "Accept-Encoding")
    if not isinstance(accept_encoding, str) or not accept_encoding:
        raise SmokeError(
            f"normal Document request ExtraInfo missing transport Accept-Encoding: {plain_request_headers}"
        )
    plain_response_extra = next(
        event
        for event in plain
        if event.get("method") == "Network.responseReceivedExtraInfo"
    )
    assert_equal(
        plain_response_extra.get("params", {}).get("blockedCookies"),
        [],
        "normal no-cookie Document response ExtraInfo",
    )

    redirect_url = f"{fixture}/redirect-start"
    redirected = request_events(await navigate_and_events(redirect_url, 2), redirect_url)
    redirect_requests = [
        event for event in redirected if event.get("method") == "Network.requestWillBeSent"
    ]
    assert_equal(
        len(redirect_requests),
        2,
        "redirected Document request count",
    )
    assert_equal(
        len(
            [
                event
                for event in redirected
                if event.get("method") == "Network.requestWillBeSentExtraInfo"
            ]
        ),
        2,
        "redirected Document request ExtraInfo count",
    )
    assert_equal(
        len(
            [
                event
                for event in redirected
                if event.get("method") == "Network.responseReceivedExtraInfo"
            ]
        ),
        2,
        "redirected Document response ExtraInfo count",
    )
    redirect_responses = [
        event.get("method")
        for event in redirected
        if event.get("method") == "Network.responseReceived"
    ]
    assert_equal(
        redirect_responses,
        ["Network.responseReceived"],
        "redirected Document final response count",
    )
    redirected_request = redirect_requests[1]
    assert_equal(
        redirected_request.get("params", {}).get("redirectHasExtraInfo"),
        True,
        "HTTP redirect redirectHasExtraInfo",
    )
    status_codes = [
        event.get("params", {}).get("statusCode")
        for event in redirected
        if event.get("method") == "Network.responseReceivedExtraInfo"
    ]
    assert_equal(status_codes, [302, 200], "redirect response ExtraInfo status sequence")
    final_response = next(
        event for event in redirected if event.get("method") == "Network.responseReceived"
    )
    assert_equal(
        final_response.get("params", {}).get("hasExtraInfo"),
        True,
        "redirect final response hasExtraInfo",
    )
    redirect_request_extras = [
        event
        for event in redirected
        if event.get("method") == "Network.requestWillBeSentExtraInfo"
    ]
    assert_equal(
        [event.get("params", {}).get("associatedCookies") for event in redirect_request_extras],
        [[], []],
        "redirect no-cookie request ExtraInfo sequence",
    )

    revalidation_url = f"{fixture}/chromium-network-revalidate"
    await navigate_and_events(revalidation_url, 1)
    revalidated = request_events(
        await navigate_and_events(revalidation_url, 1, reload=True), revalidation_url
    )
    revalidation_request_extra = next(
        event
        for event in revalidated
        if event.get("method") == "Network.requestWillBeSentExtraInfo"
    )
    assert_equal(
        _header_value(
            revalidation_request_extra.get("params", {}).get("headers") or {},
            "If-None-Match",
        ),
        '"smoke-v1"',
        "revalidation request ExtraInfo conditional header",
    )
    revalidation_response_extra = next(
        event
        for event in revalidated
        if event.get("method") == "Network.responseReceivedExtraInfo"
    )
    assert_equal(
        revalidation_response_extra.get("params", {}).get("statusCode"),
        304,
        "revalidation raw response ExtraInfo status",
    )
    assert_equal(
        _header_value(
            revalidation_response_extra.get("params", {}).get("headers") or {},
            "X-Smoke-Raw-Revalidation",
        ),
        "yes",
        "revalidation raw response ExtraInfo header",
    )
    revalidation_response = next(
        event for event in revalidated if event.get("method") == "Network.responseReceived"
    )
    assert_equal(
        revalidation_response.get("params", {}).get("response", {}).get("status"),
        200,
        "revalidation merged response status",
    )
    state.record("chromium_main_document_network_extra_info_sample")

async def _verify_chromium_cookie_blocked_reason_sample(state: SmokeState) -> None:
    cookie_name = "chromium_cdp_private_path"
    cookie_url = f"{state.fixture}/private/index.html"
    request_url = f"{state.fixture}/plain?chromium-cookie-blocked-reason"
    events = attach_cdp_event_collector(
        state.cdp,
        ["Network.requestWillBeSent", "Network.requestWillBeSentExtraInfo"],
    )
    await state.cdp.send("Network.enable")

    try:
        set_result = await state.cdp.send(
            "Network.setCookie",
            {
                "name": cookie_name,
                "value": "private-value",
                "url": cookie_url,
                "path": "/private",
            },
        )
        assert_equal(set_result.get("success"), True, "path-scoped cookie setup")

        start = len(events)
        await state.cdp.send("Page.navigate", {"url": request_url})

        def matching_extra_info() -> dict[str, Any] | None:
            request = next(
                (
                    event
                    for event in events[start:]
                    if event.get("method") == "Network.requestWillBeSent"
                    and event.get("params", {}).get("type") == "Document"
                    and event.get("params", {}).get("request", {}).get("url") == request_url
                ),
                None,
            )
            if request is None:
                return None
            request_id = request.get("params", {}).get("requestId")
            return next(
                (
                    event
                    for event in events[start:]
                    if event.get("method") == "Network.requestWillBeSentExtraInfo"
                    and event.get("params", {}).get("requestId") == request_id
                ),
                None,
            )

        await wait_until(
            lambda: matching_extra_info() is not None,
            "path-mismatched cookie request ExtraInfo",
        )
        request_extra = matching_extra_info()
        if request_extra is None:
            raise SmokeError("missing path-mismatched cookie request ExtraInfo")
        associated = request_extra.get("params", {}).get("associatedCookies") or []
        matching_cookie = next(
            (
                item
                for item in associated
                if item.get("cookie", {}).get("name") == cookie_name
            ),
            None,
        )
        if matching_cookie is None:
            raise SmokeError(f"missing path-mismatched associated cookie: {associated!r}")
        assert_equal(
            matching_cookie.get("blockedReasons"),
            ["NotOnPath"],
            "Network.CookieBlockedReason path projection",
        )
        # ExtraInfo precedes the navigation terminal. Fence the shared page at
        # load so the next Chromium sample cannot race this Page.navigate.
        await state.page.wait_for_url(request_url, wait_until="load", timeout=10_000)
        state.record("chromium_cookie_blocked_reason_sample")
    finally:
        await state.cdp.send(
            "Network.deleteCookies",
            {
                "name": cookie_name,
                "url": cookie_url,
                "path": "/private",
            },
        )

async def _verify_chromium_fetch_continuation_extra_info_sample(state: SmokeState) -> None:
    fixture = state.fixture
    await state.page.goto(f"{fixture}/chromium-cdp-lifecycle-page")

    observed_methods = [
        "Fetch.requestPaused",
        "Fetch.authRequired",
        "Network.requestWillBeSent",
        "Network.requestWillBeSentExtraInfo",
        "Network.responseReceivedExtraInfo",
        "Network.responseReceived",
        "Network.loadingFinished",
        "Network.loadingFailed",
    ]
    events = attach_cdp_event_collector(state.cdp, observed_methods)

    continued_url = f"{fixture}/api-response-stage?chromium-fetch-extra-info=1"
    continued_cookie = "chromiumFetchExtraInfoCookie=present"
    await state.page.evaluate(
        "cookie => { document.cookie = `${cookie}; Path=/api-response-stage`; }",
        continued_cookie,
    )
    await state.cdp.send(
        "Fetch.enable",
        {
            "patterns": [
                {
                    "urlPattern": "*/api-response-stage?chromium-fetch-extra-info=1",
                    "requestStage": "Request",
                    "resourceType": "Fetch",
                }
            ]
        },
    )
    try:
        continued_start = len(events)
        continued_request_id, continued_network_id = await _start_intercepted_fetch(
            state,
            events,
            continued_start,
            continued_url,
            "__chromiumFetchExtraInfo",
            "Chromium continued Fetch",
        )
        await state.cdp.send(
            "Fetch.continueRequest", {"requestId": continued_request_id}
        )
        await _wait_for_successful_fetch_extra_info(
            state,
            events,
            continued_start,
            continued_network_id,
            continued_url,
            "__chromiumFetchExtraInfo",
            "response-stage body",
            "Chromium continued Fetch",
            expected_cookie_header=continued_cookie,
        )
        state.record("chromium_fetch_continue_request_extra_info_sample")
    finally:
        await state.cdp.send("Fetch.disable")
    cancel_url = f"{fixture}/api-auth?realm=chromium-fetch-cancel"
    await state.cdp.send(
        "Fetch.enable",
        {
            "handleAuthRequests": True,
            "patterns": [
                {
                    "urlPattern": "*/api-auth?realm=chromium-fetch-cancel",
                    "requestStage": "Request",
                    "resourceType": "Fetch",
                }
            ],
        },
    )
    try:
        cancel_start = len(events)
        cancel_request_id, cancel_network_id = await _start_intercepted_fetch(
            state,
            events,
            cancel_start,
            cancel_url,
            "__chromiumFetchAuthCancel",
            "Chromium canceled authentication",
        )
        await state.cdp.send("Fetch.continueRequest", {"requestId": cancel_request_id})

        def cancel_auth_challenge() -> dict[str, Any] | None:
            return next(
                (
                    event
                    for event in events[cancel_start:]
                    if event.get("method") == "Fetch.authRequired"
                    and event.get("params", {}).get("requestId") == cancel_request_id
                ),
                None,
            )

        await wait_until(cancel_auth_challenge, "Chromium canceled authentication challenge")
        await state.cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": cancel_request_id,
                "authChallengeResponse": {"response": "CancelAuth"},
            },
        )
        await wait_until(
            lambda: len(
                _network_events_for_request(
                    events[cancel_start:], "Network.loadingFinished", cancel_network_id
                )
            )
            == 1,
            "Chromium canceled authentication network completion",
        )
        await wait_until(
            lambda: state.page.evaluate(
                "() => globalThis.__chromiumFetchAuthCancel === 'auth required'"
            ),
            "Chromium canceled authentication response body",
        )
        responses = _network_events_for_request(
            events[cancel_start:], "Network.responseReceived", cancel_network_id
        )
        assert_equal(len(responses), 1, "Chromium canceled authentication response count")
        assert_equal(
            responses[0].get("params", {}).get("response", {}).get("status"),
            401,
            "Chromium canceled authentication response status",
        )
        assert_equal(
            len(
                _network_events_for_request(
                    events[cancel_start:], "Network.loadingFailed", cancel_network_id
                )
            ),
            0,
            "Chromium canceled authentication failure count",
        )
        state.record("chromium_fetch_cancel_auth_response_sample")
    finally:
        await state.cdp.send("Fetch.disable")
        await state.page.evaluate(
            "name => { document.cookie = `${name}=; Path=/api-response-stage; Max-Age=0`; }",
            "chromiumFetchExtraInfoCookie",
        )

    auth_url = f"{fixture}/api-auth?realm=chromium-fetch-extra-info"
    await state.cdp.send(
        "Fetch.enable",
        {
            "handleAuthRequests": True,
            "patterns": [
                {
                    "urlPattern": "*/api-auth?realm=chromium-fetch-extra-info",
                    "requestStage": "Request",
                    "resourceType": "Fetch",
                }
            ],
        },
    )
    try:
        auth_start = len(events)
        auth_request_id, auth_network_id = await _start_intercepted_fetch(
            state,
            events,
            auth_start,
            auth_url,
            "__chromiumFetchAuthExtraInfo",
            "Chromium authenticated Fetch",
        )
        await state.cdp.send("Fetch.continueRequest", {"requestId": auth_request_id})

        def auth_challenges() -> list[dict[str, Any]]:
            # Chromium's Fetch.authRequired has no networkId. Keep the networkId
            # captured from requestPaused and correlate auth rounds by requestId.
            return [
                event
                for event in events[auth_start:]
                if event.get("method") == "Fetch.authRequired"
                and event.get("params", {}).get("requestId") == auth_request_id
            ]

        await wait_until(lambda: len(auth_challenges()) == 1, "Chromium first auth challenge")
        assert_equal(
            auth_challenges()[0].get("params", {}).get("resourceType"),
            "XHR",
            "Chromium first auth challenge resource type",
        )
        if "networkId" in auth_challenges()[0].get("params", {}):
            raise SmokeError(f"Chromium Fetch.authRequired exposed networkId: {auth_challenges()[0]}")
        first_challenge = auth_challenges()[0].get("params", {}).get("authChallenge") or {}
        assert_equal(
            str(first_challenge.get("scheme", "")).lower(),
            "basic",
            "Chromium auth challenge scheme",
        )
        assert_equal(
            first_challenge.get("realm"),
            "chromium-fetch-extra-info",
            "Chromium auth challenge realm",
        )

        await state.cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": auth_request_id,
                "authChallengeResponse": {
                    "response": "ProvideCredentials",
                    "username": "wrong",
                    "password": "credentials",
                },
            },
        )
        await wait_until(lambda: len(auth_challenges()) == 2, "Chromium second auth challenge")
        assert_equal(
            auth_challenges()[1].get("params", {}).get("resourceType"),
            "XHR",
            "Chromium second auth challenge resource type",
        )
        await state.cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": auth_request_id,
                "authChallengeResponse": {
                    "response": "ProvideCredentials",
                    "username": "user",
                    "password": "pass",
                },
            },
        )
        await _wait_for_successful_fetch_extra_info(
            state,
            events,
            auth_start,
            auth_network_id,
            auth_url,
            "__chromiumFetchAuthExtraInfo",
            "authenticated fetch",
            "Chromium authenticated Fetch",
        )
        state.record("chromium_fetch_auth_extra_info_sample")
    finally:
        await state.cdp.send("Fetch.disable")

async def _verify_chromium_fetch_cancel_auth_response_stage_sample(
    state: SmokeState,
) -> None:
    await state.page.goto(f"{state.fixture}/chromium-cdp-lifecycle-page")

    url = f"{state.fixture}/api-auth?realm=chromium-fetch-cancel-response-stage"
    methods = [
        "Fetch.requestPaused",
        "Fetch.authRequired",
        "Network.responseReceived",
        "Network.loadingFinished",
        "Network.loadingFailed",
    ]
    events = attach_cdp_event_collector(state.cdp, methods)
    await state.cdp.send(
        "Fetch.enable",
        {
            "handleAuthRequests": True,
            "patterns": [
                {"urlPattern": url, "requestStage": "Request"},
                {"urlPattern": url, "requestStage": "Response"},
            ],
        },
    )
    try:
        start = len(events)
        request_id, network_id = await _start_intercepted_fetch(
            state,
            events,
            start,
            url,
            "__chromiumFetchAuthCancelResponseStage",
            "Chromium canceled authentication response stage",
        )
        await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})
        try:
            await wait_until(
                lambda: any(
                    event.get("method") == "Fetch.authRequired"
                    and event.get("params", {}).get("requestId") == request_id
                    for event in events[start:]
                ),
                "Chromium response-stage authentication challenge",
            )
        except SmokeError as error:
            raise SmokeError(f"{error}; observed events: {events[start:]}") from error
        await state.cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": request_id,
                "authChallengeResponse": {"response": "CancelAuth"},
            },
        )

        response_pause: dict[str, Any] | None = None

        def saw_response_pause() -> bool:
            nonlocal response_pause
            response_pause = next(
                (
                    event
                    for event in events[start:]
                    if event.get("method") == "Fetch.requestPaused"
                    and event.get("params", {}).get("requestId") == request_id
                    and event.get("params", {}).get("responseStatusCode") == 401
                ),
                None,
            )
            return response_pause is not None

        await wait_until(saw_response_pause, "Chromium canceled 401 response-stage pause")
        assert_equal(
            await state.page.evaluate(
                "() => globalThis.__chromiumFetchAuthCancelResponseStage"
            ),
            "pending",
            "Chromium canceled response must remain paused",
        )
        body = await state.cdp.send("Fetch.getResponseBody", {"requestId": request_id})
        encoded_body = body.get("body", "")
        if body.get("base64Encoded"):
            decoded_body = base64.b64decode(encoded_body).decode()
        else:
            decoded_body = encoded_body
        assert_equal(
            decoded_body,
            "auth required",
            "Chromium canceled response-stage body",
        )
        await state.cdp.send("Fetch.continueResponse", {"requestId": request_id})
        await wait_until(
            lambda: state.page.evaluate(
                "() => globalThis.__chromiumFetchAuthCancelResponseStage === 'auth required'"
            ),
            "Chromium canceled response-stage fetch result",
        )
        await wait_until(
            lambda: len(
                _network_events_for_request(
                    events[start:], "Network.loadingFinished", network_id
                )
            )
            == 1,
            "Chromium canceled response-stage network completion",
        )
        assert_equal(
            len(_network_events_for_request(events[start:], "Network.loadingFailed", network_id)),
            0,
            "Chromium canceled response-stage failure count",
        )
        state.record("chromium_fetch_cancel_auth_response_stage_sample")
    finally:
        await state.cdp.send("Fetch.disable")

async def _verify_chromium_worker_cancel_auth_response_sample(state: SmokeState) -> None:
    await state.page.goto(f"{state.fixture}/plain")

    methods = [
        "Fetch.requestPaused",
        "Fetch.authRequired",
        "Network.requestWillBeSentExtraInfo",
        "Network.responseReceivedExtraInfo",
    ]
    events = attach_cdp_event_collector(state.cdp, methods)

    async def verify(kind: str) -> None:
        realm = f"chromium-worker-{kind}-cancel"
        relative_url = f"/api-auth?realm={realm}"
        url = f"{state.fixture}{relative_url}"
        start = len(events)
        await state.cdp.send(
            "Fetch.enable",
            {
                "handleAuthRequests": True,
                "patterns": [{"urlPattern": url, "requestStage": "Request"}],
            },
        )
        worker_task = asyncio.create_task(
            run_worker_command(
                state.page,
                {"kind": kind, "url": relative_url},
                timeout_ms=20_000,
            )
        )
        try:
            request_pause: dict[str, Any] | None = None

            def saw_request_pause() -> bool:
                nonlocal request_pause
                request_pause = next(
                    (
                        event
                        for event in events[start:]
                        if event.get("method") == "Fetch.requestPaused"
                        and event.get("params", {}).get("request", {}).get("url") == url
                        and "responseStatusCode" not in event.get("params", {})
                    ),
                    None,
                )
                return request_pause is not None

            await wait_until(saw_request_pause, f"Chromium worker {kind} auth request pause")
            assert request_pause is not None
            request_id = request_pause.get("params", {}).get("requestId")
            network_id = request_pause.get("params", {}).get("networkId")
            if not isinstance(request_id, str) or not isinstance(network_id, str):
                raise SmokeError(f"invalid Chromium worker {kind} auth pause: {request_pause}")
            await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})
            await wait_until(
                lambda: any(
                    event.get("method") == "Fetch.authRequired"
                    and event.get("params", {}).get("requestId") == request_id
                    for event in events[start:]
                ),
                f"Chromium worker {kind} authentication challenge",
            )
            await state.cdp.send(
                "Fetch.continueWithAuth",
                {
                    "requestId": request_id,
                    "authChallengeResponse": {"response": "CancelAuth"},
                },
            )
            result = await asyncio.wait_for(worker_task, timeout=10)
            assert_equal(result.get("status"), 401, f"Chromium worker {kind} canceled status")
            assert_equal(
                result.get("text"),
                "auth required",
                f"Chromium worker {kind} canceled response body",
            )
            assert_equal(
                result.get("ok"),
                kind == "xhr",
                f"Chromium worker {kind} completion kind",
            )
            await wait_until(
                lambda: len(
                    _network_events_for_request(
                        events[start:], "Network.requestWillBeSentExtraInfo", network_id
                    )
                )
                == 1
                and len(
                    _network_events_for_request(
                        events[start:], "Network.responseReceivedExtraInfo", network_id
                    )
                )
                == 1,
                f"Chromium worker {kind} canceled auth ExtraInfo",
            )
            request_extra = _network_events_for_request(
                events[start:], "Network.requestWillBeSentExtraInfo", network_id
            )[0]
            request_headers = request_extra.get("params", {}).get("headers") or {}
            assert_equal(
                _header_value(request_headers, "host"),
                urlsplit(url).netloc,
                f"Chromium worker {kind} canceled auth raw Host header",
            )
            assert_equal(
                _header_value(request_headers, "authorization"),
                None,
                f"Chromium worker {kind} canceled auth initial request",
            )
            response_extra = _network_events_for_request(
                events[start:], "Network.responseReceivedExtraInfo", network_id
            )[0]
            assert_equal(
                response_extra.get("params", {}).get("statusCode"),
                401,
                f"Chromium worker {kind} canceled auth raw response status",
            )
        finally:
            if not worker_task.done():
                worker_task.cancel()
            await state.cdp.send("Fetch.disable")

    await verify("fetch")
    await verify("xhr")
    state.record("chromium_worker_cancel_auth_response_sample")

async def _verify_chromium_worker_auth_extra_info_sample(state: SmokeState) -> None:
    methods = [
        "Fetch.requestPaused",
        "Fetch.authRequired",
        "Network.requestWillBeSentExtraInfo",
        "Network.responseReceivedExtraInfo",
    ]
    events = attach_cdp_event_collector(state.cdp, methods)

    async def verify(kind: str) -> None:
        fixture = FixtureServer()
        fixture.start()
        origin = fixture.url
        await state.page.goto(f"{origin}/plain")

        realm = f"chromium-worker-{kind}-extra-info"
        relative_url = f"/api-auth?realm={realm}"
        url = f"{origin}{relative_url}"
        start = len(events)
        worker_task: asyncio.Task[Any] | None = None
        patterns = [{"urlPattern": url, "requestStage": "Request"}]
        if kind == "xhr":
            patterns.append({"urlPattern": url, "requestStage": "Response"})
        await state.cdp.send(
            "Fetch.enable",
            {
                "handleAuthRequests": True,
                "patterns": patterns,
            },
        )
        try:
            worker_task = asyncio.create_task(
                run_worker_command(
                    state.page,
                    {"kind": kind, "url": relative_url},
                    timeout_ms=20_000,
                )
            )
            request_pause: dict[str, Any] | None = None

            def saw_request_pause() -> bool:
                nonlocal request_pause
                request_pause = next(
                    (
                        event
                        for event in events[start:]
                        if event.get("method") == "Fetch.requestPaused"
                        and event.get("params", {}).get("request", {}).get("url") == url
                        and "responseStatusCode" not in event.get("params", {})
                    ),
                    None,
                )
                return request_pause is not None

            await wait_until(saw_request_pause, f"Chromium worker {kind} auth request pause")
            assert request_pause is not None
            request_id = request_pause.get("params", {}).get("requestId")
            network_id = request_pause.get("params", {}).get("networkId")
            if not isinstance(request_id, str) or not isinstance(network_id, str):
                raise SmokeError(f"invalid Chromium worker {kind} auth pause: {request_pause}")
            assert_equal(
                request_pause.get("params", {}).get("resourceType"),
                "XHR",
                f"Chromium worker {kind} auth Fetch-domain resource type",
            )
            await state.cdp.send("Fetch.continueRequest", {"requestId": request_id})

            def auth_required() -> dict[str, Any] | None:
                return next(
                    (
                        event
                        for event in events[start:]
                        if event.get("method") == "Fetch.authRequired"
                        and event.get("params", {}).get("requestId") == request_id
                    ),
                    None,
                )

            await wait_until(auth_required, f"Chromium worker {kind} authentication challenge")
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
            if kind == "xhr":
                response_pause: dict[str, Any] | None = None

                def saw_response_pause() -> bool:
                    nonlocal response_pause
                    response_pause = next(
                        (
                            event
                            for event in events[start:]
                            if event.get("method") == "Fetch.requestPaused"
                            and event.get("params", {}).get("requestId") == request_id
                            and event.get("params", {}).get("networkId") == network_id
                            and event.get("params", {}).get("responseStatusCode") == 200
                        ),
                        None,
                    )
                    return response_pause is not None

                await wait_until(
                    saw_response_pause,
                    "Chromium worker XHR auth response-stage pause",
                )
                if worker_task.done():
                    raise SmokeError(
                        f"Chromium worker XHR completed before continueResponse: "
                        f"{worker_task.result()!r}"
                    )
                await state.cdp.send("Fetch.continueResponse", {"requestId": request_id})
            result = await asyncio.wait_for(worker_task, timeout=10)
            assert_equal(result.get("status"), 200, f"Chromium worker {kind} auth status")
            assert_equal(
                result.get("text"),
                "authenticated fetch",
                f"Chromium worker {kind} auth body",
            )
            await wait_until(
                lambda: len(
                    _network_events_for_request(
                        events[start:], "Network.requestWillBeSentExtraInfo", network_id
                    )
                )
                == 1
                and len(
                    _network_events_for_request(
                        events[start:], "Network.responseReceivedExtraInfo", network_id
                    )
                )
                == 1,
                f"Chromium worker {kind} auth ExtraInfo",
            )
            request_extra = _network_events_for_request(
                events[start:], "Network.requestWillBeSentExtraInfo", network_id
            )[0]
            request_headers = request_extra.get("params", {}).get("headers") or {}
            assert_equal(
                _header_value(request_headers, "host"),
                urlsplit(url).netloc,
                f"Chromium worker {kind} auth raw Host header",
            )
            assert_equal(
                _header_value(request_headers, "authorization"),
                None,
                f"Chromium worker {kind} auth must expose the initial request",
            )
            response_extra = _network_events_for_request(
                events[start:], "Network.responseReceivedExtraInfo", network_id
            )[0]
            assert_equal(
                response_extra.get("params", {}).get("statusCode"),
                200,
                f"Chromium worker {kind} auth raw response status",
            )
        finally:
            if worker_task is not None and not worker_task.done():
                worker_task.cancel()
            await state.cdp.send("Fetch.disable")
            fixture.stop()

    # Chromium caches successful HTTP auth credentials by origin. Dedicated
    # fixture ports keep both cases and all later auth smokes independent.
    await verify("fetch")
    await verify("xhr")
    await state.page.goto(f"{state.fixture}/plain")

    state.record("chromium_worker_auth_extra_info_sample")

async def _verify_chromium_navigation_cancel_auth_response_sample(
    state: SmokeState,
) -> None:
    page = state.page
    cdp = state.cdp
    await page.goto(f"{state.fixture}/plain")

    url = f"{state.fixture}/api-auth?realm=chromium-navigation-cancel"
    methods = [
        "Fetch.requestPaused",
        "Fetch.authRequired",
        "Network.responseReceived",
        "Network.loadingFinished",
        "Network.loadingFailed",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Page.frameStoppedLoading",
    ]
    events = attach_cdp_event_collector(cdp, methods)
    await cdp.send("Network.enable")
    await cdp.send(
        "Fetch.enable",
        {
            "handleAuthRequests": True,
            "patterns": [
                {"urlPattern": url, "requestStage": "Request"},
                {"urlPattern": url, "requestStage": "Response"},
            ],
        },
    )
    navigation: asyncio.Task[Any] | None = None
    sample_completed = False
    try:
        navigation = asyncio.create_task(page.goto(url, wait_until="load", timeout=10_000))
        request_pause: dict[str, Any] | None = None

        def saw_request_pause() -> bool:
            nonlocal request_pause
            request_pause = next(
                (
                    event
                    for event in events
                    if event.get("method") == "Fetch.requestPaused"
                    and event.get("params", {}).get("request", {}).get("url") == url
                    and "responseStatusCode" not in event.get("params", {})
                ),
                None,
            )
            return request_pause is not None

        try:
            await wait_until(saw_request_pause, "Chromium navigation authentication request pause")
        except SmokeError as error:
            navigation_state = "pending"
            if navigation.done():
                navigation_error = navigation.exception()
                navigation_state = (
                    f"failed: {navigation_error}"
                    if navigation_error is not None
                    else f"completed: {navigation.result()}"
                )
            raise SmokeError(
                "Chromium navigation authentication request did not pause; "
                f"navigation={navigation_state}; events={events}"
            ) from error
        assert request_pause is not None
        request_id = request_pause.get("params", {}).get("requestId")
        network_id = request_pause.get("params", {}).get("networkId")
        if not isinstance(request_id, str) or not isinstance(network_id, str):
            raise SmokeError(f"invalid Chromium navigation auth pause: {request_pause}")
        await cdp.send("Fetch.continueRequest", {"requestId": request_id})
        await wait_until(
            lambda: any(
                event.get("method") == "Fetch.authRequired"
                and event.get("params", {}).get("requestId") == request_id
                for event in events
            ),
            "Chromium navigation authentication challenge",
        )
        await cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": request_id,
                "authChallengeResponse": {"response": "CancelAuth"},
            },
        )

        response_pause: dict[str, Any] | None = None

        def saw_response_pause() -> bool:
            nonlocal response_pause
            response_pause = next(
                (
                    event
                    for event in events
                    if event.get("method") == "Fetch.requestPaused"
                    and event.get("params", {}).get("requestId") == request_id
                    and event.get("params", {}).get("responseStatusCode") == 401
                ),
                None,
            )
            return response_pause is not None

        await wait_until(saw_response_pause, "Chromium navigation canceled 401 response pause")
        assert_equal(
            navigation.done(),
            False,
            "Chromium navigation remains pending at canceled auth response stage",
        )
        body = await cdp.send("Fetch.getResponseBody", {"requestId": request_id})
        encoded_body = body.get("body", "")
        if body.get("base64Encoded"):
            decoded_body = base64.b64decode(encoded_body).decode()
        else:
            decoded_body = encoded_body
        assert_equal(
            decoded_body,
            "auth required",
            "Chromium canceled navigation response-stage body",
        )
        await cdp.send("Fetch.continueResponse", {"requestId": request_id})
        try:
            response = await navigation
        except Exception as error:
            observed = [event.get("method") for event in events]
            raise SmokeError(
                "Chromium canceled navigation did not reach load; "
                f"observed events={observed}"
            ) from error
        assert_equal(
            response.status if response else None,
            401,
            "Chromium canceled navigation response status",
        )
        assert_equal(await page.text_content("body"), "auth required", "Chromium navigation body")
        await wait_until(
            lambda: len(_network_events_for_request(events, "Network.loadingFinished", network_id))
            == 1,
            "Chromium canceled navigation network completion",
        )
        assert_equal(
            len(_network_events_for_request(events, "Network.loadingFailed", network_id)),
            0,
            "Chromium canceled navigation failure count",
        )
        state.record("chromium_navigation_cancel_auth_response_sample")
        sample_completed = True
    finally:
        if navigation is not None and not navigation.done():
            navigation.cancel()
        if navigation is not None:
            with suppress(asyncio.CancelledError, Exception):
                await navigation

        cleanup_errors: list[Exception] = []
        try:
            await cdp.send("Fetch.disable")
        except Exception as error:
            cleanup_errors.append(error)
        if sample_completed and cleanup_errors:
            raise cleanup_errors[0]

async def _verify_chromium_failed_main_document_request_extra_info_sample(
    state: SmokeState,
) -> None:
    page = await state.context.new_page()
    try:
        cdp = await new_cdp_session(state.context, page)
        await _verify_failed_main_document_request_extra_info_on_session(state, cdp)
    finally:
        await page.close()

async def _verify_failed_main_document_request_extra_info_on_session(
    state: SmokeState,
    cdp: Any,
) -> None:
    observed_methods = [
        "Network.requestWillBeSent",
        "Network.requestWillBeSentExtraInfo",
        "Network.responseReceivedExtraInfo",
        "Network.responseReceived",
        "Network.loadingFinished",
        "Network.loadingFailed",
    ]
    events = attach_cdp_event_collector(cdp, observed_methods)
    await cdp.send("Network.enable")
    fixture = _alternate_loopback_origin(state.fixture)
    route = "/chromium-network-reset-before-response"
    state.fixture_server.reset_request_count(route)
    url = f"{fixture}{route}"
    start = len(events)

    navigation = await cdp.send("Page.navigate", {"url": url})
    assert_equal(
        navigation.get("errorText"),
        "net::ERR_CONNECTION_RESET",
        "failed Document navigation browser error text",
    )
    assert_equal(
        navigation.get("isDownload"),
        False,
        "failed Document navigation isDownload",
    )
    for field in ("frameId", "loaderId"):
        if not isinstance(navigation.get(field), str) or not navigation[field]:
            raise SmokeError(f"failed Document navigation missing {field}: {navigation}")

    def correlated_events() -> list[dict[str, Any]] | None:
        initial_request = next(
            (
                event
                for event in events[start:]
                if event.get("method") == "Network.requestWillBeSent"
                and event.get("params", {}).get("type") == "Document"
                and event.get("params", {}).get("request", {}).get("url") == url
            ),
            None,
        )
        if initial_request is None:
            return None
        request_id = initial_request.get("params", {}).get("requestId")
        return [
            event
            for event in events[start:]
            if event.get("params", {}).get("requestId") == request_id
        ]

    def failed_request_completed() -> bool:
        current_events = correlated_events()
        if current_events is None:
            return False
        methods = [event.get("method") for event in current_events]
        return (
            "Network.requestWillBeSentExtraInfo" in methods
            and "Network.loadingFailed" in methods
        )

    await wait_until(
        failed_request_completed,
        "failed main-document request ExtraInfo and loadingFailed",
    )
    request_events = correlated_events()
    if request_events is None:
        raise SmokeError(f"missing failed Document requestWillBeSent for {url}: {events[start:]}")

    methods = [event.get("method") for event in request_events]
    assert_equal(
        methods.count("Network.requestWillBeSent"),
        1,
        "failed Document request count",
    )
    assert_equal(
        methods.count("Network.requestWillBeSentExtraInfo"),
        1,
        "failed Document request ExtraInfo count",
    )
    assert_equal(
        methods.count("Network.responseReceivedExtraInfo"),
        0,
        "failed Document response ExtraInfo count",
    )
    assert_equal(
        methods.count("Network.responseReceived"),
        0,
        "failed Document response count",
    )
    assert_equal(
        methods.count("Network.loadingFailed"),
        1,
        "failed Document loadingFailed count",
    )
    # Chromium may retry a reset on a reused/preconnected socket. Whether the
    # navigation uses that socket depends on timing, so both counts are valid.
    fixture_request_count = state.fixture_server.request_count(route)
    expected_counts = (1,) if is_moli_browser() else (1, 2)
    if fixture_request_count not in expected_counts:
        raise SmokeError(
            "failed Document fixture request count: "
            f"expected one of {expected_counts}, got {fixture_request_count}"
        )
    loading_failed = next(
        event for event in request_events if event.get("method") == "Network.loadingFailed"
    )
    assert_equal(
        loading_failed.get("params", {}).get("errorText"),
        "net::ERR_CONNECTION_RESET",
        "failed Document Network.loadingFailed error text",
    )

    request_extra = next(
        event
        for event in request_events
        if event.get("method") == "Network.requestWillBeSentExtraInfo"
    )
    assert_equal(
        request_extra.get("params", {}).get("associatedCookies"),
        [],
        "failed no-cookie Document request ExtraInfo",
    )
    request_headers = request_extra.get("params", {}).get("headers") or {}
    assert_equal(
        _header_value(request_headers, "Host"),
        urlsplit(fixture).netloc,
        "failed Document transport-generated Host header",
    )
    accept_encoding = _header_value(request_headers, "Accept-Encoding")
    if not isinstance(accept_encoding, str) or not accept_encoding:
        raise SmokeError(
            f"failed Document request ExtraInfo missing transport Accept-Encoding: {request_headers}"
        )
    state.record(
        "chromium_failed_main_document_request_extra_info_sample",
        {"fixtureRequestCount": fixture_request_count},
    )

async def _verify_chromium_redirect_then_failed_main_document_extra_info_sample(
    state: SmokeState,
) -> None:
    page = await state.context.new_page()
    try:
        cdp = await new_cdp_session(state.context, page)
        observed_methods = [
            "Network.requestWillBeSent",
            "Network.requestWillBeSentExtraInfo",
            "Network.responseReceivedExtraInfo",
            "Network.responseReceived",
            "Network.loadingFinished",
            "Network.loadingFailed",
        ]
        events = attach_cdp_event_collector(cdp, observed_methods)
        await cdp.send("Network.enable")
        fixture = _alternate_loopback_origin(state.fixture)
        redirect_route = "/chromium-network-redirect-before-reset"
        reset_route = "/chromium-network-reset-before-response"
        state.fixture_server.reset_request_count(redirect_route)
        state.fixture_server.reset_request_count(reset_route)
        initial_url = f"{fixture}{redirect_route}"
        final_url = f"{fixture}{reset_route}"
        start = len(events)

        navigation = await cdp.send("Page.navigate", {"url": initial_url})
        assert_equal(
            navigation.get("errorText"),
            "net::ERR_CONNECTION_RESET",
            "redirected failed Document navigation browser error text",
        )
        assert_equal(
            navigation.get("isDownload"),
            False,
            "redirected failed Document navigation isDownload",
        )

        def correlated_events() -> list[dict[str, Any]] | None:
            initial_request = next(
                (
                    event
                    for event in events[start:]
                    if event.get("method") == "Network.requestWillBeSent"
                    and event.get("params", {}).get("type") == "Document"
                    and event.get("params", {}).get("request", {}).get("url")
                    == initial_url
                ),
                None,
            )
            if initial_request is None:
                return None
            request_id = initial_request.get("params", {}).get("requestId")
            return [
                event
                for event in events[start:]
                if event.get("params", {}).get("requestId") == request_id
            ]

        def redirected_failure_completed() -> bool:
            current_events = correlated_events()
            if current_events is None:
                return False
            methods = [event.get("method") for event in current_events]
            return (
                methods.count("Network.requestWillBeSent") == 2
                and methods.count("Network.requestWillBeSentExtraInfo") == 2
                and methods.count("Network.responseReceivedExtraInfo") == 1
                and methods.count("Network.loadingFailed") == 1
            )

        await wait_until(
            redirected_failure_completed,
            "redirect response, final request ExtraInfo, and loadingFailed",
        )
        request_events = correlated_events()
        if request_events is None:
            raise SmokeError(
                f"missing redirected failed Document request for {initial_url}: {events[start:]}"
            )
        methods = [event.get("method") for event in request_events]
        assert_equal(
            methods.count("Network.responseReceived"),
            0,
            "redirected failed Document final response count",
        )
        requests = [
            event
            for event in request_events
            if event.get("method") == "Network.requestWillBeSent"
        ]
        assert_equal(
            requests[1].get("params", {}).get("request", {}).get("url"),
            final_url,
            "redirected failed Document final request URL",
        )
        assert_equal(
            requests[1].get("params", {}).get("redirectResponse", {}).get("status"),
            302,
            "redirected failed Document redirect response status",
        )
        assert_equal(
            requests[1].get("params", {}).get("redirectHasExtraInfo"),
            True,
            "redirected failed Document redirectHasExtraInfo",
        )
        response_extra = next(
            event
            for event in request_events
            if event.get("method") == "Network.responseReceivedExtraInfo"
        )
        assert_equal(
            response_extra.get("params", {}).get("statusCode"),
            302,
            "redirected failed Document raw redirect status",
        )
        request_extras = [
            event
            for event in request_events
            if event.get("method") == "Network.requestWillBeSentExtraInfo"
        ]
        for index, request_extra in enumerate(request_extras):
            request_headers = request_extra.get("params", {}).get("headers") or {}
            assert_equal(
                _header_value(request_headers, "Host"),
                urlsplit(fixture).netloc,
                f"redirected failed Document hop {index} Host header",
            )
        loading_failed = next(
            event
            for event in request_events
            if event.get("method") == "Network.loadingFailed"
        )
        assert_equal(
            loading_failed.get("params", {}).get("errorText"),
            "net::ERR_CONNECTION_RESET",
            "redirected failed Document loadingFailed error text",
        )
        assert_equal(
            state.fixture_server.request_count(redirect_route),
            1,
            "redirected failed Document initial request count",
        )
        assert_equal(
            state.fixture_server.request_count(reset_route),
            1,
            "redirected failed Document final request count",
        )
        state.record("chromium_redirect_then_failed_main_document_extra_info_sample")
    finally:
        await page.close()

async def _verify_chromium_main_document_response_stage_extra_info_sample(
    state: SmokeState,
) -> None:
    observed_methods = [
        "Network.requestWillBeSent",
        "Network.requestWillBeSentExtraInfo",
        "Network.responseReceivedExtraInfo",
        "Fetch.requestPaused",
        "Network.responseReceived",
        "Network.loadingFinished",
        "Network.loadingFailed",
    ]
    events = attach_cdp_event_collector(state.cdp, observed_methods)
    fixture = _alternate_loopback_origin(state.fixture)
    url = f"{fixture}/plain?chromium-response-stage-extra-info"
    navigation_task: asyncio.Task[dict[str, Any]] | None = None

    await state.cdp.send(
        "Fetch.enable",
        {
            "patterns": [
                {
                    "urlPattern": url,
                    "resourceType": "Document",
                    "requestStage": "Response",
                }
            ]
        },
    )
    try:
        navigation_task = asyncio.create_task(state.cdp.send("Page.navigate", {"url": url}))

        def response_stage_pause() -> dict[str, Any] | None:
            return next(
                (
                    event
                    for event in events
                    if event.get("method") == "Fetch.requestPaused"
                    and event.get("params", {}).get("request", {}).get("url") == url
                    and event.get("params", {}).get("resourceType") == "Document"
                    and event.get("params", {}).get("responseStatusCode") == 200
                ),
                None,
            )

        await wait_until(response_stage_pause, "main-document response-stage pause")
        paused = response_stage_pause()
        assert paused is not None
        network_id = paused.get("params", {}).get("networkId")
        fetch_request_id = paused.get("params", {}).get("requestId")
        if not isinstance(network_id, str) or not network_id:
            raise SmokeError(f"missing response-stage networkId: {paused}")
        if not isinstance(fetch_request_id, str) or not fetch_request_id:
            raise SmokeError(f"missing response-stage Fetch requestId: {paused}")

        correlated = [
            event
            for event in events
            if event.get("params", {}).get("requestId") == network_id
            or (
                event.get("method") == "Fetch.requestPaused"
                and event.get("params", {}).get("networkId") == network_id
            )
        ]
        correlated_methods = [event.get("method") for event in correlated]
        assert_equal(
            correlated_methods.count("Network.requestWillBeSentExtraInfo"),
            1,
            "response-stage pre-pause request ExtraInfo count",
        )
        assert_equal(
            correlated_methods.count("Network.responseReceivedExtraInfo"),
            1,
            "response-stage pre-pause response ExtraInfo count",
        )
        if "Network.responseReceived" in correlated_methods:
            raise SmokeError(
                "Network.responseReceived must remain hidden until response-stage continue: "
                f"{correlated_methods}"
            )
        pause_index = correlated_methods.index("Fetch.requestPaused")
        for extra_info_method in (
            "Network.requestWillBeSentExtraInfo",
            "Network.responseReceivedExtraInfo",
        ):
            if correlated_methods.index(extra_info_method) > pause_index:
                raise SmokeError(
                    f"{extra_info_method} must precede the response-stage pause: "
                    f"{correlated_methods}"
                )

        response_extra = next(
            event
            for event in correlated
            if event.get("method") == "Network.responseReceivedExtraInfo"
        )
        assert_equal(
            response_extra.get("params", {}).get("statusCode"),
            200,
            "response-stage original response ExtraInfo status",
        )
        await state.cdp.send(
            "Fetch.continueResponse",
            {
                "requestId": fetch_request_id,
                "responseCode": 201,
                "responsePhrase": "Created",
                "responseHeaders": [
                    {"name": "content-type", "value": "text/html; charset=utf-8"},
                    {"name": "x-smoke-override", "value": "yes"},
                ],
            },
        )
        navigation_result = await asyncio.wait_for(navigation_task, timeout=10)
        if navigation_result.get("errorText"):
            raise SmokeError(f"response-stage Page.navigate failed: {navigation_result}")

        def response_and_terminal_arrived() -> bool:
            request_events = [
                event
                for event in events
                if event.get("params", {}).get("requestId") == network_id
            ]
            return any(
                event.get("method") == "Network.responseReceived" for event in request_events
            ) and any(
                event.get("method") in {"Network.loadingFinished", "Network.loadingFailed"}
                for event in request_events
            )

        await wait_until(
            response_and_terminal_arrived,
            "continued main-document response and terminal Network events",
        )
        completed = [
            event
            for event in events
            if event.get("params", {}).get("requestId") == network_id
            or (
                event.get("method") == "Fetch.requestPaused"
                and event.get("params", {}).get("networkId") == network_id
            )
        ]
        completed_methods = [event.get("method") for event in completed]
        assert_equal(
            completed_methods.count("Network.responseReceivedExtraInfo"),
            1,
            "response-stage override must not duplicate response ExtraInfo",
        )
        assert_equal(
            completed_methods.count("Network.responseReceived"),
            1,
            "response-stage continued response count",
        )
        response = next(
            event
            for event in completed
            if event.get("method") == "Network.responseReceived"
        )
        if completed_methods.index("Network.responseReceived") < completed_methods.index(
            "Fetch.requestPaused"
        ):
            raise SmokeError(
                "Network.responseReceived must follow the response-stage pause: "
                f"{completed_methods}"
            )
        assert_equal(
            response.get("params", {}).get("response", {}).get("status"),
            201,
            "response-stage overridden response status",
        )
        assert_equal(
            response.get("params", {}).get("hasExtraInfo"),
            True,
            "response-stage overridden response hasExtraInfo",
        )
        response_headers = response.get("params", {}).get("response", {}).get("headers") or {}
        override_header = next(
            (
                value
                for name, value in response_headers.items()
                if str(name).lower() == "x-smoke-override"
            ),
            None,
        )
        assert_equal(
            override_header,
            "yes",
            "response-stage overridden response header",
        )
        state.record("chromium_main_document_response_stage_extra_info_sample")
    finally:
        await state.cdp.send("Fetch.disable")
        if navigation_task is not None and not navigation_task.done():
            navigation_task.cancel()
            try:
                await navigation_task
            except asyncio.CancelledError:
                pass


async def run_network_group(state: SmokeState) -> None:
    await _verify_chromium_page_lifecycle_order(state)
    await _verify_chromium_main_document_network_extra_info_sample(state)
    await _verify_chromium_cookie_blocked_reason_sample(state)
    await _verify_chromium_worker_cancel_auth_response_sample(state)
    await _verify_chromium_worker_auth_extra_info_sample(state)
    await _verify_chromium_fetch_cancel_auth_response_stage_sample(state)
    await _verify_chromium_navigation_cancel_auth_response_sample(state)
    await _verify_chromium_fetch_continuation_extra_info_sample(state)
    await _verify_chromium_failed_main_document_request_extra_info_sample(state)
    await _verify_chromium_redirect_then_failed_main_document_extra_info_sample(state)
    await _verify_chromium_main_document_response_stage_extra_info_sample(state)
