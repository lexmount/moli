from __future__ import annotations


import asyncio
from contextlib import suppress
from typing import Any

from ..diagnostics import new_cdp_session
from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..fixture import ProxyAuthFixtureServer
from ..helpers import attach_cdp_event_collector


OBSERVED_METHODS = [
    "Network.requestWillBeSent",
    "Network.requestWillBeSentExtraInfo",
    "Network.responseReceivedExtraInfo",
    "Network.responseReceived",
    "Network.loadingFinished",
    "Network.loadingFailed",
    "Fetch.requestPaused",
    "Fetch.authRequired",
]


async def run_proxy_auth_group(state: SmokeState) -> None:
    await _verify_http_proxy_auth_retry(state)
    await _verify_https_connect_cancel_auth(state)


def _events_for_request(
    events: list[dict[str, Any]], method: str, request_id: str
) -> list[dict[str, Any]]:
    return [
        event
        for event in events
        if event.get("method") == method
        and event.get("params", {}).get("requestId") == request_id
    ]


def _event_for_url(
    events: list[dict[str, Any]], method: str, url: str
) -> dict[str, Any] | None:
    return next(
        (
            event
            for event in events
            if event.get("method") == method
            and event.get("params", {}).get("request", {}).get("url") == url
        ),
        None,
    )


async def _open_proxy_page(
    state: SmokeState, proxy: ProxyAuthFixtureServer
) -> tuple[Any, Any, Any, list[dict[str, Any]]]:
    context = await state.browser.new_context(proxy={"server": proxy.url})
    page = await context.new_page()
    cdp = await new_cdp_session(context, page)
    events = attach_cdp_event_collector(cdp, OBSERVED_METHODS)
    await cdp.send("Network.enable")
    await cdp.send("Fetch.enable", {"handleAuthRequests": True})
    return context, page, cdp, events


async def _begin_navigation(page: Any, events: list[dict[str, Any]], url: str) -> tuple[asyncio.Task[Any], dict[str, Any]]:
    navigation = asyncio.create_task(page.goto(url, wait_until="load", timeout=10_000))
    await wait_until(
        lambda: _event_for_url(events, "Fetch.requestPaused", url) is not None,
        f"proxy request pause for {url}",
    )
    paused = _event_for_url(events, "Fetch.requestPaused", url)
    if paused is None:
        raise SmokeError(f"missing Fetch.requestPaused for {url}")
    return navigation, paused


async def _wait_for_auth_challenge(
    events: list[dict[str, Any]], request_id: str
) -> dict[str, Any]:
    def challenge() -> dict[str, Any] | None:
        return next(
            (
                event
                for event in events
                if event.get("method") == "Fetch.authRequired"
                and event.get("params", {}).get("requestId") == request_id
            ),
            None,
        )

    await wait_until(lambda: challenge() is not None, "proxy Fetch.authRequired")
    event = challenge()
    if event is None:
        raise SmokeError("missing proxy Fetch.authRequired")
    return event


def _assert_proxy_challenge(event: dict[str, Any], proxy_url: str) -> None:
    params = event.get("params", {})
    challenge = params.get("authChallenge") or {}
    assert_equal(params.get("resourceType"), "Document", "proxy auth resource type")
    if "networkId" in params:
        raise SmokeError(f"Fetch.authRequired must not expose networkId: {event}")
    assert_equal(challenge.get("source"), "Proxy", "proxy auth source")
    assert_equal(challenge.get("origin"), proxy_url, "proxy auth origin")
    assert_equal(str(challenge.get("scheme", "")).lower(), "basic", "proxy auth scheme")
    assert_equal(challenge.get("realm"), "smoke-proxy", "proxy auth realm")


async def _verify_http_proxy_auth_retry(state: SmokeState) -> None:
    proxy = ProxyAuthFixtureServer()
    proxy.start()
    context = None
    navigation: asyncio.Task[Any] | None = None
    try:
        context, page, cdp, events = await _open_proxy_page(state, proxy)
        url = "http://example.test/proxy-auth"
        navigation, paused = await _begin_navigation(page, events, url)
        fetch_request_id = paused["params"]["requestId"]
        network_request_id = paused["params"]["networkId"]
        await cdp.send("Fetch.continueRequest", {"requestId": fetch_request_id})
        auth = await _wait_for_auth_challenge(events, fetch_request_id)
        _assert_proxy_challenge(auth, proxy.url)
        await cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": fetch_request_id,
                "authChallengeResponse": {
                    "response": "ProvideCredentials",
                    "username": "user",
                    "password": "pass",
                },
            },
        )
        response = await navigation
        assert_equal(response.status if response else None, 200, "proxy auth navigation status")
        await wait_until(
            lambda: len(
                _events_for_request(events, "Network.loadingFinished", network_request_id)
            )
            == 1,
            "proxy auth loadingFinished",
        )
        await wait_until(
            lambda: len(
                _events_for_request(
                    events, "Network.responseReceivedExtraInfo", network_request_id
                )
            )
            == 1,
            "proxy auth response ExtraInfo",
        )
        request_extra = _events_for_request(
            events, "Network.requestWillBeSentExtraInfo", network_request_id
        )
        assert_equal(len(request_extra), 1, "proxy auth request ExtraInfo count")
        request_headers = request_extra[0].get("params", {}).get("headers") or {}
        if any(name.lower() == "proxy-authorization" for name in request_headers):
            raise SmokeError(f"proxy credential retry leaked into request ExtraInfo: {request_extra}")
        if any(name.lower().startswith("get http") for name in request_headers):
            raise SmokeError(f"proxy absolute request line was parsed as a header: {request_extra}")
        response_extra = _events_for_request(
            events, "Network.responseReceivedExtraInfo", network_request_id
        )
        assert_equal(
            response_extra[0].get("params", {}).get("statusCode"),
            200,
            "proxy auth final response ExtraInfo status",
        )
        requests = [request for request in proxy.requests if "example.test" in request]
        assert_equal(len(requests), 2, "proxy transport request count")
        if "Proxy-Authorization:" in requests[0]:
            raise SmokeError(f"initial proxy request unexpectedly had credentials: {requests[0]}")
        if "Proxy-Authorization: Basic" not in requests[1]:
            raise SmokeError(f"proxy retry did not carry credentials: {requests[1]}")
        state.record("chromium_http_proxy_auth_transport_sample")
    finally:
        if navigation is not None:
            if not navigation.done():
                navigation.cancel()
            with suppress(Exception, asyncio.CancelledError):
                await navigation
        if context is not None:
            await context.close()
        proxy.stop()


async def _verify_https_connect_cancel_auth(state: SmokeState) -> None:
    proxy = ProxyAuthFixtureServer()
    proxy.start()
    context = None
    navigation: asyncio.Task[Any] | None = None
    try:
        context, page, cdp, events = await _open_proxy_page(state, proxy)
        url = "https://example.test/proxy-auth"
        navigation, paused = await _begin_navigation(page, events, url)
        fetch_request_id = paused["params"]["requestId"]
        network_request_id = paused["params"]["networkId"]
        await cdp.send("Fetch.continueRequest", {"requestId": fetch_request_id})
        auth = await _wait_for_auth_challenge(events, fetch_request_id)
        _assert_proxy_challenge(auth, proxy.url)
        await cdp.send(
            "Fetch.continueWithAuth",
            {
                "requestId": fetch_request_id,
                "authChallengeResponse": {"response": "CancelAuth"},
            },
        )
        try:
            await navigation
        except Exception as error:
            if "ERR_HTTP_RESPONSE_CODE_FAILURE" not in str(error):
                raise SmokeError(f"unexpected CONNECT cancellation error: {error}") from error
        else:
            raise SmokeError("HTTPS CONNECT proxy cancellation unexpectedly navigated successfully")

        await wait_until(
            lambda: len(
                _events_for_request(events, "Network.loadingFailed", network_request_id)
            )
            == 1,
            "CONNECT cancellation loadingFailed",
        )
        responses = _events_for_request(events, "Network.responseReceived", network_request_id)
        assert_equal(len(responses), 1, "CONNECT cancellation response count")
        response = responses[0].get("params", {})
        assert_equal(
            response.get("response", {}).get("status"),
            407,
            "CONNECT cancellation response status",
        )
        assert_equal(response.get("hasExtraInfo"), False, "CONNECT response hasExtraInfo")
        assert_equal(
            len(
                _events_for_request(
                    events, "Network.requestWillBeSentExtraInfo", network_request_id
                )
            ),
            0,
            "CONNECT request ExtraInfo count",
        )
        assert_equal(
            len(
                _events_for_request(
                    events, "Network.responseReceivedExtraInfo", network_request_id
                )
            ),
            0,
            "CONNECT response ExtraInfo count",
        )
        failed = _events_for_request(events, "Network.loadingFailed", network_request_id)[0]
        assert_equal(
            failed.get("params", {}).get("errorText"),
            "net::ERR_HTTP_RESPONSE_CODE_FAILURE",
            "CONNECT cancellation error text",
        )
        requests = [request for request in proxy.requests if "example.test:443" in request]
        if not requests or any(
            not request.startswith("CONNECT example.test:443 HTTP/1.1")
            for request in requests
        ):
            raise SmokeError(f"expected only target CONNECT requests, got: {requests}")
        state.record(
            "chromium_https_connect_cancel_auth_transport_sample",
            {"connectAttempts": len(requests)},
        )
    finally:
        if navigation is not None and not navigation.done():
            navigation.cancel()
            with suppress(asyncio.CancelledError):
                await navigation
        if context is not None:
            await context.close()
        proxy.stop()
