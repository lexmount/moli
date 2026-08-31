from __future__ import annotations

import asyncio
import json
import urllib.request
from contextlib import suppress
from typing import Any, Awaitable

from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..helpers import attach_cdp_event_collector
from ..progress import await_with_progress


async def run_multi_page_isolation_contracts(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    """Exercise shared-renderer boundaries that can silently cross Page owners."""

    contracts = (
        _busy_runtime_termination_is_target_local,
        _stop_loading_is_target_local,
        _same_target_session_policies_follow_chromium,
        _activation_churn_preserves_target_ownership,
    )
    for contract in contracts:
        await await_with_progress(
            f"multi-page/{contract.__name__.removeprefix('_')}",
            contract(browser, fixture, results),
            timeout_seconds=20,
        )


async def _busy_runtime_termination_is_target_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    busy: asyncio.Task[Any] | None = None
    peer: asyncio.Task[Any] | None = None
    try:
        page_a, page_b = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            page_a.goto(f"{fixture}/plain?multi-page-runtime=a", wait_until="load"),
            page_b.goto(f"{fixture}/plain?multi-page-runtime=b", wait_until="load"),
        )
        session_a = await context.new_cdp_session(page_a)
        session_b = await context.new_cdp_session(page_b)
        sessions = [session_a, session_b]
        await asyncio.to_thread(
            _read_fixture_json,
            f"{fixture}/inspector-routing-witness/reset",
        )

        busy = asyncio.create_task(
            session_a.send(
                "Runtime.evaluate",
                {
                    "expression": """const witness = new XMLHttpRequest();
witness.open('GET', '/inspector-routing-witness/entered', false);
witness.send();
globalThis.__multiPageBusyEntered =
  (globalThis.__multiPageBusyEntered || 0) + 1;
for (;;) {}""",
                    "returnByValue": True,
                },
            )
        )

        async def busy_loop_entered() -> bool:
            try:
                status = await asyncio.to_thread(
                    _read_fixture_json,
                    f"{fixture}/inspector-routing-witness/status",
                )
            except Exception:
                return False
            return status.get("enteredCount") == 1

        await wait_until(
            busy_loop_entered,
            "target A non-yielding Runtime evaluation",
            timeout_ms=5_000,
        )
        peer = asyncio.create_task(
            session_b.send(
                "Runtime.evaluate",
                {
                    "expression": (
                        "globalThis.__multiPagePeerRan = "
                        "(globalThis.__multiPagePeerRan || 0) + 1"
                    ),
                    "returnByValue": True,
                },
            )
        )
        await asyncio.sleep(0)

        terminate = await asyncio.wait_for(
            session_a.send("Runtime.terminateExecution"),
            timeout=5,
        )
        assert_equal(terminate, {}, "target A Runtime.terminateExecution response")
        busy_result, peer_result = await asyncio.wait_for(
            asyncio.gather(busy, peer, return_exceptions=True),
            timeout=5,
        )
        if not isinstance(busy_result, BaseException):
            raise SmokeError(
                "target A non-yielding evaluation was not terminated: "
                f"{busy_result!r}"
            )
        if "terminated" not in str(busy_result).lower():
            raise SmokeError(
                "target A termination returned an unexpected error: "
                f"{busy_result!r}"
            )
        if isinstance(peer_result, BaseException):
            raise SmokeError(
                "target B Runtime command was terminated with target A: "
                f"{peer_result!r}"
            )
        assert_equal(
            _runtime_value(peer_result),
            1,
            "target B queued Runtime command after target A termination",
        )

        recovery_a, recovery_b = await asyncio.gather(
            session_a.send(
                "Runtime.evaluate",
                {
                    "expression": "globalThis.__multiPageBusyEntered",
                    "returnByValue": True,
                },
            ),
            session_b.send(
                "Runtime.evaluate",
                {
                    "expression": "globalThis.__multiPagePeerRan",
                    "returnByValue": True,
                },
            ),
        )
        assert_equal(
            [_runtime_value(recovery_a), _runtime_value(recovery_b)],
            [1, 1],
            "target-local Runtime recovery after termination",
        )
        record_contract(
            results,
            "multi_page_busy_runtime_termination_is_target_local",
            contract=(
                "Runtime.terminateExecution interrupts non-yielding JavaScript on its "
                "Page target without terminating or misrouting a queued command on a peer; "
                "both target sessions recover."
            ),
            source="Chromium DevToolsSession executable oracle",
            commands=["Runtime.evaluate x4", "Runtime.terminateExecution"],
            observed={
                "busyTargetTerminated": True,
                "peerValue": _runtime_value(peer_result),
                "recoveryValues": [
                    _runtime_value(recovery_a),
                    _runtime_value(recovery_b),
                ],
            },
        )
    finally:
        if busy is not None and not busy.done() and sessions:
            with suppress(Exception):
                await asyncio.wait_for(
                    sessions[0].send("Runtime.terminateExecution"),
                    timeout=2,
                )
        for task in (busy, peer):
            if task is not None and not task.done():
                task.cancel()
        await asyncio.gather(
            *(task for task in (busy, peer) if task is not None),
            return_exceptions=True,
        )
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _stop_loading_is_target_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    paused_request_id: str | None = None
    try:
        page_a, page_b = await asyncio.gather(context.new_page(), context.new_page())
        session_a = await context.new_cdp_session(page_a)
        session_b = await context.new_cdp_session(page_b)
        sessions = [session_a, session_b]
        events_a = attach_cdp_event_collector(
            session_a,
            ["Fetch.requestPaused", "Network.loadingFailed"],
        )
        await asyncio.gather(
            session_a.send("Network.enable"),
            session_b.send("Network.enable"),
            session_a.send(
                "Fetch.enable",
                {
                    "patterns": [
                        {
                            "urlPattern": "*delayed-image.png*",
                            "requestStage": "Request",
                        }
                    ]
                },
            ),
        )

        response_a = await page_a.goto(
            f"{fixture}/lifecycle-load-state?multi-page-stop=a",
            wait_until="domcontentloaded",
            timeout=5_000,
        )
        assert_equal(
            response_a.status if response_a is not None else None,
            200,
            "target A DOMContentLoaded navigation status",
        )
        await wait_until(
            lambda: any(
                event.get("method") == "Fetch.requestPaused"
                and "delayed-image.png"
                in event.get("params", {}).get("request", {}).get("url", "")
                for event in events_a
            ),
            "target A paused image request",
            timeout_ms=5_000,
        )
        paused = next(
            event
            for event in events_a
            if event.get("method") == "Fetch.requestPaused"
            and "delayed-image.png"
            in event.get("params", {}).get("request", {}).get("url", "")
        )
        paused_request_id = paused.get("params", {}).get("requestId")
        network_id = paused.get("params", {}).get("networkId")
        if not isinstance(paused_request_id, str) or not isinstance(network_id, str):
            raise SmokeError(f"paused image request had no CDP identifiers: {paused!r}")

        before_stop = await page_a.evaluate(
            """() => ({
              dcl: document.body.dataset.dcl,
              load: document.body.dataset.load,
              readyState: document.readyState,
            })"""
        )
        assert_equal(
            before_stop,
            {"dcl": "1", "load": "0", "readyState": "interactive"},
            "target A lifecycle before Page.stopLoading",
        )

        response_b = await page_b.goto(
            f"{fixture}/lifecycle-load-state?multi-page-stop=b",
            wait_until="load",
            timeout=5_000,
        )
        assert_equal(
            response_b.status if response_b is not None else None,
            200,
            "target B load while target A image is paused",
        )
        peer_state = await page_b.evaluate(
            """() => ({
              load: document.body.dataset.load,
              readyState: document.readyState,
            })"""
        )
        assert_equal(
            peer_state,
            {"load": "1", "readyState": "complete"},
            "target B lifecycle while target A is paused",
        )

        stop = await asyncio.wait_for(session_a.send("Page.stopLoading"), timeout=5)
        assert_equal(stop, {}, "target A Page.stopLoading response")
        await wait_until(
            lambda: any(
                event.get("method") == "Network.loadingFailed"
                and event.get("params", {}).get("requestId") == network_id
                for event in events_a
            ),
            "target A canceled image request",
            timeout_ms=5_000,
        )
        loading_failed = next(
            event
            for event in events_a
            if event.get("method") == "Network.loadingFailed"
            and event.get("params", {}).get("requestId") == network_id
        )
        assert_equal(
            loading_failed.get("params", {}).get("canceled"),
            True,
            "target A stopped image request cancellation flag",
        )
        after_stop = await page_a.evaluate(
            """() => ({
              dcl: document.body.dataset.dcl,
              load: document.body.dataset.load,
              readyState: document.readyState,
            })"""
        )
        assert_equal(
            after_stop,
            {"dcl": "1", "load": "0", "readyState": "complete"},
            "target A lifecycle after Page.stopLoading",
        )
        stale_interception_error = await _expect_protocol_error(
            session_a.send(
                "Fetch.continueRequest",
                {"requestId": paused_request_id},
            ),
            "stale Fetch interception after Page.stopLoading",
        )
        paused_request_id = None
        await session_a.send("Fetch.disable")

        await asyncio.gather(
            page_a.goto(
                f"{fixture}/plain?multi-page-stop=recovered-a",
                wait_until="load",
            ),
            page_b.goto(
                f"{fixture}/plain?multi-page-stop=recovered-b",
                wait_until="load",
            ),
        )
        assert_equal(
            await asyncio.gather(page_a.text_content("main"), page_b.text_content("main")),
            ["plain ok", "plain ok"],
            "both targets after target-local Page.stopLoading",
        )
        record_contract(
            results,
            "multi_page_stop_loading_is_target_local",
            contract=(
                "Page.stopLoading cancels only its target's paused subresource, retires the "
                "Fetch interception id, preserves the committed Document, and does not "
                "delay a peer target's load."
            ),
            source="Chromium Page and Fetch executable oracle",
            commands=[
                "Fetch.enable",
                "Fetch.requestPaused",
                "Page.stopLoading",
                "Network.loadingFailed",
                "Fetch.continueRequest",
            ],
            observed={
                "beforeStop": before_stop,
                "afterStop": after_stop,
                "peerState": peer_state,
                "staleInterceptionRejected": True,
                "staleInterceptionError": stale_interception_error,
            },
        )
    finally:
        if sessions:
            if paused_request_id is not None:
                with suppress(Exception):
                    await sessions[0].send(
                        "Fetch.continueRequest",
                        {"requestId": paused_request_id},
                    )
            with suppress(Exception):
                await sessions[0].send("Fetch.disable")
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _same_target_session_policies_follow_chromium(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        owner, peer = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            owner.goto(f"{fixture}/plain?multi-page-policy=owner", wait_until="load"),
            peer.goto(f"{fixture}/plain?multi-page-policy=peer", wait_until="load"),
        )
        first = await context.new_cdp_session(owner)
        second = await context.new_cdp_session(owner)
        peer_session = await context.new_cdp_session(peer)
        sessions = [first, second, peer_session]
        await asyncio.gather(
            first.send("Network.enable"),
            second.send("Network.enable"),
            peer_session.send("Network.enable"),
        )

        await second.send(
            "Emulation.setUserAgentOverride",
            {
                "userAgent": "MultiPageAttachmentSecond/1.0",
                "acceptLanguage": "ja-JP",
            },
        )
        await first.send(
            "Emulation.setUserAgentOverride",
            {
                "userAgent": "MultiPageFirstSetLast/1.0",
                "acceptLanguage": "fr-FR",
            },
        )
        await asyncio.gather(
            first.send(
                "Network.setExtraHTTPHeaders",
                {"headers": {"x-moli-session-first": "first"}},
            ),
            second.send(
                "Network.setExtraHTTPHeaders",
                {"headers": {"x-moli-session-second": "second"}},
            ),
            peer_session.send(
                "Network.setExtraHTTPHeaders",
                {"headers": {"x-moli-session-peer": "peer"}},
            ),
        )
        await first.send("Emulation.setLocaleOverride", {"locale": "fr-FR"})
        await first.send(
            "Emulation.setTimezoneOverride",
            {"timezoneId": "Europe/Paris"},
        )
        locale_error = await _expect_protocol_error(
            second.send("Emulation.setLocaleOverride", {"locale": "ja-JP"}),
            "second same-target locale claim",
        )
        timezone_error = await _expect_protocol_error(
            second.send(
                "Emulation.setTimezoneOverride",
                {"timezoneId": "Asia/Tokyo"},
            ),
            "second same-target timezone claim",
        )
        if "Another locale override is already in effect" not in locale_error:
            raise SmokeError(f"unexpected locale claim error: {locale_error}")
        if "Timezone override is already in effect" not in timezone_error:
            raise SmokeError(f"unexpected timezone claim error: {timezone_error}")

        owner_before_token = "multi-page-session-policy-before"
        peer_before_token = "multi-page-session-policy-peer-before"
        await asyncio.gather(
            owner.goto(
                f"{fixture}/profile-headers?token={owner_before_token}",
                wait_until="load",
            ),
            peer.goto(
                f"{fixture}/profile-headers?token={peer_before_token}",
                wait_until="load",
            ),
        )
        owner_wire_before, peer_wire_before = await asyncio.gather(
            _read_fixture_profile(fixture, owner_before_token),
            _read_fixture_profile(fixture, peer_before_token),
        )
        owner_runtime_before = await _runtime_identity(owner)
        assert_equal(
            {
                "userAgent": owner_wire_before.get("userAgent"),
                "acceptLanguage": owner_wire_before.get("acceptLanguage"),
                "first": owner_wire_before.get("sessionHeaderFirst"),
                "second": owner_wire_before.get("sessionHeaderSecond"),
                "peer": owner_wire_before.get("sessionHeaderPeer"),
            },
            {
                "userAgent": "MultiPageAttachmentSecond/1.0",
                "acceptLanguage": "ja-JP",
                "first": "first",
                "second": "second",
                "peer": None,
            },
            "same-target network policy aggregation",
        )
        assert_equal(
            owner_runtime_before,
            {
                "userAgent": "MultiPageAttachmentSecond/1.0",
                "language": "ja-JP",
                "timezone": "Europe/Paris",
            },
            "same-target live renderer policy before detach",
        )
        assert_equal(
            {
                "first": peer_wire_before.get("sessionHeaderFirst"),
                "second": peer_wire_before.get("sessionHeaderSecond"),
                "peer": peer_wire_before.get("sessionHeaderPeer"),
            },
            {"first": None, "second": None, "peer": "peer"},
            "peer target network policy isolation",
        )

        await first.detach()
        sessions.remove(first)
        await second.send("Emulation.setLocaleOverride", {"locale": "ja-JP"})
        await second.send(
            "Emulation.setTimezoneOverride",
            {"timezoneId": "Asia/Tokyo"},
        )
        owner_after_first_token = "multi-page-session-policy-after-first"
        await owner.goto(
            f"{fixture}/profile-headers?token={owner_after_first_token}",
            wait_until="load",
        )
        owner_wire_after_first = await _read_fixture_profile(
            fixture,
            owner_after_first_token,
        )
        owner_runtime_after_first = await _runtime_identity(owner)
        assert_equal(
            {
                "userAgent": owner_wire_after_first.get("userAgent"),
                "first": owner_wire_after_first.get("sessionHeaderFirst"),
                "second": owner_wire_after_first.get("sessionHeaderSecond"),
            },
            {
                "userAgent": "MultiPageAttachmentSecond/1.0",
                "first": None,
                "second": "second",
            },
            "same-target policy after first session detach",
        )
        assert_equal(
            owner_runtime_after_first,
            {
                "userAgent": "MultiPageAttachmentSecond/1.0",
                "language": "ja-JP",
                "timezone": "Asia/Tokyo",
            },
            "released locale/timezone claims acquired by surviving session",
        )

        await second.detach()
        sessions.remove(second)
        owner_after_all_token = "multi-page-session-policy-after-all"
        peer_after_token = "multi-page-session-policy-peer-after"
        await asyncio.gather(
            owner.goto(
                f"{fixture}/profile-headers?token={owner_after_all_token}",
                wait_until="load",
            ),
            peer.goto(
                f"{fixture}/profile-headers?token={peer_after_token}",
                wait_until="load",
            ),
        )
        owner_wire_after_all, peer_wire_after = await asyncio.gather(
            _read_fixture_profile(fixture, owner_after_all_token),
            _read_fixture_profile(fixture, peer_after_token),
        )
        owner_runtime_after_all = await _runtime_identity(owner)
        assert_equal(
            [
                owner_wire_after_all.get("sessionHeaderFirst"),
                owner_wire_after_all.get("sessionHeaderSecond"),
                owner_wire_after_all.get("sessionHeaderPeer"),
            ],
            [None, None, None],
            "all owner session headers after detach",
        )
        if owner_runtime_after_all["userAgent"] in {
            "MultiPageFirstSetLast/1.0",
            "MultiPageAttachmentSecond/1.0",
        }:
            raise SmokeError(
                "detached owner retained a session user agent: "
                f"{owner_runtime_after_all!r}"
            )
        assert_equal(
            peer_wire_after.get("sessionHeaderPeer"),
            "peer",
            "peer target policy after all owner sessions detach",
        )
        record_contract(
            results,
            "multi_page_same_target_session_policy_aggregation",
            contract=(
                "Enabled Network sessions merge non-conflicting headers and select wire "
                "and live renderer identity by attachment order; locale/timezone use "
                "exclusive claims, and detach "
                "recomputes only the owning target."
            ),
            source="Chromium NetworkHandler and InspectorEmulationAgent executable oracle",
            commands=[
                "Network.enable x3",
                "Network.setExtraHTTPHeaders x3",
                "Emulation.setUserAgentOverride x2",
                "Emulation.setLocaleOverride",
                "Emulation.setTimezoneOverride",
                "Target.detachFromTarget x2",
            ],
            observed={
                "wireBeforeDetach": owner_wire_before,
                "runtimeBeforeDetach": owner_runtime_before,
                "runtimeAfterFirstDetach": owner_runtime_after_first,
                "runtimeAfterAllDetach": owner_runtime_after_all,
                "exclusiveClaimsRejected": True,
                "peerPolicySurvived": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await _close_context(context)


async def _activation_churn_preserves_target_ownership(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    browser_session = await browser.new_browser_cdp_session()
    sessions: list[Any] = []
    try:
        pages = [await context.new_page() for _ in range(4)]
        initial_urls = [
            f"{fixture}/plain?multi-page-activation={index}" for index in range(4)
        ]
        await asyncio.gather(
            *(
                page.goto(url, wait_until="load")
                for page, url in zip(pages, initial_urls, strict=True)
            )
        )
        sessions = [await context.new_cdp_session(page) for page in pages]
        for index, session in enumerate(sessions):
            await asyncio.gather(
                session.send("Page.enable"),
                session.send("Network.enable"),
            )
            await session.send(
                "Network.setExtraHTTPHeaders",
                {"headers": {"x-moli-profile-smoke": f"activation-{index}"}},
            )
            await session.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {
                    "source": (
                        "globalThis.__multiPageActivationOwner = "
                        f"'activation-{index}';"
                    )
                },
            )
            await session.send(
                "Runtime.evaluate",
                {
                    "expression": (
                        "globalThis.__multiPageActivationOwner = "
                        f"'activation-{index}'"
                    ),
                    "returnByValue": True,
                },
            )

        target_infos = (await browser_session.send("Target.getTargets")).get(
            "targetInfos",
            [],
        )
        target_ids_by_url = {
            info.get("url"): info.get("targetId")
            for info in target_infos
            if info.get("type") == "page"
        }
        target_ids = [target_ids_by_url.get(url) for url in initial_urls]
        if not all(isinstance(target_id, str) for target_id in target_ids):
            raise SmokeError(
                "could not resolve every activation target: "
                f"urls={initial_urls!r}, targets={target_infos!r}"
            )

        activation_order = [2, 0, 3, 1, 0, 2]
        for active_index in activation_order:
            await browser_session.send(
                "Target.activateTarget",
                {"targetId": target_ids[active_index]},
            )
            owner_values = await asyncio.gather(
                *(
                    session.send(
                        "Runtime.evaluate",
                        {
                            "expression": "globalThis.__multiPageActivationOwner",
                            "returnByValue": True,
                        },
                    )
                    for session in sessions
                )
            )
            assert_equal(
                [_runtime_value(value) for value in owner_values],
                [f"activation-{index}" for index in range(4)],
                f"session ownership after activating target {active_index}",
            )
            frame_trees = await asyncio.gather(
                *(session.send("Page.getFrameTree") for session in sessions)
            )
            assert_equal(
                [
                    tree.get("frameTree", {}).get("frame", {}).get("url")
                    for tree in frame_trees
                ],
                initial_urls,
                f"frame ownership after activating target {active_index}",
            )

        close_result = await browser_session.send(
            "Target.closeTarget",
            {"targetId": target_ids[2]},
        )
        assert_equal(close_result.get("success"), True, "close active churn target")
        await wait_until(
            pages[2].is_closed,
            "active churn target close",
            timeout_ms=5_000,
        )

        survivor_indexes = [0, 1, 3]
        replacement = await context.new_page()
        survivor_tokens = [f"multi-page-activation-survivor-{index}" for index in survivor_indexes]
        replacement_token = "multi-page-activation-replacement"
        await asyncio.gather(
            *(
                pages[index].goto(
                    f"{fixture}/profile-headers?token={token}",
                    wait_until="load",
                )
                for index, token in zip(
                    survivor_indexes,
                    survivor_tokens,
                    strict=True,
                )
            ),
            replacement.goto(
                f"{fixture}/profile-headers?token={replacement_token}",
                wait_until="load",
            ),
        )
        survivor_owners = await asyncio.gather(
            *(
                pages[index].evaluate("globalThis.__multiPageActivationOwner")
                for index in survivor_indexes
            )
        )
        assert_equal(
            survivor_owners,
            [f"activation-{index}" for index in survivor_indexes],
            "preload ownership after activation churn and active-target close",
        )
        assert_equal(
            await replacement.evaluate("globalThis.__multiPageActivationOwner"),
            None,
            "replacement target does not inherit closed target preload state",
        )
        profiles = await asyncio.gather(
            *(_read_fixture_profile(fixture, token) for token in survivor_tokens),
            _read_fixture_profile(fixture, replacement_token),
        )
        assert_equal(
            [profile.get("extraHeader") for profile in profiles],
            [
                *(f"activation-{index}" for index in survivor_indexes),
                None,
            ],
            "network policy after activation churn and active-target close",
        )
        survivor_frame_trees = await asyncio.gather(
            *(sessions[index].send("Page.getFrameTree") for index in survivor_indexes)
        )
        assert_equal(
            [
                tree.get("frameTree", {}).get("frame", {}).get("url")
                for tree in survivor_frame_trees
            ],
            [
                f"{fixture}/profile-headers?token={token}"
                for token in survivor_tokens
            ],
            "surviving sessions after active-target close",
        )
        record_contract(
            results,
            "multi_page_activation_churn_preserves_target_ownership",
            contract=(
                "Repeated Target.activateTarget calls change foreground selection without "
                "moving session, frame, preload, or Network state; closing the active Page "
                "retires only that target and a replacement starts clean."
            ),
            source="Chromium stable WebContents/DevToolsSession ownership model",
            commands=[
                "Target.activateTarget x6",
                "Runtime.evaluate",
                "Page.getFrameTree",
                "Page.enable x4",
                "Page.addScriptToEvaluateOnNewDocument",
                "Network.setExtraHTTPHeaders",
                "Target.closeTarget",
            ],
            observed={
                "activationOrder": activation_order,
                "survivorOwners": survivor_owners,
                "survivorHeaders": [
                    profile.get("extraHeader") for profile in profiles[:-1]
                ],
                "replacementStartedClean": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        with suppress(Exception):
            await browser_session.detach()
        await _close_context(context)


async def _expect_protocol_error(
    awaitable: Awaitable[Any],
    label: str,
) -> str:
    try:
        result = await asyncio.wait_for(awaitable, timeout=5)
    except Exception as error:
        return str(error)
    raise SmokeError(f"{label} unexpectedly succeeded: {result!r}")


async def _runtime_identity(page: Any) -> dict[str, Any]:
    value = await page.evaluate(
        """() => ({
          userAgent: navigator.userAgent,
          language: navigator.language,
          timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        })"""
    )
    if not isinstance(value, dict):
        raise SmokeError(f"Runtime identity was not an object: {value!r}")
    return value


async def _read_fixture_profile(fixture: str, token: str) -> dict[str, Any]:
    value = await asyncio.to_thread(
        _read_fixture_json,
        f"{fixture}/profile-result?token={token}",
    )
    if not isinstance(value, dict):
        raise SmokeError(f"fixture recorded no profile for {token}: {value!r}")
    return value


def _runtime_value(response: dict[str, Any]) -> Any:
    return response.get("result", {}).get("value")


async def _close_context(context: Any) -> None:
    try:
        await asyncio.wait_for(context.close(), timeout=5)
    except Exception as error:
        raise SmokeError(
            f"BrowserContext.close failed: {type(error).__name__}: {error}"
        ) from error


def _read_fixture_json(url: str) -> Any:
    with urllib.request.urlopen(url, timeout=2) as response:
        return json.loads(response.read().decode("utf-8"))
