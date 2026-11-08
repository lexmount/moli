from __future__ import annotations


import asyncio
import math
from typing import Any

from playwright.async_api import Error as PlaywrightError

from ..diagnostics import new_cdp_session
from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..helpers import attach_cdp_event_collector


async def run_document_content_group(state: SmokeState) -> None:
    try:
        await _verify_multi_session_events_with_pending_runtime_command(state)
        await _verify_root_replacement_identity_history_and_cleanup(state)
        await _verify_body_stylesheet_pauses_and_resumes_parser(state)
        await _verify_child_document_replacement(state)
        await _verify_errors_leave_the_document_unchanged(state)
    finally:
        try:
            await state.cdp.send("Runtime.disable")
        except PlaywrightError:
            pass


async def _verify_multi_session_events_with_pending_runtime_command(
    state: SmokeState,
) -> None:
    page = state.page
    runtime_cdp = state.cdp
    content_cdp = await new_cdp_session(state.context, page)
    pending_runtime: asyncio.Task[Any] | None = None
    runtime_released = False
    runtime_lifecycle_disabled = False
    url = f"{state.fixture}/plain?document-content-multi-session"
    try:
        await _goto_and_wait_for_frame_stopped_loading(page, runtime_cdp, url)
        for cdp in (runtime_cdp, content_cdp):
            await cdp.send("Page.enable")
            await cdp.send("Page.setLifecycleEventsEnabled", {"enabled": True})
            await cdp.send("Runtime.enable")
            await cdp.send("DOM.enable")

        runtime_events, runtime_complete = _document_content_completion_observer(
            runtime_cdp
        )
        content_events, content_complete = _document_content_completion_observer(
            content_cdp
        )
        pending_runtime = asyncio.create_task(
            runtime_cdp.send(
                "Runtime.evaluate",
                {
                    "expression": """
                        new Promise(resolve => {
                          globalThis.__documentContentPendingRuntimeStarted = true;
                          globalThis.__releaseDocumentContentPendingRuntime = resolve;
                        })
                    """,
                    "awaitPromise": True,
                    "returnByValue": True,
                },
            )
        )

        async def pending_runtime_started() -> bool:
            return await _evaluate_value(
                content_cdp,
                "globalThis.__documentContentPendingRuntimeStarted === true",
            )

        await wait_until(
            pending_runtime_started,
            "pending Runtime command on another attached session",
        )
        frame = await _root_frame(content_cdp)
        frame_id = frame["id"]
        loader_id = frame.get("loaderId")
        if not loader_id:
            raise SmokeError(
                f"multi-session root frame has no loaderId before replacement: {frame}"
            )
        classic_marker = "document-content-multi-session-classic"
        result = await content_cdp.send(
            "Page.setDocumentContent",
            {
                "frameId": frame_id,
                "html": (
                    '<main id="document-content-multi-session">'
                    "multi-session replacement"
                    "</main><script>"
                    f"console.log('{classic_marker}')"
                    "</script>"
                ),
            },
        )
        assert_equal(result, {}, "multi-session Page.setDocumentContent response")

        try:
            await asyncio.wait_for(
                asyncio.gather(runtime_complete.wait(), content_complete.wait()),
                timeout=10,
            )
        except TimeoutError as error:
            raise SmokeError(
                "setDocumentContent did not fan out DOM/load completion while another "
                "session had a pending Runtime command; "
                f"runtime session events={[event['method'] for event in runtime_events]}, "
                f"content session events={[event['method'] for event in content_events]}"
            ) from error

        _assert_detailed_document_content_event_trace(
            runtime_events,
            "pending Runtime session setDocumentContent fan-out",
            frame_id=frame_id,
            loader_id=loader_id,
            url=url,
            classic_marker=classic_marker,
        )
        _assert_detailed_document_content_event_trace(
            content_events,
            "command session setDocumentContent fan-out",
            frame_id=frame_id,
            loader_id=loader_id,
            url=url,
            classic_marker=classic_marker,
        )
        if pending_runtime.done():
            raise SmokeError(
                "pending Runtime.evaluate completed before its promise was explicitly released"
            )
        assert_equal(
            await _evaluate_value(
                content_cdp,
                "document.querySelector('#document-content-multi-session')?.textContent",
            ),
            "multi-session replacement",
            "multi-session setDocumentContent DOM",
        )
        assert_equal(
            await _evaluate_value(
                content_cdp,
                "__releaseDocumentContentPendingRuntime('released'); 'released'",
            ),
            "released",
            "release pending Runtime command after setDocumentContent",
        )
        runtime_released = True
        pending_runtime_result = await asyncio.wait_for(pending_runtime, timeout=10)
        assert_equal(
            pending_runtime_result.get("result", {}).get("value"),
            "released",
            "pending Runtime command result after setDocumentContent",
        )

        await runtime_cdp.send(
            "Page.setLifecycleEventsEnabled",
            {"enabled": False},
        )
        runtime_lifecycle_disabled = True
        runtime_second_start = len(runtime_events)
        content_second_start = len(content_events)
        second_result = await content_cdp.send(
            "Page.setDocumentContent",
            {
                "frameId": frame_id,
                "html": (
                    '<main id="document-content-session-local-lifecycle">'
                    "session-local lifecycle"
                    "</main>"
                ),
            },
        )
        assert_equal(
            second_result,
            {},
            "session-local lifecycle Page.setDocumentContent response",
        )
        await asyncio.gather(
            _wait_for_document_content_event_sequence(
                runtime_events,
                runtime_second_start,
                "lifecycle-disabled session replacement events",
                lifecycle_enabled=False,
            ),
            _wait_for_document_content_event_sequence(
                content_events,
                content_second_start,
                "lifecycle-enabled session replacement events",
                lifecycle_enabled=True,
            ),
        )
        runtime_second = runtime_events[runtime_second_start:]
        content_second = content_events[content_second_start:]
        _assert_detailed_document_content_event_trace(
            runtime_second,
            "lifecycle-disabled session setDocumentContent fan-out",
            frame_id=frame_id,
            loader_id=loader_id,
            url=url,
            lifecycle_enabled=False,
        )
        _assert_detailed_document_content_event_trace(
            content_second,
            "lifecycle-enabled session setDocumentContent fan-out",
            frame_id=frame_id,
            loader_id=loader_id,
            url=url,
        )
        assert_equal(
            await _evaluate_value(
                content_cdp,
                (
                    "document.querySelector("
                    "'#document-content-session-local-lifecycle'"
                    ")?.textContent"
                ),
            ),
            "session-local lifecycle",
            "session-local lifecycle replacement DOM",
        )
        state.record(
            "cdp_set_document_content_multi_session_event_fanout",
            {
                "sessions": 2,
                "replacements": 2,
                "sessionLocalLifecycle": True,
            },
        )
    finally:
        if runtime_lifecycle_disabled:
            try:
                await runtime_cdp.send(
                    "Page.setLifecycleEventsEnabled",
                    {"enabled": True},
                )
            except PlaywrightError:
                pass
        if pending_runtime is not None and not pending_runtime.done():
            if not runtime_released:
                try:
                    await content_cdp.send(
                        "Runtime.evaluate",
                        {
                            "expression": (
                                "globalThis.__releaseDocumentContentPendingRuntime?."
                                "('cleanup')"
                            )
                        },
                    )
                except PlaywrightError:
                    pass
            try:
                await asyncio.wait_for(pending_runtime, timeout=2)
            except (TimeoutError, PlaywrightError):
                pending_runtime.cancel()
        await content_cdp.detach()


async def _verify_root_replacement_identity_history_and_cleanup(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp
    url = f"{state.fixture}/plain?document-content-identity"
    await _goto_and_wait_for_frame_stopped_loading(page, cdp, url)
    await cdp.send("Page.enable")
    await cdp.send("Page.setLifecycleEventsEnabled", {"enabled": True})
    await cdp.send("Runtime.enable")
    await cdp.send("DOM.enable")

    before_frame = await _root_frame(cdp)
    old_root_handle = await page.query_selector("main")
    if old_root_handle is None:
        raise SmokeError("document-content identity setup did not expose the old root")
    before = await page.evaluate(
        """
        () => {
          globalThis.__documentContentOldDocument = document;
          globalThis.__documentContentOldNode = document.querySelector("main");
          globalThis.__documentContentOldHtml = document.documentElement;
          globalThis.__documentContentRealmMarker = 73;
          globalThis.__documentContentPublicCalls = [];
          globalThis.__documentContentMutationRecords = [];
          globalThis.__documentContentObserver = new MutationObserver(records => {
            __documentContentMutationRecords.push(...records);
          });
          __documentContentObserver.observe(document, { childList: true, subtree: true });
          history.replaceState({ marker: 41 }, "");
          document.open = () => __documentContentPublicCalls.push("open");
          document.write = () => __documentContentPublicCalls.push("write");
          document.close = () => __documentContentPublicCalls.push("close");
          return {
            historyLength: history.length,
            url: location.href,
          };
        }
        """
    )

    events = attach_cdp_event_collector(
        cdp,
        [
            "Page.documentOpened",
            "Page.frameStartedNavigating",
            "Page.frameStartedLoading",
            "Page.frameNavigated",
            "Page.frameStoppedLoading",
            "Page.lifecycleEvent",
            "Page.domContentEventFired",
            "Page.loadEventFired",
            "DOM.documentUpdated",
            "Runtime.consoleAPICalled",
            "Runtime.executionContextsCleared",
            "Runtime.executionContextDestroyed",
        ],
    )
    first_event_start = len(events)
    result = await cdp.send(
        "Page.setDocumentContent",
        {
            "frameId": before_frame["id"],
            "html": """
                <style>#document-content-first { color: rgb(21, 22, 23); }</style>
                <main id="document-content-first">first replacement</main>
                <script>
                  globalThis.__documentContentInlineRuns =
                    (globalThis.__documentContentInlineRuns || 0) + 1;
                  console.log("document-content-smoke-classic");
                </script>
            """,
        },
    )
    assert_equal(result, {}, "root Page.setDocumentContent response")

    try:
        await wait_until(
            lambda: _has_event(events, "Page.documentOpened")
            and _has_event(events, "DOM.documentUpdated")
            and _has_event(events, "Page.loadEventFired")
            and _has_lifecycle_event(events, "load"),
            "setDocumentContent document-open/DOM/load events",
        )
    except SmokeError as error:
        raise SmokeError(
            f"{error}; observed events: {[event['method'] for event in events]}"
        ) from error
    _assert_document_content_event_order(
        events[first_event_start:],
        "root Page.setDocumentContent",
        classic_marker="document-content-smoke-classic",
    )
    first_state = await page.evaluate(
        """
        () => ({
          sameDocument: document === __documentContentOldDocument,
          oldNodeConnected: __documentContentOldNode.isConnected,
          historyLength: history.length,
          historyState: history.state,
          url: location.href,
          realmMarker: __documentContentRealmMarker,
          publicCalls: __documentContentPublicCalls,
          text: document.querySelector("#document-content-first")?.textContent,
          inlineRuns: __documentContentInlineRuns,
          styleSheets: document.styleSheets.length,
          color: getComputedStyle(document.querySelector("#document-content-first")).color,
          sawOldHtmlRemoval: __documentContentMutationRecords.some(
            record => Array.from(record.removedNodes).includes(__documentContentOldHtml)
          ),
          sawNewHtmlInsertion: __documentContentMutationRecords.some(
            record => Array.from(record.addedNodes).includes(document.documentElement)
          ),
        })
        """
    )
    assert_equal(
        first_state,
        {
            "sameDocument": True,
            "oldNodeConnected": False,
            "historyLength": before["historyLength"],
            "historyState": {"marker": 41},
            "url": before["url"],
            "realmMarker": 73,
            "publicCalls": [],
            "text": "first replacement",
            "inlineRuns": 1,
            "styleSheets": 1,
            "color": "rgb(21, 22, 23)",
            "sawOldHtmlRemoval": True,
            "sawNewHtmlInsertion": True,
        },
        "root setDocumentContent identity/history/mutation state",
    )
    assert_equal(
        await old_root_handle.evaluate(
            "node => ({ connected: node.isConnected, text: node.textContent })"
        ),
        {"connected": False, "text": "plain ok"},
        "old ElementHandle after setDocumentContent",
    )

    first_replacement_handle = await page.query_selector("#document-content-first")
    if first_replacement_handle is None:
        raise SmokeError("first document-content replacement did not expose its root")
    second_event_start = len(events)
    result = await cdp.send(
        "Page.setDocumentContent",
        {
            "frameId": before_frame["id"],
            "html": '<main id="document-content-second">second replacement</main>',
        },
    )
    assert_equal(result, {}, "repeated root Page.setDocumentContent response")
    try:
        await wait_until(
            lambda: _has_event(events[second_event_start:], "DOM.documentUpdated")
            and _has_event(events[second_event_start:], "Page.loadEventFired")
            and _has_lifecycle_event(events[second_event_start:], "load"),
            "repeated setDocumentContent DOM/load events",
        )
    except SmokeError as error:
        raise SmokeError(
            f"{error}; observed events: "
            f"{[event['method'] for event in events[second_event_start:]]}"
        ) from error
    _assert_document_content_event_order(
        events[second_event_start:],
        "repeated root Page.setDocumentContent",
    )
    second_state = await page.evaluate(
        """
        () => ({
          sameDocument: document === __documentContentOldDocument,
          historyLength: history.length,
          historyState: history.state,
          url: location.href,
          realmMarker: __documentContentRealmMarker,
          publicCalls: __documentContentPublicCalls,
          text: document.querySelector("#document-content-second")?.textContent,
          styleSheets: document.styleSheets.length,
        })
        """
    )
    assert_equal(
        second_state,
        {
            "sameDocument": True,
            "historyLength": before["historyLength"],
            "historyState": {"marker": 41},
            "url": before["url"],
            "realmMarker": 73,
            "publicCalls": [],
            "text": "second replacement",
            "styleSheets": 0,
        },
        "repeated root setDocumentContent state",
    )
    assert_equal(
        await first_replacement_handle.evaluate(
            "node => ({ connected: node.isConnected, text: node.textContent })"
        ),
        {"connected": False, "text": "first replacement"},
        "first replacement ElementHandle after repeated setDocumentContent",
    )

    after_frame = await _root_frame(cdp)
    for field in ("id", "loaderId", "url"):
        assert_equal(
            after_frame.get(field),
            before_frame.get(field),
            f"setDocumentContent preserved root frame {field}",
        )
    forbidden = {
        "Page.frameStartedNavigating",
        "Page.frameStartedLoading",
        "Page.frameNavigated",
        "Page.frameStoppedLoading",
        "Runtime.executionContextsCleared",
        "Runtime.executionContextDestroyed",
    }
    observed_forbidden = [event["method"] for event in events if event["method"] in forbidden]
    assert_equal(
        observed_forbidden,
        [],
        "setDocumentContent navigation/realm events",
    )
    await old_root_handle.dispose()
    await first_replacement_handle.dispose()
    state.record(
        "cdp_set_document_content_identity_history_replacement",
        {"replacements": 2},
    )


async def _verify_body_stylesheet_pauses_and_resumes_parser(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp
    await _goto_and_wait_for_frame_stopped_loading(
        page,
        cdp,
        f"{state.fixture}/plain?document-content-stylesheet",
    )
    await cdp.send("Page.enable")
    await cdp.send("Page.setLifecycleEventsEnabled", {"enabled": True})
    frame = await _root_frame(cdp)
    stylesheet_url = f"{state.fixture}/document-content-gated.css"
    gate = state.fixture_server.document_content_stylesheet_gate
    gate.reset()
    lifecycle_events = attach_cdp_event_collector(cdp, ["Page.lifecycleEvent"])
    try:
        result = await asyncio.wait_for(
            cdp.send(
                "Page.setDocumentContent",
                {
                    "frameId": frame["id"],
                    "html": (
                        '<body><main id="document-content-before-sheet">before</main>'
                        f'<link rel="stylesheet" href="{stylesheet_url}">'
                        '<footer id="document-content-after-sheet">after</footer></body>'
                    ),
                },
            ),
            timeout=5,
        )
        assert_equal(result, {}, "stylesheet-gated Page.setDocumentContent response")
        request_seen = await asyncio.wait_for(
            asyncio.to_thread(gate.request_seen.wait, 5),
            timeout=6,
        )
        if not request_seen:
            raise SmokeError("body stylesheet request did not reach the fixture gate")
        assert_equal(
            await _evaluate_value(
                cdp,
                """
                ({
                  before: document.querySelector("#document-content-before-sheet")?.textContent,
                  after: document.querySelector("#document-content-after-sheet"),
                })
                """,
            ),
            {"before": "before", "after": None},
            "body stylesheet parser pause boundary",
        )

        gate.release_response.set()
        response_completed = await asyncio.wait_for(
            asyncio.to_thread(gate.response_completed.wait, 5),
            timeout=6,
        )
        if not response_completed:
            raise SmokeError("fixture did not complete the gated stylesheet response")

        async def parser_resumed() -> bool:
            value = await _evaluate_value(
                cdp,
                """
                (() => {
                  const after = document.querySelector("#document-content-after-sheet");
                  return after
                    ? {
                        text: after.textContent,
                        color: getComputedStyle(after).color,
                        readyState: document.readyState,
                      }
                    : null;
                })()
                """,
            )
            return value == {
                "text": "after",
                "color": "rgb(71, 72, 73)",
                "readyState": "complete",
            }

        await wait_until(
            parser_resumed,
            "setDocumentContent parser resume after body stylesheet",
        )
        await wait_until(
            lambda: any(
                event["params"].get("frameId") == frame["id"]
                and event["params"].get("name") == "load"
                for event in lifecycle_events
            ),
            "setDocumentContent load lifecycle after body stylesheet",
        )
    finally:
        gate.release_response.set()

    state.record("cdp_set_document_content_body_stylesheet_parser_pause")


async def _verify_child_document_replacement(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp
    await page.set_content(
        """
        <main id="document-content-parent">parent remains</main>
        <iframe id="document-content-child"
                srcdoc="<main id='document-content-child-old'>old child</main>"></iframe>
        """,
        wait_until="load",
    )
    before_tree = await cdp.send("Page.getFrameTree")
    before_root = before_tree.get("frameTree", {})
    children = before_root.get("childFrames") or []
    if len(children) != 1:
        raise SmokeError(f"expected one child frame before setDocumentContent: {before_tree}")
    before_child = children[0].get("frame", {})
    if not before_child.get("id"):
        raise SmokeError(f"child frame has no frameId before setDocumentContent: {before_tree}")

    await page.evaluate(
        """
        () => {
          const frame = document.querySelector("#document-content-child");
          globalThis.__documentContentOldChildDocument = frame.contentDocument;
          globalThis.__documentContentOldChildNode =
            frame.contentDocument.querySelector("#document-content-child-old");
        }
        """
    )
    result = await cdp.send(
        "Page.setDocumentContent",
        {
            "frameId": before_child["id"],
            "html": (
                '<main id="document-content-child-new">new child</main>'
                "<script>window.__documentContentChildInlineRuns = "
                "(window.__documentContentChildInlineRuns || 0) + 1;</script>"
            ),
        },
    )
    assert_equal(result, {}, "child Page.setDocumentContent response")
    child_state = await _evaluate_value(
        cdp,
        """
        (() => {
          const frame = document.querySelector("#document-content-child");
          return {
            parentText: document.querySelector("#document-content-parent")?.textContent,
            sameDocument: frame.contentDocument === __documentContentOldChildDocument,
            oldNodeConnected: __documentContentOldChildNode.isConnected,
            childText: frame.contentDocument
              .querySelector("#document-content-child-new")?.textContent,
            inlineRuns: frame.contentWindow.__documentContentChildInlineRuns,
          };
        })()
        """,
    )
    assert_equal(
        child_state,
        {
            "parentText": "parent remains",
            "sameDocument": True,
            "oldNodeConnected": False,
            "childText": "new child",
            "inlineRuns": 1,
        },
        "child setDocumentContent state",
    )

    after_tree = await cdp.send("Page.getFrameTree")
    after_root = after_tree.get("frameTree", {})
    after_children = after_root.get("childFrames") or []
    if len(after_children) != 1:
        raise SmokeError(f"expected one child frame after setDocumentContent: {after_tree}")
    after_child = after_children[0].get("frame", {})
    for field in ("id", "loaderId", "url"):
        assert_equal(
            after_child.get(field),
            before_child.get(field),
            f"child setDocumentContent preserved frame {field}",
        )
    state.record("cdp_set_document_content_child_replacement")


async def _verify_errors_leave_the_document_unchanged(state: SmokeState) -> None:
    page = state.page
    cdp = state.cdp
    await page.set_content(
        '<main id="document-content-still-present">unchanged</main>',
        wait_until="load",
    )
    await page.evaluate(
        """
        () => {
          globalThis.__documentContentBeforeErrorDocument = document;
          globalThis.__documentContentBeforeErrorNode =
            document.querySelector("#document-content-still-present");
        }
        """
    )
    frame = await _root_frame(cdp)

    invalid_frame_error = await _send_expect_error(
        cdp,
        "Page.setDocumentContent",
        {
            "frameId": "FRAME-does-not-exist",
            "html": '<main id="document-content-should-not-appear">bad</main>',
        },
    )
    if "No frame for given id found" not in invalid_frame_error:
        raise SmokeError(
            "invalid-frame Page.setDocumentContent returned the wrong error: "
            f"{invalid_frame_error}"
        )
    invalid_params_error = await _send_expect_error(
        cdp,
        "Page.setDocumentContent",
        {"frameId": frame["id"]},
    )
    if "Invalid parameters" not in invalid_params_error:
        raise SmokeError(
            "missing-html Page.setDocumentContent returned the wrong error: "
            f"{invalid_params_error}"
        )

    assert_equal(
        await page.evaluate(
            """
            () => ({
              sameDocument: document === __documentContentBeforeErrorDocument,
              sameNode:
                document.querySelector("#document-content-still-present")
                  === __documentContentBeforeErrorNode,
              connected: __documentContentBeforeErrorNode.isConnected,
              text: __documentContentBeforeErrorNode.textContent,
              badNode: document.querySelector("#document-content-should-not-appear"),
            })
            """
        ),
        {
            "sameDocument": True,
            "sameNode": True,
            "connected": True,
            "text": "unchanged",
            "badNode": None,
        },
        "failed Page.setDocumentContent atomicity",
    )
    state.record("cdp_set_document_content_error_atomicity")


async def _root_frame(cdp: Any) -> dict[str, Any]:
    tree = await cdp.send("Page.getFrameTree")
    frame = tree.get("frameTree", {}).get("frame", {})
    if not frame.get("id"):
        raise SmokeError(f"Page.getFrameTree did not expose a root frame: {tree}")
    return frame


async def _goto_and_wait_for_frame_stopped_loading(
    page: Any,
    cdp: Any,
    url: str,
) -> Any:
    """Navigate and consume the terminal loading edge for that exact turn."""
    await cdp.send("Page.enable")
    frame_id = (await _root_frame(cdp))["id"]
    started = False
    stopped = asyncio.Event()

    def on_started(params: dict[str, Any]) -> None:
        nonlocal started
        if params.get("frameId") == frame_id:
            started = True

    def on_stopped(params: dict[str, Any]) -> None:
        if started and params.get("frameId") == frame_id:
            stopped.set()

    cdp.on("Page.frameStartedLoading", on_started)
    cdp.on("Page.frameStoppedLoading", on_stopped)
    try:
        response = await page.goto(url, wait_until="load", timeout=10_000)
        try:
            await asyncio.wait_for(stopped.wait(), timeout=10)
        except TimeoutError as error:
            raise SmokeError(
                f"navigation did not emit Page.frameStoppedLoading for {frame_id}: {url}"
            ) from error
        return response
    finally:
        cdp.remove_listener("Page.frameStartedLoading", on_started)
        cdp.remove_listener("Page.frameStoppedLoading", on_stopped)


async def _evaluate_value(cdp: Any, expression: str) -> Any:
    result = await cdp.send(
        "Runtime.evaluate",
        {
            "expression": expression,
            "returnByValue": True,
            "awaitPromise": True,
        },
    )
    if result.get("exceptionDetails"):
        raise SmokeError(f"Runtime.evaluate failed: {result}")
    return result.get("result", {}).get("value")


async def _send_expect_error(
    cdp: Any,
    method: str,
    params: dict[str, Any],
) -> str:
    try:
        await cdp.send(method, params)
    except PlaywrightError as error:
        return str(error)
    raise SmokeError(f"{method} unexpectedly succeeded for {params}")


def _has_event(events: list[dict[str, Any]], method: str) -> bool:
    return any(event["method"] == method for event in events)


def _has_lifecycle_event(events: list[dict[str, Any]], name: str) -> bool:
    return any(
        event["method"] == "Page.lifecycleEvent"
        and event["params"].get("name") == name
        for event in events
    )


def _document_content_event_label(event: dict[str, Any]) -> str:
    method = event["method"]
    params = event["params"]
    if method == "Page.lifecycleEvent":
        return f"{method}:{params.get('name')}"
    if method == "Runtime.consoleAPICalled":
        return f"{method}:{(params.get('args') or [{}])[0].get('value')}"
    return method


def _required_document_content_event_labels(
    *,
    lifecycle_enabled: bool,
    classic_marker: str | None = None,
) -> list[str]:
    required = ["Page.documentOpened"]
    if lifecycle_enabled:
        required.append("Page.lifecycleEvent:init")
    if classic_marker is not None:
        required.append(f"Runtime.consoleAPICalled:{classic_marker}")
    required.extend(["DOM.documentUpdated", "Page.domContentEventFired"])
    if lifecycle_enabled:
        required.append("Page.lifecycleEvent:DOMContentLoaded")
    required.append("Page.loadEventFired")
    if lifecycle_enabled:
        required.append("Page.lifecycleEvent:load")
    return required


def _contains_ordered_labels(observed: list[str], required: list[str]) -> bool:
    next_required = 0
    for event in observed:
        if event == required[next_required]:
            next_required += 1
            if next_required == len(required):
                return True
    return False


async def _wait_for_document_content_event_sequence(
    events: list[dict[str, Any]],
    start: int,
    label: str,
    *,
    lifecycle_enabled: bool,
    classic_marker: str | None = None,
) -> None:
    required = _required_document_content_event_labels(
        lifecycle_enabled=lifecycle_enabled,
        classic_marker=classic_marker,
    )
    try:
        await wait_until(
            lambda: _contains_ordered_labels(
                [_document_content_event_label(event) for event in events[start:]],
                required,
            ),
            label,
        )
    except SmokeError as error:
        observed = [_document_content_event_label(event) for event in events[start:]]
        raise SmokeError(
            f"{error}; required={required}, observed={observed}"
        ) from error


def _assert_finite_timestamp(value: Any, label: str) -> None:
    if (
        not isinstance(value, (int, float))
        or isinstance(value, bool)
        or not math.isfinite(value)
        or value < 0
    ):
        raise SmokeError(
            f"{label}: expected a finite non-negative timestamp, got {value!r}"
        )


def _assert_detailed_document_content_event_trace(
    events: list[dict[str, Any]],
    label: str,
    *,
    frame_id: str,
    loader_id: str,
    url: str,
    lifecycle_enabled: bool = True,
    classic_marker: str | None = None,
) -> None:
    required = _required_document_content_event_labels(
        lifecycle_enabled=lifecycle_enabled,
        classic_marker=classic_marker,
    )
    observed = [_document_content_event_label(event) for event in events]
    if not _contains_ordered_labels(observed, required):
        raise SmokeError(
            f"{label} did not match Chromium's event order; "
            f"required={required}, observed={observed}"
        )
    for required_event in required:
        assert_equal(
            observed.count(required_event),
            1,
            f"{label} {required_event} count",
        )

    document_opened = [
        event for event in events if event["method"] == "Page.documentOpened"
    ]
    assert_equal(len(document_opened), 1, f"{label} Page.documentOpened count")
    opened_frame = document_opened[0]["params"].get("frame") or {}
    assert_equal(opened_frame.get("id"), frame_id, f"{label} opened frame id")
    assert_equal(
        opened_frame.get("loaderId"),
        loader_id,
        f"{label} opened frame loader id",
    )
    assert_equal(opened_frame.get("url"), url, f"{label} opened frame url")
    assert_equal(
        opened_frame.get("mimeType"),
        "text/html",
        f"{label} opened frame MIME type",
    )
    assert_equal(
        opened_frame.get("parentId"),
        None,
        f"{label} root frame parent id",
    )

    lifecycle_events = [
        event
        for event in events
        if event["method"] == "Page.lifecycleEvent"
        and event["params"].get("name") in {"init", "DOMContentLoaded", "load"}
    ]
    expected_lifecycle_names = (
        ["init", "DOMContentLoaded", "load"] if lifecycle_enabled else []
    )
    assert_equal(
        [event["params"].get("name") for event in lifecycle_events],
        expected_lifecycle_names,
        f"{label} lifecycle names",
    )
    for event in lifecycle_events:
        params = event["params"]
        event_name = params.get("name")
        assert_equal(
            params.get("frameId"),
            frame_id,
            f"{label} lifecycle {event_name} frame id",
        )
        assert_equal(
            params.get("loaderId"),
            loader_id,
            f"{label} lifecycle {event_name} loader id",
        )
        _assert_finite_timestamp(
            params.get("timestamp"),
            f"{label} lifecycle {event_name} timestamp",
        )

    for method in ("Page.domContentEventFired", "Page.loadEventFired"):
        matching = [event for event in events if event["method"] == method]
        assert_equal(len(matching), 1, f"{label} {method} count")
        _assert_finite_timestamp(
            matching[0]["params"].get("timestamp"),
            f"{label} {method} timestamp",
        )

    document_updates = [
        event for event in events if event["method"] == "DOM.documentUpdated"
    ]
    assert_equal(len(document_updates), 1, f"{label} DOM.documentUpdated count")
    assert_equal(
        document_updates[0]["params"],
        {},
        f"{label} DOM.documentUpdated params",
    )

    forbidden_methods = {
        "Page.frameScheduledNavigation",
        "Page.frameRequestedNavigation",
        "Page.frameStartedNavigating",
        "Page.frameStartedLoading",
        "Page.frameNavigated",
        "Page.frameStoppedLoading",
        "Page.frameClearedScheduledNavigation",
        "Page.navigatedWithinDocument",
        "Runtime.executionContextsCleared",
        "Runtime.executionContextDestroyed",
    }
    assert_equal(
        [event["method"] for event in events if event["method"] in forbidden_methods],
        [],
        f"{label} navigation/realm teardown events",
    )


def _assert_document_content_event_order(
    events: list[dict[str, Any]],
    label: str,
    *,
    classic_marker: str | None = None,
) -> None:
    observed = [_document_content_event_label(event) for event in events]
    required = _required_document_content_event_labels(
        lifecycle_enabled=True,
        classic_marker=classic_marker,
    )
    if not _contains_ordered_labels(observed, required):
        raise SmokeError(
            f"{label} did not match Chromium's event order; "
            f"required={required}, observed={observed}"
        )


def _document_content_completion_observer(
    cdp: Any,
) -> tuple[list[dict[str, Any]], asyncio.Event]:
    events: list[dict[str, Any]] = []
    complete = asyncio.Event()
    observed: set[str] = set()
    lifecycle_names: set[str] = set()

    def record(method: str, params: dict[str, Any]) -> None:
        events.append({"method": method, "params": params})
        observed.add(method)
        if method == "Page.lifecycleEvent" and isinstance(params.get("name"), str):
            lifecycle_names.add(params["name"])
        if (
            {"DOM.documentUpdated", "Page.loadEventFired"} <= observed
            and "load" in lifecycle_names
        ):
            complete.set()

    for method in (
        "Page.documentOpened",
        "Page.frameScheduledNavigation",
        "Page.frameRequestedNavigation",
        "Page.frameStartedNavigating",
        "Page.frameStartedLoading",
        "Page.frameNavigated",
        "Page.frameStoppedLoading",
        "Page.frameClearedScheduledNavigation",
        "Page.navigatedWithinDocument",
        "Page.lifecycleEvent",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "DOM.documentUpdated",
        "Runtime.consoleAPICalled",
        "Runtime.executionContextsCleared",
        "Runtime.executionContextDestroyed",
    ):
        cdp.on(method, lambda params, method=method: record(method, params))
    return events, complete
