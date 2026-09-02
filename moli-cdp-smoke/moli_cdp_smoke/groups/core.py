from __future__ import annotations

import asyncio

from playwright.async_api import TimeoutError as PlaywrightTimeoutError

from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..helpers import attach_cdp_event_collector


def _assert_websocket_event_constraints(events: list[str], label: str) -> None:
    # Window load and WebSocket open come from distinct task sources, so only
    # assert the ordering guaranteed inside each lifecycle/protocol sequence.
    expected = ["dcl", "load", "open", "echo:OK"]
    if len(events) != len(expected) or set(events) != set(expected):
        raise SmokeError(
            f"{label}: expected each of {expected!r} exactly once, got {events!r}"
        )
    if events.index("dcl") > events.index("load"):
        raise SmokeError(f"{label}: DOMContentLoaded must precede load, got {events!r}")
    if events.index("open") > events.index("echo:OK"):
        raise SmokeError(f"{label}: WebSocket open must precede its echo, got {events!r}")


async def run_core_group(state: SmokeState) -> None:
    page = state.page
    context = state.context
    fixture = state.fixture

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    assert_equal(await page.text_content("main"), "plain ok", "plain page text")
    state.record("new_page_goto_plain")

    second = await context.new_page()
    await second.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    assert_equal(await second.text_content("main"), "plain ok", "second page text")
    state.record("second_page_same_context")
    await second.close()

    await run_add_init_script_workflows(state)
    await run_exposed_binding_workflows(state)

    await page.goto(f"{fixture}/iframe", wait_until="load", timeout=10_000)
    child = next((frame for frame in page.frames if "/child" in frame.url), None)
    if child is None:
        raise SmokeError(f"missing child frame; frames={[frame.url for frame in page.frames]}")
    assert_equal((await child.text_content("body", timeout=5_000)).strip(), "child body text", "child frame text")
    state.record("iframe_child_text_content", {"frameCount": len(page.frames)})

    await page.goto(f"{fixture}/wait-for-function", wait_until="domcontentloaded", timeout=10_000)
    await page.wait_for_function("() => globalThis.__ready === true", timeout=5_000)
    state.record("wait_for_function_timer")

    # Reduced from Playwright page-wait-for-function.spec.ts.
    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    string_watchdog = page.wait_for_function("window.__SMOKE_WAIT_FOR_FUNCTION === 1", timeout=5_000)
    await page.evaluate("() => { window.__SMOKE_WAIT_FOR_FUNCTION = 1; }")
    await string_watchdog
    result = await page.wait_for_function("() => 5", timeout=5_000)
    assert_equal(await result.json_value(), 5, "wait_for_function returns JSHandle value")
    await page.wait_for_function("value => value === 7", arg=7, timeout=5_000)
    await page.set_content("<div id='wait-handle'></div>")
    div = await page.query_selector("#wait-handle")
    if div is None:
        raise SmokeError("missing wait_for_function ElementHandle fixture")
    handle_watchdog = page.wait_for_function("element => !element.parentElement", arg=div, timeout=5_000)
    await page.evaluate("element => element.remove()", div)
    await handle_watchdog
    timeout_error = None
    try:
        await page.wait_for_function("() => false", timeout=25)
    except PlaywrightTimeoutError as error:
        timeout_error = str(error)
    if timeout_error is None or "Timeout 25ms exceeded" not in timeout_error:
        raise SmokeError(f"unexpected wait_for_function timeout error: {timeout_error}")
    state.record("wait_for_function_argument_and_timeout_workflow")

    await page.goto(f"{fixture}/wait-states", wait_until="load", timeout=10_000)
    attached = await page.wait_for_selector("#attached", state="attached", timeout=5_000)
    if attached is None:
        raise SmokeError("wait_for_selector attached returned no handle")
    assert_equal(await attached.text_content(), "attached ready", "wait_for_selector attached text")
    await page.locator("#delayed-button").wait_for(state="attached", timeout=5_000)
    await page.wait_for_function("() => !document.querySelector('#delayed-button')?.disabled", timeout=5_000)
    # OnDemand layout intentionally reuses its latest sampled geometry across
    # DOM/style mutations. Reach the fixture's final DOM-only state before the
    # first explicit refresh so this smoke does not require snapshot invalidation.
    await page.screenshot()
    await page.locator("#visible").wait_for(state="visible", timeout=5_000)
    assert_equal(await page.text_content("#visible", timeout=5_000), "visible ready", "locator visible text")
    await page.wait_for_selector("#hide-me", state="hidden", timeout=5_000)
    detached = await page.wait_for_selector("#detach-me", state="detached", timeout=5_000)
    assert_equal(detached, None, "wait_for_selector detached returns null")
    await page.locator("#delayed-button").evaluate("button => button.click()")
    assert_equal(await page.text_content("#clicked", timeout=5_000), "clicked", "locator click after enabled wait")
    state.record("wait_for_selector_state_workflow")

    await page.goto(f"{fixture}/streamed-reveal-page", wait_until="load", timeout=10_000)
    await page.wait_for_function(
        "() => document.querySelector('[data-message-author-role=\"assistant\"]')?.textContent === 'OK'",
        timeout=5_000,
    )
    assert_equal(
        await page.text_content("[data-message-author-role='assistant']", timeout=5_000),
        "OK",
        "streamed reveal assistant text",
    )
    reveal_events = await page.evaluate("() => globalThis.__smokeRevealEvents || []")
    assert_equal(reveal_events, ["revealed"], "streamed reveal rAF event")
    state.record("streamed_reveal_request_animation_frame_dom")

    await page.goto(f"{fixture}/fetch-stream-client-nav-page", wait_until="load", timeout=10_000)
    await page.wait_for_function(
        "() => location.pathname === '/conversation/fetch-stream' && "
        "document.querySelector('[data-message-author-role=\"assistant\"]')?.textContent === 'OK'",
        timeout=5_000,
    )
    assert_equal(page.url, f"{fixture}/conversation/fetch-stream", "fetch stream client navigation URL")
    assert_equal(
        await page.text_content("[data-message-author-role='assistant']", timeout=5_000),
        "OK",
        "fetch stream client navigation assistant text",
    )
    fetch_stream_events = await page.evaluate("() => globalThis.__smokeFetchStreamEvents || []")
    if fetch_stream_events[:2] != ["dcl", "load"]:
        raise SmokeError(f"fetch stream client navigation lifecycle order mismatch: {fetch_stream_events}")
    if "send" not in fetch_stream_events or "response:200" not in fetch_stream_events:
        raise SmokeError(f"fetch stream client navigation missing fetch markers: {fetch_stream_events}")
    if "rendered:OK" not in fetch_stream_events:
        raise SmokeError(f"fetch stream client navigation did not render response: {fetch_stream_events}")
    state.record("fetch_stream_client_navigation_dom")

    await page.goto(f"{fixture}/chatgpt-live-channel-page", wait_until="load", timeout=10_000)
    await page.wait_for_function(
        "() => location.pathname === '/c/smoke-live' && "
        "document.querySelector('[data-message-author-role=\"assistant\"]')?.textContent === 'OK'",
        timeout=5_000,
    )
    assert_equal(page.url, f"{fixture}/c/smoke-live", "ChatGPT-like live channel navigation URL")
    assert_equal(
        await page.text_content("[data-message-author-role='assistant']", timeout=5_000),
        "OK",
        "ChatGPT-like live channel assistant text",
    )
    chatgpt_live_events = await page.evaluate("() => globalThis.__smokeChatGptLiveEvents || []")
    if chatgpt_live_events[:2] != ["dcl", "load"]:
        raise SmokeError(f"ChatGPT-like live channel lifecycle order mismatch: {chatgpt_live_events}")
    for marker in (
        "prepare:200",
        "ws:open",
        "conversation:200",
        "ws:delta:O",
        "ws:delta:K",
        "ws:done",
        "rendered:OK",
    ):
        if marker not in chatgpt_live_events:
            raise SmokeError(f"ChatGPT-like live channel missing {marker}: {chatgpt_live_events}")
    live_document_url = f"{fixture}/chatgpt-live-channel-page"
    live_document_request = next(
        (
            event
            for event in reversed(state.subresource_events)
            if event.get("method") == "Network.requestWillBeSent"
            and (event.get("params") or {}).get("type") == "Document"
            and (event.get("params") or {}).get("request", {}).get("url") == live_document_url
        ),
        None,
    )
    live_loader_id = live_document_request and (live_document_request.get("params") or {}).get("loaderId")
    if not live_loader_id:
        raise SmokeError(f"missing ChatGPT-like live channel document loaderId: {state.subresource_events[-20:]}")
    for suffix in (
        "/backend-api/f/conversation/prepare",
        "/backend-api/f/conversation",
        "/ws-chatgpt-live?conversation_id=smoke-live",
    ):
        request = next(
            (
                event
                for event in reversed(state.subresource_events)
                if event.get("method") == "Network.requestWillBeSent"
                and (event.get("params") or {}).get("documentURL") == live_document_url
                and str((event.get("params") or {}).get("request", {}).get("url") or "").endswith(suffix)
            ),
            None,
        )
        loader_id = request and (request.get("params") or {}).get("loaderId")
        assert_equal(
            loader_id,
            live_loader_id,
            f"ChatGPT-like live channel subresource loaderId for {suffix}",
        )
    state.record("chatgpt_like_live_channel_dom")

    await page.goto(f"{fixture}/chatgpt-client-id-map-page", wait_until="load", timeout=10_000)
    await page.wait_for_function(
        "() => location.pathname === '/c/smoke-live' && "
        "document.querySelector('[data-message-author-role=\"assistant\"]')?.textContent === 'OK'",
        timeout=5_000,
    )
    assert_equal(page.url, f"{fixture}/c/smoke-live", "ChatGPT-like client id map navigation URL")
    assert_equal(
        await page.text_content("[data-message-author-role='assistant']", timeout=5_000),
        "OK",
        "ChatGPT-like client id map assistant text",
    )
    client_map_events = await page.evaluate("() => globalThis.__smokeChatGptClientMapEvents || []")
    if client_map_events[:2] != ["dcl", "load"]:
        raise SmokeError(f"ChatGPT-like client id map lifecycle order mismatch: {client_map_events}")
    for marker in (
        "rendered:empty",
        "prepare:200",
        "ws:open",
        "conversation:200",
        "ws:delta:O",
        "ws:delta:K",
        "ws:done",
        "mapped:client-new-thread->smoke-live",
        "selector:smoke-live",
        "rendered:OK",
    ):
        if marker not in client_map_events:
            raise SmokeError(f"ChatGPT-like client id map missing {marker}: {client_map_events}")
    mapped_index = client_map_events.index("mapped:client-new-thread->smoke-live")
    selector_index = client_map_events.index("selector:smoke-live")
    rendered_index = client_map_events.index("rendered:OK")
    if not (mapped_index < selector_index < rendered_index):
        raise SmokeError(f"ChatGPT-like client id map render order mismatch: {client_map_events}")
    state.record("chatgpt_like_client_id_map_live_dom")

    await page.goto(f"{fixture}/websocket-open-page", wait_until="load", timeout=10_000)
    await page.wait_for_function(
        "() => document.querySelector('[data-message-author-role=\"assistant\"]')?.textContent === 'OK'",
        timeout=5_000,
    )
    assert_equal(
        await page.text_content("[data-message-author-role='assistant']", timeout=5_000),
        "OK",
        "websocket page assistant text",
    )
    _assert_websocket_event_constraints(
        await page.evaluate("() => globalThis.__smokeWsEvents || []"),
        "websocket page lifecycle events",
    )
    state.record("websocket_page_dom")

    await page.goto(f"{fixture}/websocket-client-nav-page", wait_until="load", timeout=10_000)
    await page.wait_for_function(
        "() => location.pathname === '/conversation/smoke' && "
        "document.querySelector('[data-message-author-role=\"assistant\"]')?.textContent === 'OK'",
        timeout=5_000,
    )
    assert_equal(page.url, f"{fixture}/conversation/smoke", "websocket client-side navigation URL")
    assert_equal(
        await page.text_content("[data-message-author-role='assistant']", timeout=5_000),
        "OK",
        "websocket client navigation assistant text",
    )
    _assert_websocket_event_constraints(
        await page.evaluate("() => globalThis.__smokeWsEvents || []"),
        "websocket client navigation lifecycle events",
    )
    state.record("websocket_client_navigation_dom")

    await page.goto(f"{fixture}/lifecycle-load-state", wait_until="domcontentloaded", timeout=10_000)
    assert_equal(await page.get_attribute("body", "data-dcl"), "1", "DOMContentLoaded marker")
    await page.wait_for_load_state("load", timeout=10_000)
    assert_equal(await page.get_attribute("body", "data-load"), "1", "load marker")
    await page.wait_for_function("() => document.querySelector('#delayed')?.complete === true", timeout=5_000)
    await page.wait_for_load_state("networkidle", timeout=10_000)
    state.record("page_lifecycle_load_state_waits")

    await page.goto(f"{fixture}/dialog", wait_until="load", timeout=10_000)
    async with page.expect_event("dialog", timeout=5_000) as dialog_info:
        alert_task = asyncio.create_task(page.evaluate("() => alert('fixture alert')"))
    dialog = await dialog_info.value
    assert_equal(dialog.type, "alert", "dialog type")
    assert_equal(dialog.message, "fixture alert", "dialog message")
    await dialog.accept()
    await alert_task
    state.record("javascript_dialog_alert")

    async with page.expect_event("dialog", timeout=5_000) as confirm_info:
        confirm_task = asyncio.create_task(page.evaluate("() => confirm('fixture confirm')"))
    confirm_dialog = await confirm_info.value
    assert_equal(confirm_dialog.type, "confirm", "confirm dialog type")
    assert_equal(confirm_dialog.message, "fixture confirm", "confirm dialog message")
    await confirm_dialog.dismiss()
    assert_equal(await confirm_task, False, "confirm dismiss return value")
    state.record("javascript_dialog_confirm_dismiss")

    async with page.expect_event("dialog", timeout=5_000) as prompt_info:
        prompt_task = asyncio.create_task(page.evaluate("() => prompt('fixture prompt', 'fixture default')"))
    prompt_dialog = await prompt_info.value
    assert_equal(prompt_dialog.type, "prompt", "prompt dialog type")
    assert_equal(prompt_dialog.message, "fixture prompt", "prompt dialog message")
    assert_equal(prompt_dialog.default_value, "fixture default", "prompt default value")
    await prompt_dialog.dismiss()
    assert_equal(await prompt_task, None, "prompt dismiss return value")
    state.record("javascript_dialog_prompt_dismiss")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    async with page.expect_popup(timeout=5_000) as popup_info:
        await page.evaluate("(url) => window.open(url, '_blank')", f"{fixture}/plain?popup=playwright")
    popup = await popup_info.value
    assert_equal(popup.url, f"{fixture}/plain?popup=playwright", "popup target URL")
    await popup.close()
    state.record("popup_target_workflow")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    context_routed_popup_url = f"{fixture}/popup-context-document"
    await context.route(
        "**/popup-context-document",
        lambda route: route.fulfill(
            status=200,
            content_type="text/html; charset=utf-8",
            body="<!doctype html><main>context routed popup document</main>",
        ),
    )
    try:
        async with page.expect_popup(timeout=5_000) as context_routed_popup_info:
            await page.evaluate("(url) => window.open(url, '_blank')", context_routed_popup_url)
        context_routed_popup = await context_routed_popup_info.value
        await wait_until(lambda: context_routed_popup.url == context_routed_popup_url, "context-routed popup URL")
        await context_routed_popup.wait_for_load_state("load", timeout=10_000)
        assert_equal(
            await context_routed_popup.text_content("main", timeout=5_000),
            "context routed popup document",
            "context route should fulfill popup initial document",
        )
        await context_routed_popup.close()
    finally:
        await context.unroute("**/popup-context-document")
    state.record("popup_context_route_initial_document")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    opener_page_route_seen = asyncio.Event()

    async def fulfill_opener_page_boundary(route):
        opener_page_route_seen.set()
        await route.fulfill(
            status=200,
            content_type="text/html; charset=utf-8",
            body="<!doctype html><main>opener page route</main>",
        )

    await page.route("**/popup-page-route-boundary", fulfill_opener_page_boundary)
    await context.route(
        "**/popup-page-route-boundary",
        lambda route: route.fulfill(
            status=200,
            content_type="text/html; charset=utf-8",
            body="<!doctype html><main>context route boundary</main>",
        ),
    )
    boundary_popup = None
    try:
        async with page.expect_popup(timeout=5_000) as boundary_popup_info:
            await page.evaluate(
                "(url) => window.open(url, '_blank')",
                f"{fixture}/popup-page-route-boundary",
            )
        boundary_popup = await boundary_popup_info.value
        await boundary_popup.wait_for_load_state("load", timeout=10_000)
        assert_equal(
            await boundary_popup.text_content("main", timeout=5_000),
            "context route boundary",
            "opener page.route must not intercept popup initial document",
        )
        if opener_page_route_seen.is_set():
            raise SmokeError("opener page.route intercepted a popup initial document")
    finally:
        if boundary_popup is not None:
            await boundary_popup.close()
        await page.unroute("**/popup-page-route-boundary")
        await context.unroute("**/popup-page-route-boundary")
    state.record("popup_initial_document_page_route_boundary")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    first_popup_route_seen = asyncio.Event()
    first_popup_route_release = asyncio.Event()

    async def fulfill_concurrent_popup(route):
        slug = route.request.url.rsplit("/", 1)[-1]
        if slug == "popup-concurrent-first":
            first_popup_route_seen.set()
            await asyncio.wait_for(first_popup_route_release.wait(), timeout=10)
        await route.fulfill(
            status=200,
            content_type="text/html; charset=utf-8",
            body=f"<!doctype html><main>fulfilled {slug}</main>",
        )

    await context.route("**/popup-concurrent-*", fulfill_concurrent_popup)
    first_popup = None
    second_popup = None
    try:
        async with page.expect_popup(timeout=5_000) as first_popup_info:
            await page.evaluate(
                "(url) => window.open(url, '_blank')",
                f"{fixture}/popup-concurrent-first",
            )
        first_popup = await first_popup_info.value
        await asyncio.wait_for(first_popup_route_seen.wait(), timeout=5)

        async with page.expect_popup(timeout=5_000) as second_popup_info:
            await page.evaluate(
                "(url) => window.open(url, '_blank')",
                f"{fixture}/popup-concurrent-second",
            )
        second_popup = await second_popup_info.value
        await second_popup.wait_for_load_state("load", timeout=10_000)
        assert_equal(
            await second_popup.text_content("main", timeout=5_000),
            "fulfilled popup-concurrent-second",
            "second popup initial document should load while first popup route is held",
        )

        first_popup_route_release.set()
        await first_popup.wait_for_load_state("load", timeout=10_000)
        assert_equal(
            await first_popup.text_content("main", timeout=5_000),
            "fulfilled popup-concurrent-first",
            "first popup initial document should load after held route resumes",
        )
    finally:
        for popup in (first_popup, second_popup):
            if popup is not None:
                await popup.close()
        await context.unroute("**/popup-concurrent-*")
    state.record("popup_context_route_concurrent_initial_documents")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    async with page.expect_popup(timeout=5_000) as routed_popup_info:
        await page.evaluate("(url) => window.open(url, '_blank')", f"{fixture}/plain?popup=route")
    routed_popup = await routed_popup_info.value
    await wait_until(lambda: routed_popup.url == f"{fixture}/plain?popup=route", "popup route URL")
    routed_popup_url = f"{fixture}/plain?popup=route"
    await routed_popup.wait_for_load_state("load", timeout=10_000)
    assert_equal(await routed_popup.text_content("main", timeout=5_000), "plain ok", "popup route initial text")
    await routed_popup.route(
        "**/popup-api",
        lambda route: route.fulfill(
            status=200,
            content_type="application/json; charset=utf-8",
            body='{"source":"popup route","ok":true}',
        ),
    )
    routed_payload = await asyncio.wait_for(
        routed_popup.evaluate(
            """
            async () => {
              const response = await fetch('/popup-api');
              return {
                url: location.href,
                mainText: document.querySelector('main')?.textContent,
                api: await response.json(),
              };
            }
            """
        ),
        timeout=10,
    )
    assert_equal(routed_payload.get("url"), routed_popup_url, "popup route materialized URL")
    assert_equal(routed_payload.get("mainText"), "plain ok", "popup route document text")
    assert_equal(routed_payload.get("api", {}).get("source"), "popup route", "popup route fulfilled source")
    assert_equal(routed_payload.get("api", {}).get("ok"), True, "popup route fulfilled ok")
    await routed_popup.unroute("**/popup-api")
    await routed_popup.close()
    state.record("popup_route_evaluate_workflow")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    async with page.expect_popup(timeout=5_000) as cdp_popup_info:
        await page.evaluate("(url) => window.open(url, '_blank')", f"{fixture}/plain?popup=cdp")
    cdp_popup = await cdp_popup_info.value
    await wait_until(lambda: cdp_popup.url == f"{fixture}/plain?popup=cdp", "popup CDPSession URL")
    await cdp_popup.wait_for_load_state("load", timeout=10_000)
    assert_equal(await cdp_popup.text_content("main", timeout=5_000), "plain ok", "popup CDPSession initial text")
    popup_cdp = await context.new_cdp_session(cdp_popup)
    popup_network_events = attach_cdp_event_collector(
        popup_cdp,
        [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
        ],
    )
    try:
        await popup_cdp.send("Network.enable")
        popup_fetch_text = await cdp_popup.evaluate("async () => await fetch('/api?popup-cdp=1').then(r => r.text())")
        assert_equal(popup_fetch_text, "fixture api body", "popup CDPSession fetch body")
        await wait_until(
            lambda: _network_request_finished_for_url(
                popup_network_events, f"{fixture}/api?popup-cdp=1"
            ),
            "popup attached CDPSession Network events",
        )
    finally:
        await popup_cdp.detach()
        await cdp_popup.close()
    state.record("popup_attached_cdp_session_network_events")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    async with page.expect_popup(timeout=5_000) as precedence_popup_info:
        await page.evaluate("(url) => window.open(url, '_blank')", f"{fixture}/plain?popup=precedence")
    precedence_popup = await precedence_popup_info.value
    await wait_until(lambda: precedence_popup.url == f"{fixture}/plain?popup=precedence", "popup precedence URL")
    await precedence_popup.wait_for_load_state("load", timeout=10_000)
    assert_equal(await precedence_popup.text_content("main", timeout=5_000), "plain ok", "popup precedence initial text")
    await context.route(
        "**/popup-route-precedence",
        lambda route: route.fulfill(status=200, body="context route"),
    )
    await precedence_popup.route(
        "**/popup-route-precedence",
        lambda route: route.fulfill(status=200, body="popup route"),
    )
    route_text = await precedence_popup.evaluate(
        "async () => await fetch('/popup-route-precedence').then(response => response.text())"
    )
    assert_equal(route_text, "popup route", "popup page.route should override context.route")
    await precedence_popup.unroute("**/popup-route-precedence")
    await context.unroute("**/popup-route-precedence")
    await precedence_popup.close()
    state.record("popup_page_route_precedence")

    await page.set_content(
        f"<a id='popup-link' href='{fixture}/plain?popup=anchor' target='_blank'>open popup</a>"
    )
    async with page.expect_popup(timeout=5_000) as anchor_popup_info:
        await page.locator("#popup-link").evaluate("anchor => anchor.click()")
    anchor_popup = await anchor_popup_info.value
    assert_equal(anchor_popup.url, f"{fixture}/plain?popup=anchor", "anchor target popup URL")
    await anchor_popup.close()
    state.record("anchor_target_popup_workflow")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    named_first_url = f"{fixture}/plain?popup=named-first"
    named_second_url = f"{fixture}/plain?popup=named-second"
    async with page.expect_popup(timeout=5_000) as named_popup_info:
        await page.evaluate("(url) => window.open(url, 'namedSmokePopup')", named_first_url)
    named_popup = await named_popup_info.value
    assert_equal(named_popup.url, named_first_url, "named popup first URL")
    try:
        async with page.expect_popup(timeout=1_000):
            await page.evaluate("(url) => window.open(url, 'namedSmokePopup')", named_second_url)
    except PlaywrightTimeoutError:
        pass
    else:
        raise SmokeError("window.open with an existing named target created a second popup")
    await named_popup.close()
    state.record("popup_named_target_reuse")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    await page.evaluate("(url) => window.open(url, '_self')", f"{fixture}/history-a?open-self=1")
    await page.wait_for_url(f"{fixture}/history-a?open-self=1", timeout=10_000)
    assert_equal(await page.text_content("main"), "history a", "window.open _self navigation text")
    state.record("window_open_self_navigation")

    await context.add_cookies([{"name": "manualCookie", "value": "manual", "url": fixture}])
    await page.goto(f"{fixture}/echo-cookie", wait_until="load", timeout=10_000)
    cookie_echo = await page.text_content("body")
    if "manualCookie=manual" not in cookie_echo:
        raise SmokeError(f"manual cookie was not sent: {cookie_echo}")
    await page.goto(f"{fixture}/set-cookie", wait_until="load", timeout=10_000)
    cookies = await context.cookies(fixture)
    if not any(cookie.get("name") == "serverCookie" and cookie.get("value") == "server" for cookie in cookies):
        raise SmokeError(f"server Set-Cookie did not reach context: {cookies}")
    await page.goto(f"{fixture}/echo-cookie", wait_until="load", timeout=10_000)
    server_cookie_echo = await page.text_content("body")
    if "serverCookie=server" not in server_cookie_echo:
        raise SmokeError(f"server cookie was not sent back: {server_cookie_echo}")
    state.record("cookie_profile_round_trip")

    redirect_response = await page.goto(f"{fixture}/redirect-start", wait_until="load", timeout=10_000)
    assert_equal(page.url, f"{fixture}/redirect-final", "redirect final page URL")
    assert_equal(redirect_response.url if redirect_response else None, f"{fixture}/redirect-final", "redirect final response URL")
    assert_equal(redirect_response.status if redirect_response else None, 200, "redirect final response status")
    assert_equal(await page.text_content("main"), "redirect final", "redirect final text")
    state.record("redirect_final_response")

    await page.goto(f"{fixture}/history-a", wait_until="load", timeout=10_000)
    await page.goto(f"{fixture}/history-b", wait_until="load", timeout=10_000)
    back_response = await page.go_back(wait_until="load", timeout=10_000)
    assert_equal(page.url, f"{fixture}/history-a", "history goBack URL")
    assert_equal(back_response.url if back_response else None, f"{fixture}/history-a", "history goBack response URL")
    assert_equal(await page.text_content("main"), "history a", "history goBack text")
    forward_response = await page.go_forward(wait_until="load", timeout=10_000)
    assert_equal(page.url, f"{fixture}/history-b", "history goForward URL")
    assert_equal(forward_response.url if forward_response else None, f"{fixture}/history-b", "history goForward response URL")
    assert_equal(await page.text_content("main"), "history b", "history goForward text")
    state.record("history_back_forward")

    background_page = await context.new_page()
    await background_page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    await page.bring_to_front()
    back_after_switch = await page.go_back(wait_until="load", timeout=10_000)
    assert_equal(page.url, f"{fixture}/history-a", "history goBack URL after target switch")
    assert_equal(back_after_switch.url if back_after_switch else None, f"{fixture}/history-a", "history goBack response URL after target switch")
    forward_after_switch = await page.go_forward(wait_until="load", timeout=10_000)
    assert_equal(page.url, f"{fixture}/history-b", "history goForward URL after target switch")
    assert_equal(forward_after_switch.url if forward_after_switch else None, f"{fixture}/history-b", "history goForward response URL after target switch")
    await background_page.close()
    state.record("history_back_forward_after_target_switch")

    await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
    await page.reload(wait_until="load", timeout=10_000)
    assert_equal(await page.text_content("main"), "plain ok", "page reload text")

    await page.goto(f"{fixture}/plain#first", wait_until="load", timeout=10_000)
    await page.evaluate("() => history.pushState({ smoke: true }, '', '#pushed')")
    assert_equal(await page.evaluate("() => location.href"), f"{fixture}/plain#pushed", "history.pushState location.href")
    await page.evaluate("() => location.hash = 'hash-assigned'")
    assert_equal(await page.evaluate("() => location.href"), f"{fixture}/plain#hash-assigned", "hash assignment location.href")
    state.record("page_lifecycle_navigation_variants")


async def run_add_init_script_workflows(state: SmokeState) -> None:
    # Reduced from Playwright page-add-init-script.spec.ts and
    # browsercontext-add-init-script.spec.ts. Keep these in isolated contexts:
    # init scripts are intentionally sticky for later navigations.
    fixture = state.fixture

    context = await state.browser.new_context()
    try:
        page = await context.new_page()
        await context.add_init_script("window.__contextTemp = 123;")
        await page.add_init_script("window.__pageInjected = window.__contextTemp;")
        await page.goto(f"{fixture}/init-script-tamperable", wait_until="load", timeout=10_000)
        snapshot = await page.evaluate("() => window.__initSnapshot")
        assert_equal(
            snapshot,
            {
                "injected": None,
                "contextTemp": 123,
                "pageInjected": 123,
                "scriptOne": None,
                "scriptTwo": None,
                "trailingSecret": None,
            },
            "context and page add_init_script run before page script",
        )

        second_page = await context.new_page()
        await second_page.goto(f"{fixture}/init-script-tamperable", wait_until="load", timeout=10_000)
        second_snapshot = await second_page.evaluate("() => window.__initSnapshot")
        assert_equal(
            second_snapshot,
            {
                "injected": None,
                "contextTemp": 123,
                "pageInjected": None,
                "scriptOne": None,
                "scriptTwo": None,
                "trailingSecret": None,
            },
            "context add_init_script applies to later pages without page script leakage",
        )
    finally:
        await context.close()

    path_context = await state.browser.new_context()
    try:
        page = await path_context.new_page()
        init_path = state.temp_dir / "playwright-init-script.js"
        init_path.write_text("window.__initInjected = 456;", encoding="utf-8")
        await page.add_init_script("// trailing comment")
        await page.add_init_script("window.__initTrailingSecret = 42;")
        await page.add_init_script(path=str(init_path))
        await page.add_init_script("window.__scriptOne = 1;")
        await page.add_init_script("window.__scriptTwo = 2;")
        await page.goto(f"{fixture}/init-script-tamperable", wait_until="load", timeout=10_000)
        snapshot = await page.evaluate("() => window.__initSnapshot")
        assert_equal(
            snapshot,
            {
                "injected": 456,
                "contextTemp": None,
                "pageInjected": None,
                "scriptOne": 1,
                "scriptTwo": 2,
                "trailingSecret": 42,
            },
            "page add_init_script supports path, content, trailing comments, and multiple scripts",
        )
        await page.goto(f"{fixture}/init-script-tamperable?second", wait_until="load", timeout=10_000)
        repeat_snapshot = await page.evaluate("() => window.__initSnapshot")
        assert_equal(repeat_snapshot, snapshot, "page add_init_script persists across navigations")
    finally:
        await path_context.close()

    state.record("playwright_add_init_script_workflows")


async def run_exposed_binding_workflows(state: SmokeState) -> None:
    # Reduced from Playwright page-expose-function.spec.ts and
    # browsercontext-expose-function.spec.ts. Keep this isolated because exposed
    # functions intentionally survive navigations within their page/context.
    fixture = state.fixture
    context = await state.browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)

        binding_sources = []

        async def add_binding(source, a, b):
            binding_sources.append(source)
            return a + b

        await page.expose_binding("add", add_binding)
        assert_equal(
            await page.evaluate("async () => await window.add(5, 6)"),
            11,
            "page.expose_binding result",
        )
        if not binding_sources:
            raise SmokeError("page.expose_binding did not receive a source object")
        source = binding_sources[-1]
        assert_equal(source.get("page"), page, "page.expose_binding source page")
        assert_equal(source.get("context"), context, "page.expose_binding source context")
        assert_equal(source.get("frame"), page.main_frame, "page.expose_binding source frame")

        await page.expose_function("compute", lambda a, b: a * b)
        assert_equal(
            await page.evaluate("async () => await window.compute(9, 4)"),
            36,
            "page.expose_function awaited result",
        )
        await page.goto(f"{fixture}/plain?after-expose", wait_until="load", timeout=10_000)
        assert_equal(
            await page.evaluate("async () => await window.compute(3, 5)"),
            15,
            "page.expose_function survives navigation",
        )

        duplicate_error = None
        try:
            await page.expose_function("compute", lambda a, b: a + b)
        except Exception as error:
            duplicate_error = str(error)
        if duplicate_error is None or 'Function "compute" has been already registered' not in duplicate_error:
            raise SmokeError(f"unexpected duplicate expose_function error: {duplicate_error}")

        await page.expose_function("promiseCompute", async_multiply)
        assert_equal(
            await page.evaluate("async () => await window.promiseCompute(6, 7)"),
            42,
            "page.expose_function awaits returned Python coroutine",
        )

        await page.goto(f"{fixture}/iframe", wait_until="load", timeout=10_000)
        child = next((frame for frame in page.frames if "/child" in frame.url), None)
        if child is None:
            raise SmokeError(f"missing child frame for exposed function; frames={[frame.url for frame in page.frames]}")
        assert_equal(
            await child.evaluate("async () => await window.compute(2, 8)"),
            16,
            "page.expose_function works in child frames",
        )

        context_page = await context.new_page()
        await context.expose_function("contextAdd", lambda a, b: a + b)
        await context_page.goto(f"{fixture}/plain?context-expose", wait_until="load", timeout=10_000)
        assert_equal(
            await context_page.evaluate("async () => await window.contextAdd(10, 5)"),
            15,
            "browser_context.expose_function applies to later pages",
        )
        await context_page.close()
    finally:
        await context.close()

    state.record("playwright_exposed_binding_workflows")


async def async_multiply(a, b):
    await asyncio.sleep(0)
    return a * b


def _network_request_finished_for_url(events: list[dict[str, object]], expected_url: str) -> bool:
    request = next(
        (
            event
            for event in events
            if event.get("method") == "Network.requestWillBeSent"
            and (event.get("params") or {}).get("request", {}).get("url") == expected_url
        ),
        None,
    )
    request_id = request and (request.get("params") or {}).get("requestId")
    return bool(
        request_id
        and any(
            event.get("method") == "Network.responseReceived"
            and (event.get("params") or {}).get("requestId") == request_id
            for event in events
        )
        and any(
            event.get("method") == "Network.loadingFinished"
            and (event.get("params") or {}).get("requestId") == request_id
            for event in events
        )
    )
