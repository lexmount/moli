from __future__ import annotations

import asyncio
from contextlib import suppress
from typing import Any

from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..helpers import attach_cdp_event_collector
from .multi_page_support import (
    MultiPageCase,
    close_context,
    expect_protocol_error,
    read_fixture_json,
)


def multi_page_contract_cases() -> tuple[MultiPageCase, ...]:
    return (
        _remote_object_and_execution_context_ownership,
        _target_local_device_metrics,
        _new_document_script_session_lifecycle,
        _same_target_new_document_script_session_ownership,
        _isolated_world_script_detach_lifecycle,
        _same_name_isolated_worlds_are_session_local,
        _runtime_binding_session_lifecycle,
        _same_target_remote_objects_are_session_local,
        _same_target_dom_node_and_backend_node_ownership,
        _same_source_scripts_keep_session_authority,
        _network_enablement_is_session_local,
        _page_lifecycle_enablement_is_session_local,
        _runtime_enablement_and_context_replay_are_session_local,
        _same_name_runtime_bindings_keep_session_authority,
        _session_detach_cancels_only_its_pending_await,
        _fetch_interception_is_session_local,
        _network_and_emulation_profiles_are_target_local,
        _closed_target_new_document_script_does_not_leak,
        _broadcast_channel_context_partition,
        _same_target_navigation_supersession,
        _target_destroy_event_cardinality,
        _network_response_body_target_ownership,
        _websocket_survives_peer_target_close,
    )


async def _remote_object_and_execution_context_ownership(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page_a, page_b = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            page_a.goto(f"{fixture}/plain?multi-page-object=a", wait_until="load"),
            page_b.goto(f"{fixture}/plain?multi-page-object=b", wait_until="load"),
        )
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page_a),
            context.new_cdp_session(page_b),
        )
        sessions = [session_a, session_b]
        await asyncio.gather(
            session_a.send("Runtime.enable"),
            session_b.send("Runtime.enable"),
        )

        remote = (
            await session_a.send(
                "Runtime.evaluate",
                {"expression": "({owner: 'page-a', value: 7})"},
            )
        ).get("result", {})
        object_id = remote.get("objectId")
        if not isinstance(object_id, str) or not object_id:
            raise SmokeError(f"page A evaluation returned no objectId: {remote!r}")

        frame_tree = await session_a.send("Page.getFrameTree")
        frame_id = frame_tree.get("frameTree", {}).get("frame", {}).get("id")
        if not isinstance(frame_id, str) or not frame_id:
            raise SmokeError(f"page A frame tree returned no frame id: {frame_tree!r}")
        isolated = await session_a.send(
            "Page.createIsolatedWorld",
            {"frameId": frame_id, "worldName": "multi-page-owner-a"},
        )
        execution_context_id = isolated.get("executionContextId")
        if not isinstance(execution_context_id, int):
            raise SmokeError(
                f"page A isolated world returned no executionContextId: {isolated!r}"
            )

        foreign_object_error = await expect_protocol_error(
            session_b.send("Runtime.getProperties", {"objectId": object_id}),
            "page B resolving page A objectId",
        )
        foreign_context_error = await expect_protocol_error(
            session_b.send(
                "Runtime.evaluate",
                {
                    "expression": "1",
                    "contextId": execution_context_id,
                    "returnByValue": True,
                },
            ),
            "page B evaluating in page A executionContextId",
        )

        own_properties = await session_a.send(
            "Runtime.getProperties",
            {"objectId": object_id, "ownProperties": True},
        )
        enumerable_names = {
            prop.get("name")
            for prop in own_properties.get("result", [])
            if prop.get("enumerable")
        }
        assert_equal(
            enumerable_names,
            {"owner", "value"},
            "owner session resolves its own remote object",
        )
        peer_value = await session_b.send(
            "Runtime.evaluate",
            {"expression": "6 * 7", "returnByValue": True},
        )
        assert_equal(
            peer_value.get("result", {}).get("value"),
            42,
            "peer session after rejected foreign identifiers",
        )

        await page_a.goto(
            f"{fixture}/plain?multi-page-object=a-replaced",
            wait_until="load",
        )
        stale_object_error = await expect_protocol_error(
            session_a.send("Runtime.getProperties", {"objectId": object_id}),
            "old objectId after its owning Document was replaced",
        )
        assert_equal(
            await page_b.evaluate("() => location.search"),
            "?multi-page-object=b",
            "peer target after owner Document replacement",
        )
        record_contract(
            results,
            "multi_page_remote_object_and_execution_context_ownership",
            contract=(
                "RemoteObject and executionContext identifiers are target-local; "
                "foreign and stale identifiers fail without poisoning either session."
            ),
            source="Chromium CDP oracle",
            commands=[
                "Runtime.evaluate",
                "Runtime.getProperties",
                "Page.createIsolatedWorld",
            ],
            observed={
                "foreignObjectRejected": bool(foreign_object_error),
                "foreignContextRejected": bool(foreign_context_error),
                "staleObjectRejected": bool(stale_object_error),
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _target_local_device_metrics(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context(viewport={"width": 800, "height": 600})
    sessions: list[Any] = []
    try:
        page_a, page_b = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            page_a.goto(f"{fixture}/plain?multi-page-metrics=a", wait_until="load"),
            page_b.goto(f"{fixture}/plain?multi-page-metrics=b", wait_until="load"),
        )
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page_a),
            context.new_cdp_session(page_b),
        )
        sessions = [session_a, session_b]
        await asyncio.gather(
            session_a.send(
                "Emulation.setDeviceMetricsOverride",
                {
                    "width": 420,
                    "height": 310,
                    "deviceScaleFactor": 2,
                    "mobile": False,
                },
            ),
            session_b.send(
                "Emulation.setDeviceMetricsOverride",
                {
                    "width": 530,
                    "height": 390,
                    "deviceScaleFactor": 1.5,
                    "mobile": False,
                },
            ),
        )
        expected = [[420, 310, 2], [530, 390, 1.5]]
        before_navigation = await asyncio.gather(
            _viewport_metrics(page_a),
            _viewport_metrics(page_b),
        )
        assert_equal(before_navigation, expected, "target-local device metrics")

        await asyncio.gather(
            page_a.goto(
                f"{fixture}/plain?multi-page-metrics=a-replaced",
                wait_until="load",
            ),
            page_b.goto(
                f"{fixture}/plain?multi-page-metrics=b-replaced",
                wait_until="load",
            ),
        )
        after_navigation = await asyncio.gather(
            _viewport_metrics(page_a),
            _viewport_metrics(page_b),
        )
        assert_equal(
            after_navigation,
            expected,
            "target-local device metrics after concurrent Document replacement",
        )

        await session_a.send("Emulation.clearDeviceMetricsOverride")
        cleared_a, unchanged_b = await asyncio.gather(
            _viewport_metrics(page_a),
            _viewport_metrics(page_b),
        )
        if cleared_a[:2] == expected[0][:2] or cleared_a[2] != 1:
            raise SmokeError(
                "clearing page A metrics did not restore its unscaled viewport: "
                f"{cleared_a!r}"
            )
        assert_equal(
            unchanged_b,
            expected[1],
            "clearing page A metrics does not mutate page B",
        )
        record_contract(
            results,
            "multi_page_target_local_device_metrics",
            contract=(
                "Device metrics are target-local, survive cross-Document navigation, "
                "and clearing one target does not mutate a peer."
            ),
            source="Chromium CDP oracle",
            commands=[
                "Emulation.setDeviceMetricsOverride",
                "Emulation.clearDeviceMetricsOverride",
            ],
            observed={"before": before_navigation, "after": after_navigation},
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _new_document_script_session_lifecycle(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page_a, page_b = await asyncio.gather(context.new_page(), context.new_page())
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page_a),
            context.new_cdp_session(page_b),
        )
        sessions = [session_a, session_b]
        await asyncio.gather(
            session_a.send("Page.enable"),
            session_b.send("Page.enable"),
        )
        script_a, script_b = await asyncio.gather(
            session_a.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": "globalThis.__multiPageInit = 'page-a';"},
            ),
            session_b.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": "globalThis.__multiPageInit = 'page-b';"},
            ),
        )
        identifier_b = script_b.get("identifier")
        if not isinstance(script_a.get("identifier"), str) or not isinstance(
            identifier_b, str
        ):
            raise SmokeError(
                "new-Document script registration returned invalid identifiers: "
                f"{script_a!r}, {script_b!r}"
            )

        await asyncio.gather(
            page_a.goto(f"{fixture}/plain?multi-page-init=a", wait_until="load"),
            page_b.goto(f"{fixture}/plain?multi-page-init=b", wait_until="load"),
        )
        assert_equal(
            await asyncio.gather(
                page_a.evaluate("globalThis.__multiPageInit"),
                page_b.evaluate("globalThis.__multiPageInit"),
            ),
            ["page-a", "page-b"],
            "target-local new-Document scripts",
        )

        await session_a.detach()
        sessions.remove(session_a)
        detached_live_child_value = await page_a.evaluate(
            """() => new Promise(resolve => {
              const frame = document.createElement('iframe');
              frame.srcdoc = '<!doctype html><body>detached-session-child</body>';
              frame.onload = () => resolve(
                frame.contentWindow.__multiPageInit ?? null
              );
              document.body.append(frame);
            })"""
        )
        assert_equal(
            detached_live_child_value,
            None,
            "detaching owner session removes script from the live Page registry",
        )
        await asyncio.gather(
            page_a.goto(
                f"{fixture}/plain?multi-page-init=a-after-detach",
                wait_until="load",
            ),
            page_b.goto(
                f"{fixture}/plain?multi-page-init=b-next",
                wait_until="load",
            ),
        )
        assert_equal(
            await page_a.evaluate("globalThis.__multiPageInit"),
            None,
            "detaching owner session removes its new-Document script",
        )
        assert_equal(
            await page_b.evaluate("globalThis.__multiPageInit"),
            "page-b",
            "peer session script survives owner detach",
        )

        await session_b.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            {"identifier": identifier_b},
        )
        await page_b.goto(
            f"{fixture}/plain?multi-page-init=b-removed",
            wait_until="load",
        )
        assert_equal(
            await page_b.evaluate("globalThis.__multiPageInit"),
            None,
            "explicitly removed new-Document script",
        )

        replacement_session = await context.new_cdp_session(page_a)
        sessions.append(replacement_session)
        replacement_value = await replacement_session.send(
            "Runtime.evaluate",
            {"expression": "40 + 2", "returnByValue": True},
        )
        assert_equal(
            replacement_value.get("result", {}).get("value"),
            42,
            "replacement session after script-owner detach",
        )
        record_contract(
            results,
            "multi_page_new_document_script_session_lifecycle",
            contract=(
                "Page.addScriptToEvaluateOnNewDocument is session- and target-local; "
                "it persists across navigation until removed or its owner detaches."
            ),
            source="Chromium CDP oracle",
            commands=[
                "Page.addScriptToEvaluateOnNewDocument",
                "Page.removeScriptToEvaluateOnNewDocument",
            ],
            observed={
                "targetValues": ["page-a", "page-b"],
                "removedFromLivePage": True,
                "removedOnDetach": True,
                "removedExplicitly": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _same_target_new_document_script_session_ownership(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        await asyncio.gather(
            session_a.send("Page.enable"),
            session_b.send("Page.enable"),
        )
        script_a, script_b = await asyncio.gather(
            session_a.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": "globalThis.__multiPageSessionA = 'session-a';"},
            ),
            session_b.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": "globalThis.__multiPageSessionB = 'session-b';"},
            ),
        )
        identifier_a = script_a.get("identifier")
        identifier_b = script_b.get("identifier")
        if not isinstance(identifier_a, str) or not isinstance(identifier_b, str):
            raise SmokeError(
                "same-target new-Document script registration returned invalid "
                f"identifiers: {script_a!r}, {script_b!r}"
            )

        await page.goto(
            f"{fixture}/plain?multi-page-session-scripts=both",
            wait_until="load",
        )
        assert_equal(
            await _same_target_session_script_values(page),
            ["session-a", "session-b"],
            "same-target scripts from two CDP sessions",
        )

        await session_a.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            {"identifier": identifier_a},
        )
        await page.goto(
            f"{fixture}/plain?multi-page-session-scripts=a-removed",
            wait_until="load",
        )
        assert_equal(
            await _same_target_session_script_values(page),
            [None, "session-b"],
            "removing one session's script preserves its same-target peer",
        )

        await session_b.detach()
        sessions.remove(session_b)
        detached_live_child_value = await page.evaluate(
            """() => new Promise(resolve => {
              const frame = document.createElement('iframe');
              frame.srcdoc = '<!doctype html><body>same-target-detached-child</body>';
              frame.onload = () => resolve(
                frame.contentWindow.__multiPageSessionB ?? null
              );
              document.body.append(frame);
            })"""
        )
        assert_equal(
            detached_live_child_value,
            None,
            "detaching one same-target session updates the live Page registry",
        )
        await page.goto(
            f"{fixture}/plain?multi-page-session-scripts=both-gone",
            wait_until="load",
        )
        assert_equal(
            await _same_target_session_script_values(page),
            [None, None],
            "same-target session-owned scripts after remove and detach",
        )

        replacement_session = await context.new_cdp_session(page)
        sessions.append(replacement_session)
        replacement_value = await replacement_session.send(
            "Runtime.evaluate",
            {"expression": "21 * 2", "returnByValue": True},
        )
        assert_equal(
            replacement_value.get("result", {}).get("value"),
            42,
            "replacement session after same-target script cleanup",
        )
        record_contract(
            results,
            "multi_page_same_target_new_document_script_session_ownership",
            contract=(
                "Multiple CDP sessions on one Page own independent new-Document "
                "scripts; remove and detach clean only the calling session's script."
            ),
            source="Chromium CDP oracle",
            commands=[
                "Page.addScriptToEvaluateOnNewDocument",
                "Page.removeScriptToEvaluateOnNewDocument",
                "Target.detachFromTarget",
            ],
            observed={
                "bothInitiallyInjected": True,
                "peerSurvivedRemove": True,
                "detachedScriptRemovedFromLivePage": True,
                "bothEventuallyRemoved": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _isolated_world_script_detach_lifecycle(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        owner_session = await context.new_cdp_session(page)
        sessions = [owner_session]
        await owner_session.send("Page.enable")
        registration = await owner_session.send(
            "Page.addScriptToEvaluateOnNewDocument",
            {
                "source": "globalThis.__multiPageUtilityInit = 'utility-ready';",
                "worldName": "multi-page-utility",
            },
        )
        if not isinstance(registration.get("identifier"), str):
            raise SmokeError(
                "isolated-world script registration returned no identifier: "
                f"{registration!r}"
            )
        await page.goto(
            f"{fixture}/plain?multi-page-utility-script=owner",
            wait_until="load",
        )
        owner_context_id = await _create_isolated_world(
            owner_session,
            "multi-page-utility",
        )
        await owner_session.send(
            "Runtime.addBinding",
            {
                "name": "multiPageUtilityBinding",
                "executionContextId": owner_context_id,
            },
        )
        assert_equal(
            await _evaluate_context_value(
                owner_session,
                owner_context_id,
                "[globalThis.__multiPageUtilityInit ?? null, "
                "typeof globalThis.multiPageUtilityBinding]",
            ),
            ["utility-ready", "function"],
            "isolated-world session state before owner detach",
        )

        await owner_session.detach()
        sessions.remove(owner_session)
        replacement_session = await context.new_cdp_session(page)
        sessions.append(replacement_session)
        await replacement_session.send("Page.enable")
        live_context_id = await _create_isolated_world(
            replacement_session,
            "multi-page-utility",
        )
        assert_equal(
            await _evaluate_context_value(
                replacement_session,
                live_context_id,
                "[globalThis.__multiPageUtilityInit ?? null, "
                "typeof globalThis.multiPageUtilityBinding]",
            ),
            [None, "undefined"],
            "detach retires the prior session's live isolated world and binding",
        )

        await page.goto(
            f"{fixture}/plain?multi-page-utility-script=after-detach",
            wait_until="load",
        )
        future_context_id = await _create_isolated_world(
            replacement_session,
            "multi-page-utility",
        )
        assert_equal(
            await _evaluate_context_value(
                replacement_session,
                future_context_id,
                "[globalThis.__multiPageUtilityInit ?? null, "
                "typeof globalThis.multiPageUtilityBinding]",
            ),
            [None, "undefined"],
            "detached session's isolated-world state is absent from a future Document",
        )
        record_contract(
            results,
            "multi_page_isolated_world_script_detach_lifecycle",
            contract=(
                "Detaching a CDP session retires its named isolated world, binding, and "
                "future-Document script without contaminating a replacement session."
            ),
            source="Chromium CDP oracle",
            commands=[
                "Page.addScriptToEvaluateOnNewDocument",
                "Page.createIsolatedWorld",
                "Runtime.addBinding",
                "Target.detachFromTarget",
            ],
            observed={
                "replacementLiveWorldState": [None, "undefined"],
                "replacementFutureDocumentState": [None, "undefined"],
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _same_name_isolated_worlds_are_session_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        await asyncio.gather(
            session_a.send("Page.enable"),
            session_b.send("Page.enable"),
        )
        await asyncio.gather(
            session_a.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {
                    "source": "globalThis.__multiPageWorldA = 'session-a';",
                    "worldName": "multi-page-shared-name",
                },
            ),
            session_b.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {
                    "source": "globalThis.__multiPageWorldB = 'session-b';",
                    "worldName": "multi-page-shared-name",
                },
            ),
        )
        await page.goto(
            f"{fixture}/iframe?multi-page-worlds=initial",
            wait_until="load",
        )
        frame_ids = await _root_and_child_frame_ids(session_a)
        contexts_a = await _create_isolated_worlds_for_frames(
            session_a,
            frame_ids,
            "multi-page-shared-name",
        )
        contexts_b = await _create_isolated_worlds_for_frames(
            session_b,
            frame_ids,
            "multi-page-shared-name",
        )
        if any(left == right for left, right in zip(contexts_a, contexts_b, strict=True)):
            raise SmokeError(
                "same-name isolated worlds reused an execution context across sessions: "
                f"{contexts_a!r} vs {contexts_b!r}"
            )
        expression = (
            "[globalThis.__multiPageWorldA ?? null, "
            "globalThis.__multiPageWorldB ?? null]"
        )
        assert_equal(
            await asyncio.gather(
                *(
                    _evaluate_context_value(session_a, context_id, expression)
                    for context_id in contexts_a
                )
            ),
            [["session-a", None], ["session-a", None]],
            "session A owns distinct root and child worlds",
        )
        assert_equal(
            await asyncio.gather(
                *(
                    _evaluate_context_value(session_b, context_id, expression)
                    for context_id in contexts_b
                )
            ),
            [[None, "session-b"], [None, "session-b"]],
            "session B owns distinct root and child worlds",
        )

        await session_a.detach()
        sessions.remove(session_a)
        replacement_session = await context.new_cdp_session(page)
        sessions.append(replacement_session)
        await replacement_session.send("Page.enable")
        replacement_contexts = await _create_isolated_worlds_for_frames(
            replacement_session,
            frame_ids,
            "multi-page-shared-name",
        )
        assert_equal(
            await asyncio.gather(
                *(
                    _evaluate_context_value(
                        replacement_session,
                        context_id,
                        expression,
                    )
                    for context_id in replacement_contexts
                )
            ),
            [[None, None], [None, None]],
            "replacement session receives clean same-name root and child worlds",
        )
        assert_equal(
            await asyncio.gather(
                *(
                    _evaluate_context_value(session_b, context_id, expression)
                    for context_id in contexts_b
                )
            ),
            [[None, "session-b"], [None, "session-b"]],
            "detaching session A does not retire session B worlds",
        )

        await page.goto(
            f"{fixture}/iframe?multi-page-worlds=future",
            wait_until="load",
        )
        future_frame_ids = await _root_and_child_frame_ids(session_b)
        future_b_contexts = await _create_isolated_worlds_for_frames(
            session_b,
            future_frame_ids,
            "multi-page-shared-name",
        )
        future_replacement_contexts = await _create_isolated_worlds_for_frames(
            replacement_session,
            future_frame_ids,
            "multi-page-shared-name",
        )
        assert_equal(
            await asyncio.gather(
                *(
                    _evaluate_context_value(session_b, context_id, expression)
                    for context_id in future_b_contexts
                )
            ),
            [[None, "session-b"], [None, "session-b"]],
            "surviving session script owns future root and child worlds",
        )
        assert_equal(
            await asyncio.gather(
                *(
                    _evaluate_context_value(
                        replacement_session,
                        context_id,
                        expression,
                    )
                    for context_id in future_replacement_contexts
                )
            ),
            [[None, None], [None, None]],
            "replacement session remains clean in a future Document",
        )
        record_contract(
            results,
            "multi_page_same_name_isolated_world_session_ownership",
            contract=(
                "Named isolated worlds are keyed by CDP session, frame, and Document; "
                "same-name preload scripts never share a realm across sessions."
            ),
            source="Chromium InspectorPageAgent oracle",
            commands=[
                "Page.addScriptToEvaluateOnNewDocument",
                "Page.createIsolatedWorld",
                "Target.detachFromTarget",
            ],
            observed={
                "rootAndChildContextsDistinct": True,
                "replacementWorldsClean": True,
                "peerWorldsSurvivedDetach": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _runtime_binding_session_lifecycle(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        await page.goto(
            f"{fixture}/plain?multi-page-bindings=initial",
            wait_until="load",
        )
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        events_a = attach_cdp_event_collector(session_a, ["Runtime.bindingCalled"])
        events_b = attach_cdp_event_collector(session_b, ["Runtime.bindingCalled"])
        await asyncio.gather(
            session_a.send("Runtime.enable"),
            session_b.send("Runtime.enable"),
        )
        await asyncio.gather(
            session_a.send("Runtime.addBinding", {"name": "multiPageBindingA"}),
            session_b.send("Runtime.addBinding", {"name": "multiPageBindingB"}),
        )
        assert_equal(
            await page.evaluate(
                "() => [typeof multiPageBindingA, typeof multiPageBindingB]"
            ),
            ["function", "function"],
            "both session bindings share the current default world",
        )
        await page.evaluate(
            "() => { multiPageBindingA('a-before'); multiPageBindingB('b-before'); }"
        )
        await wait_until(
            lambda: _binding_payloads(events_a, "multiPageBindingA") == ["a-before"]
            and _binding_payloads(events_b, "multiPageBindingB") == ["b-before"],
            "session-local Runtime.bindingCalled delivery",
        )
        assert_equal(
            _binding_payloads(events_a, "multiPageBindingB"),
            [],
            "session A does not receive session B binding events",
        )
        assert_equal(
            _binding_payloads(events_b, "multiPageBindingA"),
            [],
            "session B does not receive session A binding events",
        )

        await session_a.detach()
        sessions.remove(session_a)
        events_b.clear()
        assert_equal(
            await page.evaluate(
                "() => [typeof multiPageBindingA, typeof multiPageBindingB]"
            ),
            ["function", "function"],
            "detach does not delete an already-installed global binding function",
        )
        await page.evaluate(
            "() => { multiPageBindingA('a-detached'); multiPageBindingB('b-live'); }"
        )
        await wait_until(
            lambda: _binding_payloads(events_b, "multiPageBindingB") == ["b-live"],
            "surviving binding event after peer detach",
        )
        assert_equal(
            _binding_payloads(events_b, "multiPageBindingA"),
            [],
            "detached binding function has no surviving session subscriber",
        )

        detached_child_bindings = await page.evaluate(
            """() => new Promise(resolve => {
              const frame = document.createElement('iframe');
              frame.srcdoc = '<!doctype html><body>binding-child</body>';
              frame.onload = () => resolve([
                typeof frame.contentWindow.multiPageBindingA,
                typeof frame.contentWindow.multiPageBindingB,
              ]);
              document.body.append(frame);
            })"""
        )
        assert_equal(
            detached_child_bindings,
            ["undefined", "function"],
            "a child realm created after detach receives only live-session bindings",
        )

        await page.goto(
            f"{fixture}/plain?multi-page-bindings=future",
            wait_until="load",
        )
        assert_equal(
            await page.evaluate(
                "() => [typeof multiPageBindingA, typeof multiPageBindingB]"
            ),
            ["undefined", "function"],
            "only the surviving session binding is installed in a future Document",
        )
        events_b.clear()
        await page.evaluate("() => multiPageBindingB('b-future')")
        await wait_until(
            lambda: _binding_payloads(events_b, "multiPageBindingB") == ["b-future"],
            "surviving binding event in a future Document",
        )
        record_contract(
            results,
            "multi_page_runtime_binding_session_lifecycle",
            contract=(
                "Runtime bindings share a live default-world property but keep session-local "
                "event authority; detach drops future registration without deleting the "
                "already-installed function."
            ),
            source="Chromium V8RuntimeAgentImpl oracle",
            commands=[
                "Runtime.addBinding",
                "Runtime.bindingCalled",
                "Target.detachFromTarget",
            ],
            observed={
                "currentGlobalAfterDetach": ["function", "function"],
                "newChildAfterDetach": ["undefined", "function"],
                "futureGlobalAfterDetach": ["undefined", "function"],
                "bindingEventsSessionLocal": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _same_target_remote_objects_are_session_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        await page.goto(
            f"{fixture}/plain?multi-page-session-objects",
            wait_until="load",
        )
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        contexts_a = attach_cdp_event_collector(
            session_a, ["Runtime.executionContextCreated"]
        )
        contexts_b = attach_cdp_event_collector(
            session_b, ["Runtime.executionContextCreated"]
        )
        await asyncio.gather(
            session_a.send("Runtime.enable"),
            session_b.send("Runtime.enable"),
        )
        await asyncio.gather(
            wait_until(
                lambda: _default_execution_context_id(contexts_a) is not None,
                "session A default execution context",
            ),
            wait_until(
                lambda: _default_execution_context_id(contexts_b) is not None,
                "session B default execution context",
            ),
        )
        context_id_a = _default_execution_context_id(contexts_a)
        context_id_b = _default_execution_context_id(contexts_b)
        assert_equal(
            context_id_a,
            context_id_b,
            "same-target execution context IDs are shared across Runtime sessions",
        )
        cross_session_context = await session_b.send(
            "Runtime.evaluate",
            {
                "expression": "6 * 7",
                "contextId": context_id_a,
                "returnByValue": True,
            },
        )
        assert_equal(
            cross_session_context.get("result", {}).get("value"),
            42,
            "peer session evaluates in the target-scoped execution context",
        )
        remote_a = (
            await session_a.send(
                "Runtime.evaluate",
                {"expression": "({session: 'a', value: 21})"},
            )
        ).get("result", {})
        object_a = remote_a.get("objectId")
        if not isinstance(object_a, str) or not object_a:
            raise SmokeError(f"session A returned no objectId: {remote_a!r}")

        foreign_error = await expect_protocol_error(
            session_b.send("Runtime.getProperties", {"objectId": object_a}),
            "same-target session B resolving session A objectId",
        )
        own_properties = await session_a.send(
            "Runtime.getProperties",
            {"objectId": object_a, "ownProperties": True},
        )
        assert_equal(
            {
                prop.get("name")
                for prop in own_properties.get("result", [])
                if prop.get("enumerable")
            },
            {"session", "value"},
            "same-target object owner resolves its handle",
        )
        await session_a.send("Runtime.releaseObject", {"objectId": object_a})
        released_error = await expect_protocol_error(
            session_a.send("Runtime.getProperties", {"objectId": object_a}),
            "released same-target objectId",
        )

        grouped_a, grouped_b = await asyncio.gather(
            session_a.send(
                "Runtime.evaluate",
                {
                    "expression": "({groupOwner: 'a'})",
                    "objectGroup": "shared-name-group",
                },
            ),
            session_b.send(
                "Runtime.evaluate",
                {
                    "expression": "({groupOwner: 'b'})",
                    "objectGroup": "shared-name-group",
                },
            ),
        )
        grouped_object_a = grouped_a.get("result", {}).get("objectId")
        grouped_object_b = grouped_b.get("result", {}).get("objectId")
        if not isinstance(grouped_object_a, str) or not isinstance(grouped_object_b, str):
            raise SmokeError(
                "same-name object groups returned invalid handles: "
                f"{grouped_a!r}, {grouped_b!r}"
            )
        await session_a.send(
            "Runtime.releaseObjectGroup",
            {"objectGroup": "shared-name-group"},
        )
        released_group_error = await expect_protocol_error(
            session_a.send("Runtime.getProperties", {"objectId": grouped_object_a}),
            "released owner object group",
        )
        peer_group_properties = await session_b.send(
            "Runtime.getProperties",
            {"objectId": grouped_object_b, "ownProperties": True},
        )
        assert_equal(
            next(
                prop.get("value", {}).get("value")
                for prop in peer_group_properties.get("result", [])
                if prop.get("name") == "groupOwner"
            ),
            "b",
            "same-name peer object group survives owner release",
        )

        await session_a.detach()
        sessions.remove(session_a)
        peer = await session_b.send(
            "Runtime.evaluate",
            {"expression": "6 * 7", "returnByValue": True},
        )
        assert_equal(
            peer.get("result", {}).get("value"),
            42,
            "peer Runtime session after object owner detach",
        )
        record_contract(
            results,
            "multi_page_same_target_remote_object_session_ownership",
            contract=(
                "Execution context IDs are target-scoped, while remote objects and object "
                "groups are Runtime-session scoped; release and detach cannot damage a peer."
            ),
            source="Chromium V8 Inspector oracle",
            commands=[
                "Runtime.evaluate",
                "Runtime.getProperties",
                "Runtime.releaseObject",
                "Runtime.releaseObjectGroup",
                "Target.detachFromTarget",
            ],
            observed={
                "sharedExecutionContextId": context_id_a,
                "crossSessionContextEvaluation": 42,
                "foreignObjectRejected": bool(foreign_error),
                "releasedObjectRejected": bool(released_error),
                "releasedGroupRejected": bool(released_group_error),
                "peerSameNameGroupSurvived": True,
                "peerValueAfterDetach": 42,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _same_target_dom_node_and_backend_node_ownership(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        await page.goto(
            f"{fixture}/plain?multi-page-dom-session-ownership",
            wait_until="load",
        )
        await page.evaluate(
            """() => {
              document.body.innerHTML =
                '<main><div id="owner-node">owner</div><div id="peer-node">peer</div></main>';
            }"""
        )
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        await asyncio.gather(
            session_a.send("DOM.enable"),
            session_b.send("DOM.enable"),
            session_a.send("Runtime.enable"),
            session_b.send("Runtime.enable"),
        )
        document_a, document_b = await asyncio.gather(
            session_a.send("DOM.getDocument", {"depth": -1}),
            session_b.send("DOM.getDocument", {"depth": 0}),
        )
        root_a = document_a.get("root", {}).get("nodeId")
        root_b = document_b.get("root", {}).get("nodeId")
        if not isinstance(root_a, int) or not isinstance(root_b, int):
            raise SmokeError(
                f"DOM.getDocument returned invalid roots: {document_a!r}, {document_b!r}"
            )
        owner_query = await session_a.send(
            "DOM.querySelector",
            {"nodeId": root_a, "selector": "#owner-node"},
        )
        owner_node_id = owner_query.get("nodeId")
        if not isinstance(owner_node_id, int) or owner_node_id <= 0:
            raise SmokeError(f"DOM.querySelector returned no owner node: {owner_query!r}")
        owner_description = await session_a.send(
            "DOM.describeNode",
            {"nodeId": owner_node_id},
        )
        backend_node_id = owner_description.get("node", {}).get("backendNodeId")
        if not isinstance(backend_node_id, int) or backend_node_id <= 0:
            raise SmokeError(
                f"DOM.describeNode returned no backendNodeId: {owner_description!r}"
            )

        foreign_node_error = await expect_protocol_error(
            session_b.send("DOM.describeNode", {"nodeId": owner_node_id}),
            "unpublished DOM nodeId in peer session",
        )
        resolved = await session_b.send(
            "DOM.resolveNode",
            {"backendNodeId": backend_node_id},
        )
        peer_object_id = resolved.get("object", {}).get("objectId")
        if not isinstance(peer_object_id, str):
            raise SmokeError(f"DOM.resolveNode returned no objectId: {resolved!r}")
        text_value = await session_b.send(
            "Runtime.callFunctionOn",
            {
                "objectId": peer_object_id,
                "functionDeclaration": "function() { return this.textContent; }",
                "returnByValue": True,
            },
        )
        assert_equal(
            text_value.get("result", {}).get("value"),
            "owner",
            "peer resolves target-scoped backendNodeId",
        )
        foreign_object_error = await expect_protocol_error(
            session_a.send("Runtime.getProperties", {"objectId": peer_object_id}),
            "DOM-resolved peer Runtime objectId",
        )

        await session_a.detach()
        sessions.remove(session_a)
        peer_after_detach = await session_b.send(
            "Runtime.callFunctionOn",
            {
                "objectId": peer_object_id,
                "functionDeclaration": "function() { return this.id; }",
                "returnByValue": True,
            },
        )
        assert_equal(
            peer_after_detach.get("result", {}).get("value"),
            "owner-node",
            "peer DOM-resolved object after other DOM session detaches",
        )
        record_contract(
            results,
            "multi_page_same_target_dom_node_session_ownership",
            contract=(
                "DOM nodeId mappings and resolved Runtime handles are session-local, while "
                "backendNodeId identifies a live node across sessions on the same target."
            ),
            source="Chromium InspectorDOMAgent oracle",
            commands=[
                "DOM.getDocument",
                "DOM.querySelector",
                "DOM.describeNode",
                "DOM.resolveNode",
                "Runtime.callFunctionOn",
                "Target.detachFromTarget",
            ],
            observed={
                "unpublishedForeignNodeRejected": bool(foreign_node_error),
                "backendNodeResolvedByPeer": True,
                "resolvedObjectRejectedByForeignSession": bool(foreign_object_error),
                "peerObjectSurvivedDetach": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _same_source_scripts_keep_session_authority(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        await asyncio.gather(
            session_a.send("Page.enable"),
            session_b.send("Page.enable"),
        )
        source = "globalThis.__sameSourceRuns = (globalThis.__sameSourceRuns || 0) + 1;"
        registration_a, registration_b = await asyncio.gather(
            session_a.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": source},
            ),
            session_b.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": source},
            ),
        )
        identifier_a = registration_a.get("identifier")
        identifier_b = registration_b.get("identifier")
        if not isinstance(identifier_a, str) or not isinstance(identifier_b, str):
            raise SmokeError(
                "same-source registrations returned invalid identifiers: "
                f"{registration_a!r}, {registration_b!r}"
            )
        assert_equal(
            identifier_a,
            identifier_b,
            "Page script identifiers use a session-local namespace",
        )

        await page.goto(
            f"{fixture}/plain?multi-page-same-source=both",
            wait_until="load",
        )
        assert_equal(
            await page.evaluate("globalThis.__sameSourceRuns ?? 0"),
            2,
            "identical scripts from two sessions both execute",
        )
        await session_b.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            {"identifier": identifier_a},
        )
        await page.goto(
            f"{fixture}/plain?multi-page-same-source=b-remove",
            wait_until="load",
        )
        assert_equal(
            await page.evaluate("globalThis.__sameSourceRuns ?? 0"),
            1,
            "colliding identifier removes only session B's registration",
        )
        repeated_remove_error = await expect_protocol_error(
            session_b.send(
                "Page.removeScriptToEvaluateOnNewDocument",
                {"identifier": identifier_a},
            ),
            "session B removing its already-removed script identifier",
        )

        await session_a.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            {"identifier": identifier_a},
        )
        await page.goto(
            f"{fixture}/plain?multi-page-same-source=owner-remove",
            wait_until="load",
        )
        assert_equal(
            await page.evaluate("globalThis.__sameSourceRuns ?? 0"),
            0,
            "session A can still remove its colliding identifier",
        )

        duplicate_source = (
            "globalThis.__sameSessionRuns = (globalThis.__sameSessionRuns || 0) + 1;"
        )
        duplicate_a, duplicate_b = await asyncio.gather(
            session_a.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": duplicate_source},
            ),
            session_a.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": duplicate_source},
            ),
        )
        assert_equal(
            [duplicate_a.get("identifier"), duplicate_b.get("identifier")],
            ["1", "2"],
            "same-session duplicate sources keep distinct identifiers",
        )
        await page.goto(
            f"{fixture}/plain?multi-page-same-source=same-session",
            wait_until="load",
        )
        assert_equal(
            await page.evaluate("globalThis.__sameSessionRuns ?? 0"),
            2,
            "same-session duplicate registrations both execute",
        )
        await asyncio.gather(
            *(
                session_a.send(
                    "Page.removeScriptToEvaluateOnNewDocument",
                    {"identifier": identifier},
                )
                for identifier in ("1", "2")
            )
        )
        reused = await session_a.send(
            "Page.addScriptToEvaluateOnNewDocument",
            {"source": "globalThis.__reusedScriptIdentifier = true;"},
        )
        assert_equal(
            reused.get("identifier"),
            "1",
            "empty session script registry reuses identifier one",
        )
        await session_a.send(
            "Page.removeScriptToEvaluateOnNewDocument",
            {"identifier": "1"},
        )
        await session_b.detach()
        sessions.remove(session_b)
        await page.goto(
            f"{fixture}/plain?multi-page-same-source=detached",
            wait_until="load",
        )
        assert_equal(
            await page.evaluate("globalThis.__sameSourceRuns ?? 0"),
            0,
            "detaching the final owner removes its identical script",
        )
        record_contract(
            results,
            "multi_page_same_source_script_session_authority",
            contract=(
                "Page script identifiers are session-local and may collide; removing an "
                "identifier affects only the registration owned by that session."
            ),
            source="Chromium InspectorPageAgent oracle",
            commands=[
                "Page.addScriptToEvaluateOnNewDocument",
                "Page.removeScriptToEvaluateOnNewDocument",
                "Target.detachFromTarget",
            ],
            observed={
                "identifierCollision": identifier_a,
                "repeatedRemoveRejected": bool(repeated_remove_error),
                "executionCounts": [2, 1, 0, 0],
                "sameSessionDuplicateIdentifiers": ["1", "2"],
                "identifierAfterEmptyRegistry": "1",
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _network_enablement_is_session_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        events_a = attach_cdp_event_collector(
            session_a,
            ["Network.responseReceived", "Network.loadingFinished"],
        )
        events_b = attach_cdp_event_collector(
            session_b,
            ["Network.responseReceived", "Network.loadingFinished"],
        )
        await asyncio.gather(
            session_a.send("Network.enable"),
            session_b.send("Network.enable"),
        )
        first_url = f"{fixture}/plain?multi-page-network-sessions=both"
        await page.goto(first_url, wait_until="load")
        request_a = _document_request_id(events_a, first_url)
        request_b = _document_request_id(events_b, first_url)

        events_a.clear()
        events_b.clear()
        await session_a.send("Network.disable")
        disabled_cache_error = await expect_protocol_error(
            session_a.send("Network.getResponseBody", {"requestId": request_a}),
            "disabled Network session response-body cache",
        )
        retained_body_b = await session_b.send(
            "Network.getResponseBody",
            {"requestId": request_b},
        )
        if "plain ok" not in retained_body_b.get("body", ""):
            raise SmokeError(
                "peer Network response-body cache was cleared by another session: "
                f"{retained_body_b!r}"
            )
        second_url = f"{fixture}/plain?multi-page-network-sessions=b-only"
        await page.goto(second_url, wait_until="load")
        await wait_until(
            lambda: _has_document_response(events_b, second_url),
            "surviving Network session after peer disable",
        )
        await asyncio.sleep(0.1)
        assert_equal(
            _has_document_response(events_a, second_url),
            False,
            "Network.disable silences only its own session",
        )
        request_b_only = _document_request_id(events_b, second_url)
        body_b = await session_b.send(
            "Network.getResponseBody",
            {"requestId": request_b_only},
        )
        if "plain ok" not in body_b.get("body", ""):
            raise SmokeError(f"surviving Network session returned wrong body: {body_b!r}")

        await session_a.send("Network.enable")
        events_a.clear()
        events_b.clear()
        third_url = f"{fixture}/plain?multi-page-network-sessions=reenabled"
        await page.goto(third_url, wait_until="load")
        await asyncio.gather(
            wait_until(
                lambda: _has_document_response(events_a, third_url),
                "re-enabled Network session",
            ),
            wait_until(
                lambda: _has_document_response(events_b, third_url),
                "continuously enabled Network session",
            ),
        )
        record_contract(
            results,
            "multi_page_same_target_network_enablement_isolation",
            contract=(
                "Network.enable and Network.disable are per DevTools session on one Page; "
                "disabling one observer neither silences nor invalidates the peer observer."
            ),
            source="Chromium InspectorNetworkAgent oracle",
            commands=["Network.enable", "Network.disable", "Network.getResponseBody"],
            observed={
                "initialRequestIds": [request_a, request_b],
                "disabledSessionSilent": True,
                "disabledSessionCacheCleared": bool(disabled_cache_error),
                "peerBodyRetained": True,
                "reenabledFanout": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _page_lifecycle_enablement_is_session_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        events_a = attach_cdp_event_collector(session_a, ["Page.lifecycleEvent"])
        events_b = attach_cdp_event_collector(session_b, ["Page.lifecycleEvent"])
        await asyncio.gather(
            session_a.send("Page.enable"),
            session_b.send("Page.enable"),
        )
        await asyncio.gather(
            session_a.send("Page.setLifecycleEventsEnabled", {"enabled": True}),
            session_b.send("Page.setLifecycleEventsEnabled", {"enabled": True}),
        )
        events_a.clear()
        events_b.clear()
        await page.goto(
            f"{fixture}/plain?multi-page-lifecycle=both",
            wait_until="load",
        )
        await asyncio.gather(
            wait_until(
                lambda: _has_lifecycle_event(events_a, "load"),
                "session A Page.lifecycleEvent load",
            ),
            wait_until(
                lambda: _has_lifecycle_event(events_b, "load"),
                "session B Page.lifecycleEvent load",
            ),
        )

        await session_a.send(
            "Page.setLifecycleEventsEnabled",
            {"enabled": False},
        )
        events_a.clear()
        events_b.clear()
        await page.goto(
            f"{fixture}/plain?multi-page-lifecycle=b-only",
            wait_until="load",
        )
        await wait_until(
            lambda: _has_lifecycle_event(events_b, "load"),
            "surviving Page lifecycle observer",
        )
        await asyncio.sleep(0.1)
        assert_equal(
            _has_lifecycle_event(events_a, "load"),
            False,
            "disabling lifecycle events silences only one session",
        )

        await session_a.send(
            "Page.setLifecycleEventsEnabled",
            {"enabled": True},
        )
        await wait_until(
            lambda: _has_lifecycle_event(events_a, "load"),
            "re-enabled session current lifecycle snapshot",
        )
        events_a.clear()
        events_b.clear()
        await page.goto(
            f"{fixture}/plain?multi-page-lifecycle=reenabled",
            wait_until="load",
        )
        await asyncio.gather(
            wait_until(
                lambda: _has_lifecycle_event(events_a, "load"),
                "re-enabled Page lifecycle observer",
            ),
            wait_until(
                lambda: _has_lifecycle_event(events_b, "load"),
                "continuous Page lifecycle observer",
            ),
        )
        record_contract(
            results,
            "multi_page_same_target_lifecycle_event_enablement_isolation",
            contract=(
                "Page lifecycle event enablement is scoped to one DevTools session on a "
                "shared target; disable and re-enable do not alter a peer observer."
            ),
            source="Chromium InspectorPageAgent oracle",
            commands=["Page.enable", "Page.setLifecycleEventsEnabled", "Page.lifecycleEvent"],
            observed={
                "initialFanout": True,
                "disabledSessionSilent": True,
                "reenabledFanout": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _same_name_runtime_bindings_keep_session_authority(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        await page.goto(
            f"{fixture}/plain?multi-page-shared-binding=initial",
            wait_until="load",
        )
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        events_a = attach_cdp_event_collector(session_a, ["Runtime.bindingCalled"])
        events_b = attach_cdp_event_collector(session_b, ["Runtime.bindingCalled"])
        await asyncio.gather(
            session_a.send("Runtime.enable"),
            session_b.send("Runtime.enable"),
        )
        await asyncio.gather(
            session_a.send(
                "Runtime.addBinding",
                {"name": "multiPageSharedBinding"},
            ),
            session_b.send(
                "Runtime.addBinding",
                {"name": "multiPageSharedBinding"},
            ),
        )
        await page.evaluate("() => multiPageSharedBinding('both-live')")
        await wait_until(
            lambda: _binding_payloads(events_a, "multiPageSharedBinding")
            == ["both-live"]
            and _binding_payloads(events_b, "multiPageSharedBinding") == ["both-live"],
            "same-name binding fanout to both owners",
        )

        await session_a.send(
            "Runtime.removeBinding",
            {"name": "multiPageSharedBinding"},
        )
        events_a.clear()
        events_b.clear()
        await page.evaluate("() => multiPageSharedBinding('b-after-a-remove')")
        await wait_until(
            lambda: _binding_payloads(events_b, "multiPageSharedBinding")
            == ["b-after-a-remove"],
            "same-name binding surviving peer removeBinding",
        )
        await asyncio.sleep(0.1)
        assert_equal(
            _binding_payloads(events_a, "multiPageSharedBinding"),
            [],
            "Runtime.removeBinding silences only its owning session",
        )

        await session_a.detach()
        sessions.remove(session_a)
        events_b.clear()
        await page.evaluate("() => multiPageSharedBinding('b-survives')")
        await wait_until(
            lambda: _binding_payloads(events_b, "multiPageSharedBinding")
            == ["b-survives"],
            "same-name binding surviving owner after peer detach",
        )
        await page.goto(
            f"{fixture}/plain?multi-page-shared-binding=future",
            wait_until="load",
        )
        assert_equal(
            await page.evaluate("typeof multiPageSharedBinding"),
            "function",
            "surviving same-name binding in a future Document",
        )
        events_b.clear()
        await page.evaluate("() => multiPageSharedBinding('b-future')")
        await wait_until(
            lambda: _binding_payloads(events_b, "multiPageSharedBinding")
            == ["b-future"],
            "same-name binding surviving owner in a future Document",
        )
        record_contract(
            results,
            "multi_page_same_name_runtime_binding_session_authority",
            contract=(
                "Two Runtime sessions may own the same binding name; removeBinding and detach "
                "retire only one owner's events while the peer remains authoritative."
            ),
            source="Chromium V8RuntimeAgentImpl oracle",
            commands=[
                "Runtime.addBinding",
                "Runtime.removeBinding",
                "Runtime.bindingCalled",
                "Target.detachFromTarget",
            ],
            observed={
                "initialOwnersNotified": 2,
                "removedOwnerSilent": True,
                "survivingOwnerNotified": True,
                "futureDocumentBinding": "function",
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _runtime_enablement_and_context_replay_are_session_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        await page.goto(
            f"{fixture}/plain?multi-page-runtime-enablement",
            wait_until="load",
        )
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        events_a = attach_cdp_event_collector(
            session_a,
            ["Runtime.executionContextCreated"],
        )
        events_b = attach_cdp_event_collector(
            session_b,
            ["Runtime.executionContextCreated"],
        )

        await session_a.send("Runtime.enable")
        events_a.clear()
        events_b.clear()
        await page.evaluate(
            """() => new Promise(resolve => {
              const frame = document.createElement('iframe');
              frame.id = 'runtime-enabled-a';
              frame.srcdoc = '<!doctype html><body>runtime-a</body>';
              frame.onload = resolve;
              document.body.append(frame);
            })"""
        )
        await wait_until(
            lambda: bool(events_a),
            "Runtime-enabled session receives a new child context",
        )
        await asyncio.sleep(0.05)
        assert_equal(
            events_b,
            [],
            "Runtime-disabled peer receives no child context event",
        )

        await session_b.send("Runtime.enable")
        await wait_until(
            lambda: len(events_b) >= 2,
            "newly Runtime-enabled peer receives existing context replay",
        )
        replayed_context_count = len(events_b)
        events_a.clear()
        events_b.clear()

        await session_a.send("Runtime.disable")
        await page.evaluate(
            """() => new Promise(resolve => {
              const frame = document.createElement('iframe');
              frame.id = 'runtime-enabled-b';
              frame.srcdoc = '<!doctype html><body>runtime-b</body>';
              frame.onload = resolve;
              document.body.append(frame);
            })"""
        )
        await wait_until(
            lambda: bool(events_b),
            "surviving Runtime-enabled session receives the second child context",
        )
        await asyncio.sleep(0.05)
        assert_equal(
            events_a,
            [],
            "Runtime.disable silences only its owning session",
        )
        record_contract(
            results,
            "multi_page_runtime_enablement_and_context_replay",
            contract=(
                "Runtime domain enablement is session-local; enabling replays existing "
                "contexts, and disabling one session does not silence a peer."
            ),
            source="Chromium V8RuntimeAgentImpl oracle",
            commands=["Runtime.enable", "Runtime.disable"],
            observed={
                "disabledPeerInitiallySilent": True,
                "replayedContextCount": replayed_context_count,
                "disabledOwnerFinallySilent": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _session_detach_cancels_only_its_pending_await(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page = await context.new_page()
        await page.goto(
            f"{fixture}/plain?multi-page-pending-await",
            wait_until="load",
        )
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [session_a, session_b]
        pending = asyncio.create_task(
            session_a.send(
                "Runtime.evaluate",
                {
                    "expression": "new Promise(resolve => setTimeout(() => resolve('late'), 1000))",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            )
        )
        await asyncio.sleep(0.05)
        await session_a.detach()
        sessions.remove(session_a)
        canceled = (await asyncio.gather(pending, return_exceptions=True))[0]
        if not isinstance(canceled, BaseException):
            raise SmokeError(
                "detached Runtime session unexpectedly completed its pending await: "
                f"{canceled!r}"
            )
        peer = await asyncio.wait_for(
            session_b.send(
                "Runtime.evaluate",
                {"expression": "21 * 2", "returnByValue": True},
            ),
            timeout=2,
        )
        assert_equal(
            peer.get("result", {}).get("value"),
            42,
            "peer Runtime session after pending owner detach",
        )
        replacement = await context.new_cdp_session(page)
        sessions.append(replacement)
        replacement_value = await replacement.send(
            "Runtime.evaluate",
            {"expression": "40 + 2", "returnByValue": True},
        )
        assert_equal(
            replacement_value.get("result", {}).get("value"),
            42,
            "replacement Runtime session after pending owner detach",
        )
        record_contract(
            results,
            "multi_page_pending_runtime_await_session_detach",
            contract=(
                "Detaching a session cancels only that session's pending awaitPromise command; "
                "peer and replacement sessions remain immediately usable."
            ),
            source="Chromium DevToolsSession oracle",
            commands=["Runtime.evaluate", "Target.detachFromTarget"],
            observed={
                "pendingOwnerCanceled": True,
                "peerValue": 42,
                "replacementValue": 42,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _fetch_interception_is_session_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    navigation: asyncio.Task[Any] | None = None
    try:
        page = await context.new_page()
        interceptor, peer = await asyncio.gather(
            context.new_cdp_session(page),
            context.new_cdp_session(page),
        )
        sessions = [interceptor, peer]
        paused = attach_cdp_event_collector(interceptor, ["Fetch.requestPaused"])
        await interceptor.send(
            "Fetch.enable",
            {
                "patterns": [
                    {
                        "urlPattern": "*multi-page-fetch-detach*",
                        "requestStage": "Request",
                    }
                ]
            },
        )
        url = f"{fixture}/plain?multi-page-fetch-detach"
        navigation = asyncio.create_task(page.goto(url, wait_until="load", timeout=5000))
        await wait_until(
            lambda: any(
                event.get("method") == "Fetch.requestPaused"
                and event.get("params", {}).get("request", {}).get("url") == url
                for event in paused
            ),
            "Fetch.requestPaused before interceptor detach",
        )
        paused_request = next(
            event
            for event in paused
            if event.get("method") == "Fetch.requestPaused"
            and event.get("params", {}).get("request", {}).get("url") == url
        )
        request_id = paused_request.get("params", {}).get("requestId")
        if not isinstance(request_id, str):
            raise SmokeError(f"Fetch.requestPaused returned no requestId: {paused_request!r}")

        await peer.send("Fetch.disable")
        await asyncio.sleep(0.1)
        assert_equal(
            navigation.done(),
            False,
            "peer Fetch.disable cannot release another session's paused request",
        )
        await interceptor.send(
            "Fetch.continueRequest",
            {"requestId": request_id},
        )
        response = await asyncio.wait_for(navigation, timeout=5)
        assert_equal(
            response.status if response is not None else None,
            200,
            "Fetch owner continues its paused navigation",
        )
        await asyncio.wait_for(interceptor.detach(), timeout=2)
        sessions.remove(interceptor)
        recovery = await page.goto(
            f"{fixture}/plain?multi-page-fetch-detach=recovery",
            wait_until="load",
            timeout=5_000,
        )
        assert_equal(
            recovery.status if recovery is not None else None,
            200,
            "future navigation after detached Fetch owner",
        )
        record_contract(
            results,
            "multi_page_fetch_interceptor_session_detach_recovery",
            contract=(
                "Fetch interception is session-owned: a peer cannot disable or release its "
                "paused request, while owner detach removes interception from future requests."
            ),
            source="Chromium Fetch domain oracle",
            commands=[
                "Fetch.enable",
                "Fetch.disable",
                "Fetch.requestPaused",
                "Fetch.continueRequest",
                "Target.detachFromTarget",
            ],
            observed={
                "pausedBeforeDetach": True,
                "peerDisableDidNotRelease": True,
                "ownerContinuedNavigationStatus": 200,
                "recoveryNavigationStatus": 200,
            },
        )
    finally:
        if navigation is not None and not navigation.done():
            navigation.cancel()
        if navigation is not None:
            await asyncio.gather(navigation, return_exceptions=True)
        with suppress(Exception):
            await asyncio.wait_for(page.close(), timeout=2)
        with suppress(Exception):
            await asyncio.wait_for(
                asyncio.gather(
                    *(session.detach() for session in sessions),
                    return_exceptions=True,
                ),
                timeout=2,
            )
        await close_context(context)


async def _network_and_emulation_profiles_are_target_local(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page_a, page_b = await asyncio.gather(context.new_page(), context.new_page())
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page_a),
            context.new_cdp_session(page_b),
        )
        sessions = [session_a, session_b]
        await asyncio.gather(
            session_a.send("Page.enable"),
            session_b.send("Page.enable"),
            session_a.send("Network.enable"),
            session_b.send("Network.enable"),
        )
        profiles = (
            (
                session_a,
                "target-a",
                "MultiPageAgentA/1.0",
                "fr-FR",
                "Europe/Paris",
            ),
            (
                session_b,
                "target-b",
                "MultiPageAgentB/1.0",
                "ja-JP",
                "Asia/Tokyo",
            ),
        )
        await asyncio.gather(
            *(
                asyncio.gather(
                    session.send(
                        "Network.setExtraHTTPHeaders",
                        {"headers": {"x-moli-profile-smoke": marker}},
                    ),
                    session.send(
                        "Emulation.setUserAgentOverride",
                        {
                            "userAgent": user_agent,
                            "acceptLanguage": language,
                        },
                    ),
                    session.send(
                        "Emulation.setLocaleOverride",
                        {"locale": language},
                    ),
                    session.send(
                        "Emulation.setTimezoneOverride",
                        {"timezoneId": timezone},
                    ),
                )
                for session, marker, user_agent, language, timezone in profiles
            )
        )

        tokens = ("multi-page-profile-a", "multi-page-profile-b")
        navigation_results = await asyncio.gather(
            session_a.send(
                "Page.navigate",
                {"url": f"{fixture}/profile-headers?token={tokens[0]}"},
            ),
            session_b.send(
                "Page.navigate",
                {"url": f"{fixture}/profile-headers?token={tokens[1]}"},
            ),
        )
        if any(result.get("errorText") for result in navigation_results):
            raise SmokeError(f"profile navigation failed: {navigation_results!r}")
        wire_profiles: list[Any] = [None, None]

        async def collect_wire_profile(index: int, token: str) -> None:
            async def read_profile() -> bool:
                try:
                    profile = await asyncio.to_thread(
                        read_fixture_json,
                        f"{fixture}/profile-result?token={token}",
                    )
                except Exception:
                    return False
                if profile is None:
                    return False
                wire_profiles[index] = profile
                return True

            await wait_until(read_profile, f"profile target {index} request")

        await asyncio.gather(
            *(
                collect_wire_profile(
                    index,
                    token,
                )
                for index, token in enumerate(tokens)
            )
        )
        runtime_profiles: list[Any] = [None, None]

        async def collect_runtime_profile(index: int, session: Any) -> None:
            last_observation: Any = None
            for _ in range(100):
                try:
                    response = await session.send(
                        "Runtime.evaluate",
                        {
                            "expression": """({
                              userAgent: navigator.userAgent,
                              language: navigator.language,
                              timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
                            })""",
                            "returnByValue": True,
                        },
                    )
                except Exception as error:
                    last_observation = f"{type(error).__name__}: {error}"
                    await asyncio.sleep(0.05)
                    continue
                last_observation = response
                profile = response.get("result", {}).get("value")
                if not isinstance(profile, dict):
                    await asyncio.sleep(0.05)
                    continue
                runtime_profiles[index] = profile
                return
            raise SmokeError(
                f"profile target {index} runtime unavailable: {last_observation!r}"
            )

        await asyncio.gather(
            *(
                collect_runtime_profile(
                    index,
                    session,
                )
                for index, session in enumerate((session_a, session_b))
            )
        )
        expected_runtime_profiles = [
            {
                "userAgent": "MultiPageAgentA/1.0",
                "language": "fr-FR",
                "timezone": "Europe/Paris",
            },
            {
                "userAgent": "MultiPageAgentB/1.0",
                "language": "ja-JP",
                "timezone": "Asia/Tokyo",
            },
        ]
        assert_equal(
            runtime_profiles,
            expected_runtime_profiles,
            "target-local Runtime emulation profiles",
        )
        assert_equal(
            [
                {
                    "userAgent": profile.get("userAgent"),
                    "acceptLanguage": profile.get("acceptLanguage"),
                    "extraHeader": profile.get("extraHeader"),
                }
                for profile in wire_profiles
            ],
            [
                {
                    "userAgent": "MultiPageAgentA/1.0",
                    "acceptLanguage": "fr-FR",
                    "extraHeader": "target-a",
                },
                {
                    "userAgent": "MultiPageAgentB/1.0",
                    "acceptLanguage": "ja-JP",
                    "extraHeader": "target-b",
                },
            ],
            "target-local navigation request profiles",
        )

        await session_a.detach()
        sessions.remove(session_a)
        detached_token = "multi-page-profile-a-detached"
        await page_a.goto(
            f"{fixture}/profile-headers?token={detached_token}",
            wait_until="load",
        )
        detached_wire_profile = await asyncio.to_thread(
            read_fixture_json,
            f"{fixture}/profile-result?token={detached_token}",
        )
        detached_runtime_profile, surviving_runtime_profile = await asyncio.gather(
            page_a.evaluate(
                """() => ({
                  userAgent: navigator.userAgent,
                  language: navigator.language,
                  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
                })"""
            ),
            page_b.evaluate(
                """() => ({
                  userAgent: navigator.userAgent,
                  language: navigator.language,
                  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
                })"""
            ),
        )
        if detached_wire_profile.get("extraHeader") is not None:
            raise SmokeError(
                "detached target retained its session-owned extra header: "
                f"{detached_wire_profile!r}"
            )
        if detached_runtime_profile == expected_runtime_profiles[0]:
            raise SmokeError(
                "detached target retained every session-owned emulation override: "
                f"{detached_runtime_profile!r}"
            )
        assert_equal(
            surviving_runtime_profile,
            expected_runtime_profiles[1],
            "peer target emulation profile after owner detach",
        )
        record_contract(
            results,
            "multi_page_network_and_emulation_profile_ownership",
            contract=(
                "Headers, user agent, locale, and timezone overrides are target-local; "
                "detaching their owning session clears that target without mutating a peer."
            ),
            source="Chromium Network and Emulation agent oracle",
            commands=[
                "Network.setExtraHTTPHeaders",
                "Emulation.setUserAgentOverride",
                "Emulation.setLocaleOverride",
                "Emulation.setTimezoneOverride",
                "Target.detachFromTarget",
            ],
            observed={
                "initialRuntimeProfiles": runtime_profiles,
                "initialWireProfiles": wire_profiles,
                "detachedRuntimeProfile": detached_runtime_profile,
                "detachedWireProfile": detached_wire_profile,
                "peerProfileSurvived": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _closed_target_new_document_script_does_not_leak(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        owner, peer = await asyncio.gather(context.new_page(), context.new_page())
        owner_session, peer_session = await asyncio.gather(
            context.new_cdp_session(owner),
            context.new_cdp_session(peer),
        )
        sessions = [owner_session, peer_session]
        await asyncio.gather(
            owner_session.send("Page.enable"),
            peer_session.send("Page.enable"),
        )
        await asyncio.gather(
            owner_session.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": "globalThis.__multiPageClosedOwner = 'owner';"},
            ),
            peer_session.send(
                "Page.addScriptToEvaluateOnNewDocument",
                {"source": "globalThis.__multiPageSurvivingPeer = 'peer';"},
            ),
        )
        await asyncio.gather(
            owner.goto(
                f"{fixture}/plain?multi-page-close-script=owner",
                wait_until="load",
            ),
            peer.goto(
                f"{fixture}/plain?multi-page-close-script=peer",
                wait_until="load",
            ),
        )
        assert_equal(
            await asyncio.gather(
                owner.evaluate("globalThis.__multiPageClosedOwner"),
                peer.evaluate("globalThis.__multiPageSurvivingPeer"),
            ),
            ["owner", "peer"],
            "target-local scripts before owner target close",
        )

        await owner.close()
        replacement = await context.new_page()
        await replacement.goto(
            f"{fixture}/plain?multi-page-close-script=replacement",
            wait_until="load",
        )
        replacement_owner_value, surviving_peer_value = await asyncio.gather(
            replacement.evaluate("globalThis.__multiPageClosedOwner ?? null"),
            peer.evaluate("globalThis.__multiPageSurvivingPeer ?? null"),
        )
        assert_equal(
            replacement_owner_value,
            None,
            "closed target script does not leak into a replacement target",
        )
        assert_equal(
            surviving_peer_value,
            "peer",
            "closing a scripted target preserves a peer target's script state",
        )
        replacement_session = await context.new_cdp_session(replacement)
        sessions.append(replacement_session)
        replacement_result = await replacement_session.send(
            "Runtime.evaluate",
            {"expression": "7 * 6", "returnByValue": True},
        )
        assert_equal(
            replacement_result.get("result", {}).get("value"),
            42,
            "replacement target session after scripted owner close",
        )
        record_contract(
            results,
            "multi_page_closed_target_new_document_script_does_not_leak",
            contract=(
                "Closing a Page discards that target's session-owned new-Document "
                "scripts; a new target starts clean while a live peer remains unchanged."
            ),
            source="Chromium CDP oracle",
            commands=[
                "Page.addScriptToEvaluateOnNewDocument",
                "Target.closeTarget",
                "Target.createTarget",
            ],
            observed={
                "replacementInheritedClosedTargetScript": False,
                "survivingPeerRetainedScript": True,
                "replacementSessionUsable": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _broadcast_channel_context_partition(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    try:
        page_a_sender, page_a_receiver = await asyncio.gather(
            context_a.new_page(),
            context_a.new_page(),
        )
        page_b_sender, page_b_receiver = await asyncio.gather(
            context_b.new_page(),
            context_b.new_page(),
        )
        pages = [page_a_sender, page_a_receiver, page_b_sender, page_b_receiver]
        await asyncio.gather(
            *(
                page.goto(
                    f"{fixture}/plain?multi-page-channel={index}",
                    wait_until="load",
                )
                for index, page in enumerate(pages)
            )
        )
        await asyncio.gather(*(_install_broadcast_channel(page) for page in pages))

        await asyncio.gather(
            page_a_sender.evaluate(
                "__multiPageChannel.postMessage({owner: 'a', sequence: 1})"
            ),
            page_b_sender.evaluate(
                "__multiPageChannel.postMessage({owner: 'b', sequence: 1})"
            ),
        )
        await asyncio.gather(
            page_a_receiver.wait_for_function(
                "() => __multiPageMessages.length === 1", timeout=5_000
            ),
            page_b_receiver.wait_for_function(
                "() => __multiPageMessages.length === 1", timeout=5_000
            ),
        )
        initial_messages = await asyncio.gather(
            *(page.evaluate("globalThis.__multiPageMessages") for page in pages)
        )
        assert_equal(
            initial_messages,
            [
                [],
                [{"owner": "a", "sequence": 1}],
                [],
                [{"owner": "b", "sequence": 1}],
            ],
            "BroadcastChannel sender exclusion and BrowserContext partition",
        )

        await asyncio.gather(page_a_receiver.close(), page_b_receiver.close())
        replacement_receiver = await context_a.new_page()
        await replacement_receiver.goto(
            f"{fixture}/plain?multi-page-channel=replacement",
            wait_until="load",
        )
        await _install_broadcast_channel(replacement_receiver)
        await page_a_sender.evaluate(
            "__multiPageChannel.postMessage({owner: 'a', sequence: 2})"
        )
        await replacement_receiver.wait_for_function(
            "() => __multiPageMessages.length === 1", timeout=5_000
        )
        assert_equal(
            await replacement_receiver.evaluate("globalThis.__multiPageMessages"),
            [{"owner": "a", "sequence": 2}],
            "replacement same-context BroadcastChannel receiver",
        )
        assert_equal(
            await page_b_sender.evaluate("globalThis.__multiPageMessages"),
            [],
            "replacement delivery does not cross BrowserContext",
        )
        record_contract(
            results,
            "multi_page_broadcast_channel_context_partition",
            contract=(
                "BroadcastChannel fans out to peer Pages in the same BrowserContext and "
                "origin, excludes the sender, and never crosses BrowserContext partitions."
            ),
            source="Chromium DOM oracle",
            commands=["Runtime.evaluate", "Target.closeTarget"],
            observed={
                "sameContextDelivered": True,
                "senderExcluded": True,
                "crossContextDelivered": False,
                "replacementReceiverDelivered": True,
            },
        )
    finally:
        await close_context(context_a)
        await close_context(context_b)


async def _same_target_navigation_supersession(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    route_started = asyncio.Event()
    release_route = asyncio.Event()
    first_navigation: asyncio.Task[Any] | None = None

    async def hold_first_navigation(route: Any) -> None:
        route_started.set()
        await release_route.wait()
        with suppress(Exception):
            await route.continue_()

    try:
        victim, peer = await asyncio.gather(context.new_page(), context.new_page())
        await peer.goto(
            f"{fixture}/plain?multi-page-supersession=peer-initial",
            wait_until="load",
        )
        await victim.route("**/multi-page-supersession-held", hold_first_navigation)
        first_navigation = asyncio.create_task(
            victim.goto(
                f"{fixture}/multi-page-supersession-held",
                wait_until="load",
                timeout=10_000,
            )
        )
        await asyncio.wait_for(route_started.wait(), timeout=5)

        peer_navigation = asyncio.create_task(
            peer.goto(
                f"{fixture}/plain?multi-page-supersession=peer-winner",
                wait_until="load",
                timeout=10_000,
            )
        )
        winning_navigation = asyncio.create_task(
            victim.goto(
                f"{fixture}/plain?multi-page-supersession=victim-winner",
                wait_until="load",
                timeout=10_000,
            )
        )
        release_route.set()
        first_result = (
            await asyncio.gather(first_navigation, return_exceptions=True)
        )[0]
        if not isinstance(first_result, BaseException):
            raise SmokeError(
                "a newer same-target navigation did not reject the held predecessor: "
                f"{first_result!r}"
            )
        await asyncio.gather(peer_navigation, winning_navigation)
        assert_equal(
            [victim.url, peer.url],
            [
                f"{fixture}/plain?multi-page-supersession=victim-winner",
                f"{fixture}/plain?multi-page-supersession=peer-winner",
            ],
            "same-target navigation winner and independent peer",
        )
        assert_equal(
            await asyncio.gather(
                victim.text_content("main"),
                peer.text_content("main"),
            ),
            ["plain ok", "plain ok"],
            "Documents after same-target navigation supersession",
        )
        record_contract(
            results,
            "multi_page_same_target_navigation_supersession",
            contract=(
                "A newer navigation aborts an older held navigation on the same target "
                "without delaying or canceling a peer target's navigation."
            ),
            source="Chromium navigation oracle",
            commands=["Fetch.enable", "Page.navigate"],
            observed={
                "predecessorRejected": True,
                "winner": victim.url,
                "peer": peer.url,
            },
        )
    finally:
        release_route.set()
        if first_navigation is not None and not first_navigation.done():
            first_navigation.cancel()
        with suppress(Exception):
            await context.unroute("**/multi-page-supersession-held")
        await close_context(context)


async def _target_destroy_event_cardinality(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    context_closed = False
    browser_cdp = await browser.new_browser_cdp_session()
    destroyed: list[str] = []
    browser_cdp.on(
        "Target.targetDestroyed",
        lambda event: destroyed.append(event.get("targetId", "")),
    )
    try:
        await browser_cdp.send("Target.setDiscoverTargets", {"discover": True})
        pages = await asyncio.gather(*(context.new_page() for _ in range(5)))
        urls = [f"{fixture}/plain?multi-page-destroy={index}" for index in range(5)]
        await asyncio.gather(
            *(
                page.goto(url, wait_until="load")
                for page, url in zip(pages, urls, strict=True)
            )
        )
        target_infos = (await browser_cdp.send("Target.getTargets")).get(
            "targetInfos", []
        )
        target_by_url = {
            info.get("url"): info.get("targetId")
            for info in target_infos
            if info.get("type") == "page"
        }
        target_ids = {target_by_url.get(url) for url in urls}
        if None in target_ids or len(target_ids) != len(urls):
            raise SmokeError(
                "Target.getTargets did not expose every multi-page target: "
                f"urls={urls!r}, infos={target_infos!r}"
            )
        live_target_ids = {str(target_id) for target_id in target_ids}
        closing_indexes = (0, 2, 4)
        first_closed_ids = {
            str(target_by_url[urls[index]]) for index in closing_indexes
        }
        await asyncio.gather(*(pages[index].close() for index in closing_indexes))
        await wait_until(
            lambda: first_closed_ids <= set(destroyed),
            "Target.targetDestroyed for non-tail target closes",
        )
        await browser_cdp.send("Target.getTargets")
        first_destroyed = [
            target_id for target_id in destroyed if target_id in first_closed_ids
        ]
        assert_equal(
            len(first_destroyed),
            len(first_closed_ids),
            "one targetDestroyed event per explicitly closed target",
        )
        assert_equal(
            {page.url for page in context.pages},
            {urls[1], urls[3]},
            "surviving Pages after alternating target closes",
        )

        await asyncio.wait_for(context.close(), timeout=5)
        context_closed = True
        await wait_until(
            lambda: live_target_ids <= set(destroyed),
            "Target.targetDestroyed for BrowserContext disposal",
        )
        await browser_cdp.send("Target.getTargets")
        all_destroyed = [
            target_id for target_id in destroyed if target_id in live_target_ids
        ]
        assert_equal(
            len(all_destroyed),
            len(live_target_ids),
            "one targetDestroyed event per disposed-context target",
        )
        assert_equal(
            len(set(all_destroyed)),
            len(live_target_ids),
            "unique targetDestroyed ownership",
        )
        record_contract(
            results,
            "multi_page_target_destroy_event_cardinality",
            contract=(
                "Each Page target emits exactly one Target.targetDestroyed event, both "
                "for individual close and for BrowserContext disposal."
            ),
            source="Chromium Target domain oracle",
            commands=[
                "Target.setDiscoverTargets",
                "Target.getTargets",
                "Target.closeTarget",
                "Target.disposeBrowserContext",
            ],
            observed={
                "targets": len(live_target_ids),
                "destroyedEvents": len(all_destroyed),
                "uniqueTargetIds": len(set(all_destroyed)),
            },
        )
    finally:
        with suppress(Exception):
            await browser_cdp.send("Target.setDiscoverTargets", {"discover": False})
        with suppress(Exception):
            await browser_cdp.detach()
        if not context_closed:
            await close_context(context)


async def _network_response_body_target_ownership(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        page_a, page_b = await asyncio.gather(context.new_page(), context.new_page())
        session_a, session_b = await asyncio.gather(
            context.new_cdp_session(page_a),
            context.new_cdp_session(page_b),
        )
        sessions = [session_a, session_b]
        events_a = attach_cdp_event_collector(
            session_a,
            ["Network.responseReceived", "Network.loadingFinished"],
        )
        events_b = attach_cdp_event_collector(
            session_b,
            ["Network.responseReceived", "Network.loadingFinished"],
        )
        await asyncio.gather(
            session_a.send("Network.enable"),
            session_b.send("Network.enable"),
        )
        url_a = f"{fixture}/plain?multi-page-network-body=a"
        url_b = f"{fixture}/plain?multi-page-network-body=b"
        await asyncio.gather(
            page_a.goto(url_a, wait_until="load"),
            page_b.goto(url_b, wait_until="load"),
        )
        request_a = _document_request_id(events_a, url_a)
        request_b = _document_request_id(events_b, url_b)
        if request_a == request_b:
            raise SmokeError(
                "concurrent Page targets reused a Document requestId: "
                f"{request_a!r}"
            )
        body_a = await session_a.send(
            "Network.getResponseBody",
            {"requestId": request_a},
        )
        body_b = await session_b.send(
            "Network.getResponseBody",
            {"requestId": request_b},
        )
        assert_equal(
            ["plain ok" in body_a.get("body", ""), "plain ok" in body_b.get("body", "")],
            [True, True],
            "target sessions resolve their own response bodies",
        )
        foreign_a = await expect_protocol_error(
            session_a.send("Network.getResponseBody", {"requestId": request_b}),
            "page A resolving page B requestId",
        )
        foreign_b = await expect_protocol_error(
            session_b.send("Network.getResponseBody", {"requestId": request_a}),
            "page B resolving page A requestId",
        )

        await page_b.close()
        sessions.remove(session_b)
        body_a_after_peer_close = await session_a.send(
            "Network.getResponseBody",
            {"requestId": request_a},
        )
        assert_equal(
            body_a_after_peer_close.get("body"),
            body_a.get("body"),
            "peer target close does not evict the surviving target response body",
        )
        record_contract(
            results,
            "multi_page_network_response_body_target_ownership",
            contract=(
                "Document request identifiers and response-body caches are target-local; "
                "foreign ids fail and peer close cannot evict a surviving cache entry."
            ),
            source="Chromium Network domain oracle",
            commands=["Network.enable", "Network.getResponseBody", "Target.closeTarget"],
            observed={
                "requestIdsDistinct": True,
                "foreignARejected": bool(foreign_a),
                "foreignBRejected": bool(foreign_b),
                "survivingBodyRetained": True,
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


async def _websocket_survives_peer_target_close(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    sessions: list[Any] = []
    try:
        owner, peer = await asyncio.gather(context.new_page(), context.new_page())
        await asyncio.gather(
            owner.goto(f"{fixture}/plain?multi-page-websocket=owner", wait_until="load"),
            peer.goto(f"{fixture}/plain?multi-page-websocket=peer", wait_until="load"),
        )
        owner_session, peer_session = await asyncio.gather(
            context.new_cdp_session(owner),
            context.new_cdp_session(peer),
        )
        sessions = [owner_session, peer_session]
        event_methods = [
            "Network.webSocketCreated",
            "Network.webSocketHandshakeResponseReceived",
            "Network.webSocketFrameSent",
            "Network.webSocketFrameReceived",
            "Network.webSocketClosed",
        ]
        owner_events = attach_cdp_event_collector(owner_session, event_methods)
        peer_events = attach_cdp_event_collector(peer_session, event_methods)
        await asyncio.gather(
            owner_session.send("Network.enable"),
            peer_session.send("Network.enable"),
        )

        owner_payload = "owner-websocket-payload-is-deliberately-long"
        peer_payload = "peer-short"
        owner_echo, peer_echo = await asyncio.gather(
            _open_persistent_websocket(owner, owner_payload),
            _open_persistent_websocket(peer, peer_payload),
        )
        assert_equal(owner_echo, f"echo:{owner_payload}", "owner WebSocket echo")
        assert_equal(peer_echo, f"echo:{peer_payload}", "peer WebSocket echo")
        await asyncio.gather(
            wait_until(
                lambda: _has_received_websocket_payload_length(
                    owner_events, len(owner_echo)
                ),
                "owner WebSocket frame",
            ),
            wait_until(
                lambda: _has_received_websocket_payload_length(
                    peer_events, len(peer_echo)
                ),
                "peer WebSocket frame",
            ),
        )
        if _has_received_websocket_payload_length(peer_events, len(owner_echo)):
            raise SmokeError("peer Network session received owner WebSocket frame")
        if _has_received_websocket_payload_length(owner_events, len(peer_echo)):
            raise SmokeError("owner Network session received peer WebSocket frame")

        await owner.close()
        peer_after_close_payload = "peer-after-owner-target-close"
        peer_after_close_echo = await _send_persistent_websocket(
            peer,
            peer_after_close_payload,
        )
        assert_equal(
            peer_after_close_echo,
            f"echo:{peer_after_close_payload}",
            "peer WebSocket after owner target close",
        )
        await wait_until(
            lambda: _has_received_websocket_payload_length(
                peer_events, len(peer_after_close_echo)
            ),
            "peer WebSocket frame after owner target close",
        )
        assert_equal(
            await peer.evaluate("() => location.search"),
            "?multi-page-websocket=peer",
            "peer Document after owner WebSocket target close",
        )
        await peer.evaluate(
            """() => new Promise(resolve => {
              const socket = globalThis.__multiPageSocket;
              if (!socket || socket.readyState === WebSocket.CLOSED) {
                resolve();
                return;
              }
              socket.addEventListener('close', () => resolve(), {once: true});
              socket.close(1000, 'done');
            })"""
        )
        await wait_until(
            lambda: any(
                event.get("method") == "Network.webSocketClosed"
                for event in peer_events
            ),
            "peer Network.webSocketClosed",
        )
        record_contract(
            results,
            "multi_page_websocket_survives_peer_target_close",
            contract=(
                "WebSocket state and Network events belong to one Page target; closing "
                "a peer target does not close or reroute a surviving Page's socket."
            ),
            source="Chromium Network domain oracle",
            commands=["Network.enable", "Target.closeTarget"],
            observed={
                "ownerEchoLength": len(owner_echo),
                "peerEchoLength": len(peer_echo),
                "peerAfterCloseEchoLength": len(peer_after_close_echo),
            },
        )
    finally:
        await asyncio.gather(
            *(session.detach() for session in sessions),
            return_exceptions=True,
        )
        await close_context(context)


def _default_execution_context_id(events: list[dict[str, Any]]) -> int | None:
    for event in events:
        if event.get("method") != "Runtime.executionContextCreated":
            continue
        context = event.get("params", {}).get("context", {})
        if context.get("auxData", {}).get("isDefault") is not True:
            continue
        context_id = context.get("id")
        if isinstance(context_id, int):
            return context_id
    return None


def _has_lifecycle_event(events: list[dict[str, Any]], name: str) -> bool:
    return any(
        event.get("method") == "Page.lifecycleEvent"
        and event.get("params", {}).get("name") == name
        for event in events
    )


async def _viewport_metrics(page: Any) -> list[float]:
    return await page.evaluate(
        "() => [innerWidth, innerHeight, devicePixelRatio]"
    )


async def _same_target_session_script_values(page: Any) -> list[str | None]:
    return await page.evaluate(
        """() => [
          globalThis.__multiPageSessionA ?? null,
          globalThis.__multiPageSessionB ?? null,
        ]"""
    )


async def _create_isolated_world(session: Any, world_name: str) -> int:
    frame_tree = await session.send("Page.getFrameTree")
    frame_id = frame_tree.get("frameTree", {}).get("frame", {}).get("id")
    if not isinstance(frame_id, str) or not frame_id:
        raise SmokeError(f"Page.getFrameTree returned no main frame id: {frame_tree!r}")
    world = await session.send(
        "Page.createIsolatedWorld",
        {"frameId": frame_id, "worldName": world_name},
    )
    context_id = world.get("executionContextId")
    if not isinstance(context_id, int):
        raise SmokeError(f"Page.createIsolatedWorld returned no context id: {world!r}")
    return context_id


async def _root_and_child_frame_ids(session: Any) -> list[str]:
    frame_tree = (await session.send("Page.getFrameTree")).get("frameTree", {})
    root_id = frame_tree.get("frame", {}).get("id")
    children = frame_tree.get("childFrames", [])
    child_id = children[0].get("frame", {}).get("id") if children else None
    if not isinstance(root_id, str) or not isinstance(child_id, str):
        raise SmokeError(
            "Page.getFrameTree returned no root/child frame pair: "
            f"{frame_tree!r}"
        )
    return [root_id, child_id]


async def _create_isolated_worlds_for_frames(
    session: Any,
    frame_ids: list[str],
    world_name: str,
) -> list[int]:
    context_ids: list[int] = []
    for frame_id in frame_ids:
        world = await session.send(
            "Page.createIsolatedWorld",
            {"frameId": frame_id, "worldName": world_name},
        )
        context_id = world.get("executionContextId")
        if not isinstance(context_id, int):
            raise SmokeError(
                "Page.createIsolatedWorld returned no context id for frame "
                f"{frame_id!r}: {world!r}"
            )
        context_ids.append(context_id)
    return context_ids


async def _evaluate_context_value(
    session: Any,
    context_id: int,
    expression: str,
) -> Any:
    evaluation = await session.send(
        "Runtime.evaluate",
        {
            "contextId": context_id,
            "expression": expression,
            "returnByValue": True,
        },
    )
    return evaluation.get("result", {}).get("value")


def _binding_payloads(events: list[dict[str, Any]], name: str) -> list[str]:
    return [
        str(event.get("params", {}).get("payload"))
        for event in events
        if event.get("method") == "Runtime.bindingCalled"
        and event.get("params", {}).get("name") == name
    ]


def _document_request_id(events: list[dict[str, Any]], url: str) -> str:
    for event in events:
        if event.get("method") != "Network.responseReceived":
            continue
        params = event.get("params", {})
        if params.get("type") != "Document":
            continue
        if params.get("response", {}).get("url") != url:
            continue
        request_id = params.get("requestId")
        if isinstance(request_id, str) and request_id:
            return request_id
    raise SmokeError(
        f"Network.responseReceived exposed no Document requestId for {url!r}: {events!r}"
    )


def _has_document_response(events: list[dict[str, Any]], url: str) -> bool:
    return any(
        event.get("method") == "Network.responseReceived"
        and event.get("params", {}).get("type") == "Document"
        and event.get("params", {}).get("response", {}).get("url") == url
        for event in events
    )


async def _install_broadcast_channel(page: Any) -> None:
    await page.evaluate(
        """() => {
          globalThis.__multiPageMessages = [];
          globalThis.__multiPageChannel = new BroadcastChannel('multi-page-contract');
          globalThis.__multiPageChannel.onmessage = event => {
            globalThis.__multiPageMessages.push(event.data);
          };
        }"""
    )


async def _open_persistent_websocket(page: Any, payload: str) -> str:
    return await page.evaluate(
        """payload => new Promise((resolve, reject) => {
          const url = new URL('/ws-echo', location.href);
          url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
          const socket = new WebSocket(url.href, 'smoke');
          globalThis.__multiPageSocket = socket;
          const timer = setTimeout(() => {
            socket.close();
            reject(new Error(`WebSocket open timed out at ${socket.readyState}`));
          }, 5000);
          socket.onopen = () => socket.send(payload);
          socket.onmessage = event => {
            clearTimeout(timer);
            resolve(event.data);
          };
          socket.onerror = () => {
            clearTimeout(timer);
            reject(new Error(`WebSocket failed at ${socket.readyState}`));
          };
        })""",
        payload,
    )


async def _send_persistent_websocket(page: Any, payload: str) -> str:
    return await page.evaluate(
        """payload => new Promise((resolve, reject) => {
          const socket = globalThis.__multiPageSocket;
          if (!socket || socket.readyState !== WebSocket.OPEN) {
            reject(new Error(`WebSocket is not open: ${socket?.readyState}`));
            return;
          }
          const timer = setTimeout(() => {
            reject(new Error(`WebSocket reply timed out at ${socket.readyState}`));
          }, 5000);
          const receive = event => {
            clearTimeout(timer);
            resolve(event.data);
          };
          socket.addEventListener('message', receive, {once: true});
          socket.send(payload);
        })""",
        payload,
    )


def _has_received_websocket_payload_length(
    events: list[dict[str, Any]],
    payload_length: int,
) -> bool:
    for event in events:
        if event.get("method") != "Network.webSocketFrameReceived":
            continue
        response = event.get("params", {}).get("response", {})
        observed_length = response.get("payloadLength")
        if not isinstance(observed_length, int):
            payload_data = response.get("payloadData")
            if isinstance(payload_data, str):
                observed_length = len(payload_data.encode("utf-8"))
        if observed_length == payload_length:
            return True
    return False
