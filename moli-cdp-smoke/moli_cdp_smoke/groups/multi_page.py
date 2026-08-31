from __future__ import annotations

import asyncio
import hashlib
from contextlib import suppress
from pathlib import Path
from typing import Any

from ..assertions import SmokeError, assert_equal, record, wait_until
from ..helpers import attach_cdp_event_collector, run_worker_command
from .multi_page_contracts import run_multi_page_contracts


async def run_multi_page_group(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _close_startup_target(browser)
    await _page_churn_and_session_reattach(browser, fixture, results)
    await _same_context_storage_and_history(browser, fixture, results)
    await _mixed_navigation_outcomes_and_recovery(browser, fixture, results)
    await _concurrent_route_outcomes(browser, fixture, results)
    await _inflight_navigation_close_and_peer_recovery(browser, fixture, results)
    await _target_local_session_events(browser, fixture, results)
    await _target_local_dialogs(browser, fixture, results)
    await _concurrent_downloads_and_peer(browser, fixture, results)
    await _popup_tree_survives_middle_close(browser, fixture, results)
    await _background_tasks_workers_and_screenshots(browser, fixture, results)
    await _context_teardown_with_pending_commands(browser, fixture, results)
    await run_multi_page_contracts(browser, fixture, results)


async def _page_churn_and_session_reattach(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    browser_cdp = await browser.new_browser_cdp_session()
    sessions: list[Any] = []
    try:
        pages = await asyncio.gather(*(context.new_page() for _ in range(6)))
        urls = [f"{fixture}/plain?multi-page-churn={index}" for index in range(6)]
        await asyncio.gather(
            *(
                page.goto(url, wait_until="load", timeout=10_000)
                for page, url in zip(pages, urls, strict=True)
            )
        )

        sessions = await asyncio.gather(
            *(context.new_cdp_session(page) for page in pages)
        )
        writes = await asyncio.gather(
            *(
                _send_cdp(
                    session,
                    "Runtime.evaluate",
                    {
                        "expression": (
                            f"globalThis.__moliMultiPageChurn = 'page-{index}'; "
                            "globalThis.__moliMultiPageChurn"
                        ),
                        "returnByValue": True,
                    },
                )
                for index, session in enumerate(sessions)
            )
        )
        assert_equal(
            [_runtime_value(write) for write in writes],
            [f"page-{index}" for index in range(6)],
            "six target-local Runtime writes",
        )

        for index in (1, 3, 5):
            await sessions[index].detach()
            sessions[index] = await context.new_cdp_session(pages[index])
        reads = await asyncio.gather(
            *(
                _send_cdp(
                    session,
                    "Runtime.evaluate",
                    {
                        "expression": "globalThis.__moliMultiPageChurn",
                        "returnByValue": True,
                    },
                )
                for session in sessions
            )
        )
        assert_equal(
            [_runtime_value(read) for read in reads],
            [f"page-{index}" for index in range(6)],
            "Runtime state after alternating session reattach",
        )

        retired_indexes = (1, 3, 4)
        await asyncio.gather(*(pages[index].close() for index in retired_indexes))
        survivors = [pages[index] for index in (0, 2, 5)]
        assert_equal(
            await asyncio.gather(
                *(page.evaluate("globalThis.__moliMultiPageChurn") for page in survivors)
            ),
            ["page-0", "page-2", "page-5"],
            "surviving page state after non-tail target close",
        )

        replacements = await asyncio.gather(*(context.new_page() for _ in range(3)))
        replacement_urls = [
            f"{fixture}/plain?multi-page-replacement={index}" for index in range(3)
        ]
        await asyncio.gather(
            *(
                page.goto(url, wait_until="load", timeout=10_000)
                for page, url in zip(replacements, replacement_urls, strict=True)
            )
        )
        expected_urls = set(urls[index] for index in (0, 2, 5)) | set(
            replacement_urls
        )
        retired_urls = {urls[index] for index in retired_indexes}

        async def target_registry_matches_live_pages() -> bool:
            infos = (await _send_cdp(browser_cdp, "Target.getTargets")).get(
                "targetInfos", []
            )
            page_urls = {
                info.get("url") for info in infos if info.get("type") == "page"
            }
            return expected_urls <= page_urls and not (retired_urls & page_urls)

        await wait_until(
            target_registry_matches_live_pages,
            "target registry after middle-page churn",
        )
        record(
            results,
            "multi_page_churn_and_session_reattach",
            {"createdPages": 9, "closedMiddlePages": len(retired_indexes)},
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        with suppress(Exception):
            await browser_cdp.detach()
        await _close_context(context)


async def _same_context_storage_and_history(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    context_a_closed = False
    try:
        page_a, page_a_peer, page_b = await asyncio.gather(
            context_a.new_page(),
            context_a.new_page(),
            context_b.new_page(),
        )
        await asyncio.gather(
            page_a.goto(f"{fixture}/plain?storage=a", wait_until="load"),
            page_a_peer.goto(f"{fixture}/plain?storage=a-peer", wait_until="load"),
            page_b.goto(f"{fixture}/plain?storage=b", wait_until="load"),
        )
        await page_a.evaluate(
            """() => {
              localStorage.clear();
              sessionStorage.clear();
              localStorage.setItem('multi-page-local', 'context-a');
              sessionStorage.setItem('multi-page-session', 'page-a');
              document.cookie = 'multiPageCookie=context-a; Path=/';
            }"""
        )
        same_context = await page_a_peer.evaluate(
            """() => ({
              local: localStorage.getItem('multi-page-local'),
              session: sessionStorage.getItem('multi-page-session'),
              cookie: document.cookie,
            })"""
        )
        assert_equal(same_context.get("local"), "context-a", "same-context localStorage")
        assert_equal(same_context.get("session"), None, "target-local sessionStorage")
        if "multiPageCookie=context-a" not in same_context.get("cookie", ""):
            raise SmokeError(f"same-context cookie was not shared: {same_context}")

        isolated_context = await page_b.evaluate(
            """() => ({
              local: localStorage.getItem('multi-page-local'),
              session: sessionStorage.getItem('multi-page-session'),
              cookie: document.cookie,
            })"""
        )
        assert_equal(
            isolated_context,
            {"local": None, "session": None, "cookie": ""},
            "cross-context storage isolation",
        )

        await asyncio.gather(
            page_a.goto(f"{fixture}/history-a?owner=a", wait_until="load"),
            page_a_peer.goto(f"{fixture}/history-b?owner=a-peer", wait_until="load"),
        )
        await asyncio.gather(
            page_a.goto(f"{fixture}/history-b?owner=a", wait_until="load"),
            page_a_peer.goto(f"{fixture}/history-a?owner=a-peer", wait_until="load"),
        )
        await asyncio.gather(
            page_a.go_back(wait_until="commit", timeout=10_000),
            page_a_peer.go_back(wait_until="commit", timeout=10_000),
        )
        assert_equal(
            await asyncio.gather(
                page_a.text_content("main"), page_a_peer.text_content("main")
            ),
            ["history a", "history b"],
            "independent same-context target history",
        )

        await asyncio.wait_for(context_a.close(), timeout=5)
        context_a_closed = True
        assert_equal(
            await page_b.evaluate("() => document.querySelector('main')?.textContent"),
            "plain ok",
            "other context after peer context disposal",
        )
        await page_b.goto(f"{fixture}/plain?storage=b-final", wait_until="load")
        record(results, "multi_page_storage_history_and_context_disposal")
    finally:
        if not context_a_closed:
            await _close_context(context_a)
        await _close_context(context_b)


async def _mixed_navigation_outcomes_and_recovery(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        pages = await asyncio.gather(*(context.new_page() for _ in range(5)))
        sessions = await asyncio.gather(
            *(context.new_cdp_session(page) for page in pages)
        )
        responses = await asyncio.gather(
            pages[0].goto(f"{fixture}/redirect-start", wait_until="load"),
            pages[1].goto(f"{fixture}/navigation-http-error", wait_until="load"),
            pages[2].goto(f"{fixture}/missing-multi-page", wait_until="load"),
            pages[3].goto(
                "data:text/html,<main>multi-page-data</main>", wait_until="load"
            ),
            pages[4].goto(f"{fixture}/iframe", wait_until="load"),
        )
        assert_equal(
            [
                responses[0].status if responses[0] else None,
                responses[1].status if responses[1] else None,
                responses[2].status if responses[2] else None,
                responses[3].status if responses[3] else None,
                responses[4].status if responses[4] else None,
            ],
            [200, 502, 404, None, 200],
            "parallel mixed navigation statuses",
        )
        assert_equal(
            await asyncio.gather(
                pages[0].text_content("main"),
                pages[1].text_content("main"),
                pages[2].text_content("body"),
                pages[3].text_content("main"),
                pages[4].text_content("main"),
            ),
            [
                "redirect final",
                "gateway error",
                "not found: /missing-multi-page",
                "multi-page-data",
                "parent",
            ],
            "parallel mixed navigation Documents",
        )

        frame_trees = await asyncio.gather(
            *(_send_cdp(session, "Page.getFrameTree") for session in sessions)
        )
        frame_ids = [_collect_frame_ids(tree.get("frameTree", {})) for tree in frame_trees]
        for index, ids in enumerate(frame_ids):
            for other_index, other_ids in enumerate(frame_ids):
                if index != other_index and ids & other_ids:
                    raise SmokeError(
                        "frame ids crossed target ownership: "
                        f"target={index}, other={other_index}, ids={ids & other_ids}"
                    )
        assert_equal(len(frame_ids[4]), 2, "iframe target frame count")

        transport_failure: BaseException | None = None
        document_before_transport_failure = pages[2].url
        try:
            await pages[2].goto(
                "http://127.0.0.1:1/multi-page-transport-failure",
                wait_until="load",
                timeout=5_000,
            )
        except BaseException as error:
            transport_failure = error
        if transport_failure is None:
            raise SmokeError("transport-failure navigation unexpectedly succeeded")
        await wait_until(
            lambda: pages[2].url != document_before_transport_failure,
            "transport-failure error Document replacement",
        )
        await pages[2].wait_for_load_state("load", timeout=5_000)
        assert_equal(
            await pages[0].text_content("main"),
            "redirect final",
            "peer target after transport failure",
        )

        await asyncio.gather(
            *(
                page.goto(
                    f"{fixture}/plain?mixed-recovery={index}",
                    wait_until="load",
                    timeout=10_000,
                )
                for index, page in enumerate(pages)
            )
        )
        assert_equal(
            await asyncio.gather(*(page.text_content("main") for page in pages)),
            ["plain ok"] * len(pages),
            "all mixed-navigation targets recover",
        )
        record(
            results,
            "multi_page_mixed_navigation_outcomes_and_recovery",
            {"pages": len(pages), "iframeFrames": len(frame_ids[4])},
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _concurrent_route_outcomes(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()

    async def route_a(route: Any) -> None:
        if route.request.url.endswith("/abort"):
            await route.abort("failed")
            return
        marker = route.request.url.rsplit("/", 1)[-1]
        await route.fulfill(
            status=200,
            content_type="text/html",
            body=f"<!doctype html><main>context-a:{marker}</main>",
        )

    async def route_b(route: Any) -> None:
        marker = route.request.url.rsplit("/", 1)[-1]
        await route.fulfill(
            status=201,
            content_type="text/html",
            body=f"<!doctype html><main>context-b:{marker}</main>",
        )

    try:
        await context_a.route("**/multi-page-route/**", route_a)
        await context_b.route("**/multi-page-route/**", route_b)
        page_a, page_a_abort, page_b, page_b_peer = await asyncio.gather(
            context_a.new_page(),
            context_a.new_page(),
            context_b.new_page(),
            context_b.new_page(),
        )
        outcomes = await asyncio.gather(
            page_a.goto(f"{fixture}/multi-page-route/fulfilled-a", wait_until="load"),
            page_a_abort.goto(f"{fixture}/multi-page-route/abort", wait_until="load"),
            page_b.goto(f"{fixture}/multi-page-route/fulfilled-b", wait_until="load"),
            page_b_peer.goto(
                f"{fixture}/multi-page-route/fulfilled-b-peer", wait_until="load"
            ),
            return_exceptions=True,
        )
        if not isinstance(outcomes[1], BaseException):
            raise SmokeError(f"aborted target navigation did not fail: {outcomes[1]!r}")
        assert_equal(
            [
                outcomes[0].status if not isinstance(outcomes[0], BaseException) else None,
                outcomes[2].status if not isinstance(outcomes[2], BaseException) else None,
                outcomes[3].status if not isinstance(outcomes[3], BaseException) else None,
            ],
            [200, 201, 201],
            "concurrent fulfilled route statuses",
        )
        assert_equal(
            await asyncio.gather(
                page_a.text_content("main"),
                page_b.text_content("main"),
                page_b_peer.text_content("main"),
            ),
            [
                "context-a:fulfilled-a",
                "context-b:fulfilled-b",
                "context-b:fulfilled-b-peer",
            ],
            "concurrent route target ownership",
        )
        await page_a_abort.goto(
            f"{fixture}/plain?route-abort-recovery", wait_until="load"
        )
        assert_equal(
            await page_a_abort.text_content("main"),
            "plain ok",
            "aborted target recovery",
        )
        record(results, "multi_page_concurrent_fulfill_abort_and_recovery")
    finally:
        with suppress(Exception):
            await context_a.unroute("**/multi-page-route/**")
        with suppress(Exception):
            await context_b.unroute("**/multi-page-route/**")
        await _close_context(context_a)
        await _close_context(context_b)


async def _inflight_navigation_close_and_peer_recovery(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    route_started = asyncio.Event()
    release_route = asyncio.Event()

    async def hold_navigation(route: Any) -> None:
        route_started.set()
        await release_route.wait()
        with suppress(Exception):
            await route.abort("aborted")

    victim_navigation: asyncio.Task[Any] | None = None
    victim_close: asyncio.Task[Any] | None = None
    try:
        victim, peer = await asyncio.gather(context.new_page(), context.new_page())
        await peer.goto(f"{fixture}/plain?inflight-peer=initial", wait_until="load")
        await victim.route("**/multi-page-inflight/blocked", hold_navigation)
        victim_navigation = asyncio.create_task(
            victim.goto(
                f"{fixture}/multi-page-inflight/blocked",
                wait_until="load",
                timeout=10_000,
            )
        )
        await asyncio.wait_for(route_started.wait(), timeout=5)

        await peer.goto(f"{fixture}/plain?inflight-peer=while-blocked", wait_until="load")
        assert_equal(
            await peer.evaluate("() => location.search"),
            "?inflight-peer=while-blocked",
            "peer navigation while another target request is paused",
        )

        victim_close = asyncio.create_task(victim.close())
        await asyncio.sleep(0)
        release_route.set()
        close_result, navigation_result = await asyncio.wait_for(
            asyncio.gather(
                victim_close,
                victim_navigation,
                return_exceptions=True,
            ),
            timeout=5,
        )
        if isinstance(close_result, BaseException):
            raise SmokeError(
                f"closing a target with a paused navigation failed: {close_result!r}"
            )
        if not isinstance(navigation_result, BaseException):
            raise SmokeError(
                "closing a target did not reject its paused navigation: "
                f"{navigation_result!r}"
            )

        replacement = await context.new_page()
        await asyncio.gather(
            peer.goto(f"{fixture}/plain?inflight-peer=final", wait_until="load"),
            replacement.goto(
                f"{fixture}/plain?inflight-replacement", wait_until="load"
            ),
        )
        assert_equal(
            await asyncio.gather(
                peer.evaluate("() => location.search"),
                replacement.evaluate("() => location.search"),
            ),
            ["?inflight-peer=final", "?inflight-replacement"],
            "peer and replacement after paused target close",
        )
        record(results, "multi_page_inflight_navigation_close_and_peer_recovery")
    finally:
        release_route.set()
        for task in (victim_navigation, victim_close):
            if task is not None and not task.done():
                task.cancel()
        await _close_context(context)


async def _target_local_session_events(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        pages = await asyncio.gather(*(context.new_page() for _ in range(3)))
        await asyncio.gather(
            *(
                page.goto(
                    f"{fixture}/plain?session-events={index}", wait_until="load"
                )
                for index, page in enumerate(pages)
            )
        )
        primaries = await asyncio.gather(
            *(context.new_cdp_session(page) for page in pages)
        )
        auxiliaries = await asyncio.gather(
            *(context.new_cdp_session(page) for page in pages)
        )
        sessions = [*primaries, *auxiliaries]
        collectors = [
            attach_cdp_event_collector(
                session,
                ["Runtime.consoleAPICalled", "Network.requestWillBeSent"],
            )
            for session in sessions
        ]
        await asyncio.gather(
            *(_send_cdp(session, "Runtime.enable") for session in sessions),
            *(_send_cdp(session, "Network.enable") for session in sessions),
        )

        network_tokens = [f"session-network={index}" for index in range(3)]
        await asyncio.gather(
            *(
                page.goto(
                    f"{fixture}/plain?{network_tokens[index]}",
                    wait_until="load",
                )
                for index, page in enumerate(pages)
            )
        )
        for index, token in enumerate(network_tokens):
            for collector_index in (index, index + len(pages)):
                await wait_until(
                    lambda collector_index=collector_index, token=token: any(
                        token in url
                        for url in _document_request_urls(collectors[collector_index])
                    ),
                    f"Document request event for target {index} session {collector_index}",
                )
                peer_tokens = set(network_tokens) - {token}
                leaked = {
                    peer_token
                    for peer_token in peer_tokens
                    if any(
                        peer_token in url
                        for url in _document_request_urls(collectors[collector_index])
                    )
                }
                if leaked:
                    raise SmokeError(
                        f"target {index} Network sessions received peer events: {leaked}"
                    )

        tokens = [f"multi-page-console-{index}" for index in range(3)]
        await asyncio.gather(
            *(
                _send_cdp(
                    primaries[index],
                    "Runtime.evaluate",
                    {
                        "expression": f"console.log('{token}')",
                        "returnByValue": True,
                    },
                )
                for index, token in enumerate(tokens)
            )
        )

        for index, token in enumerate(tokens):
            await wait_until(
                lambda index=index, token=token: token
                in _console_values(collectors[index]),
                f"primary console event for target {index}",
            )
            await wait_until(
                lambda index=index, token=token: token
                in _console_values(collectors[index + len(pages)]),
                f"auxiliary console event for target {index}",
            )
        for index, token in enumerate(tokens):
            other_tokens = set(tokens) - {token}
            for collector_index in (index, index + len(pages)):
                leaked = other_tokens & set(_console_values(collectors[collector_index]))
                if leaked:
                    raise SmokeError(
                        f"target {index} console sessions received peer events: {leaked}"
                    )

        await primaries[1].detach()
        sessions.remove(primaries[1])
        surviving = await _send_cdp(
            auxiliaries[1],
            "Runtime.evaluate",
            {"expression": "21 * 2", "returnByValue": True},
        )
        assert_equal(_runtime_value(surviving), 42, "auxiliary after primary detach")
        replacement = await context.new_cdp_session(pages[1])
        sessions.append(replacement)
        replacement_read = await _send_cdp(
            replacement,
            "Runtime.evaluate",
            {"expression": "location.search", "returnByValue": True},
        )
        assert_equal(
            _runtime_value(replacement_read),
            "?session-network=1",
            "reattached target session route",
        )
        record(
            results,
            "multi_page_target_local_multi_session_events",
            {"pages": len(pages), "sessions": 6},
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _popup_tree_survives_middle_close(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    try:
        root, peer = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            root.goto(f"{fixture}/plain?popup-tree=root", wait_until="load"),
            peer.goto(f"{fixture}/plain?popup-tree=peer", wait_until="load"),
        )
        popup = await _open_popup(
            root,
            f"{fixture}/plain?popup-tree=middle",
            "multi-page-middle",
        )
        child = await _open_popup(
            popup,
            f"{fixture}/plain?popup-tree=child",
            "multi-page-child",
        )
        sibling = await _open_popup(
            root,
            f"{fixture}/plain?popup-tree=sibling",
            "multi-page-sibling",
        )
        await asyncio.gather(
            root.evaluate("globalThis.__popupTreeToken = 'root'"),
            popup.evaluate("globalThis.__popupTreeToken = 'middle'"),
            child.evaluate("globalThis.__popupTreeToken = 'child'"),
            sibling.evaluate("globalThis.__popupTreeToken = 'sibling'"),
            peer.evaluate("globalThis.__popupTreeToken = 'peer'"),
        )

        await popup.close()
        assert_equal(
            await asyncio.gather(
                root.evaluate("globalThis.__popupTreeToken"),
                child.evaluate("globalThis.__popupTreeToken"),
                sibling.evaluate("globalThis.__popupTreeToken"),
                peer.evaluate("globalThis.__popupTreeToken"),
            ),
            ["root", "child", "sibling", "peer"],
            "popup-tree survivors after middle target close",
        )
        assert_equal(
            await child.evaluate("() => window.opener?.closed ?? null"),
            None,
            "child clears its retired middle opener",
        )

        named = await _open_popup(
            root,
            f"{fixture}/plain?popup-tree=named-first",
            "multi-page-named",
        )
        page_count = len(context.pages)
        await root.evaluate(
            "url => window.open(url, 'multi-page-named')",
            f"{fixture}/plain?popup-tree=named-second",
        )
        await named.wait_for_url("**/plain?popup-tree=named-second", timeout=5_000)
        assert_equal(len(context.pages), page_count, "named popup target reuse")
        assert_equal(
            await sibling.evaluate("globalThis.__popupTreeToken"),
            "sibling",
            "sibling popup after named-target reuse",
        )
        record(
            results,
            "multi_page_popup_tree_middle_close_and_named_reuse",
            {"survivingPages": len(context.pages)},
        )
    finally:
        await _close_context(context)


async def _target_local_dialogs(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    try:
        prompt_page, confirm_page, peer = await asyncio.gather(
            context.new_page(),
            context.new_page(),
            context.new_page(),
        )
        await asyncio.gather(
            prompt_page.goto(f"{fixture}/dialog?target=prompt", wait_until="load"),
            confirm_page.goto(f"{fixture}/dialog?target=confirm", wait_until="load"),
            peer.goto(f"{fixture}/plain?target=dialog-peer", wait_until="load"),
        )

        async with (
            prompt_page.expect_event("dialog", timeout=5_000) as prompt_info,
            confirm_page.expect_event("dialog", timeout=5_000) as confirm_info,
        ):
            prompt_task = asyncio.create_task(
                prompt_page.evaluate(
                    "() => prompt('multi-page prompt', 'target-local default')"
                )
            )
            confirm_task = asyncio.create_task(
                confirm_page.evaluate("() => confirm('multi-page confirm')")
            )
        prompt, confirm = await asyncio.gather(prompt_info.value, confirm_info.value)
        assert_equal(prompt.type, "prompt", "target-local prompt type")
        assert_equal(prompt.message, "multi-page prompt", "target-local prompt message")
        assert_equal(
            prompt.default_value,
            "target-local default",
            "target-local prompt default",
        )
        assert_equal(confirm.type, "confirm", "target-local confirm type")
        assert_equal(confirm.message, "multi-page confirm", "target-local confirm message")
        assert_equal(
            await peer.evaluate("() => location.search"),
            "?target=dialog-peer",
            "peer target while two dialogs are open",
        )

        await confirm.accept()
        assert_equal(await confirm_task, True, "target-local confirm accept result")
        assert_equal(
            prompt_page.url,
            f"{fixture}/dialog?target=prompt",
            "accepting peer dialog does not disturb prompt owner",
        )
        await prompt.dismiss()
        assert_equal(await prompt_task, None, "target-local prompt dismiss result")
        assert_equal(
            await asyncio.gather(
                prompt_page.text_content("#alert"),
                confirm_page.text_content("#alert"),
                peer.text_content("main"),
            ),
            ["alert", "alert", "plain ok"],
            "all targets after reverse-order dialog handling",
        )
        record(results, "multi_page_target_local_concurrent_dialogs")
    finally:
        await _close_context(context)


async def _concurrent_downloads_and_peer(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    try:
        download_a, download_b, peer = await asyncio.gather(
            context.new_page(),
            context.new_page(),
            context.new_page(),
        )
        await asyncio.gather(
            download_a.goto(f"{fixture}/download-page", wait_until="load"),
            download_b.goto(f"{fixture}/download-page", wait_until="load"),
            peer.goto(f"{fixture}/plain?target=download-peer", wait_until="load"),
        )

        async with (
            download_a.expect_download(timeout=10_000) as first_info,
            download_b.expect_download(timeout=10_000) as second_info,
        ):
            await asyncio.gather(
                download_a.locator("#download").evaluate("anchor => anchor.click()"),
                download_b.locator("#download").evaluate("anchor => anchor.click()"),
            )
        first, second = await asyncio.gather(first_info.value, second_info.value)
        assert_equal(
            [first.suggested_filename, second.suggested_filename],
            ["smoke-download.txt", "smoke-download.txt"],
            "concurrent target-local download filenames",
        )
        first_path, second_path = await asyncio.gather(first.path(), second.path())
        assert_equal(
            [
                Path(first_path).read_text(encoding="utf-8"),
                Path(second_path).read_text(encoding="utf-8"),
            ],
            ["download contents", "download contents"],
            "concurrent target-local download artifacts",
        )

        async with download_a.expect_download(timeout=10_000) as slow_info:
            await download_a.locator("#slow-download").evaluate(
                "anchor => anchor.click()"
            )
        slow = await slow_info.value
        assert_equal(
            await peer.evaluate("() => location.search"),
            "?target=download-peer",
            "peer target during slow download",
        )
        await slow.cancel()
        assert_equal(await slow.failure(), "canceled", "target-local download cancel")
        assert_equal(
            await download_b.text_content("#download"),
            "download",
            "peer download page after other target cancellation",
        )
        record(results, "multi_page_concurrent_downloads_and_peer_cancel")
    finally:
        await _close_context(context)


async def _background_tasks_workers_and_screenshots(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context(viewport={"width": 320, "height": 200})
    try:
        pages = await asyncio.gather(*(context.new_page() for _ in range(4)))
        await asyncio.gather(
            *(
                page.goto(
                    f"{fixture}/plain?background-page={index}", wait_until="load"
                )
                for index, page in enumerate(pages)
            )
        )
        colors = ("#ff0000", "#00ff00", "#0000ff", "#ffff00")
        await asyncio.gather(
            *(
                page.set_content(
                    "<!doctype html>"
                    f"<style>html,body{{margin:0;background:{color}}}</style>"
                    f"<main>background-page-{index}</main>",
                    wait_until="load",
                )
                for index, (page, color) in enumerate(
                    zip(pages, colors, strict=True)
                )
            )
        )
        screenshots = await asyncio.gather(*(page.screenshot() for page in pages))
        if any(not screenshot.startswith(b"\x89PNG\r\n\x1a\n") for screenshot in screenshots):
            raise SmokeError("concurrent page screenshot returned a non-PNG payload")
        screenshot_hashes = {hashlib.sha256(screenshot).hexdigest() for screenshot in screenshots}
        assert_equal(len(screenshot_hashes), 4, "target-local concurrent screenshots")

        worker_results = await asyncio.gather(
            *(
                run_worker_command(page, {"page": index})
                for index, page in enumerate(pages[:3])
            )
        )
        assert_equal(
            [result.get("echoed", {}).get("page") for result in worker_results],
            [0, 1, 2],
            "target-local dedicated workers",
        )

        timer_tasks = [
            asyncio.create_task(
                page.evaluate(
                    """token => new Promise(resolve => {
                      setTimeout(() => resolve(`${token}:${document.querySelector('main').textContent}`), 120);
                    })""",
                    f"timer-{index}",
                )
            )
            for index, page in enumerate(pages)
        ]
        for page in reversed(pages):
            await page.bring_to_front()
        assert_equal(
            await asyncio.gather(*timer_tasks),
            [f"timer-{index}:background-page-{index}" for index in range(4)],
            "background timers across foreground activation churn",
        )

        pending = asyncio.create_task(
            pages[2].evaluate("() => new Promise(resolve => setTimeout(resolve, 10_000))")
        )
        await asyncio.sleep(0.05)
        await pages[2].close()
        pending_result = (await asyncio.gather(pending, return_exceptions=True))[0]
        if not isinstance(pending_result, BaseException):
            raise SmokeError("closing a page did not reject its pending Runtime evaluation")
        assert_equal(
            await asyncio.gather(
                pages[0].text_content("main"),
                pages[1].text_content("main"),
                pages[3].text_content("main"),
            ),
            ["background-page-0", "background-page-1", "background-page-3"],
            "peer pages after pending-evaluate target close",
        )
        record(
            results,
            "multi_page_background_timer_worker_screenshot_and_close",
            {"screenshots": 4, "workers": 3},
        )
    finally:
        await _close_context(context)


async def _context_teardown_with_pending_commands(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    context_a_closed = False
    try:
        page_a, page_a_peer, survivor = await asyncio.gather(
            context_a.new_page(),
            context_a.new_page(),
            context_b.new_page(),
        )
        await asyncio.gather(
            page_a.goto(f"{fixture}/plain?teardown=a", wait_until="load"),
            page_a_peer.goto(f"{fixture}/plain?teardown=a-peer", wait_until="load"),
            survivor.goto(f"{fixture}/plain?teardown=survivor", wait_until="load"),
        )
        pending = [
            asyncio.create_task(
                page.evaluate("() => new Promise(() => {})")
            )
            for page in (page_a, page_a_peer)
        ]
        await asyncio.sleep(0.05)
        await asyncio.wait_for(context_a.close(), timeout=5)
        context_a_closed = True
        pending_results = await asyncio.wait_for(
            asyncio.gather(*pending, return_exceptions=True),
            timeout=5,
        )
        if not all(isinstance(result, BaseException) for result in pending_results):
            raise SmokeError(
                f"disposed context did not reject every pending command: {pending_results!r}"
            )
        assert_equal(
            await survivor.evaluate("() => location.search"),
            "?teardown=survivor",
            "survivor target after pending-command context disposal",
        )
        replacement = await context_b.new_page()
        await replacement.goto(
            f"{fixture}/plain?teardown=replacement", wait_until="load"
        )
        assert_equal(
            await asyncio.gather(
                survivor.text_content("main"), replacement.text_content("main")
            ),
            ["plain ok", "plain ok"],
            "new and existing pages after peer context disposal",
        )
        record(
            results,
            "multi_page_context_teardown_with_pending_commands",
            {"pendingCommands": len(pending)},
        )
    finally:
        if not context_a_closed:
            await _close_context(context_a)
        await _close_context(context_b)


async def _open_popup(page: Any, url: str, name: str) -> Any:
    async with page.expect_popup(timeout=5_000) as popup_info:
        opened = await page.evaluate(
            "({url, name}) => Boolean(window.open(url, name))",
            {"url": url, "name": name},
        )
    assert_equal(opened, True, f"window.open returned a WindowProxy for {name}")
    popup = await popup_info.value
    await popup.wait_for_load_state("load", timeout=10_000)
    return popup


async def _close_startup_target(browser: Any) -> None:
    session = await browser.new_browser_cdp_session()
    try:
        targets = await _send_cdp(session, "Target.getTargets")
        startup = [
            info
            for info in targets.get("targetInfos", [])
            if info.get("type") == "page" and info.get("targetId") == "moli-default"
        ]
        if not startup:
            startup = [
                info
                for info in targets.get("targetInfos", [])
                if info.get("type") == "page" and info.get("url") == "about:blank"
            ]
        if len(startup) != 1:
            raise SmokeError(f"expected one startup Page target, got: {startup!r}")
        startup_target_id = startup[0].get("targetId")
        if not isinstance(startup_target_id, str) or not startup_target_id:
            raise SmokeError(f"startup Page target has no targetId: {startup[0]!r}")
        result = await _send_cdp(
            session,
            "Target.closeTarget",
            {"targetId": startup_target_id},
        )
        assert_equal(result.get("success"), True, "close startup Page target")

        async def startup_is_absent() -> bool:
            current = await _send_cdp(session, "Target.getTargets")
            return all(
                info.get("targetId") != startup_target_id
                for info in current.get("targetInfos", [])
            )

        await wait_until(startup_is_absent, "startup Page target retirement")
    finally:
        await session.detach()


async def _send_cdp(
    session: Any,
    method: str,
    params: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return await asyncio.wait_for(session.send(method, params or {}), timeout=5)


async def _close_context(context: Any) -> None:
    try:
        await asyncio.wait_for(context.close(), timeout=5)
    except Exception as error:
        raise SmokeError(f"BrowserContext.close failed: {type(error).__name__}: {error}") from error


def _runtime_value(response: dict[str, Any]) -> Any:
    return response.get("result", {}).get("value")


def _collect_frame_ids(frame_tree: dict[str, Any]) -> set[str]:
    ids: set[str] = set()
    frame_id = frame_tree.get("frame", {}).get("id")
    if isinstance(frame_id, str):
        ids.add(frame_id)
    for child in frame_tree.get("childFrames", []):
        ids.update(_collect_frame_ids(child))
    return ids


def _console_values(events: list[dict[str, Any]]) -> list[Any]:
    values: list[Any] = []
    for event in events:
        for argument in event.get("params", {}).get("args", []):
            if "value" in argument:
                values.append(argument["value"])
    return values


def _document_request_urls(events: list[dict[str, Any]]) -> list[str]:
    return [
        event.get("params", {}).get("request", {}).get("url", "")
        for event in events
        if event.get("method") == "Network.requestWillBeSent"
        and event.get("params", {}).get("type") == "Document"
    ]
