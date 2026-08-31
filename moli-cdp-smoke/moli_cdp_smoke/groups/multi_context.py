from __future__ import annotations

import asyncio
import os
import sys
from typing import Any, Awaitable

from ..assertions import SmokeError, assert_equal, record, wait_until
from ..helpers import attach_cdp_event_collector


BODY_TAKEN_CONTINUE_RESPONSE_ERROR = "Unable to continue request as is after body is taken"
RESPONSE_STAGE_BODY_BASE64 = "cmVzcG9uc2Utc3RhZ2UgYm9keQ=="


async def run_multi_context_group(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    await run_multi_page_after_startup_target_close_smoke(browser, fixture, results)
    await run_multi_context_blob_uuid_partition_smoke(browser, fixture, results)
    await run_multi_context_dialog_owner_state_smoke(browser, fixture, results)
    await run_multi_context_route_owner_state_smoke(browser, fixture, results)
    await run_multi_context_popup_evaluate_owner_state_smoke(browser, fixture, results)
    await run_multi_context_popup_initial_route_owner_state_smoke(browser, fixture, results)
    await run_multi_context_popup_page_route_evaluate_owner_state_smoke(browser, fixture, results)
    await run_multi_context_popup_cdp_session_owner_state_smoke(browser, fixture, results)
    await run_multi_context_popup_response_stage_body_owner_state_smoke(browser, fixture, results)
    await run_multi_context_popup_response_stage_stream_owner_state_smoke(browser, fixture, results)
    await run_multi_context_popup_response_stage_stream_wrong_session_smoke(browser, fixture, results)
    await run_multi_context_popup_response_stage_fulfill_fail_owner_state_smoke(browser, fixture, results)
    await run_multi_context_network_websocket_owner_state_smoke(browser, fixture, results)
    await run_multi_context_held_route_resume_smoke(browser, fixture, results)
    await run_multi_context_held_response_stage_resume_smoke(browser, fixture, results)


async def run_multi_page_after_startup_target_close_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    closed_startup_target = await _close_startup_target_if_present(browser)

    context_a = await browser.new_context()
    context_b = await browser.new_context()
    try:
        page_a, page_a_peer, page_b = await asyncio.gather(
            context_a.new_page(),
            context_a.new_page(),
            context_b.new_page(),
        )
        await asyncio.gather(
            page_a.goto(
                f"{fixture}/plain?multi-page=a", wait_until="load", timeout=10_000
            ),
            page_a_peer.goto(
                f"{fixture}/plain?multi-page=a-peer",
                wait_until="load",
                timeout=10_000,
            ),
            page_b.goto(
                f"{fixture}/plain?multi-page=b", wait_until="load", timeout=10_000
            ),
        )

        cdp_a, cdp_a_peer, cdp_b = await asyncio.gather(
            _new_cdp_session(context_a, page_a, "multi-page A"),
            _new_cdp_session(context_a, page_a_peer, "multi-page A peer"),
            _new_cdp_session(context_b, page_b, "multi-page B"),
        )
        evaluations = await asyncio.gather(
            _send_cdp(
                cdp_a,
                "Runtime.evaluate",
                {
                    "expression": "globalThis.__moliMultiPage = 'a'; __moliMultiPage",
                    "returnByValue": True,
                },
            ),
            _send_cdp(
                cdp_a_peer,
                "Runtime.evaluate",
                {
                    "expression": "globalThis.__moliMultiPage = 'a-peer'; __moliMultiPage",
                    "returnByValue": True,
                },
            ),
            _send_cdp(
                cdp_b,
                "Runtime.evaluate",
                {
                    "expression": "globalThis.__moliMultiPage = 'b'; __moliMultiPage",
                    "returnByValue": True,
                },
            ),
        )
        assert_equal(
            [evaluation.get("result", {}).get("value") for evaluation in evaluations],
            ["a", "a-peer", "b"],
            "three target-local Runtime evaluations",
        )

        await page_a_peer.close()
        survivor_values = await asyncio.gather(
            page_a.evaluate(
                "() => `${globalThis.__moliMultiPage}:${document.querySelector('main')?.textContent}`"
            ),
            page_b.evaluate(
                "() => `${globalThis.__moliMultiPage}:${document.querySelector('main')?.textContent}`"
            ),
        )
        assert_equal(
            survivor_values,
            ["a:plain ok", "b:plain ok"],
            "surviving pages after peer close",
        )
        await asyncio.gather(
            page_a.goto(
                f"{fixture}/plain?multi-page=a-final",
                wait_until="load",
                timeout=10_000,
            ),
            page_b.goto(
                f"{fixture}/plain?multi-page=b-final",
                wait_until="load",
                timeout=10_000,
            ),
        )
        assert_equal(
            await asyncio.gather(page_a.text_content("main"), page_b.text_content("main")),
            ["plain ok", "plain ok"],
            "surviving pages after concurrent renavigation",
        )
        record(
            results,
            "multi_page_after_startup_target_close",
            {"closedStartupTarget": closed_startup_target, "pageCount": 3},
        )
    finally:
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def _close_startup_target_if_present(browser: Any) -> bool:
    browser_cdp = await browser.new_browser_cdp_session()
    try:
        targets = await _send_cdp(browser_cdp, "Target.getTargets")
        startup_targets = [
            target
            for target in targets.get("targetInfos", [])
            if target.get("type") == "page" and target.get("targetId") == "moli-default"
        ]
        if len(startup_targets) > 1:
            raise SmokeError(f"multiple startup targets were published: {startup_targets!r}")
        if startup_targets:
            close_result = await _send_cdp(
                browser_cdp,
                "Target.closeTarget",
                {"targetId": "moli-default"},
            )
            assert_equal(close_result.get("success"), True, "close startup Page target")

            async def startup_target_is_absent() -> bool:
                current = await _send_cdp(browser_cdp, "Target.getTargets")
                return all(
                    target.get("targetId") != "moli-default"
                    for target in current.get("targetInfos", [])
                )

            await wait_until(startup_target_is_absent, "closed startup Page target")
            return True
        return False
    finally:
        await browser_cdp.detach()


async def run_multi_context_blob_uuid_partition_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    try:
        page_a = await context_a.new_page()
        page_a_peer = await context_a.new_page()
        page_b = await context_b.new_page()
        await asyncio.gather(
            page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000),
            page_a_peer.goto(f"{fixture}/plain", wait_until="load", timeout=10_000),
            page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000),
        )
        cdp_a = await _new_cdp_session(context_a, page_a, "context A Blob owner")
        cdp_a_peer = await _new_cdp_session(context_a, page_a_peer, "context A Blob peer")
        cdp_b = await _new_cdp_session(context_b, page_b, "context B Blob reader")

        evaluated = await _send_cdp(
            cdp_a,
            "Runtime.evaluate",
            {
                "expression": (
                    "globalThis.__partitionBlob = "
                    "new Blob(['context-a-secret'], {type:'text/plain'})"
                ),
                "returnByValue": False,
            },
        )
        object_id = evaluated.get("result", {}).get("objectId")
        if not isinstance(object_id, str) or not object_id:
            raise SmokeError(f"context A Blob should have a Runtime objectId: {evaluated}")
        resolved = await _send_cdp(cdp_a, "IO.resolveBlob", {"objectId": object_id})
        uuid = resolved.get("uuid")
        if not isinstance(uuid, str) or not uuid:
            raise SmokeError(f"context A Blob should have a DevTools UUID: {resolved}")
        handle = f"blob:{uuid}"

        same_partition_read = await _send_cdp(cdp_a_peer, "IO.read", {"handle": handle})
        assert_equal(
            same_partition_read.get("data"),
            "context-a-secret",
            "same browser context Blob UUID read",
        )
        await _send_cdp(cdp_a_peer, "IO.close", {"handle": handle})

        await _expect_cdp_error(
            _send_cdp(cdp_b, "IO.read", {"handle": handle}),
            "Read failed",
            "cross browser context Blob UUID read",
        )
        record(results, "multi_context_blob_uuid_partition")
    finally:
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_dialog_owner_state_smoke(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/dialog", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/dialog", wait_until="load", timeout=10_000)

        async with page_a.expect_event("dialog", timeout=5_000) as dialog_a_info:
            prompt_task = asyncio.create_task(
                page_a.evaluate("() => prompt('context A prompt', 'context A default')")
            )
        dialog_a = await dialog_a_info.value
        assert_equal(dialog_a.type, "prompt", "context A dialog type")
        assert_equal(dialog_a.message, "context A prompt", "context A dialog message")
        assert_equal(dialog_a.default_value, "context A default", "context A prompt default value")

        async with page_b.expect_event("dialog", timeout=5_000) as dialog_b_info:
            confirm_task = asyncio.create_task(page_b.evaluate("() => confirm('context B confirm')"))
        dialog_b = await dialog_b_info.value
        assert_equal(dialog_b.type, "confirm", "context B dialog type")
        assert_equal(dialog_b.message, "context B confirm", "context B dialog message")

        await dialog_b.dismiss()
        assert_equal(await confirm_task, False, "context B confirm dismiss return value")
        await dialog_a.dismiss()
        assert_equal(await prompt_task, None, "context A prompt dismiss return value")

        assert_equal(
            await page_a.text_content("#alert", timeout=5_000),
            "alert",
            "context A remains usable after delayed dialog handling",
        )
        assert_equal(
            await page_b.text_content("#alert", timeout=5_000),
            "alert",
            "context B remains usable after dialog handling",
        )
        record(results, "multi_context_dialog_owner_state")
    finally:
        await context_a.close()
        await context_b.close()


async def run_multi_context_route_owner_state_smoke(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    route_a_seen = asyncio.Event()
    route_a_continued = asyncio.Event()
    route_b_seen = asyncio.Event()
    route_b_continued = asyncio.Event()
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)

        async def continue_context_a(route: Any) -> None:
            route_a_seen.set()
            headers = dict(route.request.headers)
            headers["x-smoke-route"] = "context-a"
            await route.continue_(headers=headers)
            route_a_continued.set()

        async def continue_context_b(route: Any) -> None:
            route_b_seen.set()
            headers = dict(route.request.headers)
            headers["x-smoke-route"] = "context-b"
            await route.continue_(headers=headers)
            route_b_continued.set()

        await context_a.route("**/api-continue", continue_context_a)
        await context_b.route("**/api-continue", continue_context_b)

        fetch_a = asyncio.create_task(
            page_a.evaluate("async () => await fetch('/api-continue').then(r => r.json())")
        )
        await asyncio.wait_for(route_a_seen.wait(), timeout=5)
        await asyncio.wait_for(route_a_continued.wait(), timeout=5)
        result_a = await asyncio.wait_for(fetch_a, timeout=10)
        assert_equal(result_a.get("routeHeader"), "context-a", "context A route header")

        fetch_b = asyncio.create_task(
            page_b.evaluate("async () => await fetch('/api-continue').then(r => r.json())")
        )
        await asyncio.wait_for(route_b_seen.wait(), timeout=5)
        await asyncio.wait_for(route_b_continued.wait(), timeout=5)
        result_b = await asyncio.wait_for(fetch_b, timeout=10)
        assert_equal(result_b.get("routeHeader"), "context-b", "context B route header")
        record(results, "multi_context_route_owner_state")
    finally:
        await context_a.close()
        await context_b.close()


async def run_multi_context_popup_evaluate_owner_state_smoke(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)

        popup_a = await _open_popup(page_a, f"{fixture}/plain?popup=context-a")
        popup_b = await _open_popup(page_b, f"{fixture}/plain?popup=context-b")

        result_a = await popup_a.evaluate(
            """
            () => {
              localStorage.setItem('popup-owner', 'context-a');
              return { url: location.href, owner: localStorage.getItem('popup-owner') };
            }
            """
        )
        result_b = await popup_b.evaluate(
            """
            () => {
              localStorage.setItem('popup-owner', 'context-b');
              return { url: location.href, owner: localStorage.getItem('popup-owner') };
            }
            """
        )

        assert_equal(result_a.get("owner"), "context-a", "context A popup evaluate owner")
        assert_equal(result_b.get("owner"), "context-b", "context B popup evaluate owner")
        if "popup=context-a" not in result_a.get("url", ""):
            raise SmokeError(f"context A popup evaluated in wrong page: {result_a}")
        if "popup=context-b" not in result_b.get("url", ""):
            raise SmokeError(f"context B popup evaluated in wrong page: {result_b}")
        record(results, "multi_context_popup_evaluate_owner_state")
    finally:
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_popup_initial_route_owner_state_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    route_a_seen = asyncio.Event()
    route_b_seen = asyncio.Event()
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)

        async def fulfill_context_a(route: Any) -> None:
            route_a_seen.set()
            await route.fulfill(
                status=200,
                content_type="text/html; charset=utf-8",
                body="<!doctype html><main>context A popup route</main>",
            )

        async def fulfill_context_b(route: Any) -> None:
            route_b_seen.set()
            await route.fulfill(
                status=200,
                content_type="text/html; charset=utf-8",
                body="<!doctype html><main>context B popup route</main>",
            )

        await context_a.route("**/popup-context-isolated", fulfill_context_a)
        await context_b.route("**/popup-context-isolated", fulfill_context_b)

        popup_a = await _open_popup_with_text(
            page_a,
            f"{fixture}/popup-context-isolated",
            "context A popup route",
            "context A popup initial route",
        )
        popup_b = await _open_popup_with_text(
            page_b,
            f"{fixture}/popup-context-isolated",
            "context B popup route",
            "context B popup initial route",
        )
        await asyncio.wait_for(route_a_seen.wait(), timeout=5)
        await asyncio.wait_for(route_b_seen.wait(), timeout=5)
        assert_equal(
            await popup_a.evaluate("() => document.querySelector('main')?.textContent"),
            "context A popup route",
            "context A popup should keep its route owner",
        )
        assert_equal(
            await popup_b.evaluate("() => document.querySelector('main')?.textContent"),
            "context B popup route",
            "context B popup should keep its route owner",
        )
        record(results, "multi_context_popup_initial_route_owner_state")
        await popup_a.close()
        await popup_b.close()
    finally:
        await _unroute_best_effort(context_a, "**/popup-context-isolated")
        await _unroute_best_effort(context_b, "**/popup-context-isolated")
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_popup_page_route_evaluate_owner_state_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    route_a_seen = asyncio.Event()
    route_b_seen = asyncio.Event()
    popup_a: Any | None = None
    popup_b: Any | None = None
    route_pattern = "**/popup-page-route-api**"
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)

        popup_a = await _open_popup(page_a, f"{fixture}/plain?popup=page-route-a")
        popup_b = await _open_popup(page_b, f"{fixture}/plain?popup=page-route-b")

        async def fulfill_popup_a(route: Any) -> None:
            route_a_seen.set()
            await route.fulfill(
                status=200,
                content_type="application/json; charset=utf-8",
                body='{"owner":"context-a","source":"popup-a"}',
            )

        async def fulfill_popup_b(route: Any) -> None:
            route_b_seen.set()
            await route.fulfill(
                status=200,
                content_type="application/json; charset=utf-8",
                body='{"owner":"context-b","source":"popup-b"}',
            )

        await popup_a.route(route_pattern, fulfill_popup_a)
        await popup_b.route(route_pattern, fulfill_popup_b)

        result_a, result_b = await asyncio.gather(
            _fetch_popup_page_route(popup_a, "context-a"),
            _fetch_popup_page_route(popup_b, "context-b"),
        )
        await asyncio.wait_for(route_a_seen.wait(), timeout=5)
        await asyncio.wait_for(route_b_seen.wait(), timeout=5)

        assert_equal(result_a.get("status"), 200, "context A popup page.route status")
        assert_equal(result_b.get("status"), 200, "context B popup page.route status")
        assert_equal(
            result_a.get("contentType"),
            "application/json; charset=utf-8",
            "context A popup page.route content type",
        )
        assert_equal(
            result_b.get("contentType"),
            "application/json; charset=utf-8",
            "context B popup page.route content type",
        )
        assert_equal(
            result_a.get("body", {}).get("owner"),
            "context-a",
            "context A popup route owner",
        )
        assert_equal(
            result_b.get("body", {}).get("owner"),
            "context-b",
            "context B popup route owner",
        )
        if "popup=page-route-a" not in result_a.get("url", ""):
            raise SmokeError(f"context A popup route evaluated in wrong popup: {result_a}")
        if "popup=page-route-b" not in result_b.get("url", ""):
            raise SmokeError(f"context B popup route evaluated in wrong popup: {result_b}")
        record(results, "multi_context_popup_page_route_evaluate_owner_state")
    finally:
        for popup in (popup_a, popup_b):
            if popup is not None:
                try:
                    await popup.unroute(route_pattern)
                except Exception:
                    pass
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_popup_cdp_session_owner_state_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    cdp_a: Any | None = None
    cdp_b: Any | None = None
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        popup_a = await _open_popup(page_a, f"{fixture}/plain?popup=cdp-a")
        popup_b = await _open_popup(page_b, f"{fixture}/plain?popup=cdp-b")
        cdp_a = await _new_cdp_session(context_a, popup_a, "context A popup")
        cdp_b = await _new_cdp_session(context_b, popup_b, "context B popup")
        events_a = attach_cdp_event_collector(
            cdp_a,
            [
                "Network.requestWillBeSent",
                "Network.responseReceived",
                "Network.loadingFinished",
            ],
        )
        events_b = attach_cdp_event_collector(
            cdp_b,
            [
                "Network.requestWillBeSent",
                "Network.responseReceived",
                "Network.loadingFinished",
            ],
        )
        await _send_cdp(cdp_a, "Network.enable")
        await _send_cdp(cdp_b, "Network.enable")

        result_a, result_b = await asyncio.gather(
            popup_a.evaluate("async () => await fetch('/api-continue?popup=cdp-a').then(r => r.json())"),
            popup_b.evaluate("async () => await fetch('/api-continue?popup=cdp-b').then(r => r.json())"),
        )
        assert_equal(result_a.get("method"), "GET", "context A popup fetch method")
        assert_equal(result_b.get("method"), "GET", "context B popup fetch method")
        expected_a = f"{fixture}/api-continue?popup=cdp-a"
        expected_b = f"{fixture}/api-continue?popup=cdp-b"
        await wait_until(
            lambda: _network_request_finished_for_url(events_a, expected_a),
            "context A popup CDPSession Network events",
        )
        await wait_until(
            lambda: _network_request_finished_for_url(events_b, expected_b),
            "context B popup CDPSession Network events",
        )
        _assert_no_cross_network_output(events_a, forbidden_url=expected_b, label="context A popup")
        _assert_no_cross_network_output(events_b, forbidden_url=expected_a, label="context B popup")
        record(results, "multi_context_popup_cdp_session_owner_state")
    finally:
        for cdp in (cdp_a, cdp_b):
            if cdp is not None:
                try:
                    await cdp.detach()
                except Exception:
                    pass
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_popup_response_stage_body_owner_state_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    cdp_a: Any | None = None
    cdp_b: Any | None = None
    fetch_a: asyncio.Task[Any] | None = None
    fetch_b: asyncio.Task[Any] | None = None
    paused_a: dict[str, Any] | None = None
    paused_b: dict[str, Any] | None = None
    continued_a = False
    continued_b = False
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        popup_a = await _open_popup(page_a, f"{fixture}/plain?popup=response-stage-a")
        popup_b = await _open_popup(page_b, f"{fixture}/plain?popup=response-stage-b")
        cdp_a = await _new_cdp_session(context_a, popup_a, "context A popup response-stage")
        cdp_b = await _new_cdp_session(context_b, popup_b, "context B popup response-stage")
        response_stage_methods = [
            "Fetch.requestPaused",
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
        ]
        events_a = attach_cdp_event_collector(cdp_a, response_stage_methods)
        events_b = attach_cdp_event_collector(cdp_b, response_stage_methods)
        await _send_cdp(cdp_a, "Network.enable")
        await _send_cdp(cdp_b, "Network.enable")
        await _enable_response_stage_fetch(cdp_a)
        await _enable_response_stage_fetch(cdp_b)

        start_a = len(events_a)
        start_b = len(events_b)
        fetch_a = asyncio.create_task(_fetch_response_stage_body(popup_a, "popup-a"))
        fetch_b = asyncio.create_task(_fetch_response_stage_body(popup_b, "popup-b"))

        expected_a = f"{fixture}/api-response-stage?popup=popup-a"
        expected_b = f"{fixture}/api-response-stage?popup=popup-b"

        def saw_pause_a() -> bool:
            nonlocal paused_a
            paused_a = _response_stage_pause_for_url(events_a[start_a:], expected_a)
            return paused_a is not None

        def saw_pause_b() -> bool:
            nonlocal paused_b
            paused_b = _response_stage_pause_for_url(events_b[start_b:], expected_b)
            return paused_b is not None

        await wait_until(saw_pause_a, "context A popup response-stage pause")
        await wait_until(saw_pause_b, "context B popup response-stage pause")
        assert paused_a is not None
        assert paused_b is not None
        network_id_a = _pause_network_id(paused_a, "context A popup")
        network_id_b = _pause_network_id(paused_b, "context B popup")
        request_id_a = _pause_request_id(paused_a, "context A popup")
        request_id_b = _pause_request_id(paused_b, "context B popup")

        body_a = await _send_cdp(cdp_a, "Fetch.getResponseBody", {"requestId": request_id_a})
        body_b = await _send_cdp(cdp_b, "Fetch.getResponseBody", {"requestId": request_id_b})
        assert_equal(body_a.get("body"), "response-stage body", "context A popup Fetch.getResponseBody")
        assert_equal(body_b.get("body"), "response-stage body", "context B popup Fetch.getResponseBody")
        assert_equal(body_a.get("base64Encoded"), False, "context A popup Fetch.getResponseBody encoding")
        assert_equal(body_b.get("base64Encoded"), False, "context B popup Fetch.getResponseBody encoding")

        if fetch_a.done() or fetch_b.done():
            raise SmokeError("popup response-stage fetch completed before continueResponse")

        await _send_cdp(cdp_b, "Fetch.continueResponse", {"requestId": request_id_b})
        continued_b = True
        assert_equal(
            await asyncio.wait_for(fetch_b, timeout=10),
            "response-stage body",
            "context B popup response-stage body while A is held",
        )
        await wait_until(
            lambda: _network_sequence_complete_for_url(events_b[start_b:], network_id_b, expected_b),
            "context B popup response-stage Network events while A is held",
        )
        if fetch_a.done():
            raise SmokeError("context A popup response-stage fetch completed before release")
        if _has_loading_finished(events_a[start_a:], network_id_a):
            raise SmokeError("context A popup Network.loadingFinished fired before continueResponse")

        await _send_cdp(cdp_a, "Fetch.continueResponse", {"requestId": request_id_a})
        continued_a = True
        assert_equal(
            await asyncio.wait_for(fetch_a, timeout=10),
            "response-stage body",
            "context A popup response-stage body after release",
        )
        await wait_until(
            lambda: _network_sequence_complete_for_url(events_a[start_a:], network_id_a, expected_a),
            "context A popup response-stage Network events after release",
        )
        _assert_no_cross_network_output(events_a[start_a:], forbidden_url=expected_b, label="context A popup")
        _assert_no_cross_network_output(events_b[start_b:], forbidden_url=expected_a, label="context B popup")
        record(results, "multi_context_popup_response_stage_body_owner_state")
    finally:
        for task in (fetch_a, fetch_b):
            if task is not None and not task.done():
                task.cancel()
        if cdp_b is not None and paused_b is not None and not continued_b:
            await _continue_response_best_effort(cdp_b, paused_b, "context B popup")
        if cdp_a is not None and paused_a is not None and not continued_a:
            await _continue_response_best_effort(cdp_a, paused_a, "context A popup")
        for cdp in (cdp_a, cdp_b):
            if cdp is not None:
                try:
                    await _send_cdp(cdp, "Fetch.disable")
                except Exception:
                    pass
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_popup_response_stage_stream_owner_state_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    cdp_a: Any | None = None
    cdp_b: Any | None = None
    fetch_a: asyncio.Task[Any] | None = None
    fetch_b: asyncio.Task[Any] | None = None
    paused_a: dict[str, Any] | None = None
    paused_b: dict[str, Any] | None = None
    stream_a: str | None = None
    stream_b: str | None = None
    continued_a = False
    continued_b = False
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        popup_a = await _open_popup(page_a, f"{fixture}/plain?popup=response-stream-a")
        popup_b = await _open_popup(page_b, f"{fixture}/plain?popup=response-stream-b")
        cdp_a = await _new_cdp_session(context_a, popup_a, "context A popup response-stage stream")
        cdp_b = await _new_cdp_session(context_b, popup_b, "context B popup response-stage stream")
        response_stage_methods = [
            "Fetch.requestPaused",
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
        ]
        events_a = attach_cdp_event_collector(cdp_a, response_stage_methods)
        events_b = attach_cdp_event_collector(cdp_b, response_stage_methods)
        await _send_cdp(cdp_a, "Network.enable")
        await _send_cdp(cdp_b, "Network.enable")
        await _enable_response_stage_fetch(cdp_a)
        await _enable_response_stage_fetch(cdp_b)

        start_a = len(events_a)
        start_b = len(events_b)
        fetch_a = asyncio.create_task(_fetch_response_stage_body(popup_a, "popup-stream-a"))
        fetch_b = asyncio.create_task(_fetch_response_stage_body(popup_b, "popup-stream-b"))
        expected_a = f"{fixture}/api-response-stage?popup=popup-stream-a"
        expected_b = f"{fixture}/api-response-stage?popup=popup-stream-b"

        def saw_pause_a() -> bool:
            nonlocal paused_a
            paused_a = _response_stage_pause_for_url(events_a[start_a:], expected_a)
            return paused_a is not None

        def saw_pause_b() -> bool:
            nonlocal paused_b
            paused_b = _response_stage_pause_for_url(events_b[start_b:], expected_b)
            return paused_b is not None

        await wait_until(saw_pause_a, "context A popup response-stage stream pause")
        await wait_until(saw_pause_b, "context B popup response-stage stream pause")
        assert paused_a is not None
        assert paused_b is not None
        network_id_a = _pause_network_id(paused_a, "context A popup stream")
        network_id_b = _pause_network_id(paused_b, "context B popup stream")
        request_id_a = _pause_request_id(paused_a, "context A popup stream")
        request_id_b = _pause_request_id(paused_b, "context B popup stream")

        stream_a = _stream_handle(
            await _send_cdp(cdp_a, "Fetch.takeResponseBodyAsStream", {"requestId": request_id_a}),
            "context A popup",
        )
        stream_b = _stream_handle(
            await _send_cdp(cdp_b, "Fetch.takeResponseBodyAsStream", {"requestId": request_id_b}),
            "context B popup",
        )

        first_a = await _send_cdp(cdp_a, "IO.read", {"handle": stream_a, "offset": 0, "size": 8})
        assert_equal(first_a.get("base64Encoded"), False, "context A popup stream first chunk encoding")
        assert_equal(first_a.get("data"), "response", "context A popup stream first chunk")
        assert_equal(first_a.get("eof"), False, "context A popup stream first chunk eof")

        offset_b = await _send_cdp(cdp_b, "IO.read", {"handle": stream_b, "offset": 9, "size": 5})
        assert_equal(offset_b.get("base64Encoded"), False, "context B popup stream offset encoding")
        assert_equal(offset_b.get("data"), "stage", "context B popup stream offset chunk")
        assert_equal(offset_b.get("eof"), False, "context B popup stream offset eof")
        first_b = await _send_cdp(cdp_b, "IO.read", {"handle": stream_b, "offset": 0, "size": 8})
        assert_equal(first_b.get("base64Encoded"), False, "context B popup stream first chunk encoding")
        assert_equal(first_b.get("data"), "response", "context B popup stream first chunk")
        tail_b = await _send_cdp(cdp_b, "IO.read", {"handle": stream_b})
        assert_equal(tail_b.get("base64Encoded"), False, "context B popup stream tail encoding")
        assert_equal(tail_b.get("data"), "-stage body", "context B popup stream tail")
        assert_equal(tail_b.get("eof"), True, "context B popup stream tail eof")

        if fetch_a.done() or fetch_b.done():
            raise SmokeError("popup response-stage stream fetch completed before fulfillRequest")

        await _expect_cdp_error(
            _send_cdp(cdp_b, "Fetch.continueResponse", {"requestId": request_id_b}),
            BODY_TAKEN_CONTINUE_RESPONSE_ERROR,
            "context B popup stream continueResponse after body taken",
        )
        await _fulfill_response_stage_body(cdp_b, request_id_b)
        continued_b = True
        assert_equal(
            await asyncio.wait_for(fetch_b, timeout=10),
            "response-stage body",
            "context B popup stream fulfilled body while A is held",
        )
        await wait_until(
            lambda: _network_sequence_complete_for_url(events_b[start_b:], network_id_b, expected_b),
            "context B popup response-stage stream fulfilled Network events while A is held",
        )
        await _send_cdp(cdp_b, "IO.close", {"handle": stream_b})
        await _expect_cdp_error(
            _send_cdp(cdp_b, "IO.read", {"handle": stream_b}),
            "StreamHandleNotFound",
            "context B popup stream read after close",
        )
        stream_b = None

        if fetch_a.done():
            raise SmokeError("context A popup response-stage stream fetch completed before fulfillRequest")
        if _has_loading_finished(events_a[start_a:], network_id_a):
            raise SmokeError("context A popup response-stage stream Network.loadingFinished fired before fulfillRequest")

        tail_a = await _send_cdp(cdp_a, "IO.read", {"handle": stream_a})
        assert_equal(tail_a.get("base64Encoded"), False, "context A popup stream tail encoding")
        assert_equal(tail_a.get("data"), "-stage body", "context A popup stream tail")
        assert_equal(tail_a.get("eof"), True, "context A popup stream tail eof")
        await _expect_cdp_error(
            _send_cdp(cdp_a, "Fetch.continueResponse", {"requestId": request_id_a}),
            BODY_TAKEN_CONTINUE_RESPONSE_ERROR,
            "context A popup stream continueResponse after body taken",
        )
        await _fulfill_response_stage_body(cdp_a, request_id_a)
        continued_a = True
        assert_equal(
            await asyncio.wait_for(fetch_a, timeout=10),
            "response-stage body",
            "context A popup stream fulfilled body after release",
        )
        await wait_until(
            lambda: _network_sequence_complete_for_url(events_a[start_a:], network_id_a, expected_a),
            "context A popup response-stage stream fulfilled Network events after release",
        )
        await _send_cdp(cdp_a, "IO.close", {"handle": stream_a})
        stream_a = None
        _assert_no_cross_network_output(events_a[start_a:], forbidden_url=expected_b, label="context A popup")
        _assert_no_cross_network_output(events_b[start_b:], forbidden_url=expected_a, label="context B popup")
        record(results, "multi_context_popup_response_stage_stream_owner_state")
    finally:
        for task in (fetch_a, fetch_b):
            if task is not None and not task.done():
                task.cancel()
        for cdp, handle in ((cdp_b, stream_b), (cdp_a, stream_a)):
            if cdp is not None and handle is not None:
                await _close_io_stream_best_effort(cdp, handle)
        if cdp_b is not None and paused_b is not None and not continued_b:
            await _continue_response_best_effort(cdp_b, paused_b, "context B popup stream")
        if cdp_a is not None and paused_a is not None and not continued_a:
            await _continue_response_best_effort(cdp_a, paused_a, "context A popup stream")
        for cdp in (cdp_a, cdp_b):
            if cdp is not None:
                try:
                    await _send_cdp(cdp, "Fetch.disable")
                except Exception:
                    pass
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_popup_response_stage_stream_wrong_session_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    cdp_a: Any | None = None
    cdp_b: Any | None = None
    fetch_a: asyncio.Task[Any] | None = None
    fetch_b: asyncio.Task[Any] | None = None
    paused_a: dict[str, Any] | None = None
    paused_b: dict[str, Any] | None = None
    stream_a: str | None = None
    stream_b: str | None = None
    continued_a = False
    continued_b = False
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        popup_a = await _open_popup(page_a, f"{fixture}/plain?popup=response-stream-owner-a")
        popup_b = await _open_popup(page_b, f"{fixture}/plain?popup=response-stream-owner-b")
        cdp_a = await _new_cdp_session(context_a, popup_a, "context A popup response-stage stream owner")
        cdp_b = await _new_cdp_session(context_b, popup_b, "context B popup response-stage stream owner")
        response_stage_methods = [
            "Fetch.requestPaused",
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
        ]
        events_a = attach_cdp_event_collector(cdp_a, response_stage_methods)
        events_b = attach_cdp_event_collector(cdp_b, response_stage_methods)
        await _send_cdp(cdp_a, "Network.enable")
        await _send_cdp(cdp_b, "Network.enable")
        await _enable_response_stage_fetch(cdp_a)
        await _enable_response_stage_fetch(cdp_b)

        start_a = len(events_a)
        start_b = len(events_b)
        fetch_a = asyncio.create_task(_fetch_response_stage_body(popup_a, "popup-stream-owner-a"))
        fetch_b = asyncio.create_task(_fetch_response_stage_body(popup_b, "popup-stream-owner-b"))
        expected_a = f"{fixture}/api-response-stage?popup=popup-stream-owner-a"
        expected_b = f"{fixture}/api-response-stage?popup=popup-stream-owner-b"

        def saw_pause_a() -> bool:
            nonlocal paused_a
            paused_a = _response_stage_pause_for_url(events_a[start_a:], expected_a)
            return paused_a is not None

        def saw_pause_b() -> bool:
            nonlocal paused_b
            paused_b = _response_stage_pause_for_url(events_b[start_b:], expected_b)
            return paused_b is not None

        await wait_until(saw_pause_a, "context A popup response-stage stream owner pause")
        await wait_until(saw_pause_b, "context B popup response-stage stream owner pause")
        assert paused_a is not None
        assert paused_b is not None
        network_id_a = _pause_network_id(paused_a, "context A popup stream owner")
        network_id_b = _pause_network_id(paused_b, "context B popup stream owner")
        request_id_a = _pause_request_id(paused_a, "context A popup stream owner")
        request_id_b = _pause_request_id(paused_b, "context B popup stream owner")

        stream_a = _stream_handle(
            await _send_cdp(cdp_a, "Fetch.takeResponseBodyAsStream", {"requestId": request_id_a}),
            "context A popup",
        )
        stream_b = _stream_handle(
            await _send_cdp(cdp_b, "Fetch.takeResponseBodyAsStream", {"requestId": request_id_b}),
            "context B popup",
        )
        _trace_multi_context(
            f"popup response-stage stream handles: context A={stream_a!r}, context B={stream_b!r}"
        )
        if stream_a == stream_b:
            raise SmokeError("popup response-stage streams should be target-owner scoped")

        await _expect_cdp_error(
            _send_cdp(cdp_b, "IO.read", {"handle": stream_a, "offset": 0, "size": 4}),
            "StreamHandleNotFound",
            "context B should not read context A popup stream",
        )
        await _expect_cdp_error(
            _send_cdp(cdp_b, "IO.close", {"handle": stream_a}),
            "StreamHandleNotFound",
            "context B should not close context A popup stream",
        )
        await _expect_cdp_error(
            _send_cdp(cdp_a, "IO.read", {"handle": stream_b, "offset": 0, "size": 4}),
            "StreamHandleNotFound",
            "context A should not read context B popup stream",
        )
        await _expect_cdp_error(
            _send_cdp(cdp_a, "IO.close", {"handle": stream_b}),
            "StreamHandleNotFound",
            "context A should not close context B popup stream",
        )

        first_a = await _send_cdp(cdp_a, "IO.read", {"handle": stream_a, "offset": 0, "size": 8})
        assert_equal(first_a.get("data"), "response", "context A popup stream remains readable by owner")
        tail_a = await _send_cdp(cdp_a, "IO.read", {"handle": stream_a})
        assert_equal(tail_a.get("data"), "-stage body", "context A popup stream owner tail")
        assert_equal(tail_a.get("eof"), True, "context A popup stream owner tail eof")

        first_b = await _send_cdp(cdp_b, "IO.read", {"handle": stream_b, "offset": 0, "size": 8})
        assert_equal(first_b.get("data"), "response", "context B popup stream remains readable by owner")
        tail_b = await _send_cdp(cdp_b, "IO.read", {"handle": stream_b})
        assert_equal(tail_b.get("data"), "-stage body", "context B popup stream owner tail")
        assert_equal(tail_b.get("eof"), True, "context B popup stream owner tail eof")

        await _expect_cdp_error(
            _send_cdp(cdp_a, "Fetch.continueResponse", {"requestId": request_id_a}),
            BODY_TAKEN_CONTINUE_RESPONSE_ERROR,
            "context A popup stream owner continueResponse after body taken",
        )
        await _fulfill_response_stage_body(cdp_a, request_id_a)
        continued_a = True
        await _expect_cdp_error(
            _send_cdp(cdp_b, "Fetch.continueResponse", {"requestId": request_id_b}),
            BODY_TAKEN_CONTINUE_RESPONSE_ERROR,
            "context B popup stream owner continueResponse after body taken",
        )
        await _fulfill_response_stage_body(cdp_b, request_id_b)
        continued_b = True
        assert_equal(
            await asyncio.wait_for(fetch_a, timeout=10),
            "response-stage body",
            "context A popup stream owner fulfilled body",
        )
        assert_equal(
            await asyncio.wait_for(fetch_b, timeout=10),
            "response-stage body",
            "context B popup stream owner fulfilled body",
        )
        await wait_until(
            lambda: _network_sequence_complete_for_url(events_a[start_a:], network_id_a, expected_a),
            "context A popup response-stage stream owner Network events",
        )
        await wait_until(
            lambda: _network_sequence_complete_for_url(events_b[start_b:], network_id_b, expected_b),
            "context B popup response-stage stream owner Network events",
        )
        await _send_cdp(cdp_a, "IO.close", {"handle": stream_a})
        stream_a = None
        await _send_cdp(cdp_b, "IO.close", {"handle": stream_b})
        stream_b = None
        _assert_no_cross_network_output(events_a[start_a:], forbidden_url=expected_b, label="context A popup")
        _assert_no_cross_network_output(events_b[start_b:], forbidden_url=expected_a, label="context B popup")
        record(results, "multi_context_popup_response_stage_stream_wrong_session")
    finally:
        for task in (fetch_a, fetch_b):
            if task is not None and not task.done():
                task.cancel()
        for cdp, handle in ((cdp_b, stream_b), (cdp_a, stream_a)):
            if cdp is not None and handle is not None:
                await _close_io_stream_best_effort(cdp, handle)
        if cdp_b is not None and paused_b is not None and not continued_b:
            await _continue_response_best_effort(cdp_b, paused_b, "context B popup stream owner")
        if cdp_a is not None and paused_a is not None and not continued_a:
            await _continue_response_best_effort(cdp_a, paused_a, "context A popup stream owner")
        for cdp in (cdp_a, cdp_b):
            if cdp is not None:
                try:
                    await _send_cdp(cdp, "Fetch.disable")
                except Exception:
                    pass
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_popup_response_stage_fulfill_fail_owner_state_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    cdp_a: Any | None = None
    cdp_b: Any | None = None
    fetch_a: asyncio.Task[Any] | None = None
    fetch_b: asyncio.Task[Any] | None = None
    paused_a: dict[str, Any] | None = None
    paused_b: dict[str, Any] | None = None
    fulfilled_a = False
    failed_b = False
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        popup_a = await _open_popup(page_a, f"{fixture}/plain?popup=response-fulfill-a")
        popup_b = await _open_popup(page_b, f"{fixture}/plain?popup=response-fail-b")
        cdp_a = await _new_cdp_session(context_a, popup_a, "context A popup response-stage fulfill")
        cdp_b = await _new_cdp_session(context_b, popup_b, "context B popup response-stage fail")
        response_stage_methods = [
            "Fetch.requestPaused",
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
            "Network.loadingFailed",
        ]
        events_a = attach_cdp_event_collector(cdp_a, response_stage_methods)
        events_b = attach_cdp_event_collector(cdp_b, response_stage_methods)
        await _send_cdp(cdp_a, "Network.enable")
        await _send_cdp(cdp_b, "Network.enable")
        await _enable_response_stage_fetch(cdp_a)
        await _enable_response_stage_fetch(cdp_b)

        start_a = len(events_a)
        start_b = len(events_b)
        fetch_a = asyncio.create_task(_fetch_response_stage_result(popup_a, "popup-fulfill-a"))
        fetch_b = asyncio.create_task(_fetch_response_stage_result(popup_b, "popup-fail-b"))
        expected_a = f"{fixture}/api-response-stage?popup=popup-fulfill-a"
        expected_b = f"{fixture}/api-response-stage?popup=popup-fail-b"

        def saw_pause_a() -> bool:
            nonlocal paused_a
            paused_a = _response_stage_pause_for_url(events_a[start_a:], expected_a)
            return paused_a is not None

        def saw_pause_b() -> bool:
            nonlocal paused_b
            paused_b = _response_stage_pause_for_url(events_b[start_b:], expected_b)
            return paused_b is not None

        await wait_until(saw_pause_a, "context A popup response-stage fulfill pause")
        await wait_until(saw_pause_b, "context B popup response-stage fail pause")
        assert paused_a is not None
        assert paused_b is not None
        network_id_a = _pause_network_id(paused_a, "context A popup fulfill")
        network_id_b = _pause_network_id(paused_b, "context B popup fail")
        request_id_a = _pause_request_id(paused_a, "context A popup fulfill")
        request_id_b = _pause_request_id(paused_b, "context B popup fail")

        if fetch_a.done() or fetch_b.done():
            raise SmokeError("popup response-stage fulfill/fail fetch completed before CDP action")

        await _send_cdp(
            cdp_a,
            "Fetch.fulfillRequest",
            {
                "requestId": request_id_a,
                "responseCode": 202,
                "responseHeaders": [
                    {"name": "content-type", "value": "text/plain"},
                    {"name": "x-smoke-fulfilled", "value": "context-a"},
                ],
                "body": "ZnVsZmlsbGVkLXBvcHVwLWE=",
            },
        )
        fulfilled_a = True
        result_a = await asyncio.wait_for(fetch_a, timeout=10)
        assert_equal(result_a.get("status"), 202, "context A popup fulfilled response status")
        assert_equal(result_a.get("header"), "context-a", "context A popup fulfilled response header")
        assert_equal(result_a.get("body"), "fulfilled-popup-a", "context A popup fulfilled response body")
        await wait_until(
            lambda: _network_sequence_complete_for_url(events_a[start_a:], network_id_a, expected_a),
            "context A popup response-stage fulfill Network events",
        )

        if fetch_b.done():
            raise SmokeError("context B popup response-stage fetch completed before failRequest")
        if _has_loading_finished(events_b[start_b:], network_id_b) or _has_loading_failed(events_b[start_b:], network_id_b):
            raise SmokeError("context B popup Network terminal event fired before failRequest")

        await _send_cdp(
            cdp_b,
            "Fetch.failRequest",
            {"requestId": request_id_b, "errorReason": "Aborted"},
        )
        failed_b = True
        result_b = await asyncio.wait_for(fetch_b, timeout=10)
        error_b = result_b.get("error")
        if not isinstance(error_b, str) or not error_b:
            raise SmokeError(f"context B popup failRequest should reject fetch: {result_b}")
        await wait_until(
            lambda: _has_loading_failed(events_b[start_b:], network_id_b),
            "context B popup response-stage fail Network.loadingFailed",
        )

        _assert_no_cross_network_output(events_a[start_a:], forbidden_url=expected_b, label="context A popup")
        _assert_no_cross_network_output(events_b[start_b:], forbidden_url=expected_a, label="context B popup")
        record(results, "multi_context_popup_response_stage_fulfill_fail_owner_state")
    finally:
        for task in (fetch_a, fetch_b):
            if task is not None and not task.done():
                task.cancel()
        if cdp_b is not None and paused_b is not None and not failed_b:
            await _continue_response_best_effort(cdp_b, paused_b, "context B popup fail")
        if cdp_a is not None and paused_a is not None and not fulfilled_a:
            await _continue_response_best_effort(cdp_a, paused_a, "context A popup fulfill")
        for cdp in (cdp_a, cdp_b):
            if cdp is not None:
                try:
                    await _send_cdp(cdp, "Fetch.disable")
                except Exception:
                    pass
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def run_multi_context_network_websocket_owner_state_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        cdp_a = await _new_cdp_session(context_a, page_a, "context A")
        cdp_b = await _new_cdp_session(context_b, page_b, "context B")
        network_methods = [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
            "Network.loadingFailed",
            "Network.webSocketCreated",
            "Network.webSocketWillSendHandshakeRequest",
            "Network.webSocketHandshakeResponseReceived",
            "Network.webSocketFrameSent",
            "Network.webSocketFrameReceived",
        ]
        events_a = attach_cdp_event_collector(cdp_a, network_methods)
        events_b = attach_cdp_event_collector(cdp_b, network_methods)
        await _send_cdp(cdp_a, "Network.enable")
        await _send_cdp(cdp_b, "Network.enable")
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)

        start_a = len(events_a)
        start_b = len(events_b)
        result_a_task = asyncio.create_task(
            _trigger_fetch_and_websocket(page_a, "a", "context-a-websocket")
        )
        result_b_task = asyncio.create_task(_trigger_fetch_and_websocket(page_b, "b", "b"))
        result_a, result_b = await asyncio.gather(
            asyncio.wait_for(result_a_task, timeout=10),
            asyncio.wait_for(result_b_task, timeout=10),
        )

        assert_equal(result_a.get("api", {}).get("routeHeader"), None, "context A fetch reached fixture")
        assert_equal(result_a.get("api", {}).get("method"), "GET", "context A fetch method")
        assert_equal(result_a.get("ws"), "echo:context-a-websocket", "context A websocket echo")
        assert_equal(result_b.get("api", {}).get("routeHeader"), None, "context B fetch reached fixture")
        assert_equal(result_b.get("api", {}).get("method"), "GET", "context B fetch method")
        assert_equal(result_b.get("ws"), "echo:b", "context B websocket echo")

        expected_a_api = f"{fixture}/api-continue?owner=a"
        expected_b_api = f"{fixture}/api-continue?owner=b"
        expected_ws_url = fixture.replace("http:", "ws:", 1) + "/ws-echo"

        await wait_until(
            lambda: _network_request_finished_for_url(events_a[start_a:], expected_a_api),
            "context A fetch Network event sequence",
        )
        await wait_until(
            lambda: _network_request_finished_for_url(events_b[start_b:], expected_b_api),
            "context B fetch Network event sequence",
        )
        await wait_until(
            lambda: _websocket_received_payload_length(
                events_a[start_a:], expected_ws_url, len("echo:context-a-websocket")
            ),
            "context A WebSocket Network event sequence",
        )
        await wait_until(
            lambda: _websocket_received_payload_length(events_b[start_b:], expected_ws_url, len("echo:b")),
            "context B WebSocket Network event sequence",
        )

        _assert_no_cross_network_output(events_a[start_a:], forbidden_url=expected_b_api, label="context A")
        _assert_no_cross_network_output(events_b[start_b:], forbidden_url=expected_a_api, label="context B")
        _assert_no_cross_websocket_payload(
            events_a[start_a:], forbidden_payload_length=len("echo:b"), label="context A"
        )
        _assert_no_cross_websocket_payload(
            events_b[start_b:],
            forbidden_payload_length=len("echo:context-a-websocket"),
            label="context B",
        )
        record(
            results,
            "multi_context_network_websocket_owner_state",
            {"eventsA": len(events_a) - start_a, "eventsB": len(events_b) - start_b},
        )
    finally:
        await context_a.close()
        await context_b.close()


async def run_multi_context_held_route_resume_smoke(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    route_a_seen = asyncio.Event()
    route_a_release = asyncio.Event()
    route_a_continued = asyncio.Event()
    route_b_seen = asyncio.Event()
    route_b_continued = asyncio.Event()
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)

        async def hold_context_a(route: Any) -> None:
            route_a_seen.set()
            await asyncio.wait_for(route_a_release.wait(), timeout=10)
            headers = dict(route.request.headers)
            headers["x-smoke-route"] = "context-a-held"
            await route.continue_(headers=headers)
            route_a_continued.set()

        async def continue_context_b(route: Any) -> None:
            route_b_seen.set()
            headers = dict(route.request.headers)
            headers["x-smoke-route"] = "context-b-during-held"
            await route.continue_(headers=headers)
            route_b_continued.set()

        await context_a.route("**/api-continue", hold_context_a)
        await context_b.route("**/api-continue", continue_context_b)

        fetch_a = asyncio.create_task(
            page_a.evaluate("async () => await fetch('/api-continue').then(r => r.json())")
        )
        await asyncio.wait_for(route_a_seen.wait(), timeout=5)

        fetch_b = asyncio.create_task(
            page_b.evaluate("async () => await fetch('/api-continue').then(r => r.json())")
        )
        await asyncio.wait_for(route_b_seen.wait(), timeout=5)
        await asyncio.wait_for(route_b_continued.wait(), timeout=5)
        result_b = await asyncio.wait_for(fetch_b, timeout=10)
        assert_equal(result_b.get("routeHeader"), "context-b-during-held", "context B route header while A is held")

        if fetch_a.done():
            raise SmokeError("context A held route completed before release")

        route_a_release.set()
        await asyncio.wait_for(route_a_continued.wait(), timeout=5)
        result_a = await asyncio.wait_for(fetch_a, timeout=10)
        assert_equal(result_a.get("routeHeader"), "context-a-held", "context A held route header after release")
        record(results, "multi_context_held_route_resume")
    finally:
        await context_a.unroute("**/api-continue")
        await context_b.unroute("**/api-continue")
        await context_a.close()
        await context_b.close()


async def run_multi_context_held_response_stage_resume_smoke(
    browser: Any, fixture: str, results: list[dict[str, Any]]
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    cdp_a: Any | None = None
    cdp_b: Any | None = None
    fetch_a: asyncio.Task[Any] | None = None
    fetch_b: asyncio.Task[Any] | None = None
    paused_a: dict[str, Any] | None = None
    paused_b: dict[str, Any] | None = None
    continued_a = False
    continued_b = False
    try:
        page_a = await context_a.new_page()
        page_b = await context_b.new_page()
        cdp_a = await _new_cdp_session(context_a, page_a, "context A")
        cdp_b = await _new_cdp_session(context_b, page_b, "context B")
        response_stage_methods = [
            "Fetch.requestPaused",
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
        ]
        events_a = attach_cdp_event_collector(cdp_a, response_stage_methods)
        events_b = attach_cdp_event_collector(cdp_b, response_stage_methods)

        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_b.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await _send_cdp(cdp_a, "Network.enable")
        await _send_cdp(cdp_b, "Network.enable")
        await _enable_response_stage_fetch(cdp_a)
        await _enable_response_stage_fetch(cdp_b)

        start_a = len(events_a)
        fetch_a = asyncio.create_task(
            page_a.evaluate(
                "async () => await fetch('/api-response-stage').then(response => response.text())"
            )
        )

        def saw_pause_a() -> bool:
            nonlocal paused_a
            paused_a = _response_stage_pause_for(events_a[start_a:], fixture)
            return paused_a is not None

        _trace_multi_context("waiting for context A response-stage pause")
        await wait_until(saw_pause_a, "context A response-stage pause")
        _trace_multi_context("context A response-stage paused")
        assert paused_a is not None
        network_id_a = _pause_network_id(paused_a, "context A")

        start_b = len(events_b)
        fetch_b = asyncio.create_task(
            page_b.evaluate(
                "async () => await fetch('/api-response-stage').then(response => response.text())"
            )
        )

        def saw_pause_b() -> bool:
            nonlocal paused_b
            paused_b = _response_stage_pause_for(events_b[start_b:], fixture)
            return paused_b is not None

        _trace_multi_context("waiting for context B response-stage pause")
        await wait_until(saw_pause_b, "context B response-stage pause")
        _trace_multi_context("context B response-stage paused")
        assert paused_b is not None
        network_id_b = _pause_network_id(paused_b, "context B")
        await _send_cdp(
            cdp_b,
            "Fetch.continueResponse",
            {"requestId": _pause_request_id(paused_b, "context B")},
        )
        continued_b = True
        assert_equal(
            await asyncio.wait_for(fetch_b, timeout=10),
            "response-stage body",
            "context B response-stage body while A is held",
        )
        await wait_until(
            lambda: _network_sequence_complete(events_b[start_b:], network_id_b, fixture),
            "context B response-stage Network events while A is held",
        )

        if fetch_a.done():
            raise SmokeError("context A response-stage fetch completed before continueResponse")
        if _has_loading_finished(events_a[start_a:], network_id_a):
            raise SmokeError("context A response-stage Network.loadingFinished fired before continueResponse")

        await _send_cdp(
            cdp_a,
            "Fetch.continueResponse",
            {"requestId": _pause_request_id(paused_a, "context A")},
        )
        continued_a = True
        assert_equal(
            await asyncio.wait_for(fetch_a, timeout=10),
            "response-stage body",
            "context A response-stage body after release",
        )
        await wait_until(
            lambda: _network_sequence_complete(events_a[start_a:], network_id_a, fixture),
            "context A response-stage Network events after release",
        )
        record(results, "multi_context_held_response_stage_resume")
    finally:
        for task in (fetch_a, fetch_b):
            if task is not None and not task.done():
                task.cancel()
        if cdp_b is not None and paused_b is not None and not continued_b:
            await _continue_response_best_effort(cdp_b, paused_b, "context B")
        if cdp_a is not None and paused_a is not None and not continued_a:
            await _continue_response_best_effort(cdp_a, paused_a, "context A")
        for cdp in (cdp_a, cdp_b):
            if cdp is not None:
                try:
                    await _send_cdp(cdp, "Fetch.disable")
                except Exception:
                    pass
        await _close_context_best_effort(context_a)
        await _close_context_best_effort(context_b)


async def _enable_response_stage_fetch(cdp: Any) -> None:
    await _send_cdp(
        cdp,
        "Fetch.enable",
        {
            "patterns": [
                {
                    "urlPattern": "*/api-response-stage*",
                    "requestStage": "Response",
                    "resourceType": "Fetch",
                }
            ]
        },
    )


async def _open_popup(page: Any, url: str) -> Any:
    async with page.expect_popup(timeout=5_000) as popup_info:
        await page.evaluate("(url) => window.open(url, '_blank')", url)
    popup = await popup_info.value
    await wait_until(lambda: popup.url == url, "popup URL")
    await popup.wait_for_load_state("load", timeout=10_000)
    assert_equal(await popup.text_content("main", timeout=5_000), "plain ok", "popup initial text")
    assert_equal(popup.url, url, "popup loaded URL")
    return popup


async def _open_popup_with_text(page: Any, url: str, expected_text: str, label: str) -> Any:
    async with page.expect_popup(timeout=5_000) as popup_info:
        await page.evaluate("(url) => window.open(url, '_blank')", url)
    popup = await popup_info.value
    await wait_until(lambda: popup.url == url, f"{label} URL")
    await popup.wait_for_load_state("load", timeout=10_000)
    assert_equal(await popup.text_content("main", timeout=5_000), expected_text, f"{label} text")
    assert_equal(popup.url, url, f"{label} loaded URL")
    return popup


async def _trigger_fetch_and_websocket(page: Any, owner: str, payload: str) -> dict[str, Any]:
    return await page.evaluate(
        """
        async ({ owner, payload }) => {
          const api = await fetch(`/api-continue?owner=${owner}`).then(response => response.json());
          const ws = await new Promise((resolve, reject) => {
            const url = new URL('/ws-echo', location.href);
            url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
            const socket = new WebSocket(url.href, 'smoke');
            const timer = setTimeout(() => {
              socket.close();
              reject(new Error(`websocket timed out at readyState=${socket.readyState}`));
            }, 5000);
            socket.onopen = () => socket.send(payload);
            socket.onmessage = event => {
              clearTimeout(timer);
              const data = event.data;
              socket.close(1000, 'done');
              resolve(data);
            };
            socket.onerror = () => {
              clearTimeout(timer);
              reject(new Error(`websocket error at readyState=${socket.readyState}`));
            };
          });
          return { api, ws };
        }
        """,
        {"owner": owner, "payload": payload},
    )


async def _fetch_popup_page_route(popup: Any, owner: str) -> dict[str, Any]:
    return await popup.evaluate(
        """
        async owner => {
          const response = await fetch(`/popup-page-route-api?owner=${owner}`);
          return {
            url: location.href,
            status: response.status,
            contentType: response.headers.get('content-type'),
            body: await response.json(),
          };
        }
        """,
        owner,
    )


async def _fetch_response_stage_body(page: Any, owner: str) -> str:
    return await page.evaluate(
        """
        async owner => {
          const response = await fetch(`/api-response-stage?popup=${owner}`);
          return await response.text();
        }
        """,
        owner,
    )


async def _fetch_response_stage_result(page: Any, owner: str) -> dict[str, Any]:
    return await page.evaluate(
        """
        async owner => {
          try {
            const response = await fetch(`/api-response-stage?popup=${owner}`);
            return {
              url: location.href,
              status: response.status,
              header: response.headers.get('x-smoke-fulfilled'),
              body: await response.text(),
            };
          } catch (error) {
            return {
              url: location.href,
              error: `${error?.name || 'Error'}:${error?.message || String(error)}`,
            };
          }
        }
        """,
        owner,
    )


async def _new_cdp_session(context: Any, page: Any, label: str) -> Any:
    try:
        return await asyncio.wait_for(context.new_cdp_session(page), timeout=5)
    except asyncio.TimeoutError as error:
        raise SmokeError(f"{label} new_cdp_session timed out") from error


async def _send_cdp(cdp: Any, method: str, params: dict[str, Any] | None = None) -> Any:
    return await asyncio.wait_for(cdp.send(method, params or {}), timeout=5)


async def _fulfill_response_stage_body(cdp: Any, request_id: str) -> None:
    await _send_cdp(
        cdp,
        "Fetch.fulfillRequest",
        {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                {"name": "content-type", "value": "text/plain; charset=utf-8"},
            ],
            "body": RESPONSE_STAGE_BODY_BASE64,
        },
    )


async def _continue_response_best_effort(cdp: Any, paused: dict[str, Any], label: str) -> None:
    try:
        await _send_cdp(
            cdp,
            "Fetch.continueResponse",
            {"requestId": _pause_request_id(paused, label)},
        )
    except Exception as error:
        _trace_multi_context(f"{label} cleanup continueResponse failed: {error!r}")


async def _close_io_stream_best_effort(cdp: Any, handle: str) -> None:
    try:
        await _send_cdp(cdp, "IO.close", {"handle": handle})
    except Exception as error:
        _trace_multi_context(f"cleanup IO.close failed for {handle}: {error!r}")


async def _close_context_best_effort(context: Any) -> None:
    task = asyncio.create_task(context.close())
    done, _pending = await asyncio.wait({task}, timeout=5)
    if done:
        await asyncio.gather(task, return_exceptions=True)
        return
    task.cancel()
    _trace_multi_context(
        "BrowserContext.close did not finish within 5s; leaving browser shutdown to runner"
    )


async def _unroute_best_effort(context: Any, url: str) -> None:
    try:
        await context.unroute(url)
    except Exception:
        pass


def _trace_multi_context(message: str) -> None:
    if os.environ.get("MOLI_SMOKE_TRACE_BG") == "1":
        print(f"[multi-context] {message}", file=sys.stderr, flush=True)


def _response_stage_pause_for(
    events: list[dict[str, Any]], fixture: str
) -> dict[str, Any] | None:
    return _response_stage_pause_for_url(events, f"{fixture}/api-response-stage")


def _response_stage_pause_for_url(
    events: list[dict[str, Any]], expected_url: str
) -> dict[str, Any] | None:
    return next(
        (
            event
            for event in events
            if event["method"] == "Fetch.requestPaused"
            and event["params"].get("request", {}).get("url")
            == expected_url
            and event["params"].get("resourceType") == "XHR"
            and event["params"].get("responseStatusCode") == 200
        ),
        None,
    )


def _pause_request_id(paused: dict[str, Any], label: str) -> str:
    request_id = paused["params"].get("requestId")
    if not isinstance(request_id, str) or not request_id:
        raise SmokeError(f"missing {label} response-stage requestId: {paused}")
    return request_id


def _pause_network_id(paused: dict[str, Any], label: str) -> str:
    network_id = paused["params"].get("networkId")
    if not isinstance(network_id, str) or not network_id:
        raise SmokeError(f"missing {label} response-stage networkId: {paused}")
    return network_id


def _stream_handle(result: dict[str, Any], label: str) -> str:
    handle = result.get("stream")
    if not isinstance(handle, str) or not handle:
        raise SmokeError(f"missing {label} Fetch.takeResponseBodyAsStream handle: {result}")
    return handle


def _network_sequence_complete(events: list[dict[str, Any]], request_id: str, fixture: str) -> bool:
    return _network_sequence_complete_for_url(events, request_id, f"{fixture}/api-response-stage")


def _network_sequence_complete_for_url(
    events: list[dict[str, Any]], request_id: str, expected_url: str
) -> bool:
    return (
        any(
            event["method"] == "Network.requestWillBeSent"
            and event["params"].get("requestId") == request_id
            and event["params"].get("request", {}).get("url") == expected_url
            for event in events
        )
        and any(
            event["method"] == "Network.responseReceived"
            and event["params"].get("requestId") == request_id
            and event["params"].get("response", {}).get("url") == expected_url
            for event in events
        )
        and _has_loading_finished(events, request_id)
    )


def _network_request_finished_for_url(events: list[dict[str, Any]], expected_url: str) -> bool:
    request = next(
        (
            event
            for event in events
            if event["method"] == "Network.requestWillBeSent"
            and event["params"].get("request", {}).get("url") == expected_url
        ),
        None,
    )
    request_id = request and request["params"].get("requestId")
    return bool(request_id and _has_loading_finished(events, request_id))


def _websocket_received_payload_length(
    events: list[dict[str, Any]], expected_url: str, payload_length: int
) -> bool:
    created = next(
        (
            event
            for event in events
            if event["method"] == "Network.webSocketCreated"
            and event["params"].get("url") == expected_url
        ),
        None,
    )
    request_id = created and created["params"].get("requestId")
    return bool(
        request_id
        and any(
            event["method"] == "Network.webSocketHandshakeResponseReceived"
            and event["params"].get("requestId") == request_id
            for event in events
        )
        and any(
            event["method"] == "Network.webSocketFrameReceived"
            and event["params"].get("requestId") == request_id
            and event["params"].get("response", {}).get("payloadLength") == payload_length
            for event in events
        )
    )


def _assert_no_cross_network_output(
    events: list[dict[str, Any]], *, forbidden_url: str, label: str
) -> None:
    leaked = next(
        (
            event
            for event in events
            if event.get("method") in {"Network.requestWillBeSent", "Network.responseReceived"}
            and (
                event["params"].get("request", {}).get("url") == forbidden_url
                or event["params"].get("response", {}).get("url") == forbidden_url
            )
        ),
        None,
    )
    if leaked is not None:
        raise SmokeError(f"{label} received another context's Network event: {leaked}")


def _assert_no_cross_websocket_payload(
    events: list[dict[str, Any]], *, forbidden_payload_length: int, label: str
) -> None:
    leaked = next(
        (
            event
            for event in events
            if event.get("method") == "Network.webSocketFrameReceived"
            and event["params"].get("response", {}).get("payloadLength")
            == forbidden_payload_length
        ),
        None,
    )
    if leaked is not None:
        raise SmokeError(f"{label} received another context's WebSocket frame event: {leaked}")


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


def _has_loading_finished(events: list[dict[str, Any]], request_id: str) -> bool:
    return any(
        event["method"] == "Network.loadingFinished"
        and event["params"].get("requestId") == request_id
        for event in events
    )


def _has_loading_failed(events: list[dict[str, Any]], request_id: str) -> bool:
    return any(
        event["method"] == "Network.loadingFailed"
        and event["params"].get("requestId") == request_id
        for event in events
    )
