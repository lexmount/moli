from __future__ import annotations

import asyncio
from typing import Any, Callable, Awaitable

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..pdf_document import assert_pdf_envelope
from ..png_image import decode_png


async def run_playwright_compat_group(state: SmokeState) -> None:
    await _verify_playwright_first_screenshot(state)
    await _verify_playwright_context_route_metadata_sample(state)
    await _verify_playwright_page_route_precedence_sample(state)
    await _verify_playwright_context_route_fallback_sample(state)
    await _verify_playwright_route_times_sample(state)
    await _verify_playwright_route_fallback_chain_sample(state)
    await _verify_playwright_route_terminal_handlers_sample(state)
    await _verify_playwright_page_context_fallback_chain_sample(state)
    await _verify_playwright_route_fulfill_set_cookie_sample(state)
    await _verify_playwright_route_fulfill_headers_sample(state)
    await _verify_playwright_pdf_sample(state)
    await _verify_playwright_cdp_session_sample(state)
    await _verify_playwright_cdp_session_network_event_sample(state)
    await _verify_playwright_cdp_session_error_sample(state)
    await _verify_playwright_main_frame_cdp_session_sample(state)
    await _verify_playwright_cdp_session_detach_sample(state)
    await _verify_playwright_browser_cdp_session_sample(state)


async def _verify_playwright_first_screenshot(state: SmokeState) -> None:
    # Each case starts on a new page. A raw CDP capture here would hide a broken
    # getLayoutMetrics preflight in Playwright's own screenshot implementation.
    for name, options, size in [
        ("viewport", {}, (320, 240)),
        ("clip", {"clip": {"x": 10, "y": 15, "width": 60, "height": 50}}, (60, 50)),
    ]:
        page = await state.context.new_page()
        try:
            await page.set_viewport_size({"width": 320, "height": 240})
            await page.goto(
                "data:text/html,<!doctype html><body style='margin:0'>"
                "<div style='width:80px;height:900px;background:rgb(20,30,40)'></div>",
                wait_until="load",
                timeout=10_000,
            )
            assert_equal(
                await page.evaluate("""() => [
                    document.documentElement.clientWidth,
                    document.documentElement.clientHeight,
                    document.querySelector('div').getBoundingClientRect().width
                ]"""),
                [0, 0, 0],
                f"unpublished geometry before first Playwright {name} screenshot",
            )
            if name == "viewport":
                # Playwright derives fullPage clips from DOM dimensions, which
                # are zero before publication. CDP must reject that empty clip.
                try:
                    await page.screenshot(full_page=True, timeout=10_000)
                except Exception as error:
                    if "positive finite width, height, and scale" not in str(error):
                        raise SmokeError(f"unexpected cold full-page error: {error}") from error
                else:
                    raise SmokeError("cold Playwright full-page screenshot should reject its empty clip")
                assert_equal(
                    await page.evaluate("""() => [
                        document.documentElement.clientWidth,
                        document.documentElement.clientHeight,
                        document.querySelector('div').getBoundingClientRect().width
                    ]"""),
                    [0, 0, 0],
                    "rejected full-page screenshot must not publish layout",
                )
            image = decode_png(await page.screenshot(timeout=10_000, **options))
            assert_equal((image.width, image.height), size, f"first Playwright {name} screenshot size")
            assert_equal(image.pixel(5, 5), (20, 30, 40, 255), f"first Playwright {name} screenshot pixel")
            if name == "viewport":
                # A successful viewport capture publishes the DOM dimensions
                # used by subsequent Playwright full-page requests.
                full_image = decode_png(await page.screenshot(full_page=True, timeout=10_000))
                assert_equal(full_image.height, 900, "published Playwright full-page screenshot height")
                assert_equal(
                    full_image.pixel(5, 880),
                    (20, 30, 40, 255),
                    "published Playwright full-page bottom pixel",
                )
                # Playwright clips against DOM dimensions before sending CDP.
                # A full-page clip therefore needs a previously published frame.
                clipped = decode_png(await page.screenshot(
                    full_page=True,
                    clip={"x": 10, "y": 15, "width": 60, "height": 50},
                    timeout=10_000,
                ))
                assert_equal((clipped.width, clipped.height), (60, 50), "published full-page clip size")
                assert_equal(clipped.pixel(5, 5), (20, 30, 40, 255), "published full-page clip pixel")
        finally:
            await page.close()
    state.record(
        "playwright_first_screenshot_without_published_layout",
        {
            "modes": ["viewport", "clip"],
            "rejected_unpublished_modes": ["full-page"],
            "published_modes": ["full-page", "full-page-clip"],
        },
    )


async def _with_fresh_page(state: SmokeState, body: Callable[[Any, Any], Awaitable[None]]) -> None:
    context = state.context
    try:
        await body(context, state.page)
    finally:
        await state.page.unroute_all()
        await context.unroute_all()


async def _verify_playwright_context_route_metadata_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        intercepted: dict[str, Any] = {}

        async def handler(route: Any) -> None:
            request = route.request
            try:
                frame = request.frame
                intercepted.update(
                    {
                        "url": request.url,
                        "hasUserAgent": bool(request.headers.get("user-agent")),
                        "method": request.method,
                        "postData": request.post_data,
                        "isNavigationRequest": request.is_navigation_request(),
                        "resourceType": request.resource_type,
                        "frameUrl": frame.url,
                        "isMainFrame": frame == page.main_frame,
                    }
                )
            except Exception as error:
                intercepted["handlerError"] = str(error)
            finally:
                await route.continue_()

        await context.route("**/plain", handler)
        response = await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        await context.unroute("**/plain")
        assert_equal(response.ok if response else None, True, "Playwright context route navigation response")
        if not intercepted:
            raise SmokeError("Playwright context.route should intercept the navigation request")
        if intercepted.get("handlerError"):
            raise SmokeError(f"Playwright route metadata handler failed: {intercepted}")
        if "/plain" not in intercepted.get("url", ""):
            raise SmokeError(f"Playwright routed request URL mismatch: {intercepted}")
        assert_equal(intercepted.get("hasUserAgent"), True, "Playwright route request user-agent metadata")
        assert_equal(intercepted.get("method"), "GET", "Playwright route request method metadata")
        assert_equal(intercepted.get("postData"), None, "Playwright route request postData metadata")
        assert_equal(intercepted.get("isNavigationRequest"), True, "Playwright route navigation metadata")
        assert_equal(intercepted.get("resourceType"), "document", "Playwright route resourceType metadata")
        assert_equal(intercepted.get("isMainFrame"), True, "Playwright route request frame identity")

    await _with_fresh_page(state, body)
    state.record("playwright_context_route_metadata_sample")


async def _verify_playwright_page_route_precedence_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        await context.route(
            "**/playwright-route-priority",
            lambda route: route.fulfill(status=200, body="context"),
        )
        await page.route(
            "**/playwright-route-priority",
            lambda route: route.fulfill(status=200, body="page"),
        )
        response = await page.goto(f"{state.fixture}/playwright-route-priority", timeout=10_000)
        assert_equal(response.ok if response else None, True, "Playwright page route precedence response")
        assert_equal(await response.text(), "page", "Playwright page.route should win over context.route")

    await _with_fresh_page(state, body)
    state.record("playwright_page_route_precedence_sample")


async def _verify_playwright_context_route_fallback_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        await context.route(
            "**/playwright-context-fallback",
            lambda route: route.fulfill(status=200, body="context"),
        )
        await page.route(
            "**/playwright-non-match",
            lambda route: route.fulfill(status=200, body="page"),
        )
        response = await page.goto(f"{state.fixture}/playwright-context-fallback", timeout=10_000)
        assert_equal(response.ok if response else None, True, "Playwright context route fallback response")
        assert_equal(await response.text(), "context", "Playwright context.route should handle when page.route does not match")

    await _with_fresh_page(state, body)
    state.record("playwright_context_route_fallback_sample")


async def _verify_playwright_route_times_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        hits: list[str] = []

        async def once(route: Any) -> None:
            hits.append("once")
            await asyncio.sleep(0.05)
            await route.fulfill(
                status=200,
                content_type="text/html; charset=utf-8",
                body="<html><body>intercepted</body></html>",
            )

        await context.route("**/playwright-route-times", once, times=1)
        await page.goto(f"{state.fixture}/playwright-route-times", wait_until="load", timeout=10_000)
        assert_equal(await page.text_content("body"), "intercepted", "Playwright route times first navigation")
        await page.goto(f"{state.fixture}/playwright-route-times", wait_until="load", timeout=10_000)
        assert_equal(await page.text_content("main"), "server fallback", "Playwright route times second navigation")
        assert_equal(hits, ["once"], "Playwright route times hit count")

    await _with_fresh_page(state, body)
    state.record("playwright_route_times_sample")


async def _verify_playwright_route_fallback_chain_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        intercepted: list[int] = []

        async def first(route: Any) -> None:
            intercepted.append(1)
            await route.fallback()

        async def second(route: Any) -> None:
            intercepted.append(2)
            await route.fallback()

        async def third(route: Any) -> None:
            intercepted.append(3)
            await route.fallback()

        await context.route("**/plain", first)
        await context.route("**/plain", second)
        await context.route("**/plain", third)
        response = await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        await context.unroute("**/plain")
        assert_equal(response.ok if response else None, True, "Playwright route fallback chain response")
        assert_equal(intercepted, [3, 2, 1], "Playwright route fallback should run newest matching handler first")

    await _with_fresh_page(state, body)
    state.record("playwright_route_fallback_chain_sample")


async def _verify_playwright_route_terminal_handlers_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        fulfill_failed = False

        async def fulfill_unreachable(route: Any) -> None:
            nonlocal fulfill_failed
            fulfill_failed = True
            await route.fallback()

        await context.route("**/playwright-terminal-fulfill", fulfill_unreachable)
        await context.route(
            "**/playwright-terminal-fulfill",
            lambda route: route.fulfill(status=200, body="fulfilled"),
        )
        await context.route("**/playwright-terminal-fulfill", lambda route: route.fallback())
        response = await page.goto(f"{state.fixture}/playwright-terminal-fulfill", timeout=10_000)
        assert_equal(await response.text(), "fulfilled", "Playwright route.fulfill should terminate fallback chain")
        assert_equal(fulfill_failed, False, "Playwright route.fulfill should not call older handlers")
        await context.unroute("**/playwright-terminal-fulfill")

        abort_failed = False

        async def abort_unreachable(route: Any) -> None:
            nonlocal abort_failed
            abort_failed = True
            await route.fallback()

        await context.route("**/playwright-terminal-abort", abort_unreachable)
        await context.route("**/playwright-terminal-abort", lambda route: route.abort("blockedbyclient"))
        await context.route("**/playwright-terminal-abort", lambda route: route.fallback())
        abort_error = await _expect_async_error(
            page.goto(f"{state.fixture}/playwright-terminal-abort", timeout=10_000)
        )
        if "ERR_BLOCKED_BY_CLIENT" not in abort_error:
            raise SmokeError(f"Playwright route.abort should reject with ERR_BLOCKED_BY_CLIENT: {abort_error}")
        assert_equal(abort_failed, False, "Playwright route.abort should not call older handlers")
        await context.unroute("**/playwright-terminal-abort")

    await _with_fresh_page(state, body)
    state.record("playwright_route_terminal_handlers_sample")


async def _verify_playwright_page_context_fallback_chain_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        intercepted: list[int] = []

        async def push_and_fallback(value: int, route: Any) -> None:
            intercepted.append(value)
            await route.fallback()

        await context.route("**/playwright-fallback-chain", lambda route: push_and_fallback(1, route))
        await context.route("**/playwright-fallback-chain", lambda route: push_and_fallback(2, route))
        await context.route("**/playwright-fallback-chain", lambda route: push_and_fallback(3, route))
        await page.route("**/playwright-fallback-chain", lambda route: push_and_fallback(4, route))
        await page.route("**/playwright-fallback-chain", lambda route: push_and_fallback(5, route))
        await page.route("**/playwright-fallback-chain", lambda route: push_and_fallback(6, route))
        response = await page.goto(f"{state.fixture}/playwright-fallback-chain", wait_until="load", timeout=10_000)
        assert_equal(response.ok if response else None, True, "Playwright page/context fallback chain response")
        assert_equal(intercepted, [6, 5, 4, 3, 2, 1], "Playwright page routes should fall back into context routes")
        await page.unroute("**/playwright-fallback-chain")
        await context.unroute("**/playwright-fallback-chain")

    await _with_fresh_page(state, body)
    state.record("playwright_page_context_fallback_chain_sample")


async def _verify_playwright_route_fulfill_set_cookie_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        await page.route(
            "**/playwright-route-cookie",
            lambda route: route.fulfill(
                status=200,
                content_type="text/html; charset=utf-8",
                headers={"Set-Cookie": "routeCookie=value; Path=/"},
                body="<html><main>cookie route</main></html>",
            ),
        )
        response = await page.goto(f"{state.fixture}/playwright-route-cookie", wait_until="load", timeout=10_000)
        assert_equal(response.ok if response else None, True, "Playwright route fulfill Set-Cookie response")
        cookies = await context.cookies(state.fixture)
        route_cookie = next((cookie for cookie in cookies if cookie.get("name") == "routeCookie"), None)
        if not route_cookie:
            raise SmokeError(f"Playwright route.fulfill Set-Cookie should persist cookie: {cookies}")
        assert_equal(route_cookie.get("value"), "value", "Playwright route.fulfill Set-Cookie value")

    await _with_fresh_page(state, body)
    state.record("playwright_route_fulfill_set_cookie_sample")


async def _verify_playwright_route_fulfill_headers_sample(state: SmokeState) -> None:
    async def body(_context: Any, page: Any) -> None:
        await page.route(
            "**/playwright-route-headers",
            lambda route: route.fulfill(
                status=200,
                content_type="text/plain",
                headers={"foo": "bar", "content-language": "en"},
                body="done",
            ),
        )
        response = await page.goto(f"{state.fixture}/playwright-route-headers", timeout=10_000)
        headers = await response.all_headers()
        assert_equal(headers.get("foo"), "bar", "Playwright route.fulfill custom header")
        assert_equal(headers.get("content-language"), "en", "Playwright route.fulfill content-language header")
        assert_equal(headers.get("content-type"), "text/plain", "Playwright route.fulfill content type header")
        assert_equal(await response.text(), "done", "Playwright route.fulfill response body")

    await _with_fresh_page(state, body)
    state.record("playwright_route_fulfill_headers_sample")


async def _verify_playwright_pdf_sample(state: SmokeState) -> None:
    async def body(_context: Any, page: Any) -> None:
        await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        default_pdf = await page.pdf()
        assert_pdf_envelope(default_pdf, "Playwright default page.pdf")

        option_pdf = await page.pdf(
            format="A4",
            landscape=True,
            print_background=True,
            page_ranges="1",
        )
        assert_pdf_envelope(option_pdf, "Playwright option page.pdf")
        if default_pdf == option_pdf:
            raise SmokeError("Playwright page.pdf options should change the encoded PDF")

    await _with_fresh_page(state, body)
    state.record("playwright_pdf_sample")


async def _verify_playwright_cdp_session_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        session = await context.new_cdp_session(page)
        try:
            version = await session.send("Browser.getVersion")
            if not version.get("protocolVersion"):
                raise SmokeError(f"Playwright CDPSession Browser.getVersion missing protocolVersion: {version}")
            result = await session.send("Runtime.evaluate", {"expression": "1 + 2", "returnByValue": True})
            assert_equal(result.get("result", {}).get("value"), 3, "Playwright CDPSession Runtime.evaluate")
        finally:
            await session.detach()

    await _with_fresh_page(state, body)
    state.record("playwright_cdp_session_sample")


async def _verify_playwright_cdp_session_network_event_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        session = await context.new_cdp_session(page)
        events: list[dict[str, Any]] = []
        try:
            session.on("Network.requestWillBeSent", lambda event: events.append(event))
            await session.send("Network.enable")
            await page.goto(f"{state.fixture}/plain?playwright-cdp-event", wait_until="load", timeout=10_000)
            await wait_until(
                lambda: any((event.get("request") or {}).get("url", "").endswith("/plain?playwright-cdp-event") for event in events),
                "Playwright CDPSession Network.requestWillBeSent event",
            )
        finally:
            await session.detach()

    await _with_fresh_page(state, body)
    state.record("playwright_cdp_session_network_event_sample")


async def _verify_playwright_cdp_session_error_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        session = await context.new_cdp_session(page)
        try:
            error = await _expect_async_error(session.send("ThisCommand.DoesNotExist"))
            if "ThisCommand.DoesNotExist" not in error:
                raise SmokeError(f"Playwright CDPSession unknown command error should mention method: {error}")
        finally:
            await session.detach()

    await _with_fresh_page(state, body)
    state.record("playwright_cdp_session_error_sample")


async def _verify_playwright_main_frame_cdp_session_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        session = await context.new_cdp_session(page.main_frame)
        try:
            await session.send("Runtime.enable")
            await session.send("Runtime.evaluate", {"expression": "window.__pwFrameSession = 'ok'"})
            value = await page.evaluate("() => window.__pwFrameSession")
            assert_equal(value, "ok", "Playwright main-frame CDPSession Runtime.evaluate")
        finally:
            await session.detach()

    await _with_fresh_page(state, body)
    state.record("playwright_main_frame_cdp_session_sample")


async def _verify_playwright_cdp_session_detach_sample(state: SmokeState) -> None:
    async def body(context: Any, page: Any) -> None:
        session = await context.new_cdp_session(page)
        result = await session.send("Runtime.evaluate", {"expression": "1 + 2", "returnByValue": True})
        assert_equal(result.get("result", {}).get("value"), 3, "Playwright CDPSession before detach")
        await session.detach()
        error = await _expect_async_error(
            session.send("Runtime.evaluate", {"expression": "3 + 1", "returnByValue": True})
        )
        if "closed" not in error.lower():
            raise SmokeError(f"Playwright detached CDPSession send should reject as closed: {error}")

    await _with_fresh_page(state, body)
    state.record("playwright_cdp_session_detach_sample")


async def _verify_playwright_browser_cdp_session_sample(state: SmokeState) -> None:
    session = await state.browser.new_browser_cdp_session()
    try:
        version = await session.send("Browser.getVersion")
        if not version.get("userAgent") or not version.get("protocolVersion"):
            raise SmokeError(f"Playwright browser CDPSession Browser.getVersion missing fields: {version}")
    finally:
        await session.detach()
    state.record("playwright_browser_cdp_session_sample")


async def _expect_async_error(awaitable: Any) -> str:
    try:
        await awaitable
    except Exception as error:
        return str(error)
    raise SmokeError("expected async operation to fail")
