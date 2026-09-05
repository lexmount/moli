from __future__ import annotations

import asyncio
import base64
from contextlib import suppress
from typing import Any
from urllib.parse import urlsplit
from uuid import UUID

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..fixture import FixtureServer
from ..helpers import attach_cdp_event_collector, run_worker_command


async def run_chromium_cdp_group(state: SmokeState) -> None:
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
    await _verify_chromium_page_frame_loading_sample(state)
    await _verify_chromium_page_frame_tree_sample(state)
    await _verify_chromium_child_frame_multi_session_fanout_sample(state)
    await _verify_chromium_page_frame_attached_parent_sample(state)
    await _verify_chromium_page_fragment_navigation_sample(state)
    await _verify_chromium_audits_domain_sample(state)
    await _verify_chromium_page_get_app_manifest_sample(state)
    await _verify_chromium_page_layout_metrics_sample(state)
    await _verify_chromium_runtime_sample(state)
    await _verify_chromium_input_session_state_sample(state)
    await _verify_chromium_idle_override_sample(state)
    await _verify_chromium_log_domain_sample(state)
    await _verify_chromium_io_resolve_blob_sample(state)
    await _verify_chromium_performance_enable_sample(state)
    await _verify_chromium_performance_metrics_sample(state)
    await _verify_chromium_cpu_throttling_multiple_pages_sample(state)
    await _verify_chromium_profiler_cpu_profile_sample(state)
    await _verify_chromium_profiler_cpu_profile_with_throttling_sample(state)
    await _verify_chromium_profiler_stop_without_start_sample(state)
    await _verify_chromium_profiler_sampling_interval_contract_sample(state)
    await _verify_chromium_profiler_enable_disable_contract_sample(state)
    await _verify_chromium_profiler_console_profile_sample(state)
    await _verify_chromium_profiler_nested_console_profile_sample(state)
    await _verify_chromium_profiler_parameterless_profile_end_sample(state)
    await _verify_chromium_profiler_navigation_profile_continuity_sample(state)
    await _verify_chromium_profiler_attached_session_navigation_profile_continuity_sample(state)
    await _verify_chromium_profiler_attached_session_detach_clears_state_sample(state)
    await _verify_chromium_profiler_precise_coverage_error_sample(state)
    await _verify_chromium_profiler_precise_coverage_sample(state)
    await _verify_chromium_profiler_precise_coverage_counter_reset_sample(state)
    await _verify_chromium_profiler_precise_block_coverage_sample(state)
    await _verify_chromium_profiler_best_effort_with_precise_coverage_sample(state)
    await _verify_chromium_dom_get_attributes_sample(state)
    await _verify_chromium_dom_query_selector_sample(state)
    await _verify_chromium_dom_single_text_child_projection_sample(state)
    await _verify_chromium_dom_debugger_event_listeners_sample(state)
    await _verify_chromium_dom_debugger_event_listener_breakpoint_sample(state)
    await _verify_chromium_dom_debugger_dom_breakpoint_sample(state)
    await _verify_chromium_dom_debugger_parser_mutation_no_pause_sample(state)
    await _verify_chromium_dom_debugger_xhr_breakpoint_sample(state)


async def run_computed_style_group(state: SmokeState) -> None:
    """Run the focused cross-engine computed-style breadth contract."""
    await _verify_chromium_css_computed_style_breadth_sample(state)
    await _verify_sampled_computed_sizes(state)


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
        ["Page.domContentEventFired", "Page.loadEventFired"],
    )
    await state.cdp.send("Page.enable")
    start = len(events)
    await state.cdp.send("Page.navigate", {"url": f"{state.fixture}/chromium-cdp-lifecycle-page"})

    await wait_until(
        lambda: _has_event(events[start:], "Page.domContentEventFired")
        and _has_event(events[start:], "Page.loadEventFired"),
        "Chromium Page.domContentEventFired/Page.loadEventFired sample",
    )
    methods = [event["method"] for event in events[start:]]
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
        cdp = await state.context.new_cdp_session(page)
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
    assert_equal(
        state.fixture_server.request_count(route),
        1,
        "failed Document fixture request count",
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
    state.record("chromium_failed_main_document_request_extra_info_sample")


async def _verify_chromium_redirect_then_failed_main_document_extra_info_sample(
    state: SmokeState,
) -> None:
    page = await state.context.new_page()
    try:
        cdp = await state.context.new_cdp_session(page)
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


def _alternate_loopback_origin(url: str) -> str:
    parsed = urlsplit(url)
    if parsed.hostname not in {"127.0.0.1", "localhost"} or parsed.port is None:
        raise SmokeError(f"expected loopback fixture origin, got {url}")
    hostname = "localhost" if parsed.hostname == "127.0.0.1" else "127.0.0.1"
    return f"{parsed.scheme}://{hostname}:{parsed.port}"


async def _verify_chromium_page_frame_loading_sample(state: SmokeState) -> None:
    events = attach_cdp_event_collector(
        state.cdp,
        ["Page.frameStartedLoading", "Page.frameStoppedLoading"],
    )
    await state.cdp.send("Page.enable")
    start = len(events)
    await state.cdp.send("Page.navigate", {"url": f"{state.fixture}/chromium-cdp-lifecycle-page?frame-loading"})

    await wait_until(
        lambda: _has_event(events[start:], "Page.frameStartedLoading")
        and _has_event(events[start:], "Page.frameStoppedLoading"),
        "Chromium Page.frameStartedLoading/Page.frameStoppedLoading sample",
    )
    started = next(event for event in events[start:] if event["method"] == "Page.frameStartedLoading")
    stopped = next(event for event in events[start:] if event["method"] == "Page.frameStoppedLoading")
    started_frame_id = started["params"].get("frameId")
    stopped_frame_id = stopped["params"].get("frameId")
    frame_tree = await state.cdp.send("Page.getFrameTree")
    root_frame_id = frame_tree.get("frameTree", {}).get("frame", {}).get("id")
    if not started_frame_id or started_frame_id != stopped_frame_id or started_frame_id != root_frame_id:
        raise SmokeError(f"Page frame loading events should share frameId: {events[start:]}")
    state.record("chromium_page_frame_loading_sample")


async def _verify_chromium_page_frame_tree_sample(state: SmokeState) -> None:
    events = attach_cdp_event_collector(state.cdp, ["Page.frameNavigated"])
    await state.cdp.send("Page.enable")
    start = len(events)
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/iframe")

    # The top document reaching DOMContentLoaded does not imply that an iframe
    # navigation has committed. Chromium publishes the child frameNavigated at
    # commit, so synchronize on that protocol fact before reading the tree.
    await wait_until(
        lambda: any(
            event["method"] == "Page.frameNavigated"
            and str(event["params"].get("frame", {}).get("url", "")).endswith("/child")
            for event in events[start:]
        ),
        "child Page.frameNavigated for /child iframe",
    )
    child_navigation = next(
        event["params"]["frame"]
        for event in events[start:]
        if event["method"] == "Page.frameNavigated"
        and str(event["params"].get("frame", {}).get("url", "")).endswith("/child")
    )
    result = await state.cdp.send("Page.getFrameTree")
    root = result.get("frameTree", {})
    root_frame = root.get("frame", {})
    if not root_frame.get("id") or not str(root_frame.get("url", "")).endswith("/iframe"):
        raise SmokeError(f"Page.getFrameTree root frame mismatch: {result}")
    children = root.get("childFrames") or []
    if not children:
        raise SmokeError(f"Page.getFrameTree should expose child frames: {result}")
    child_urls = [child.get("frame", {}).get("url", "") for child in children]
    if not any(str(url).endswith("/child") for url in child_urls):
        raise SmokeError(f"Page.getFrameTree missed /child iframe: {result}")
    child_frame = next(
        child.get("frame", {})
        for child in children
        if str(child.get("frame", {}).get("url", "")).endswith("/child")
    )
    if (
        child_navigation.get("id") != child_frame.get("id")
        or child_navigation.get("parentId") != root_frame.get("id")
        or not child_navigation.get("loaderId")
        or child_navigation.get("loaderId") != child_frame.get("loaderId")
    ):
        raise SmokeError(
            "child Page.frameNavigated should match Page.getFrameTree: "
            f"event={child_navigation}, tree={result}"
        )
    state.record("chromium_page_get_frame_tree_sample")


async def _verify_chromium_child_frame_multi_session_fanout_sample(
    state: SmokeState,
) -> None:
    methods = [
        "Page.frameAttached",
        "Page.frameStartedNavigating",
        "Page.frameNavigated",
        "Page.lifecycleEvent",
        "Page.frameStoppedLoading",
    ]
    page = await state.context.new_page()
    page_only = None
    lifecycle = None

    def navigation_for_url(events: list[dict[str, Any]], url: str, label: str) -> dict[str, Any]:
        matches = [
            event["params"]["frame"]
            for event in events
            if event["method"] == "Page.frameNavigated"
            and event["params"].get("frame", {}).get("url") == url
        ]
        assert_equal(len(matches), 1, f"{label} frameNavigated count")
        return matches[0]

    def navigation_frame_id_for_url(
        events: list[dict[str, Any]], url: str
    ) -> str | None:
        for event in events:
            if event["method"] != "Page.frameNavigated":
                continue
            frame = event["params"].get("frame", {})
            if frame.get("url") == url and isinstance(frame.get("id"), str):
                return frame["id"]
        return None

    def navigation_has_stopped(events: list[dict[str, Any]], url: str) -> bool:
        frame_id = navigation_frame_id_for_url(events, url)
        return frame_id is not None and any(
            event["method"] == "Page.frameStoppedLoading"
            and event["params"].get("frameId") == frame_id
            for event in events
        )

    def exact_event_index(
        events: list[dict[str, Any]],
        method: str,
        frame_id: str,
        label: str,
        *,
        lifecycle_name: str | None = None,
    ) -> int:
        indices = []
        for index, event in enumerate(events):
            if event["method"] != method:
                continue
            params = event["params"]
            event_frame_id = (
                params.get("frame", {}).get("id")
                if method == "Page.frameNavigated"
                else params.get("frameId")
            )
            if event_frame_id != frame_id:
                continue
            if lifecycle_name is not None and params.get("name") != lifecycle_name:
                continue
            indices.append(index)
        assert_equal(len(indices), 1, f"{label} {method} count")
        return indices[0]

    def assert_page_event_sequence(
        events: list[dict[str, Any]],
        navigation: dict[str, Any],
        label: str,
    ) -> None:
        frame_id = navigation["id"]
        indices = [
            exact_event_index(events, "Page.frameAttached", frame_id, label),
            exact_event_index(events, "Page.frameStartedNavigating", frame_id, label),
            exact_event_index(events, "Page.frameNavigated", frame_id, label),
            exact_event_index(events, "Page.frameStoppedLoading", frame_id, label),
        ]
        assert_equal(indices, sorted(indices), f"{label} child frame event order")
        started = events[indices[1]]["params"]
        assert_equal(started.get("url"), navigation["url"], f"{label} started URL")
        assert_equal(
            started.get("loaderId"),
            navigation["loaderId"],
            f"{label} started loaderId",
        )

    def flattened_frames(frame_tree: dict[str, Any]) -> dict[str, dict[str, Any]]:
        frames: dict[str, dict[str, Any]] = {}
        pending = [frame_tree]
        while pending:
            current = pending.pop()
            frame = current.get("frame", {})
            frame_id = frame.get("id")
            if isinstance(frame_id, str) and frame_id:
                frames[frame_id] = frame
            pending.extend(current.get("childFrames") or [])
        return frames

    async def append_frame(frame_id: str, name: str, url: str) -> None:
        await page.evaluate(
            """({id, name, url}) => {
              const frame = document.createElement('iframe');
              frame.id = id;
              frame.name = name;
              frame.src = url;
              document.body.appendChild(frame);
            }""",
            {"id": frame_id, "name": name, "url": url},
        )

    try:
        await page.goto(f"{state.fixture}/plain?child-frame-session-fanout", wait_until="load")
        page_only = await state.context.new_cdp_session(page)
        lifecycle = await state.context.new_cdp_session(page)
        page_only_events = attach_cdp_event_collector(page_only, methods)
        lifecycle_events = attach_cdp_event_collector(lifecycle, methods)
        await page_only.send("Page.enable")
        await lifecycle.send("Page.enable")
        await lifecycle.send("Page.setLifecycleEventsEnabled", {"enabled": True})
        page_only_start = len(page_only_events)
        lifecycle_start = len(lifecycle_events)

        outer_url = f"{state.fixture}/semantic-frame-child?child=fanout&nested=1"
        nested_url = f"{state.fixture}/semantic-frame-grandchild"
        await append_frame("fanout", "fanout-frame", outer_url)
        await wait_until(
            lambda: all(
                navigation_has_stopped(events[start:], url)
                for events, start in (
                    (page_only_events, page_only_start),
                    (lifecycle_events, lifecycle_start),
                )
                for url in (outer_url, nested_url)
            ),
            "both attached sessions child Page event fan-out",
        )

        page_only_batch = page_only_events[page_only_start:]
        lifecycle_batch = lifecycle_events[lifecycle_start:]
        page_only_navigations = {
            url: navigation_for_url(page_only_batch, url, "Page-only session")
            for url in (outer_url, nested_url)
        }
        lifecycle_navigations = {
            url: navigation_for_url(lifecycle_batch, url, "lifecycle session")
            for url in (outer_url, nested_url)
        }

        for url in (outer_url, nested_url):
            assert_equal(
                lifecycle_navigations[url],
                page_only_navigations[url],
                f"multi-session child frame metadata for {url}",
            )
            assert_page_event_sequence(
                page_only_batch,
                page_only_navigations[url],
                f"Page-only session {url}",
            )
            assert_page_event_sequence(
                lifecycle_batch,
                lifecycle_navigations[url],
                f"lifecycle session {url}",
            )

        outer_navigation = page_only_navigations[outer_url]
        nested_navigation = page_only_navigations[nested_url]
        page_only_tree = (await page_only.send("Page.getFrameTree"))["frameTree"]
        lifecycle_tree = (await lifecycle.send("Page.getFrameTree"))["frameTree"]
        page_only_frames = flattened_frames(page_only_tree)
        lifecycle_frames = flattened_frames(lifecycle_tree)
        root_id = page_only_tree["frame"]["id"]
        assert_equal(lifecycle_tree["frame"]["id"], root_id, "multi-session root frame id")

        for navigation, expected_parent_id in (
            (outer_navigation, root_id),
            (nested_navigation, outer_navigation["id"]),
        ):
            frame_id = navigation["id"]
            expected = {
                key: navigation.get(key)
                for key in ("id", "parentId", "loaderId", "name", "url")
            }
            assert_equal(
                expected["parentId"],
                expected_parent_id,
                f"child frame parent for {navigation['url']}",
            )
            assert_equal(
                {key: page_only_frames[frame_id].get(key) for key in expected},
                expected,
                f"Page-only frame tree metadata for {navigation['url']}",
            )
            assert_equal(
                {key: lifecycle_frames[frame_id].get(key) for key in expected},
                expected,
                f"lifecycle frame tree metadata for {navigation['url']}",
            )

        child_frame_ids = {outer_navigation["id"], nested_navigation["id"]}
        assert_equal(
            [
                event
                for event in page_only_batch
                if event["method"] == "Page.lifecycleEvent"
                and event["params"].get("frameId") in child_frame_ids
            ],
            [],
            "Page lifecycle events remain session-local",
        )
        for navigation in lifecycle_navigations.values():
            frame_id = navigation["id"]
            label = f"lifecycle session {navigation['url']}"
            navigated_index = exact_event_index(
                lifecycle_batch, "Page.frameNavigated", frame_id, label
            )
            dom_content_loaded_index = exact_event_index(
                lifecycle_batch,
                "Page.lifecycleEvent",
                frame_id,
                label,
                lifecycle_name="DOMContentLoaded",
            )
            load_index = exact_event_index(
                lifecycle_batch,
                "Page.lifecycleEvent",
                frame_id,
                label,
                lifecycle_name="load",
            )
            stopped_index = exact_event_index(
                lifecycle_batch, "Page.frameStoppedLoading", frame_id, label
            )
            lifecycle_indices = [
                navigated_index,
                dom_content_loaded_index,
                load_index,
                stopped_index,
            ]
            assert_equal(
                lifecycle_indices,
                sorted(lifecycle_indices),
                f"child lifecycle terminal order for {navigation['url']}",
            )
            for index in (dom_content_loaded_index, load_index):
                assert_equal(
                    lifecycle_batch[index]["params"].get("loaderId"),
                    navigation["loaderId"],
                    f"child lifecycle loaderId for {navigation['url']}",
                )

        await page_only.send("Page.disable")
        page_only_disable_mark = len(page_only_events)
        lifecycle_disable_mark = len(lifecycle_events)
        after_disable_url = f"{state.fixture}/semantic-frame-child?child=after-disable"
        await append_frame("after-disable", "after-disable-frame", after_disable_url)
        await wait_until(
            lambda: navigation_has_stopped(
                lifecycle_events[lifecycle_disable_mark:], after_disable_url
            ),
            "enabled session post-disable child terminal event",
        )
        assert_equal(
            page_only_events[page_only_disable_mark:],
            [],
            "Page.disable stops later child Page events for that session",
        )
        post_disable_navigation = navigation_for_url(
            lifecycle_events[lifecycle_disable_mark:],
            after_disable_url,
            "lifecycle post-disable session",
        )
        assert_page_event_sequence(
            lifecycle_events[lifecycle_disable_mark:],
            post_disable_navigation,
            "lifecycle post-disable session",
        )
        state.record(
            "chromium_page_child_frame_multi_session_fanout_sample",
            {
                "frameIds": [outer_navigation["id"], nested_navigation["id"]],
                "pageOnlyLifecycleCount": 0,
                "postDisableFrameId": post_disable_navigation["id"],
            },
        )
    finally:
        if lifecycle is not None:
            with suppress(Exception):
                await lifecycle.detach()
        if page_only is not None:
            with suppress(Exception):
                await page_only.detach()
        with suppress(Exception):
            await page.close()


async def _verify_chromium_page_frame_attached_parent_sample(state: SmokeState) -> None:
    events = attach_cdp_event_collector(state.cdp, ["Page.frameAttached"])
    await state.cdp.send("Page.enable")
    start = len(events)
    await _navigate_with_cdp_until_dom_ready(
        state,
        f"{state.fixture}/iframe?frame-attached-parent",
    )
    await wait_until(
        lambda: _has_event(events[start:], "Page.frameAttached"),
        "Chromium Page.frameAttached parent frame sample",
    )

    frame_tree = await state.cdp.send("Page.getFrameTree")
    frame_ids = _frame_tree_ids(frame_tree.get("frameTree", {}))
    attached = _events_with_method(events[start:], "Page.frameAttached")
    for event in attached:
        params = event.get("params", {})
        frame_id = params.get("frameId")
        parent_frame_id = params.get("parentFrameId")
        if not isinstance(frame_id, str) or not frame_id:
            raise SmokeError(f"Page.frameAttached should carry a non-empty frameId: {event}")
        if not isinstance(parent_frame_id, str) or not parent_frame_id:
            raise SmokeError(
                f"Page.frameAttached should carry a non-empty parentFrameId: {event}"
            )
        if frame_id not in frame_ids or parent_frame_id not in frame_ids:
            raise SmokeError(
                "Page.frameAttached should reference frames in the committed frame tree: "
                f"event={event}, frameTree={frame_tree}"
            )
    state.record("chromium_page_frame_attached_parent_sample", {"eventCount": len(attached)})


async def _verify_chromium_page_fragment_navigation_sample(state: SmokeState) -> None:
    await state.cdp.send("Page.enable")
    base_url = f"{state.fixture}/plain"
    first = await state.cdp.send("Page.navigate", {"url": base_url})
    await state.page.wait_for_load_state("load", timeout=10_000)
    if not first.get("frameId"):
        raise SmokeError(f"Page.navigate should return frameId for normal navigation: {first}")

    fragment_url = f"{base_url}#fragment"
    second = await state.cdp.send("Page.navigate", {"url": fragment_url})
    if second.get("errorText"):
        raise SmokeError(f"Page.navigate fragment navigation should not fail: {second}")
    location_result = await state.cdp.send(
        "Runtime.evaluate",
        {"expression": "location.href", "returnByValue": True},
    )
    assert_equal(
        location_result.get("result", {}).get("value"),
        fragment_url,
        "Chromium Page.navigate fragment location sample",
    )
    if second.get("frameId") and second.get("frameId") != first.get("frameId"):
        raise SmokeError(f"Page.navigate fragment should stay in the same frame: {first} -> {second}")
    state.record("chromium_page_fragment_navigation_sample")


async def _verify_chromium_page_layout_metrics_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/chromium-cdp-layout-page")
    initial = await state.cdp.send("Page.getLayoutMetrics")
    content = initial.get("cssContentSize") or {}
    viewport = initial.get("cssLayoutViewport") or {}
    visual = initial.get("cssVisualViewport") or {}
    if content.get("width", 0) <= 0 or content.get("height", 0) <= 0:
        raise SmokeError(f"Page.getLayoutMetrics should expose content size: {initial}")
    if viewport.get("clientWidth", 0) <= 0 or viewport.get("clientHeight", 0) <= 0:
        raise SmokeError(f"Page.getLayoutMetrics should expose layout viewport size: {initial}")
    if visual.get("clientWidth", 0) <= 0 or visual.get("clientHeight", 0) <= 0:
        raise SmokeError(f"Page.getLayoutMetrics should expose visual viewport size: {initial}")

    await state.cdp.send("Runtime.evaluate", {"expression": "window.scrollTo(100, 100)"})
    after_scroll = await state.cdp.send("Page.getLayoutMetrics")
    scrolled_visual = after_scroll.get("cssVisualViewport") or {}
    if scrolled_visual.get("pageX", 0) < 0 or scrolled_visual.get("pageY", 0) < 0:
        raise SmokeError(f"Page.getLayoutMetrics scroll coordinates should be non-negative: {after_scroll}")
    state.record("chromium_page_layout_metrics_sample")


async def _verify_chromium_idle_override_sample(state: SmokeState) -> None:
    primary = await state.context.new_cdp_session(state.page)
    peer = await state.context.new_cdp_session(state.page)
    same_site_fixture = FixtureServer()
    same_site_fixture.start()
    split = urlsplit(state.fixture)
    primary_origin = f"{split.scheme}://{split.hostname}:{split.port}"
    alternate_origin = f"{split.scheme}://localhost:{split.port}"
    same_site_origin = same_site_fixture.url
    primary_url = f"{primary_origin}/chromium-cdp-idle-page?first"
    alternate_url = f"{alternate_origin}/chromium-cdp-idle-page?cross-origin"
    target_info = await primary.send("Target.getTargetInfo")
    browser_context_id = target_info.get("targetInfo", {}).get("browserContextId")

    async def grant(origin: str) -> None:
        params = {"permissions": ["idleDetection"], "origin": origin}
        if browser_context_id:
            params["browserContextId"] = browser_context_id
        await primary.send(
            "Browser.grantPermissions",
            params,
        )

    async def runtime_value(cdp: Any, expression: str, *, await_promise: bool = False) -> Any:
        result = await cdp.send(
            "Runtime.evaluate",
            {
                "expression": expression,
                "returnByValue": True,
                "awaitPromise": await_promise,
            },
        )
        exception = result.get("exceptionDetails")
        if exception:
            raise SmokeError(f"Idle override Runtime.evaluate failed: {result}")
        return result.get("result", {}).get("value")

    try:
        await grant(primary_origin)
        await grant(alternate_origin)
        await grant(same_site_origin)
        await _navigate_with_cdp_until_dom_ready(state, primary_url)

        initial = await runtime_value(
            primary,
            """
            (async () => {
              globalThis.__idleEvents = [];
              globalThis.__idleDetector = new IdleDetector();
              __idleDetector.addEventListener('change', () => {
                __idleEvents.push(`${__idleDetector.userState}/${__idleDetector.screenState}`);
              });
              const before = [__idleDetector.userState, __idleDetector.screenState];
              await __idleDetector.start();
              return {before, state: [__idleDetector.userState, __idleDetector.screenState], events: __idleEvents};
            })()
            """,
            await_promise=True,
        )
        assert_equal(
            initial,
            {
                "before": [None, None],
                "state": ["active", "unlocked"],
                "events": ["active/unlocked"],
            },
            "IdleDetector initial state",
        )

        await primary.send(
            "Emulation.setIdleOverride",
            {"isUserActive": False, "isScreenUnlocked": False},
        )
        assert_equal(
            await runtime_value(
                primary,
                "({state:[__idleDetector.userState,__idleDetector.screenState],events:__idleEvents})",
            ),
            {
                "state": ["idle", "locked"],
                "events": ["active/unlocked", "idle/locked"],
            },
            "IdleDetector setIdleOverride state",
        )

        await peer.send(
            "Emulation.setIdleOverride",
            {"isUserActive": True, "isScreenUnlocked": False, "ignoredExtra": True},
        )
        await peer.detach()
        peer = None
        assert_equal(
            await runtime_value(
                primary,
                "[__idleDetector.userState,__idleDetector.screenState,__idleEvents.length]",
            ),
            ["active", "locked", 3],
            "Idle override last writer and detach persistence",
        )

        child = next((frame for frame in state.page.frames if frame != state.page.main_frame), None)
        if child is None:
            raise SmokeError("Idle override fixture should create a child frame")
        child_state = await child.evaluate(
            """
            async () => {
              const detector = new IdleDetector();
              await detector.start();
              return [detector.userState, detector.screenState];
            }
            """
        )
        assert_equal(
            child_state,
            ["active", "unlocked"],
            "top-level idle override should not affect child frame",
        )

        await primary.send("Emulation.clearIdleOverride")
        assert_equal(
            await runtime_value(
                primary,
                "[__idleDetector.userState,__idleDetector.screenState,__idleEvents.length]",
            ),
            ["active", "unlocked", 4],
            "clearIdleOverride should restore actual state",
        )
        await primary.send(
            "Emulation.setIdleOverride",
            {"isUserActive": False, "isScreenUnlocked": False},
        )

        await runtime_value(primary, "history.pushState({},'',location.pathname+'?same-document')")
        assert_equal(
            await runtime_value(primary, "[__idleDetector.userState,__idleDetector.screenState]"),
            ["idle", "locked"],
            "same-document navigation should preserve idle override",
        )

        same_origin_url = f"{primary_origin}/chromium-cdp-idle-page?same-origin"
        await _navigate_with_cdp_until_dom_ready(state, same_origin_url)
        assert_equal(
            await runtime_value(
                primary,
                "(async()=>{const d=new IdleDetector();await d.start();return [d.userState,d.screenState]})()",
                await_promise=True,
            ),
            ["idle", "locked"],
            "same-origin cross-document navigation should preserve idle override",
        )

        await _navigate_with_cdp_until_dom_ready(
            state,
            f"{same_site_origin}/chromium-cdp-idle-page?same-site-different-origin",
        )
        assert_equal(
            await runtime_value(
                primary,
                "(async()=>{const d=new IdleDetector();await d.start();return [d.userState,d.screenState]})()",
                await_promise=True,
            ),
            ["idle", "locked"],
            "same-site cross-origin navigation should preserve idle override",
        )

        other_page = await state.context.new_page()
        try:
            await other_page.goto(f"{primary_origin}/chromium-cdp-idle-page?other-target")
            assert_equal(
                await other_page.evaluate(
                    "async()=>{const d=new IdleDetector();await d.start();return [d.userState,d.screenState]}"
                ),
                ["active", "unlocked"],
                "idle override should not cross target boundaries",
            )
        finally:
            await other_page.close()

        await _navigate_with_cdp_until_dom_ready(state, alternate_url)
        assert_equal(
            await runtime_value(
                primary,
                "(async()=>{const d=new IdleDetector();await d.start();return [d.userState,d.screenState]})()",
                await_promise=True,
            ),
            ["active", "unlocked"],
            "cross-origin navigation should clear idle override",
        )

        invalid_cases = [
            {},
            {"isUserActive": True},
            {"isScreenUnlocked": True},
            {"isUserActive": None, "isScreenUnlocked": True},
            {"isUserActive": "true", "isScreenUnlocked": True},
        ]
        for params in invalid_cases:
            error = await _send_cdp_expect_optional_error(
                primary,
                "Emulation.setIdleOverride",
                params,
            )
            if error is None or "Invalid" not in error["message"]:
                raise SmokeError(
                    f"Emulation.setIdleOverride should reject invalid params {params}: {error}"
                )
        await primary.send("Emulation.clearIdleOverride", {"ignoredExtra": True})
        state.record("chromium_idle_override_sample")
    finally:
        if peer is not None:
            await peer.detach()
        await primary.detach()
        same_site_fixture.stop()


async def _verify_chromium_page_get_app_manifest_sample(state: SmokeState) -> None:
    none_url = f"{state.fixture}/chromium-app-manifest-none/path/page"
    await _navigate_with_cdp_until_dom_ready(state, none_url)
    implicit = await state.cdp.send("Page.getAppManifest")
    expected_scope = f"{state.fixture}/chromium-app-manifest-none/path/"
    expected_implicit_manifest = {
        "display": "kUndefined",
        "id": none_url,
        "orientation": "DEFAULT",
        "preferRelatedApplications": False,
        "scope": expected_scope,
        "startUrl": none_url,
    }
    assert_equal(implicit.get("url"), "", "implicit manifest URL")
    assert_equal(implicit.get("errors"), [], "implicit manifest errors")
    assert_equal(implicit.get("data"), "", "implicit manifest data")
    assert_equal(implicit.get("parsed"), {"scope": expected_scope}, "implicit parsed manifest")
    assert_equal(implicit.get("manifest"), expected_implicit_manifest, "implicit manifest")

    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-app-manifest-valid/page"
    )
    valid_manifest_route = "/chromium-app-manifests/app.webmanifest"
    state.fixture_server.reset_request_count(valid_manifest_route)
    manifest_network_start = len(state.subresource_events)
    explicit = await state.cdp.send("Page.getAppManifest")
    manifest = explicit.get("manifest") or {}
    expected_manifest_url = f"{state.fixture}/chromium-app-manifests/app.webmanifest"
    expected_manifest_scope = f"{state.fixture}/chromium-app-manifests/"
    await _assert_manifest_network_lifecycle(
        state.subresource_events,
        manifest_network_start,
        expected_manifest_url,
    )
    assert_equal(explicit.get("url"), expected_manifest_url, "explicit manifest URL")
    assert_equal(explicit.get("errors"), [], "explicit manifest errors")
    if not isinstance(explicit.get("data"), str) or '"Manifest Name"' not in explicit["data"]:
        raise SmokeError(f"Page.getAppManifest should preserve raw manifest data: {explicit}")
    assert_equal(manifest.get("name"), "Manifest Name", "manifest name")
    assert_equal(manifest.get("description"), "Manifest Description", "manifest description")
    assert_equal(
        manifest.get("id"), f"{state.fixture}/identity?x=1", "resolved manifest id"
    )
    assert_equal(
        manifest.get("startUrl"),
        f"{state.fixture}/chromium-app-manifests/start?x=2#fragment",
        "resolved manifest start URL",
    )
    assert_equal(manifest.get("scope"), expected_manifest_scope, "resolved manifest scope")
    assert_equal(manifest.get("display"), "kStandalone", "manifest display")
    assert_equal(
        manifest.get("displayOverrides"),
        ["kFullscreen", "kBrowser"],
        "manifest display overrides",
    )
    assert_equal(manifest.get("orientation"), "PORTRAIT_PRIMARY", "manifest orientation")
    assert_equal(
        manifest.get("backgroundColor"),
        "rgba(17,34,51,0.5019607843137255)",
        "manifest background color",
    )
    assert_equal(manifest.get("themeColor"), "rgba(255,0,0,1)", "manifest theme color")
    if not manifest.get("icons") or not manifest.get("shortcuts"):
        raise SmokeError(f"Page.getAppManifest should expose icons and shortcuts: {explicit}")
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        1,
        "first successful manifest request count",
    )

    matched = await state.cdp.send("Page.getAppManifest", {"manifestId": manifest["id"]})
    assert_equal(matched.get("manifest", {}).get("id"), manifest["id"], "matching manifest id")
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        1,
        "successful manifest result should be document-cached",
    )
    mismatch = await _send_cdp_expect_optional_error(
        state.cdp,
        "Page.getAppManifest",
        {"manifestId": manifest["id"] + "-mismatch"},
    )
    if not mismatch or "does not match the input" not in mismatch["message"]:
        raise SmokeError(f"Page.getAppManifest should reject a mismatched manifestId: {mismatch}")
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        1,
        "manifestId validation should reuse the document cache",
    )

    transient_link_change_network_start = len(state.subresource_events)
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "(() => {"
                "const link = document.querySelector('link[rel~=manifest]');"
                "const href = link.getAttribute('href');"
                "link.setAttribute('href', '/chromium-app-manifests/invalid.webmanifest');"
                "link.setAttribute('href', href);"
                "})()"
            ),
            "returnByValue": True,
        },
    )
    restored_link = await state.cdp.send("Page.getAppManifest")
    assert_equal(
        restored_link.get("manifest", {}).get("id"),
        manifest["id"],
        "manifest after a transient href change",
    )
    await _assert_manifest_network_lifecycle(
        state.subresource_events,
        transient_link_change_network_start,
        expected_manifest_url,
    )
    valid_requests_after_link_change = state.fixture_server.request_count(
        valid_manifest_route
    )

    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "document.querySelector('link[rel~=manifest]')"
                ".setAttribute('crossorigin', 'use-credentials')"
            ),
            "returnByValue": True,
        },
    )
    credentials_changed = await state.cdp.send("Page.getAppManifest")
    assert_equal(
        credentials_changed.get("manifest", {}).get("id"),
        manifest["id"],
        "manifest after crossorigin change",
    )
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        valid_requests_after_link_change,
        "crossorigin change should preserve Chromium's manifest cache",
    )

    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "document.querySelector('link[rel~=manifest]')"
                ".setAttribute('rel', 'alternate')"
            ),
            "returnByValue": True,
        },
    )
    removed = await state.cdp.send("Page.getAppManifest")
    assert_equal(removed.get("url"), "", "manifest after rel removal")
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        valid_requests_after_link_change,
        "rel removal should not fetch or return cached manifest",
    )

    invalid_manifest_route = "/chromium-app-manifests/invalid.webmanifest"
    state.fixture_server.reset_request_count(invalid_manifest_route)
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "const link = document.querySelector('link[rel~=alternate]');"
                "link.setAttribute('rel', 'manifest');"
                f"link.setAttribute('href', '{invalid_manifest_route}')"
            ),
            "returnByValue": True,
        },
    )
    invalid_after_href_change = await state.cdp.send("Page.getAppManifest")
    if not any(
        error.get("critical") == 1
        for error in invalid_after_href_change.get("errors") or []
    ):
        raise SmokeError(
            "href change should replace the cached manifest with the newly fetched parse result: "
            f"{invalid_after_href_change}"
        )
    assert_equal(
        state.fixture_server.request_count(invalid_manifest_route),
        1,
        "href change should fetch the new manifest",
    )
    await state.cdp.send("Page.getAppManifest")
    assert_equal(
        state.fixture_server.request_count(invalid_manifest_route),
        2,
        "critical manifest parse failures should not be cached",
    )

    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-app-manifest-invalid/page"
    )
    invalid = await state.cdp.send("Page.getAppManifest")
    if "data" in invalid:
        raise SmokeError(f"a critically invalid manifest should omit data: {invalid}")
    if not any(error.get("critical") == 1 for error in invalid.get("errors") or []):
        raise SmokeError(f"an invalid manifest should report a critical parse error: {invalid}")

    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-app-manifest-missing/page"
    )
    missing = await state.cdp.send("Page.getAppManifest")
    assert_equal(
        missing.get("url"),
        f"{state.fixture}/chromium-app-manifests/missing.webmanifest",
        "missing manifest URL",
    )
    assert_equal(missing.get("data"), "", "missing manifest data")
    assert_equal(
        missing.get("manifest", {}).get("startUrl"),
        f"{state.fixture}/chromium-app-manifest-missing/page",
        "missing manifest default start URL",
    )

    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-app-manifest-redirect/page"
    )
    redirected = await state.cdp.send("Page.getAppManifest")
    assert_equal(
        redirected.get("url"),
        f"{state.fixture}/chromium-app-manifest-final/final.webmanifest",
        "redirected manifest final URL",
    )
    assert_equal(
        redirected.get("manifest", {}).get("startUrl"),
        f"{state.fixture}/chromium-app-manifest-final/start",
        "redirected manifest resolution base",
    )

    dynamic_url = f"{state.fixture}/chromium-app-manifest-dynamic/page"
    await _navigate_with_cdp_until_dom_ready(state, dynamic_url)
    before = await state.cdp.send("Page.getAppManifest")
    assert_equal(before.get("url"), "", "dynamic manifest before insertion")
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "document.head.insertAdjacentHTML('beforeend', '<link rel=manifest href=/chromium-app-manifests/app.webmanifest>')",
            "returnByValue": True,
        },
    )
    after = await state.cdp.send("Page.getAppManifest")
    assert_equal(after.get("url"), expected_manifest_url, "dynamic manifest after insertion")

    await _navigate_with_cdp_until_dom_ready(state, dynamic_url)
    data_manifest_url = (
        "data:application/manifest+json,"
        "%7B%22start_url%22%3A%22relative-start%22%2C%22scope%22%3A%22.%2F%22%2C"
        "%22icons%22%3A%5B%7B%22src%22%3A%22icon.png%22%7D%5D%7D"
    )
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "document.head.insertAdjacentHTML('beforeend', "
                f"'<link rel=manifest href={data_manifest_url}>')"
            ),
            "returnByValue": True,
        },
    )
    embedded = await state.cdp.send("Page.getAppManifest")
    embedded_manifest = embedded.get("manifest") or {}
    expected_embedded_base = f"{state.fixture}/chromium-app-manifest-dynamic/"
    assert_equal(embedded.get("url"), data_manifest_url, "data manifest URL")
    assert_equal(
        embedded_manifest.get("startUrl"),
        expected_embedded_base + "relative-start",
        "data manifest document-relative start URL",
    )
    assert_equal(
        embedded_manifest.get("scope"),
        expected_embedded_base,
        "data manifest document-relative scope",
    )
    assert_equal(
        (embedded_manifest.get("icons") or [{}])[0].get("url"),
        expected_embedded_base + "icon.png",
        "data manifest document-relative icon URL",
    )
    state.record("chromium_page_get_app_manifest_sample")


async def _assert_manifest_network_lifecycle(
    events: list[dict[str, Any]],
    start: int,
    expected_url: str,
) -> None:
    def matching_request() -> dict[str, Any] | None:
        return next(
            (
                event
                for event in events[start:]
                if event.get("method") == "Network.requestWillBeSent"
                and event.get("params", {}).get("request", {}).get("url") == expected_url
            ),
            None,
        )

    def has_terminal() -> bool:
        request = matching_request()
        if request is None:
            return False
        request_id = request.get("params", {}).get("requestId")
        return any(
            event.get("method") in {"Network.loadingFinished", "Network.loadingFailed"}
            and event.get("params", {}).get("requestId") == request_id
            for event in events[start:]
        )

    await wait_until(has_terminal, "Page.getAppManifest Network lifecycle")
    request = matching_request()
    if request is None:
        raise SmokeError(f"manifest request event is missing for {expected_url}")
    request_params = request.get("params", {})
    assert_equal(request_params.get("type"), "Manifest", "manifest request resource type")
    request_id = request_params.get("requestId")
    matching = [
        event
        for event in events[start:]
        if event.get("params", {}).get("requestId") == request_id
    ]
    methods = [event.get("method") for event in matching]
    if "Network.loadingFailed" in methods:
        raise SmokeError(f"manifest request should not fail: {matching}")
    required = [
        "Network.requestWillBeSent",
        "Network.responseReceived",
        "Network.loadingFinished",
    ]
    if any(method not in methods for method in required):
        raise SmokeError(f"manifest request lifecycle is incomplete: {matching}")
    indexes = [methods.index(method) for method in required]
    if indexes != sorted(indexes):
        raise SmokeError(f"manifest request lifecycle is out of order: {matching}")
    response = matching[methods.index("Network.responseReceived")]
    assert_equal(
        response.get("params", {}).get("type"),
        "Manifest",
        "manifest response resource type",
    )


async def _verify_chromium_runtime_sample(state: SmokeState) -> None:
    events = attach_cdp_event_collector(state.cdp, ["Runtime.executionContextCreated"])
    await state.cdp.send("Runtime.enable")
    await wait_until(
        lambda: any(
            event["params"].get("context", {}).get("id")
            for event in events
            if event["method"] == "Runtime.executionContextCreated"
        ),
        "Chromium Runtime.executionContextCreated sample",
    )

    value_result = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "({ answer: 42, nested: { ok: true }, list: [1, 2] })",
            "returnByValue": True,
        },
    )
    assert_equal(
        value_result.get("result", {}).get("value"),
        {"answer": 42, "nested": {"ok": True}, "list": [1, 2]},
        "Chromium Runtime.evaluate returnByValue object sample",
    )

    exception_result = await state.cdp.send(
        "Runtime.evaluate",
        {"expression": "(() => { throw new Error('chromium sample throw'); })()"},
    )
    details = exception_result.get("exceptionDetails")
    if not details or "chromium sample throw" not in str(details):
        raise SmokeError(f"Runtime.evaluate should return exceptionDetails for thrown Error: {exception_result}")
    state.record("chromium_runtime_evaluate_sample")


async def _verify_chromium_input_session_state_sample(state: SmokeState) -> None:
    page = await state.context.new_page()
    primary = None
    peer = None

    async def install_input_fixture() -> None:
        await page.evaluate(
            """() => {
              document.body.innerHTML = '<input id="field" value="">';
              window.__inputEvents = [];
              const field = document.getElementById('field');
              field.addEventListener('keydown', event => window.__inputEvents.push(`keydown:${event.key}`));
              field.addEventListener('input', () => window.__inputEvents.push(`input:${field.value}`));
              field.focus();
            }"""
        )

    async def input_state() -> dict[str, Any]:
        return await page.evaluate(
            "() => ({ value: document.getElementById('field').value, events: window.__inputEvents })"
        )

    async def dispatch_key(session: Any, key: str) -> None:
        await session.send(
            "Input.dispatchKeyEvent",
            {
                "type": "keyDown",
                "key": key,
                "code": f"Key{key.upper()}",
                "text": key,
            },
        )

    try:
        await page.goto(f"{state.fixture}/plain?chromium-input-session-state")
        await install_input_fixture()
        primary = await state.context.new_cdp_session(page)
        peer = await state.context.new_cdp_session(page)

        assert_equal(
            await peer.send("Input.setIgnoreInputEvents", {"ignore": True}),
            {},
            "Input.setIgnoreInputEvents true result",
        )
        await dispatch_key(primary, "a")
        assert_equal(
            await input_state(),
            {"value": "", "events": []},
            "one Inspector session suppresses target keyboard input",
        )

        assert_equal(
            await primary.send("Input.insertText", {"text": "b"}),
            {},
            "Input.insertText while input events are ignored",
        )
        assert_equal(
            await input_state(),
            {"value": "b", "events": ["input:b"]},
            "Input.insertText bypasses input-event ignoring like Chromium",
        )

        assert_equal(
            await primary.send("Input.setIgnoreInputEvents", {"ignore": False}),
            {},
            "peer Input.setIgnoreInputEvents false result",
        )
        await dispatch_key(primary, "d")
        assert_equal(
            await input_state(),
            {"value": "b", "events": ["input:b"]},
            "one session cannot clear another session's input-event ignore handle",
        )

        await page.goto(f"{state.fixture}/plain?chromium-input-session-navigation")
        await install_input_fixture()
        await dispatch_key(primary, "n")
        assert_equal(
            await input_state(),
            {"value": "", "events": []},
            "Input.setIgnoreInputEvents survives renderer navigation",
        )

        await peer.detach()
        peer = None
        await dispatch_key(primary, "e")
        assert_equal(
            await input_state(),
            {"value": "e", "events": ["keydown:e", "input:e"]},
            "detaching the owning session releases input-event ignoring",
        )
        assert_equal(
            await primary.send("Input.cancelDragging"),
            {},
            "Input.cancelDragging idle result",
        )
        state.record("chromium_input_session_state_sample")
    finally:
        if peer is not None:
            await peer.detach()
        if primary is not None:
            await primary.detach()
        await page.close()


async def _verify_chromium_log_domain_sample(state: SmokeState) -> None:
    page = await state.context.new_page()
    await page.goto(f"{state.fixture}/plain?chromium-log-domain")
    primary = await state.context.new_cdp_session(page)
    peer = await state.context.new_cdp_session(page)
    controls = await state.context.new_cdp_session(page)
    primary_log_events = attach_cdp_event_collector(primary, ["Log.entryAdded"])
    peer_log_events = attach_cdp_event_collector(peer, ["Log.entryAdded"])
    network_events = attach_cdp_event_collector(
        primary,
        [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
            "Network.loadingFailed",
        ],
    )

    def entries_for(events: list[dict[str, Any]], url: str) -> list[dict[str, Any]]:
        return [
            event.get("params", {}).get("entry", {})
            for event in events
            if event.get("params", {}).get("entry", {}).get("url") == url
        ]

    async def failed_fetch(url: str) -> None:
        result = await primary.send(
            "Runtime.evaluate",
            {
                "expression": f"fetch({url!r}).then(response => response.status)",
                "awaitPromise": True,
                "returnByValue": True,
            },
        )
        assert_equal(
            result.get("result", {}).get("value"),
            404,
            "failed fetch status",
        )

    async def failed_image(url: str) -> None:
        result = await primary.send(
            "Runtime.evaluate",
            {
                "expression": (
                    "(() => {"
                    "const image = document.createElement('img');"
                    f"image.src = {url!r};"
                    "document.body.appendChild(image);"
                    "})()"
                ),
            },
        )
        assert_equal(
            result.get("result", {}).get("type"),
            "undefined",
            "failed image injection result",
        )

    try:
        await primary.send("Network.enable")
        fetch_url = f"{state.fixture}/chromium-log-missing-fetch"
        await failed_fetch(fetch_url)
        await wait_until(
            lambda: any(
                event.get("params", {}).get("response", {}).get("url") == fetch_url
                for event in network_events
            ),
            "Network.responseReceived for buffered Log fetch",
        )

        assert_equal(await primary.send("Log.enable"), {}, "Log.enable result")
        buffered_entries = entries_for(primary_log_events, fetch_url)
        if len(buffered_entries) != 1:
            raise SmokeError(
                "Log.enable must synchronously replay exactly one buffered network entry before "
                f"its response resolves: {primary_log_events}"
            )
        buffered = buffered_entries[0]
        network_response = next(
            event["params"]
            for event in network_events
            if event.get("params", {}).get("response", {}).get("url") == fetch_url
        )
        assert_equal(buffered.get("source"), "network", "buffered Log entry source")
        assert_equal(buffered.get("level"), "error", "buffered Log entry level")
        assert_equal(
            buffered.get("text"),
            "Failed to load resource: the server responded with a status of 404 (Not Found)",
            "buffered Log entry text",
        )
        assert_equal(
            buffered.get("networkRequestId"),
            network_response.get("requestId"),
            "Log entry Network request identity",
        )
        timestamp = buffered.get("timestamp")
        if not isinstance(timestamp, (int, float)) or timestamp < 1_000_000_000_000:
            raise SmokeError(f"Log entry timestamp must be Unix epoch milliseconds: {buffered}")

        repeated_enable_start = len(primary_log_events)
        assert_equal(await primary.send("Log.enable"), {}, "repeated Log.enable result")
        await primary.send("Runtime.evaluate", {"expression": "0"})
        assert_equal(
            len(primary_log_events),
            repeated_enable_start,
            "repeated Log.enable must not replay storage",
        )

        assert_equal(await peer.send("Log.enable"), {}, "peer Log.enable result")
        peer_entries = entries_for(peer_log_events, fetch_url)
        if len(peer_entries) != 1:
            raise SmokeError(
                "each Inspector session must independently replay shared Log storage: "
                f"{peer_log_events}"
            )
        assert_equal(
            peer_entries[0].get("networkRequestId"),
            buffered.get("networkRequestId"),
            "peer Log replay Network request identity",
        )

        assert_equal(await peer.send("Log.disable"), {}, "peer Log.disable result")
        image_url = f"{state.fixture}/chromium-log-missing-image.png"
        primary_live_start = len(primary_log_events)
        image_network_start = len(network_events)
        await failed_image(image_url)
        try:
            await wait_until(
                lambda: bool(entries_for(primary_log_events[primary_live_start:], image_url)),
                "live Log.entryAdded for failed image",
            )
        except SmokeError as error:
            image_network_events = [
                event
                for event in network_events[image_network_start:]
                if event.get("params", {}).get("request", {}).get("url") == image_url
                or event.get("params", {}).get("response", {}).get("url") == image_url
            ]
            raise SmokeError(
                f"{error}; networkEvents={image_network_events}, "
                f"primaryLogEvents={primary_log_events[primary_live_start:]}, "
                f"peerLogEvents={peer_log_events}"
            ) from error
        image_entry = entries_for(primary_log_events[primary_live_start:], image_url)[0]
        assert_equal(image_entry.get("source"), "network", "image Log entry source")
        assert_equal(image_entry.get("level"), "error", "image Log entry level")

        assert_equal(await primary.send("Log.clear"), {}, "Log.clear result")
        peer_before_reenable = len(peer_log_events)
        assert_equal(await peer.send("Log.enable"), {}, "peer Log re-enable result")
        await peer.send("Runtime.evaluate", {"expression": "0"})
        assert_equal(
            len(peer_log_events),
            peer_before_reenable,
            "Log.clear must clear target-shared storage for peer sessions",
        )

        assert_equal(await controls.send("Log.clear"), {}, "Log.clear before enable result")
        assert_equal(
            await controls.send("Log.stopViolationsReport"),
            {},
            "Log.stopViolationsReport before enable result",
        )
        disabled_start_error = await _send_cdp_expect_optional_error(
            controls,
            "Log.startViolationsReport",
            {"config": []},
        )
        if not disabled_start_error or "Log is not enabled" not in str(disabled_start_error):
            raise SmokeError(
                "Log.startViolationsReport before Log.enable must fail like Chromium: "
                f"{disabled_start_error}"
            )
        assert_equal(await controls.send("Log.enable"), {}, "controls Log.enable result")
        invalid_params_error = await _send_cdp_expect_optional_error(
            controls,
            "Log.startViolationsReport",
            {},
        )
        if not invalid_params_error or "Invalid parameters" not in str(invalid_params_error):
            raise SmokeError(
                "Log.startViolationsReport must validate config like Chromium: "
                f"{invalid_params_error}"
            )
        assert_equal(
            await controls.send(
                "Log.startViolationsReport",
                {
                    "config": [
                        {"name": "discouragedAPIUse", "threshold": -1},
                        {"name": "handler", "threshold": 50},
                        {"name": "unknown-setting", "threshold": 0},
                    ]
                },
            ),
            {},
            "Log.startViolationsReport result",
        )
        assert_equal(
            await controls.send("Log.stopViolationsReport"),
            {},
            "Log.stopViolationsReport result",
        )

        state.record(
            "chromium_log_domain_sample",
            {
                "bufferedNetworkRequestId": buffered.get("networkRequestId"),
                "bufferedTimestamp": timestamp,
                "primaryEntryCount": len(primary_log_events),
                "peerEntryCount": len(peer_log_events),
            },
        )
    finally:
        await primary.detach()
        await peer.detach()
        await controls.detach()
        await page.close()


async def _verify_chromium_audits_domain_sample(state: SmokeState) -> None:
    page = await state.context.new_page()
    primary = None
    peer = None
    late = None
    try:
        await page.goto(f"{state.fixture}/chromium-audits-quirks-page")
        primary = await state.context.new_cdp_session(page)
        peer = await state.context.new_cdp_session(page)
        primary_events = attach_cdp_event_collector(primary, ["Audits.issueAdded"])
        peer_events = attach_cdp_event_collector(peer, ["Audits.issueAdded"])

        assert_equal(await primary.send("Audits.enable"), {}, "Audits.enable result")
        if len(primary_events) != 1:
            raise SmokeError(
                "Audits.enable must synchronously replay one buffered QuirksModeIssue before "
                f"its response resolves: {primary_events}"
            )
        quirks_issue = primary_events[0].get("params", {}).get("issue", {})
        assert_equal(quirks_issue.get("code"), "QuirksModeIssue", "Audits quirks issue code")
        quirks_details = (
            quirks_issue.get("details", {}).get("quirksModeIssueDetails", {})
        )
        frame_tree = await primary.send("Page.getFrameTree")
        root_frame = frame_tree.get("frameTree", {}).get("frame", {})
        document_node_id = quirks_details.get("documentNodeId")
        if not isinstance(document_node_id, int) or document_node_id <= 0:
            raise SmokeError(f"QuirksModeIssue must carry a backend document node id: {quirks_issue}")
        assert_equal(
            quirks_details.get("isLimitedQuirksMode"),
            False,
            "Audits quirks mode kind",
        )
        assert_equal(
            quirks_details.get("frameId"),
            root_frame.get("id"),
            "Audits quirks frame identity",
        )
        assert_equal(
            quirks_details.get("loaderId"),
            root_frame.get("loaderId"),
            "Audits quirks loader identity",
        )
        assert_equal(
            quirks_details.get("url"),
            page.url,
            "Audits quirks document URL",
        )

        primary_repeat_start = len(primary_events)
        assert_equal(
            await primary.send("Audits.enable"),
            {},
            "repeated Audits.enable result",
        )
        await primary.send("Runtime.evaluate", {"expression": "0"})
        assert_equal(
            len(primary_events),
            primary_repeat_start,
            "repeated Audits.enable must not replay storage",
        )

        assert_equal(await peer.send("Audits.enable"), {}, "peer Audits.enable result")
        if len(peer_events) != 1:
            raise SmokeError(
                "each Inspector session must independently replay Audits storage: "
                f"{peer_events}"
            )
        assert_equal(
            peer_events[0].get("params", {}).get("issue"),
            quirks_issue,
            "peer Audits replay",
        )
        assert_equal(await peer.send("Audits.disable"), {}, "peer Audits.disable result")

        await page.goto(f"{state.fixture}/chromium-audits-csp-page")
        primary_csp_start = len(primary_events)
        peer_disabled_count = len(peer_events)
        evaluate = await primary.send(
            "Runtime.evaluate",
            {
                "expression": (
                    "(() => {"
                    "const script = document.createElement('script');"
                    "script.text = 'globalThis.__auditsSmokeBlocked = true';"
                    "document.body.appendChild(script);"
                    "return globalThis.__auditsSmokeBlocked === true;"
                    "})()"
                ),
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluate.get("result", {}).get("value"),
            False,
            "CSP must block the injected inline script",
        )
        csp_events = primary_events[primary_csp_start:]
        if len(csp_events) != 1:
            raise SmokeError(
                "the live Audits.issueAdded event must arrive before Runtime.evaluate resolves: "
                f"{csp_events}"
            )
        assert_equal(
            len(peer_events),
            peer_disabled_count,
            "Audits.disable must suppress live issues for only that session",
        )
        csp_issue = csp_events[0].get("params", {}).get("issue", {})
        assert_equal(
            csp_issue.get("code"),
            "ContentSecurityPolicyIssue",
            "Audits CSP issue code",
        )
        csp_details = (
            csp_issue.get("details", {}).get("contentSecurityPolicyIssueDetails", {})
        )
        assert_equal(
            csp_details.get("violatedDirective"),
            "script-src-elem",
            "Audits CSP violated directive",
        )
        assert_equal(csp_details.get("isReportOnly"), False, "Audits CSP disposition")
        assert_equal(
            csp_details.get("contentSecurityPolicyViolationType"),
            "kInlineViolation",
            "Audits CSP violation type",
        )
        violating_node_id = csp_details.get("violatingNodeId")
        if not isinstance(violating_node_id, int) or violating_node_id <= 0:
            raise SmokeError(f"CSP issue must identify the violating script node: {csp_issue}")
        source_location = csp_details.get("sourceCodeLocation")
        if source_location is not None and (
            not isinstance(source_location.get("url"), str)
            or not isinstance(source_location.get("lineNumber"), int)
            or not isinstance(source_location.get("columnNumber"), int)
        ):
            raise SmokeError(f"CSP sourceCodeLocation has an invalid CDP shape: {csp_issue}")

        peer_replay_start = len(peer_events)
        assert_equal(await peer.send("Audits.enable"), {}, "peer Audits re-enable result")
        peer_replay = peer_events[peer_replay_start:]
        if len(peer_replay) != 1:
            raise SmokeError(
                "re-enabled Audits session must replay only the current document issue storage: "
                f"{peer_replay}"
            )
        assert_equal(
            peer_replay[0].get("params", {}).get("issue", {}).get("code"),
            "ContentSecurityPolicyIssue",
            "Audits replay after navigation",
        )

        await page.goto(f"{state.fixture}/plain?chromium-audits-storage-reset")
        late = await state.context.new_cdp_session(page)
        late_events = attach_cdp_event_collector(late, ["Audits.issueAdded"])
        assert_equal(await late.send("Audits.enable"), {}, "late Audits.enable result")
        await late.send("Runtime.evaluate", {"expression": "0"})
        assert_equal(
            late_events,
            [],
            "main-frame navigation must clear target Audits issue storage",
        )

        state.record(
            "chromium_audits_domain_sample",
            {
                "quirksDocumentNodeId": document_node_id,
                "cspViolatingNodeId": violating_node_id,
                "primaryIssueCount": len(primary_events),
                "peerIssueCount": len(peer_events),
            },
        )
    finally:
        if late is not None:
            await late.detach()
        if peer is not None:
            await peer.detach()
        if primary is not None:
            await primary.detach()
        await page.close()


async def _verify_chromium_io_resolve_blob_sample(state: SmokeState) -> None:
    first = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "globalThis.__cdpSmokeBlob = new Blob(['hello world'], {type:'text/plain'})",
            "returnByValue": False,
        },
    )
    first_object_id = first.get("result", {}).get("objectId")
    if not isinstance(first_object_id, str) or not first_object_id:
        raise SmokeError(f"Runtime.evaluate should return a Blob objectId: {first}")

    first_resolution = await state.cdp.send("IO.resolveBlob", {"objectId": first_object_id})
    first_uuid = first_resolution.get("uuid")
    if not isinstance(first_uuid, str) or UUID(first_uuid).version != 4:
        raise SmokeError(f"IO.resolveBlob should return a v4 UUID: {first_resolution}")
    repeated = await state.cdp.send("IO.resolveBlob", {"objectId": first_object_id})
    assert_equal(repeated.get("uuid"), first_uuid, "IO.resolveBlob stable UUID")

    second = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "globalThis.__cdpSmokeBinaryBlob = new Blob([new Uint8Array([0,255,65])])",
            "returnByValue": False,
        },
    )
    second_object_id = second.get("result", {}).get("objectId")
    if not isinstance(second_object_id, str) or not second_object_id:
        raise SmokeError(f"Runtime.evaluate should return a second Blob objectId: {second}")
    second_resolution = await state.cdp.send("IO.resolveBlob", {"objectId": second_object_id})
    second_uuid = second_resolution.get("uuid")
    if not isinstance(second_uuid, str) or UUID(second_uuid).version != 4:
        raise SmokeError(f"second IO.resolveBlob should return a v4 UUID: {second_resolution}")
    if second_uuid == first_uuid:
        raise SmokeError("distinct Blob objects must not share a DevTools UUID")

    first_handle = f"blob:{first_uuid}"
    first_read = await state.cdp.send("IO.read", {"handle": first_handle})
    assert_equal(first_read.get("base64Encoded"), False, "text Blob IO.read encoding")
    assert_equal(first_read.get("data"), "hello world", "text Blob IO.read data")
    assert_equal(first_read.get("eof"), True, "text Blob IO.read eof")
    await state.cdp.send("IO.close", {"handle": first_handle})

    reopened = await state.cdp.send(
        "IO.read",
        {"handle": first_handle, "offset": 0, "size": 5},
    )
    assert_equal(reopened.get("data"), "hello", "closed Blob stream reopens from backing")
    assert_equal(reopened.get("eof"), False, "reopened Blob partial read eof")
    await state.cdp.send("IO.close", {"handle": first_handle})

    second_handle = f"blob:{second_uuid}"
    binary_read = await state.cdp.send("IO.read", {"handle": second_handle})
    assert_equal(binary_read.get("base64Encoded"), True, "binary Blob IO.read encoding")
    assert_equal(
        base64.b64decode(binary_read.get("data", "")),
        b"\x00\xffA",
        "binary Blob IO.read data",
    )
    await state.cdp.send("IO.close", {"handle": second_handle})

    non_blob = await state.cdp.send(
        "Runtime.evaluate",
        {"expression": "({answer: 42})", "returnByValue": False},
    )
    non_blob_error = await _send_cdp_expect_optional_error(
        state.cdp,
        "IO.resolveBlob",
        {"objectId": non_blob.get("result", {}).get("objectId")},
    )
    if not non_blob_error or "Object id doesn't reference a Blob" not in str(non_blob_error):
        raise SmokeError(f"IO.resolveBlob non-Blob error should match Chromium: {non_blob_error}")

    invalid_error = await _send_cdp_expect_optional_error(
        state.cdp,
        "IO.resolveBlob",
        {"objectId": "not-a-valid-object-id"},
    )
    if not invalid_error or "Invalid remote object id" not in str(invalid_error):
        raise SmokeError(f"IO.resolveBlob invalid object error should match Chromium: {invalid_error}")

    released = await state.cdp.send(
        "Runtime.evaluate",
        {"expression": "new Blob(['released'])", "returnByValue": False},
    )
    released_object_id = released.get("result", {}).get("objectId")
    await state.cdp.send("Runtime.releaseObject", {"objectId": released_object_id})
    released_error = await _send_cdp_expect_optional_error(
        state.cdp,
        "IO.resolveBlob",
        {"objectId": released_object_id},
    )
    if not released_error or "Could not find object with given id" not in str(released_error):
        raise SmokeError(f"IO.resolveBlob released object error should match Chromium: {released_error}")

    non_blob_session = await state.context.new_cdp_session(state.page)
    blob_session = await state.context.new_cdp_session(state.page)
    try:
        await non_blob_session.send(
            "Runtime.evaluate",
            {"expression": "({owner: 'non-blob-session'})", "returnByValue": False},
        )
        attached_blob = await blob_session.send(
            "Runtime.evaluate",
            {"expression": "new Blob(['attached'])", "returnByValue": False},
        )
        attached_object_id = attached_blob.get("result", {}).get("objectId")
        attached_resolution = await blob_session.send(
            "IO.resolveBlob",
            {"objectId": attached_object_id},
        )
        if not attached_resolution.get("uuid"):
            raise SmokeError(f"attached IO.resolveBlob should succeed: {attached_resolution}")
        cross_session_error = await _send_cdp_expect_optional_error(
            non_blob_session,
            "IO.resolveBlob",
            {"objectId": attached_object_id},
        )
        if cross_session_error is None:
            raise SmokeError("IO.resolveBlob must unwrap objectId in the calling Inspector session")
    finally:
        await non_blob_session.detach()
        await blob_session.detach()

    state.record("chromium_io_resolve_blob_sample")


async def _verify_chromium_performance_enable_sample(state: SmokeState) -> None:
    async def expect_error(method: str, params: dict[str, Any], message: str) -> None:
        error = await _send_cdp_expect_optional_error(state.cdp, method, params)
        if not error or message not in str(error):
            raise SmokeError(f"{method} should fail with {message!r}: {error}")

    for params in (
        {},
        {"timeDomain": "timeTicks"},
        {"timeDomain": "threadTicks"},
        {"timeDomain": None},
    ):
        await state.cdp.send("Performance.enable", params)
        await state.cdp.send("Performance.disable")

    await expect_error(
        "Performance.enable",
        {"timeDomain": "bogusTicks"},
        "Invalid time domain specification.",
    )
    await expect_error(
        "Performance.enable",
        {"timeDomain": "TimeTicks"},
        "Invalid time domain specification.",
    )
    await expect_error(
        "Performance.enable",
        {"timeDomain": 1},
        "Invalid parameters",
    )

    await state.cdp.send("Performance.enable", {"timeDomain": "threadTicks"})
    await state.cdp.send("Performance.enable", {"timeDomain": "threadTicks"})
    await expect_error(
        "Performance.enable",
        {},
        "Cannot change time domain while performance metrics collection is enabled.",
    )
    await expect_error(
        "Performance.setTimeDomain",
        {"timeDomain": "timeTicks"},
        "Cannot set time domain while performance metrics collection is enabled.",
    )
    await state.cdp.send("Performance.disable")
    await state.cdp.send("Performance.disable")

    await state.cdp.send("Performance.setTimeDomain", {"timeDomain": "threadTicks"})
    await expect_error(
        "Performance.setTimeDomain",
        {"timeDomain": "bogusTicks"},
        "Invalid time domain specification.",
    )
    await state.cdp.send("Performance.enable", {"timeDomain": None})
    await state.cdp.send("Performance.disable")

    attached = await state.context.new_cdp_session(state.page)
    try:
        primary_disabled = await _performance_metrics(state.cdp)
        attached_disabled = await _performance_metrics(attached)
        if primary_disabled or attached_disabled:
            raise SmokeError(
                "Performance.getMetrics should be empty before each Inspector session is enabled"
            )

        await state.cdp.send("Performance.enable")
        if await _performance_metrics(attached):
            raise SmokeError("Performance.enable must not enable an attached Inspector session")
        await attached.send("Performance.enable", {"timeDomain": "threadTicks"})
        if not await _performance_metrics(state.cdp) or not await _performance_metrics(attached):
            raise SmokeError("enabled Performance sessions should each expose metrics")
        await state.cdp.send("Performance.disable")
        if not await _performance_metrics(attached):
            raise SmokeError("disabling one Performance session must not disable another")
        await attached.send("Performance.disable")
    finally:
        await attached.detach()
    state.record("chromium_performance_enable_sample")


async def _verify_chromium_performance_metrics_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/chromium-cdp-dom-page")
    before_enable = await _performance_metrics(state.cdp)
    await state.cdp.send("Performance.enable")
    enabled_metrics = await _performance_metrics(state.cdp)
    await state.page.evaluate("() => { for (let i = 0; i < 1000; i += 1) Math.sqrt(i); }")
    after_work = await _performance_metrics(state.cdp)
    await state.cdp.send("Performance.disable")
    after_disable = await _performance_metrics(state.cdp)

    required = {
        "Timestamp",
        "Documents",
        "Frames",
        "Nodes",
        "LayoutCount",
        "RecalcStyleCount",
        "LayoutDuration",
        "RecalcStyleDuration",
        "ScriptDuration",
        "TaskDuration",
        "JSHeapUsedSize",
        "JSHeapTotalSize",
    }
    if before_enable:
        raise SmokeError(f"Performance.getMetrics before enable should be empty: {before_enable}")
    if after_disable:
        raise SmokeError(f"Performance.getMetrics after disable should be empty: {after_disable}")
    for label, metrics in [
        ("after enable", enabled_metrics),
        ("after work", after_work),
    ]:
        missing = required.difference(metrics)
        if missing:
            raise SmokeError(f"Performance.getMetrics {label} missing metrics: {sorted(missing)}")
        for name in required:
            value = metrics[name]
            if not isinstance(value, (int, float)) or value < 0:
                raise SmokeError(f"Performance.getMetrics {label} metric {name} invalid: {value!r}")
    if after_work["Timestamp"] < enabled_metrics["Timestamp"]:
        raise SmokeError(f"Performance Timestamp should be monotonic: {enabled_metrics['Timestamp']} -> {after_work['Timestamp']}")
    state.record("chromium_performance_metrics_sample")


async def _verify_chromium_cpu_throttling_multiple_pages_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain")
    second_page = await state.context.new_page()
    second_cdp = await state.context.new_cdp_session(second_page)
    try:
        await second_page.goto(
            f"{state.fixture}/plain?cpu-throttle-second",
            wait_until="load",
            timeout=10_000,
        )
        await state.cdp.send("Emulation.setCPUThrottlingRate", {"rate": 2.0})
        await second_cdp.send("Emulation.setCPUThrottlingRate", {"rate": 3.0})
        await state.cdp.send("Emulation.setCPUThrottlingRate", {"rate": 1.0})
        await second_cdp.send("Emulation.setCPUThrottlingRate", {"rate": 1.0})
    finally:
        await second_page.close()
    state.record("chromium_cpu_throttling_multiple_pages_sample")


async def _verify_chromium_profiler_cpu_profile_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain")
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await state.cdp.send("Emulation.setCPUThrottlingRate", {"rate": 2.0})
    await state.cdp.send("Profiler.start")
    try:
        burn = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerSmokeWork() {
                            let total = 0;
                            for (let i = 0; i < 50000; ++i)
                                total += Math.sqrt(i);
                            return total > 0;
                        }
                        return chromiumProfilerSmokeWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            burn.get("result", {}).get("value"),
            True,
            "Chromium Profiler CPU profile work sample",
        )
        profile_result = await state.cdp.send("Profiler.stop")
    finally:
        await state.cdp.send("Emulation.setCPUThrottlingRate", {"rate": 1.0})
        await state.cdp.send("Profiler.disable")

    profile = profile_result.get("profile") or {}
    if not isinstance(profile.get("startTime"), (int, float)) or not isinstance(profile.get("endTime"), (int, float)):
        raise SmokeError(f"Profiler.stop should return startTime/endTime: {profile_result}")
    nodes = profile.get("nodes") or []
    if not isinstance(nodes, list) or not nodes:
        raise SmokeError(f"Profiler.stop should return non-empty profile nodes: {profile_result}")
    _assert_profile_tree_shape(profile, "Profiler.stop CPU profile")
    if profile["endTime"] < profile["startTime"]:
        raise SmokeError(f"Profiler profile time range should be monotonic: {profile_result}")
    state.record("chromium_profiler_cpu_profile_sample")


async def _verify_chromium_profiler_cpu_profile_with_throttling_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-cpu-throttling")
    await state.cdp.send("Emulation.setCPUThrottlingRate", {"rate": 4.0})
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await state.cdp.send("Profiler.start")
    try:
        burn = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerThrottledWork() {
                            let count = 0;
                            const limit = 10000000;
                            const target = Date.now() + 1000;
                            for (let i = 0; i < limit && Date.now() < target; ++i)
                                count += i;
                            window.__chromiumProfilerThrottledCount = count;
                            return count >= 0;
                        }
                        return chromiumProfilerThrottledWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            burn.get("result", {}).get("value"),
            True,
            "Chromium Profiler throttled CPU work sample",
        )
        profile_result = await state.cdp.send("Profiler.stop")
    finally:
        await state.cdp.send("Emulation.setCPUThrottlingRate", {"rate": 1.0})
        await state.cdp.send("Profiler.disable")

    profile = profile_result.get("profile") or {}
    nodes = profile.get("nodes") or []
    if not isinstance(nodes, list) or not nodes:
        raise SmokeError(f"Profiler.stop should return non-empty throttled profile nodes: {profile_result}")
    _assert_profile_tree_shape(profile, "Profiler throttled CPU profile")
    if "chromiumProfilerThrottledWork" not in _profile_function_names(profile):
        raise SmokeError(f"Throttled CPU profile should include sampled work frame: {profile_result}")
    state.record("chromium_profiler_cpu_profile_with_throttling_sample")


async def _verify_chromium_profiler_stop_without_start_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-stop-without-start")
    stop_error = await _send_cdp_expect_optional_error(state.cdp, "Profiler.stop", {})
    if not stop_error or "No recording profiles found" not in str(stop_error):
        raise SmokeError(f"Profiler.stop without Profiler.start should return recording-not-found error: {stop_error}")
    state.record("chromium_profiler_stop_without_start_sample")


async def _verify_chromium_profiler_sampling_interval_contract_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-sampling-interval")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.start")
    try:
        interval_error = await _send_cdp_expect_optional_error(
            state.cdp,
            "Profiler.setSamplingInterval",
            {"interval": 200},
        )
        if not interval_error or "Cannot change sampling interval" not in str(interval_error):
            raise SmokeError(
                "Profiler.setSamplingInterval while recording should return Chromium error: "
                f"{interval_error}"
            )
        stopped = await state.cdp.send("Profiler.stop")
        if not (stopped.get("profile") or {}).get("nodes"):
            raise SmokeError(f"Profiler.stop after sampling interval contract sample should return a profile: {stopped}")
    finally:
        await state.cdp.send("Profiler.disable")
    state.record("chromium_profiler_sampling_interval_contract_sample")


async def _verify_chromium_profiler_enable_disable_contract_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-enable-disable")
    events = attach_cdp_event_collector(
        state.cdp,
        ["Profiler.consoleProfileStarted", "Profiler.consoleProfileFinished"],
    )
    start = len(events)

    start_error = await _send_cdp_expect_optional_error(state.cdp, "Profiler.start", {})
    if not start_error or "Profiler is not enabled" not in str(start_error):
        raise SmokeError(f"Profiler.start without Profiler.enable should return Chromium error: {start_error}")

    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.start")
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "console.profile('chromium-cdp-enable-disable-console-profile')",
            "returnByValue": True,
        },
    )
    await wait_until(
        lambda: _has_event(events[start:], "Profiler.consoleProfileStarted"),
        "Chromium Profiler enable-disable console profile start event",
    )

    await state.cdp.send("Profiler.disable")
    await state.cdp.send("Profiler.enable")
    stop_error = await _send_cdp_expect_optional_error(state.cdp, "Profiler.stop", {})
    if not stop_error or "No recording profiles found" not in str(stop_error):
        raise SmokeError(f"Profiler.disable should stop frontend initiated profile: {stop_error}")

    profile_end = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "console.profileEnd('chromium-cdp-enable-disable-console-profile')",
            "returnByValue": True,
        },
    )
    if profile_end.get("exceptionDetails"):
        raise SmokeError(f"console.profileEnd after Profiler.disable should not throw: {profile_end}")
    await state.cdp.send("Profiler.disable")
    state.record("chromium_profiler_enable_disable_contract_sample")


async def _verify_chromium_profiler_console_profile_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-console-profile")
    events = attach_cdp_event_collector(
        state.cdp,
        ["Profiler.consoleProfileStarted", "Profiler.consoleProfileFinished"],
    )
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    start = len(events)
    try:
        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumConsoleProfileSmokeWork() {
                            let total = 0;
                            for (let i = 0; i < 500000; ++i)
                                total += Math.sqrt(i + 1);
                            return total > 0;
                        }
                        console.profile('chromium-cdp-console-profile');
                        const result = chromiumConsoleProfileSmokeWork();
                        console.profileEnd('chromium-cdp-console-profile');
                        return result;
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            True,
            "Chromium Profiler console profile work sample",
        )
        await wait_until(
            lambda: _has_event(events[start:], "Profiler.consoleProfileStarted")
            and _has_event(events[start:], "Profiler.consoleProfileFinished"),
            "Chromium Profiler console profile started/finished events",
        )

        try:
            await state.cdp.send("Profiler.stop")
        except Exception as error:
            if "No recording profiles found" not in str(error):
                raise SmokeError(f"Profiler.stop after console.profile should report no frontend recording: {error}") from error
        else:
            raise SmokeError("console.profile must not create a frontend Profiler.start recording")
    finally:
        await state.cdp.send("Profiler.disable")

    started = next(event for event in events[start:] if event["method"] == "Profiler.consoleProfileStarted")
    finished = next(event for event in events[start:] if event["method"] == "Profiler.consoleProfileFinished")
    if started["params"].get("title") != "chromium-cdp-console-profile":
        raise SmokeError(f"consoleProfileStarted should include requested title: {started}")
    if finished["params"].get("title") != "chromium-cdp-console-profile":
        raise SmokeError(f"consoleProfileFinished should include requested title: {finished}")
    if not started["params"].get("id") or finished["params"].get("id") != started["params"].get("id"):
        raise SmokeError(f"console profile start/finish ids should match: started={started}, finished={finished}")
    profile = finished["params"].get("profile") or {}
    _assert_profile_tree_shape(profile, "consoleProfileFinished profile")
    if "chromiumConsoleProfileSmokeWork" not in _profile_function_names(profile):
        raise SmokeError(f"consoleProfileFinished profile should include sampled page work: {finished}")
    state.record("chromium_profiler_console_profile_sample")


async def _verify_chromium_profiler_nested_console_profile_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-nested-console-profile")
    events = attach_cdp_event_collector(
        state.cdp,
        ["Profiler.consoleProfileFinished"],
    )
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    start = len(events)
    try:
        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function collectProfiles() {
                            function chromiumNestedConsoleProfileBurn(seed) {
                                let total = seed;
                                for (let i = 0; i < 500000; ++i)
                                    total += Math.sqrt(i + seed);
                                return total > 0;
                            }
                            console.profile('outer');
                            chromiumNestedConsoleProfileBurn(1);
                            console.profile(42);
                            chromiumNestedConsoleProfileBurn(2);
                            console.profileEnd('outer');
                            chromiumNestedConsoleProfileBurn(3);
                            console.profileEnd(42);
                            return true;
                        }
                        return collectProfiles();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            True,
            "Chromium Profiler nested console profile work sample",
        )
        await wait_until(
            lambda: len(_events_with_method(events[start:], "Profiler.consoleProfileFinished")) >= 2,
            "Chromium nested console profile finished events",
        )
    finally:
        await state.cdp.send("Profiler.disable")

    finished = _events_with_method(events[start:], "Profiler.consoleProfileFinished")
    if len(finished) != 2:
        raise SmokeError(f"Chromium console-profile.js should finish exactly two profiles: {finished}")
    if not any(event["params"].get("title") == "outer" for event in finished):
        raise SmokeError(f"Nested console profile should finish the outer profile: {finished}")
    numeric_profile = next((event for event in finished if event["params"].get("title") == "42"), None)
    if not numeric_profile:
        raise SmokeError(f"Nested console profile should stringify numeric title 42: {finished}")
    profile = numeric_profile["params"].get("profile") or {}
    _assert_profile_tree_shape(profile, "nested consoleProfileFinished profile")
    if "collectProfiles" not in _profile_function_names(profile):
        raise SmokeError(f"Numeric nested profile should include collectProfiles frame: {numeric_profile}")
    state.record("chromium_profiler_nested_console_profile_sample")


async def _verify_chromium_profiler_parameterless_profile_end_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-parameterless-profile-end")
    events = attach_cdp_event_collector(
        state.cdp,
        ["Profiler.consoleProfileFinished"],
    )
    await state.cdp.send("Profiler.enable")
    start = len(events)
    try:
        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function collectProfiles() {
                            console.profile();
                            console.profile('titled');
                            console.profileEnd('titled');
                            console.profileEnd();
                            return true;
                        }
                        return collectProfiles();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            True,
            "Chromium Profiler parameterless profileEnd work sample",
        )
        await wait_until(
            lambda: len(_events_with_method(events[start:], "Profiler.consoleProfileFinished")) >= 2,
            "Chromium parameterless profileEnd finished events",
        )

        try:
            await state.cdp.send("Profiler.stop")
        except Exception as error:
            if "No recording profiles found" not in str(error):
                raise SmokeError(f"Profiler.stop after parameterless profileEnd should report no frontend recording: {error}") from error
        else:
            raise SmokeError("parameterless console.profileEnd must not create a frontend Profiler.start recording")
    finally:
        await state.cdp.send("Profiler.disable")

    finished = _events_with_method(events[start:], "Profiler.consoleProfileFinished")
    if len(finished) != 2:
        raise SmokeError(f"Chromium console-profileEnd-parameterless-crash.js should finish exactly two profiles: {finished}")
    if not any(event["params"].get("title") == "titled" for event in finished):
        raise SmokeError(f"Parameterless profileEnd sample should finish titled profile: {finished}")
    state.record("chromium_profiler_parameterless_profile_end_sample")


async def _verify_chromium_profiler_navigation_profile_continuity_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-navigation-before")
    await state.cdp.send("Profiler.enable")
    await state.cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await state.cdp.send("Profiler.start")
    try:
        before = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerBeforeNavigationWork() {
                            let total = 0;
                            for (let i = 0; i < 250000; ++i)
                                total += Math.sqrt(i);
                            return total > 0;
                        }
                        return chromiumProfilerBeforeNavigationWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            before.get("result", {}).get("value"),
            True,
            "Chromium Profiler pre-navigation work sample",
        )

        await state.cdp.send(
            "Page.navigate",
            {"url": f"{state.fixture}/plain?profiler-navigation-after"},
        )
        await state.page.wait_for_load_state("load", timeout=10_000)

        after = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerAfterNavigationWork() {
                            let total = 0;
                            for (let i = 0; i < 250000; ++i)
                                total += Math.sqrt(i + 1);
                            return total > 0;
                        }
                        return chromiumProfilerAfterNavigationWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            after.get("result", {}).get("value"),
            True,
            "Chromium Profiler post-navigation work sample",
        )
        profile_result = await state.cdp.send("Profiler.stop")
    finally:
        await state.cdp.send("Profiler.disable")

    profile = profile_result.get("profile") or {}
    _assert_profile_tree_shape(profile, "Profiler.stop navigation continuity profile")
    function_names = _profile_function_names(profile)
    # A document replacement creates a new isolate-local Profiler backend. The recording control
    # state is restored, but old-isolate samples are intentionally not merged into Profiler.stop.
    # The pre-navigation evaluation above proves that recording began before navigation; this
    # external smoke only requires the replacement backend to keep recording without a second
    # Profiler.start and return its own samples.
    if "chromiumProfilerAfterNavigationWork" not in function_names:
        raise SmokeError(
            "Profiler.stop after navigation should include post-navigation work "
            f"function; names={sorted(function_names)}"
        )
    state.record("chromium_profiler_navigation_profile_continuity_sample")


async def _verify_chromium_profiler_attached_session_navigation_profile_continuity_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-attached-navigation-before")
    attached_cdp = await state.context.new_cdp_session(state.page)
    await state.cdp.send("Profiler.enable")
    await attached_cdp.send("Profiler.enable")
    await attached_cdp.send("Profiler.setSamplingInterval", {"interval": 100})
    await attached_cdp.send("Profiler.start")
    try:
        primary_stop_before = await _send_cdp_expect_optional_error(state.cdp, "Profiler.stop", {})
        if not primary_stop_before or "No recording profiles found" not in str(primary_stop_before):
            raise SmokeError(
                "Primary CDP session must not observe attached Profiler.start recording: "
                f"{primary_stop_before}"
            )

        before = await attached_cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerAuxBeforeNavigationWork() {
                            let total = 0;
                            for (let i = 0; i < 250000; ++i)
                                total += Math.sqrt(i);
                            return total > 0;
                        }
                        return chromiumProfilerAuxBeforeNavigationWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            before.get("result", {}).get("value"),
            True,
            "Chromium Profiler attached pre-navigation work sample",
        )

        await attached_cdp.send(
            "Page.navigate",
            {"url": f"{state.fixture}/plain?profiler-attached-navigation-after"},
        )
        await state.page.wait_for_load_state("load", timeout=10_000)

        after = await attached_cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                        function chromiumProfilerAuxAfterNavigationWork() {
                            let total = 0;
                            for (let i = 0; i < 250000; ++i)
                                total += Math.sqrt(i + 1);
                            return total > 0;
                        }
                        return chromiumProfilerAuxAfterNavigationWork();
                    })()
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            after.get("result", {}).get("value"),
            True,
            "Chromium Profiler attached post-navigation work sample",
        )

        profile_result = await attached_cdp.send("Profiler.stop")
        primary_stop_after = await _send_cdp_expect_optional_error(state.cdp, "Profiler.stop", {})
        if not primary_stop_after or "No recording profiles found" not in str(primary_stop_after):
            raise SmokeError(
                "Primary CDP session must stay isolated after attached Profiler.stop: "
                f"{primary_stop_after}"
            )
    finally:
        await attached_cdp.send("Profiler.disable")
        await state.cdp.send("Profiler.disable")

    profile = profile_result.get("profile") or {}
    _assert_profile_tree_shape(profile, "attached Profiler.stop navigation continuity profile")
    function_names = _profile_function_names(profile)
    # As above, navigation restores the attached session's recording state, not samples from the
    # disposed isolate. Session isolation and replacement-backend sampling are the stable contract.
    if "chromiumProfilerAuxAfterNavigationWork" not in function_names:
        raise SmokeError(
            "Attached Profiler.stop after navigation should include post-navigation work "
            f"function; names={sorted(function_names)}"
        )
    state.record("chromium_profiler_attached_session_navigation_profile_continuity_sample")


async def _verify_chromium_profiler_attached_session_detach_clears_state_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-session-detach")
    old_cdp = await state.context.new_cdp_session(state.page)
    await old_cdp.send("Profiler.enable")
    await old_cdp.send("Profiler.start")
    await old_cdp.detach()

    new_cdp = await state.context.new_cdp_session(state.page)
    try:
        start_error = await _send_cdp_expect_optional_error(new_cdp, "Profiler.start", {})
        if not start_error or "Profiler is not enabled" not in str(start_error):
            raise SmokeError(
                "New attached CDP session must not inherit detached Profiler.enable/start state: "
                f"{start_error}"
            )
        stop_error = await _send_cdp_expect_optional_error(new_cdp, "Profiler.stop", {})
        if not stop_error or "No recording profiles found" not in str(stop_error):
            raise SmokeError(
                "New attached CDP session must not inherit detached recording state: "
                f"{stop_error}"
            )
    finally:
        await new_cdp.detach()

    state.record("chromium_profiler_attached_session_detach_clears_state_sample")


async def _verify_chromium_profiler_precise_coverage_error_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-precise-coverage-error")
    await state.cdp.send("Profiler.enable")
    try:
        take_error = await _send_cdp_expect_optional_error(state.cdp, "Profiler.takePreciseCoverage", {})
        if not take_error or "Precise coverage has not been started" not in str(take_error):
            raise SmokeError(
                "Profiler.takePreciseCoverage before startPreciseCoverage should return Chromium error: "
                f"{take_error}"
            )
    finally:
        await state.cdp.send("Profiler.disable")

    state.record("chromium_profiler_precise_coverage_error_sample")


async def _verify_chromium_profiler_precise_coverage_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain")
    await state.cdp.send("Profiler.enable")
    try:
        started = await state.cdp.send(
            "Profiler.startPreciseCoverage",
            {"callCount": True, "detailed": True, "allowTriggeredUpdates": False},
        )
        if not isinstance(started.get("timestamp"), (int, float)):
            raise SmokeError(f"Profiler.startPreciseCoverage should return timestamp: {started}")

        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    function chromiumProfilerCoverageSmoke(value) {
                        if (value > 0)
                            return value + 1;
                        return value - 1;
                    }
                    chromiumProfilerCoverageSmoke(41)
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            42,
            "Chromium Profiler precise coverage work sample",
        )

        precise = await state.cdp.send("Profiler.takePreciseCoverage")
        if not isinstance(precise.get("timestamp"), (int, float)):
            raise SmokeError(f"Profiler.takePreciseCoverage should return timestamp: {precise}")
        _assert_script_coverage_array(precise, "Profiler.takePreciseCoverage")

        best_effort = await state.cdp.send("Profiler.getBestEffortCoverage")
        _assert_script_coverage_array(best_effort, "Profiler.getBestEffortCoverage")
    finally:
        await state.cdp.send("Profiler.stopPreciseCoverage")
        await state.cdp.send("Profiler.disable")

    state.record("chromium_profiler_precise_coverage_sample")


async def _verify_chromium_profiler_precise_coverage_counter_reset_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-coverage-counter-reset")
    await state.cdp.send("Profiler.enable")
    precise_started = False
    try:
        await state.cdp.send(
            "Profiler.startPreciseCoverage",
            {"callCount": True, "detailed": False, "allowTriggeredUpdates": False},
        )
        precise_started = True

        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    function chromiumProfilerCoverageCounterResetSmoke() {
                        return 41;
                    }
                    chromiumProfilerCoverageCounterResetSmoke();
                    //# sourceURL=chromium-profiler-coverage-counter-reset.js
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            41,
            "Chromium Profiler coverage counter reset work sample",
        )

        first = await state.cdp.send("Profiler.takePreciseCoverage")
        first_function = _find_coverage_function(
            _find_script_coverage_by_url(first, "chromium-profiler-coverage-counter-reset.js") or {},
            "chromiumProfilerCoverageCounterResetSmoke",
        )
        if not first_function:
            raise SmokeError(f"First takePreciseCoverage should include target function: {first}")
        if _coverage_function_total_count(first_function) <= 0:
            raise SmokeError(f"First takePreciseCoverage should include executed counts: {first_function}")

        second = await state.cdp.send("Profiler.takePreciseCoverage")
        second_function = _find_coverage_function(
            _find_script_coverage_by_url(second, "chromium-profiler-coverage-counter-reset.js") or {},
            "chromiumProfilerCoverageCounterResetSmoke",
        )
        second_count = _coverage_function_total_count(second_function or {})
        if second_count != 0:
            raise SmokeError(
                "takePreciseCoverage should not report stale execution counts until code runs again: "
                f"{second}"
            )

        rerun = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": "chromiumProfilerCoverageCounterResetSmoke()",
                "returnByValue": True,
            },
        )
        assert_equal(
            rerun.get("result", {}).get("value"),
            41,
            "Chromium Profiler coverage counter reset rerun sample",
        )

        third = await state.cdp.send("Profiler.takePreciseCoverage")
        third_function = _find_coverage_function(
            _find_script_coverage_by_url(third, "chromium-profiler-coverage-counter-reset.js") or {},
            "chromiumProfilerCoverageCounterResetSmoke",
        )
        if not third_function:
            raise SmokeError(f"Third takePreciseCoverage should include target function after rerun: {third}")
        if _coverage_function_total_count(third_function) <= 0:
            raise SmokeError(f"Coverage counters should resume after code runs again: {third_function}")
    finally:
        if precise_started:
            await state.cdp.send("Profiler.stopPreciseCoverage")
        await state.cdp.send("Profiler.disable")

    state.record("chromium_profiler_precise_coverage_counter_reset_sample")


async def _verify_chromium_profiler_precise_block_coverage_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-coverage-block")
    await state.cdp.send("Profiler.enable")
    precise_started = False
    try:
        await state.cdp.send(
            "Profiler.startPreciseCoverage",
            {"callCount": True, "detailed": True, "allowTriggeredUpdates": False},
        )
        precise_started = True

        evaluated = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": """
                    function chromiumProfilerCoverageBlockSmoke(value) {
                        if (value === 0)
                            return 0;
                        if (value > 0)
                            return value + 1;
                        return value - 1;
                    }
                    chromiumProfilerCoverageBlockSmoke(41)
                    //# sourceURL=chromium-profiler-coverage-block.js
                """,
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluated.get("result", {}).get("value"),
            42,
            "Chromium Profiler block coverage work sample",
        )

        coverage = await state.cdp.send("Profiler.takePreciseCoverage")
        script = _find_script_coverage_by_url(coverage, "chromium-profiler-coverage-block.js")
        if not script:
            raise SmokeError(f"Profiler.takePreciseCoverage should include sourceURL script: {coverage}")
        function = _find_coverage_function(script, "chromiumProfilerCoverageBlockSmoke")
        if not function:
            raise SmokeError(f"Profiler.takePreciseCoverage should include target function: {script}")
        if function.get("isBlockCoverage") is not True:
            raise SmokeError(f"Detailed precise coverage should report block coverage: {function}")
        ranges = function.get("ranges") or []
        if len(ranges) < 2:
            raise SmokeError(f"Block coverage should expose multiple ranges for branch function: {function}")
        counts = [range_.get("count") for range_ in ranges if isinstance(range_, dict)]
        if not any(count == 0 for count in counts) or not any(isinstance(count, int) and count > 0 for count in counts):
            raise SmokeError(f"Block coverage should expose executed and unexecuted ranges: {function}")
    finally:
        if precise_started:
            await state.cdp.send("Profiler.stopPreciseCoverage")
        await state.cdp.send("Profiler.disable")

    state.record("chromium_profiler_precise_block_coverage_sample")


async def _verify_chromium_profiler_best_effort_with_precise_coverage_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/plain?profiler-best-effort-with-precise")

    async def run_case(case_name: str, start_params: dict[str, Any]) -> None:
        await state.cdp.send("Profiler.enable")
        precise_started = False
        try:
            await state.cdp.send("Profiler.startPreciseCoverage", start_params)
            precise_started = True

            function_name = f"chromiumProfilerBestEffortWithPrecise{case_name}"
            source_url = f"chromium-profiler-best-effort-with-precise-{case_name}.js"
            evaluated = await state.cdp.send(
                "Runtime.evaluate",
                {
                    "expression": f"""
                        function {function_name}(value) {{
                            if (value > 0)
                                return value + 1;
                            return value - 1;
                        }}
                        {function_name}(41)
                        //# sourceURL={source_url}
                    """,
                    "returnByValue": True,
                },
            )
            assert_equal(
                evaluated.get("result", {}).get("value"),
                42,
                f"Chromium Profiler best-effort with precise coverage work sample {case_name}",
            )

            first = await state.cdp.send("Profiler.getBestEffortCoverage")
            _assert_script_coverage_array(first, "Profiler.getBestEffortCoverage")
            _assert_coverage_contains_function(
                first,
                source_url,
                function_name,
                f"Profiler.getBestEffortCoverage with active precise coverage {case_name}",
            )

            second = await state.cdp.send("Profiler.getBestEffortCoverage")
            _assert_script_coverage_array(second, "Profiler.getBestEffortCoverage repeat")
            _assert_coverage_contains_function(
                second,
                source_url,
                function_name,
                f"repeated Profiler.getBestEffortCoverage with active precise coverage {case_name}",
            )
        finally:
            if precise_started:
                await state.cdp.send("Profiler.stopPreciseCoverage")
            await state.cdp.send("Profiler.disable")

    await run_case("Binary", {"detailed": True})
    await run_case("Count", {"callCount": True, "detailed": True})
    state.record("chromium_profiler_best_effort_with_precise_coverage_sample")


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
        primary = await state.context.new_cdp_session(page)
        peer = await state.context.new_cdp_session(page)
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
        owner = await state.context.new_cdp_session(page)
        peer = await state.context.new_cdp_session(page)
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
        owner = await state.context.new_cdp_session(page)
        peer = await state.context.new_cdp_session(page)
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
        session = await state.context.new_cdp_session(page)
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
        owner = await state.context.new_cdp_session(page)
        peer = await state.context.new_cdp_session(page)
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
        second_owner = await state.context.new_cdp_session(page)
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


async def _performance_metrics(cdp: Any) -> dict[str, float]:
    result = await cdp.send("Performance.getMetrics")
    metrics = result.get("metrics") or []
    return {metric.get("name"): metric.get("value") for metric in metrics if metric.get("name")}


async def _navigate_with_cdp_until_dom_ready(state: SmokeState, url: str) -> None:
    result = await state.cdp.send("Page.navigate", {"url": url})
    if result.get("errorText"):
        raise SmokeError(f"Page.navigate failed for {url}: {result}")

    async def is_ready() -> bool:
        ready_state = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": "document.readyState",
                "returnByValue": True,
            },
        )
        location = await state.cdp.send(
            "Runtime.evaluate",
            {
                "expression": "location.href",
                "returnByValue": True,
            },
        )
        return (
            location.get("result", {}).get("value") == url
            and ready_state.get("result", {}).get("value") in ["interactive", "complete"]
        )

    await wait_until(is_ready, f"CDP navigation DOM ready for {url}")


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
