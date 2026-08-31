from __future__ import annotations

import asyncio
from contextlib import suppress
from typing import Any, Awaitable

from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..helpers import attach_cdp_event_collector
from ..progress import await_with_progress


async def run_multi_page_chromium_contracts(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    """Run executable Chromium contracts that stress cross-Page ownership."""

    contracts = (
        _debugger_pause_is_target_local,
        _dom_storage_namespaces_route_across_targets,
        _navigation_history_entries_are_target_local,
        _blocked_urls_aggregate_across_target_sessions,
        _cache_disabled_aggregates_without_crossing_targets,
    )
    for contract in contracts:
        await await_with_progress(
            f"multi-page/{contract.__name__.removeprefix('_')}",
            contract(browser, fixture, results),
            timeout_seconds=20,
        )


async def _debugger_pause_is_target_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    paused_evaluation: asyncio.Task[Any] | None = None
    try:
        owner, peer = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            owner.goto(f"{fixture}/plain?multi-page-debugger=owner", wait_until="load"),
            peer.goto(f"{fixture}/plain?multi-page-debugger=peer", wait_until="load"),
        )
        owner_session, peer_session = await asyncio.gather(
            context.new_cdp_session(owner),
            context.new_cdp_session(peer),
        )
        sessions = [owner_session, peer_session]
        owner_events = attach_cdp_event_collector(
            owner_session,
            ["Debugger.paused", "Debugger.resumed"],
        )
        peer_events = attach_cdp_event_collector(
            peer_session,
            ["Debugger.paused", "Debugger.resumed"],
        )
        await asyncio.gather(
            owner_session.send("Debugger.enable"),
            peer_session.send("Debugger.enable"),
        )

        paused_evaluation = asyncio.create_task(
            owner_session.send(
                "Runtime.evaluate",
                {
                    "expression": "debugger; globalThis.__multiPageAfterPause = 21 * 2",
                    "returnByValue": True,
                },
            )
        )
        await wait_until(
            lambda: _event_count(owner_events, "Debugger.paused") == 1,
            "Debugger.paused on the owning Page",
            timeout_ms=5_000,
        )

        peer_result = await asyncio.wait_for(
            peer_session.send(
                "Runtime.evaluate",
                {"expression": "6 * 7", "returnByValue": True},
            ),
            timeout=5,
        )
        assert_equal(
            _runtime_value(peer_result),
            42,
            "peer Runtime remains live while another Page is paused",
        )
        assert_equal(
            _event_count(peer_events, "Debugger.paused"),
            0,
            "Debugger pause event stays on the owning Page",
        )

        await owner_session.send("Debugger.resume")
        owner_result = await asyncio.wait_for(paused_evaluation, timeout=5)
        paused_evaluation = None
        assert_equal(
            _runtime_value(owner_result),
            42,
            "owning Runtime evaluation resumes",
        )
        await wait_until(
            lambda: _event_count(owner_events, "Debugger.resumed") == 1,
            "Debugger.resumed on the owning Page",
        )
        assert_equal(
            _event_count(peer_events, "Debugger.resumed"),
            0,
            "Debugger resumed event stays on the owning Page",
        )

        record_contract(
            results,
            "multi_page_debugger_pause_is_target_local",
            contract=(
                "A Debugger pause suspends only its Page target; a peer target continues "
                "Runtime work and receives neither paused nor resumed events."
            ),
            source="Debian Chromium 145.0.7632.116 executable CDP oracle",
            commands=[
                "Debugger.enable x2",
                "Runtime.evaluate x2",
                "Debugger.resume",
            ],
            observed={
                "ownerPaused": 1,
                "peerPaused": 0,
                "peerValueWhilePaused": 42,
                "ownerValueAfterResume": 42,
            },
        )
    finally:
        if sessions:
            with suppress(Exception):
                await asyncio.wait_for(sessions[0].send("Debugger.resume"), timeout=1)
        if paused_evaluation is not None and not paused_evaluation.done():
            paused_evaluation.cancel()
            with suppress(asyncio.CancelledError, Exception):
                await paused_evaluation
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _dom_storage_namespaces_route_across_targets(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        owner, peer = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            owner.goto(f"{fixture}/plain?multi-page-storage=owner", wait_until="load"),
            peer.goto(f"{fixture}/plain?multi-page-storage=peer", wait_until="load"),
        )
        await asyncio.gather(
            owner.evaluate("localStorage.clear(); sessionStorage.clear()"),
            peer.evaluate("sessionStorage.clear()"),
        )
        owner_session, peer_session = await asyncio.gather(
            context.new_cdp_session(owner),
            context.new_cdp_session(peer),
        )
        sessions = [owner_session, peer_session]
        methods = [
            "DOMStorage.domStorageItemAdded",
            "DOMStorage.domStorageItemUpdated",
            "DOMStorage.domStorageItemRemoved",
            "DOMStorage.domStorageItemsCleared",
        ]
        owner_events = attach_cdp_event_collector(owner_session, methods)
        peer_events = attach_cdp_event_collector(peer_session, methods)
        await asyncio.gather(
            owner_session.send("DOMStorage.enable"),
            peer_session.send("DOMStorage.enable"),
        )

        await owner.evaluate(
            """() => {
              localStorage.setItem('multi-page-local-owner', 'owner-local');
              sessionStorage.setItem('multi-page-session-owner', 'owner-session');
            }"""
        )
        await asyncio.gather(
            wait_until(
                lambda: _has_storage_event(
                    owner_events,
                    "multi-page-local-owner",
                    is_local=True,
                )
                and _has_storage_event(
                    owner_events,
                    "multi-page-session-owner",
                    is_local=False,
                ),
                "owner localStorage and sessionStorage events",
            ),
            wait_until(
                lambda: _has_storage_event(
                    peer_events,
                    "multi-page-local-owner",
                    is_local=True,
                ),
                "peer localStorage event",
            ),
        )

        peer_values = await peer.evaluate(
            """() => ({
              local: localStorage.getItem('multi-page-local-owner'),
              session: sessionStorage.getItem('multi-page-session-owner'),
            })"""
        )
        assert_equal(
            peer_values,
            {"local": "owner-local", "session": None},
            "localStorage shares an origin namespace while sessionStorage stays target-local",
        )
        assert_equal(
            _storage_event_count(
                peer_events,
                "multi-page-session-owner",
                is_local=False,
            ),
            0,
            "peer receives no sessionStorage mutation event",
        )

        await owner.close()
        assert_equal(
            await peer.evaluate("localStorage.getItem('multi-page-local-owner')"),
            "owner-local",
            "closing the mutation source preserves the shared localStorage namespace",
        )
        assert_equal(
            await peer.evaluate("sessionStorage.getItem('multi-page-session-owner')"),
            None,
            "closing the mutation source does not expose its sessionStorage namespace",
        )

        record_contract(
            results,
            "multi_page_dom_storage_namespace_event_routing",
            contract=(
                "Enabled DOMStorage agents on same-origin Pages both observe localStorage "
                "mutations, while sessionStorage values and events remain target-local; "
                "closing the source does not erase the shared local namespace."
            ),
            source="Debian Chromium 145.0.7632.116 executable CDP oracle",
            commands=["DOMStorage.enable x2", "Runtime.evaluate", "Target.closeTarget"],
            observed={
                "ownerLocalEvents": _storage_event_count(
                    owner_events,
                    "multi-page-local-owner",
                    is_local=True,
                ),
                "ownerSessionEvents": _storage_event_count(
                    owner_events,
                    "multi-page-session-owner",
                    is_local=False,
                ),
                "peerLocalEvents": _storage_event_count(
                    peer_events,
                    "multi-page-local-owner",
                    is_local=True,
                ),
                "peerSessionEvents": 0,
                "peerValues": peer_values,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _navigation_history_entries_are_target_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        owner, peer = await asyncio.gather(context.new_page(), context.new_page())
        for index in range(4):
            await owner.goto(
                f"{fixture}/plain?multi-page-history-owner={index}",
                wait_until="load",
            )
        for index in range(2):
            await peer.goto(
                f"{fixture}/plain?multi-page-history-peer={index}",
                wait_until="load",
            )
        owner_session, peer_session = await asyncio.gather(
            context.new_cdp_session(owner),
            context.new_cdp_session(peer),
        )
        sessions = [owner_session, peer_session]

        owner_history, peer_history = await asyncio.gather(
            owner_session.send("Page.getNavigationHistory"),
            peer_session.send("Page.getNavigationHistory"),
        )
        owner_entries = owner_history.get("entries", [])
        peer_entries = peer_history.get("entries", [])
        owner_ids = [entry.get("id") for entry in owner_entries]
        peer_ids = {entry.get("id") for entry in peer_entries}
        foreign_id = next(
            (entry_id for entry_id in owner_ids if entry_id not in peer_ids),
            None,
        )
        if not isinstance(foreign_id, int):
            raise SmokeError(
                "owner history exposed no entry id outside the peer target: "
                f"owner={owner_entries!r} peer={peer_entries!r}"
            )

        peer_url_before = peer.url
        foreign_error = await _expect_protocol_error(
            peer_session.send(
                "Page.navigateToHistoryEntry",
                {"entryId": foreign_id},
            ),
            "peer navigating to a foreign history entry",
        )
        assert_equal(
            peer.url,
            peer_url_before,
            "foreign history entry rejection preserves the peer Document",
        )

        owner_entry = owner_entries[-2]
        await owner_session.send(
            "Page.navigateToHistoryEntry",
            {"entryId": owner_entry["id"]},
        )
        await wait_until(
            lambda: owner.url == owner_entry["url"],
            "owner target history traversal",
        )
        peer_entry = peer_entries[-2]
        await peer_session.send(
            "Page.navigateToHistoryEntry",
            {"entryId": peer_entry["id"]},
        )
        await wait_until(
            lambda: peer.url == peer_entry["url"],
            "peer target history traversal after foreign-id rejection",
        )

        record_contract(
            results,
            "multi_page_navigation_history_entry_ownership",
            contract=(
                "Page navigation-history entry ids are target-local capabilities: a foreign "
                "id is rejected without navigation, and both targets retain usable own history."
            ),
            source="Debian Chromium 145.0.7632.116 executable CDP oracle",
            commands=["Page.getNavigationHistory x2", "Page.navigateToHistoryEntry x3"],
            observed={
                "ownerEntryCount": len(owner_entries),
                "peerEntryCount": len(peer_entries),
                "foreignEntryRejected": bool(foreign_error),
                "ownerOwnUrl": owner.url,
                "peerOwnUrl": peer.url,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _blocked_urls_aggregate_across_target_sessions(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        owner, peer = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            owner.goto(f"{fixture}/plain?multi-page-blocked=owner", wait_until="load"),
            peer.goto(f"{fixture}/plain?multi-page-blocked=peer", wait_until="load"),
        )
        first, second, latent, peer_session = await asyncio.gather(
            context.new_cdp_session(owner),
            context.new_cdp_session(owner),
            context.new_cdp_session(owner),
            context.new_cdp_session(peer),
        )
        sessions = [first, second, latent, peer_session]
        third_url = f"{fixture}/api?multi-page-blocked-latent=1"
        await latent.send(
            "Network.setBlockedURLs",
            {"urls": ["*multi-page-blocked-latent*"]},
        )
        await asyncio.gather(
            first.send("Network.enable"),
            second.send("Network.enable"),
            peer_session.send("Network.enable"),
        )
        first_url = f"{fixture}/api?multi-page-blocked-first=1"
        second_url = f"{fixture}/api?multi-page-blocked-second=1"
        await first.send(
            "Network.setBlockedURLs",
            {"urls": ["*multi-page-blocked-first*"]},
        )
        await second.send(
            "Network.setBlockedURLs",
            {"urls": ["*multi-page-blocked-second*"]},
        )

        owner_before = await asyncio.gather(
            _fetch_result(owner, first_url),
            _fetch_result(owner, second_url),
            _fetch_result(owner, third_url),
        )
        peer_before = await asyncio.gather(
            _fetch_result(peer, first_url),
            _fetch_result(peer, second_url),
            _fetch_result(peer, third_url),
        )
        assert_equal(
            [result["ok"] for result in owner_before],
            [False, False, True],
            "only enabled same-target Network handlers contribute blocked URLs",
        )
        assert_equal(
            [result["ok"] for result in peer_before],
            [True, True, True],
            "blocked URL union remains target-local",
        )

        await latent.send("Network.enable")
        assert_equal(
            (await _fetch_result(owner, third_url))["ok"],
            False,
            "Network.enable activates a handler's retained blocked URL contribution",
        )

        await first.send("Network.setBlockedURLs", {"urls": []})
        owner_after_clear = await asyncio.gather(
            _fetch_result(owner, first_url),
            _fetch_result(owner, second_url),
            _fetch_result(owner, third_url),
        )
        assert_equal(
            [result["ok"] for result in owner_after_clear],
            [True, False, False],
            "clearing one handler preserves its peer's blocked URL contribution",
        )

        await second.detach()
        sessions.remove(second)
        owner_after_first_detach = await asyncio.gather(
            _fetch_result(owner, second_url),
            _fetch_result(owner, third_url),
        )
        assert_equal(
            [result["ok"] for result in owner_after_first_detach],
            [True, False],
            "detaching one blocker preserves another handler's contribution",
        )
        await latent.detach()
        sessions.remove(latent)
        owner_after_all_detach = await _fetch_result(owner, third_url)
        assert_equal(
            owner_after_all_detach["ok"],
            True,
            "detaching the final blocker recomputes the target policy",
        )
        assert_equal(
            (await _fetch_result(peer, third_url))["ok"],
            True,
            "peer target remains usable after owner policy detach",
        )

        record_contract(
            results,
            "multi_page_blocked_url_session_aggregation",
            contract=(
                "Enabled Network handlers on one Page contribute the union of their blocked "
                "URL patterns; clear and detach remove only the owning contribution, and a "
                "peer Page is unaffected."
            ),
            source=(
                "Debian Chromium 145.0.7632.116 executable CDP oracle and "
                "InspectorNetworkAgent::ShouldBlockRequest"
            ),
            commands=[
                "Network.enable x4",
                "Network.setBlockedURLs x4",
                "Runtime.evaluate(fetch) x14",
                "Target.detachFromTarget x2",
            ],
            observed={
                "ownerBefore": [result["ok"] for result in owner_before],
                "peerBefore": [result["ok"] for result in peer_before],
                "ownerAfterClear": [result["ok"] for result in owner_after_clear],
                "ownerAfterFirstDetach": [
                    result["ok"] for result in owner_after_first_detach
                ],
                "ownerAfterAllDetach": owner_after_all_detach["ok"],
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _cache_disabled_aggregates_without_crossing_targets(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        owner, peer = await asyncio.gather(context.new_page(), context.new_page())
        cache_page = f"{fixture}/semantic-cache-page"
        await owner.goto(cache_page, wait_until="load")
        first_generation = await owner.evaluate("__semanticCacheRequest")
        await peer.goto(cache_page, wait_until="load")
        peer_generation = await peer.evaluate("__semanticCacheRequest")
        if not isinstance(first_generation, int):
            raise SmokeError(f"cache fixture returned no integer generation: {first_generation!r}")
        assert_equal(
            peer_generation,
            first_generation,
            "peer Page initially reuses the context cache entry",
        )

        first, second, peer_session = await asyncio.gather(
            context.new_cdp_session(owner),
            context.new_cdp_session(owner),
            context.new_cdp_session(peer),
        )
        sessions = [first, second, peer_session]
        await asyncio.gather(
            first.send("Network.enable"),
            second.send("Network.enable"),
            peer_session.send("Network.enable"),
        )
        await first.send("Network.setCacheDisabled", {"cacheDisabled": True})
        await second.send("Network.setCacheDisabled", {"cacheDisabled": False})

        await owner.reload(wait_until="load")
        bypass_generation = await owner.evaluate("__semanticCacheRequest")
        assert_equal(
            bypass_generation,
            first_generation + 1,
            "one true handler keeps cache disabled despite a false peer contribution",
        )
        await peer.reload(wait_until="load")
        assert_equal(
            await peer.evaluate("__semanticCacheRequest"),
            first_generation,
            "an existing peer retains its Page-local memory-cache resource",
        )
        fresh = await context.new_page()
        await fresh.goto(cache_page, wait_until="load")
        fresh_generation = await fresh.evaluate("__semanticCacheRequest")
        assert_equal(
            fresh_generation,
            bypass_generation,
            "a fresh Page observes the bypass replacement through the HTTP cache",
        )

        await first.send("Network.setCacheDisabled", {"cacheDisabled": False})
        await owner.reload(wait_until="load")
        assert_equal(
            await owner.evaluate("__semanticCacheRequest"),
            bypass_generation,
            "normal loading reuses the response fetched during cache bypass",
        )

        await first.send("Network.setCacheDisabled", {"cacheDisabled": True})
        await owner.reload(wait_until="load")
        detach_generation = await owner.evaluate("__semanticCacheRequest")
        assert_equal(
            detach_generation,
            bypass_generation + 1,
            "re-enabled cache bypass fetches one replacement response",
        )
        await first.detach()
        sessions.remove(first)
        await owner.reload(wait_until="load")
        assert_equal(
            await owner.evaluate("__semanticCacheRequest"),
            detach_generation,
            "detaching the true contributor restores cache use and keeps its replacement",
        )
        await peer.reload(wait_until="load")
        assert_equal(
            await peer.evaluate("__semanticCacheRequest"),
            first_generation,
            "peer target retains its own earlier cached resource generation",
        )

        record_contract(
            results,
            "multi_page_cache_disabled_session_aggregation",
            contract=(
                "Network cacheDisabled is ORed across enabled handlers on one Page, does not "
                "rewrite an existing peer Page's retained resource, and publishes its bypass "
                "response to the HTTP cache for fresh Pages and later normal owner loads."
            ),
            source="Debian Chromium 145.0.7632.116 executable CDP oracle",
            commands=[
                "Network.enable x3",
                "Network.setCacheDisabled x4",
                "Page.reload x6",
                "Target.detachFromTarget",
            ],
            observed={
                "initialGeneration": first_generation,
                "bypassGeneration": bypass_generation,
                "detachGeneration": detach_generation,
                "peerGeneration": peer_generation,
                "freshGenerationAfterBypass": fresh_generation,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _fetch_result(page: Any, url: str) -> dict[str, Any]:
    value = await page.evaluate(
        """async url => {
          try {
            const response = await fetch(url);
            return {ok: true, status: response.status, text: await response.text()};
          } catch (error) {
            return {ok: false, error: String(error)};
          }
        }""",
        url,
    )
    if not isinstance(value, dict) or not isinstance(value.get("ok"), bool):
        raise SmokeError(f"fetch returned an invalid result for {url}: {value!r}")
    return value


def _event_count(events: list[dict[str, Any]], method: str) -> int:
    return sum(event.get("method") == method for event in events)


def _has_storage_event(
    events: list[dict[str, Any]],
    key: str,
    *,
    is_local: bool,
) -> bool:
    return _storage_event_count(events, key, is_local=is_local) > 0


def _storage_event_count(
    events: list[dict[str, Any]],
    key: str,
    *,
    is_local: bool,
) -> int:
    return sum(
        event.get("params", {}).get("key") == key
        and event.get("params", {})
        .get("storageId", {})
        .get("isLocalStorage")
        is is_local
        for event in events
    )


def _runtime_value(response: dict[str, Any]) -> Any:
    return response.get("result", {}).get("value")


async def _expect_protocol_error(awaitable: Awaitable[Any], label: str) -> str:
    try:
        await awaitable
    except Exception as error:
        return str(error)
    raise SmokeError(f"{label} unexpectedly succeeded")


async def _close_context(context: Any) -> None:
    try:
        await asyncio.wait_for(context.close(), timeout=5)
    except Exception as error:
        raise SmokeError(
            f"BrowserContext.close failed: {type(error).__name__}: {error}"
        ) from error
