from __future__ import annotations


import asyncio
import json
from contextlib import asynccontextmanager, suppress
from dataclasses import dataclass
from typing import Any, AsyncIterator, Awaitable, Callable

from ..diagnostics import new_cdp_session
from ..navigation import navigate_cdp
from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..helpers import attach_cdp_event_collector
from ..state import SmokeState


ERROR_DOCUMENT_URL = "chrome-error://chromewebdata/"
RESET_ROUTE = "/chromium-network-reset-before-response"
REDIRECT_ROUTE = "/chromium-network-redirect-before-reset"


ErrorDocumentScenario = Callable[[SmokeState], Awaitable[dict[str, Any]]]


@dataclass(frozen=True)
class ErrorDocumentContract:
    name: str
    contract: str
    commands: list[str]
    scenario: ErrorDocumentScenario


@dataclass
class ErrorDocumentProbe:
    page: Any
    cdp: Any
    events: list[dict[str, Any]]
    target_id: str
    initial_loader_id: str
    initial_context_unique_id: str


async def run_error_document_group(state: SmokeState) -> None:
    source = "Chromium error-document behavior and CDP Page/Runtime/Network domains"
    for item in _error_document_contracts():
        try:
            observed = await item.scenario(state)
        except Exception as error:
            state.results.append(
                {
                    "name": item.name,
                    "ok": False,
                    "contract": item.contract,
                    "source": source,
                    "commands": item.commands,
                    "errorType": type(error).__name__,
                    "error": str(error),
                }
            )
        else:
            record_contract(
                state.results,
                item.name,
                contract=item.contract,
                source=source,
                commands=item.commands,
                observed=observed,
            )


def _error_document_contracts() -> tuple[ErrorDocumentContract, ...]:
    identity_commands = [
        "Network.enable",
        "Page.enable",
        "Page.setLifecycleEventsEnabled",
        "Runtime.enable",
        "Page.navigate",
        "Page.getFrameTree",
        "Page.getNavigationHistory",
        "Runtime.evaluate",
        "Target.getTargetInfo",
    ]
    return (
        ErrorDocumentContract(
            "error_document_direct_failure_identity_and_order",
            "A main-document transport reset commits a browser-owned error Document with its own loader and realm, while frame, Target, history, Runtime, and Network expose one coherent failed-navigation identity and lifecycle.",
            identity_commands,
            _direct_failure_identity_and_order,
        ),
        ErrorDocumentContract(
            "error_document_replaces_existing_realm",
            "A transport failure after a successful document retires the old loader, realm, and globals instead of exposing the old DOM or leaving Runtime without a document.",
            identity_commands,
            _failure_replaces_existing_realm,
        ),
        ErrorDocumentContract(
            "error_document_redirect_failure_uses_final_unreachable_url",
            "When a redirect target resets before response metadata, the error Document records the final failed hop as unreachableUrl and keeps the redirect chain on one Network request identity.",
            identity_commands,
            _redirect_failure_uses_final_unreachable_url,
        ),
        ErrorDocumentContract(
            "error_document_consecutive_failures_advance_generation",
            "Consecutive transport failures each commit a fresh error Document generation and advance history to the latest unreachable URL.",
            identity_commands,
            _consecutive_failures_advance_generation,
        ),
        ErrorDocumentContract(
            "error_document_recovers_to_success_document",
            "A normal navigation after an error Document installs another fresh realm, clears unreachableUrl, and restores ordinary frame, Target, history, and Runtime identity.",
            identity_commands,
            _error_document_recovers_to_success,
        ),
        ErrorDocumentContract(
            "error_document_parallel_targets_are_isolated",
            "Concurrent failures in two targets commit independent error Documents without crossing loader, realm, Target, history, or unreachable URL ownership.",
            identity_commands,
            _parallel_targets_are_isolated,
        ),
    )


@asynccontextmanager
async def _new_probe(state: SmokeState) -> AsyncIterator[ErrorDocumentProbe]:
    page = await state.context.new_page()
    cdp = await new_cdp_session(state.context, page)
    methods = [
        "Network.requestWillBeSent",
        "Network.responseReceived",
        "Network.loadingFailed",
        "Network.loadingFinished",
        "Page.frameNavigated",
        "Page.domContentEventFired",
        "Page.loadEventFired",
        "Runtime.executionContextsCleared",
        "Runtime.executionContextCreated",
    ]
    events = attach_cdp_event_collector(cdp, methods)
    try:
        await cdp.send("Page.enable")
        await cdp.send("Network.enable")
        await cdp.send("Page.setLifecycleEventsEnabled", {"enabled": True})
        await cdp.send("Runtime.enable")
        await wait_until(
            lambda: bool(_default_contexts(events)),
            "initial default Runtime execution context",
        )
        initial_frame = _main_frame(await cdp.send("Page.getFrameTree"))
        target_id = _required_string(initial_frame, "id", "initial frame id")
        initial_loader_id = _required_string(
            initial_frame,
            "loaderId",
            "initial frame loader id",
        )
        initial_context = _default_contexts(events)[-1]
        yield ErrorDocumentProbe(
            page=page,
            cdp=cdp,
            events=events,
            target_id=target_id,
            initial_loader_id=initial_loader_id,
            initial_context_unique_id=initial_context["uniqueId"],
        )
    finally:
        with suppress(Exception):
            await cdp.detach()
        with suppress(Exception):
            await page.close()


async def _direct_failure_identity_and_order(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        failed_url = _reset_url(state, "direct")
        failure = await _commit_error_document(probe, failed_url)
        if failure["contextUniqueId"] == probe.initial_context_unique_id:
            raise SmokeError("error Document reused the initial empty Document realm")
        if failure["loaderId"] == probe.initial_loader_id:
            raise SmokeError("error Document reused the initial empty Document loader")
        return _compact_failure(failure)


async def _failure_replaces_existing_realm(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        loaded_url = f"{state.fixture}/plain?error-document=old-realm"
        loaded = await _commit_success_document(probe, loaded_url)
        marker = await _evaluate_value(
            probe.cdp,
            "globalThis.__errorDocumentOldRealmMarker = 'old-realm'",
        )
        assert_equal(marker, "old-realm", "old Document marker installation")

        failure = await _commit_error_document(
            probe,
            _reset_url(state, "replace-existing"),
        )
        assert_equal(
            failure["snapshot"].get("oldMarkerType"),
            "undefined",
            "error Document old global visibility",
        )
        if failure["contextUniqueId"] == loaded["contextUniqueId"]:
            raise SmokeError("error Document reused the successful Document realm")
        if failure["loaderId"] == loaded["loaderId"]:
            raise SmokeError("error Document reused the successful Document loader")
        if "Runtime.executionContextsCleared" not in failure["eventMethods"]:
            raise SmokeError(
                "failed navigation did not publish Runtime.executionContextsCleared"
            )
        return {
            "oldLoaderId": loaded["loaderId"],
            "errorLoaderId": failure["loaderId"],
            "oldContextId": loaded["contextId"],
            "errorContextId": failure["contextId"],
            "oldContextUniqueId": loaded["contextUniqueId"],
            "errorContextUniqueId": failure["contextUniqueId"],
        }


async def _redirect_failure_uses_final_unreachable_url(
    state: SmokeState,
) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        requested_url = f"{state.fixture}{REDIRECT_ROUTE}?error-document=redirect"
        final_url = f"{state.fixture}{RESET_ROUTE}"
        failure = await _commit_error_document(
            probe,
            requested_url,
            expected_unreachable_url=final_url,
            expected_request_urls=[requested_url, final_url],
        )
        return _compact_failure(failure)


async def _consecutive_failures_advance_generation(
    state: SmokeState,
) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        first = await _commit_error_document(probe, _reset_url(state, "first"))
        second = await _commit_error_document(probe, _reset_url(state, "second"))
        if first["loaderId"] == second["loaderId"]:
            raise SmokeError("consecutive error Documents reused one loader")
        if first["contextUniqueId"] == second["contextUniqueId"]:
            raise SmokeError("consecutive error Documents reused one realm")
        if second["historyIndex"] <= first["historyIndex"]:
            raise SmokeError(
                "consecutive error Documents did not advance navigation history: "
                f"first={first['historyIndex']} second={second['historyIndex']}"
            )
        return {
            "first": _compact_failure(first),
            "second": _compact_failure(second),
        }


async def _error_document_recovers_to_success(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        failure = await _commit_error_document(probe, _reset_url(state, "recovery"))
        recovered_url = f"{state.fixture}/plain?error-document=recovered"
        recovered = await _commit_success_document(probe, recovered_url)
        if recovered["loaderId"] == failure["loaderId"]:
            raise SmokeError("successful recovery reused the error Document loader")
        if recovered["contextUniqueId"] == failure["contextUniqueId"]:
            raise SmokeError("successful recovery reused the error Document realm")
        assert_equal(
            recovered["snapshot"].get("href"),
            recovered_url,
            "recovered Runtime location",
        )
        assert_equal(
            recovered["snapshot"].get("text"),
            "plain ok",
            "recovered Document body",
        )
        return {
            "errorLoaderId": failure["loaderId"],
            "recoveredLoaderId": recovered["loaderId"],
            "errorContextId": failure["contextId"],
            "recoveredContextId": recovered["contextId"],
            "errorContextUniqueId": failure["contextUniqueId"],
            "recoveredContextUniqueId": recovered["contextUniqueId"],
            "recoveredUrl": recovered_url,
        }


async def _parallel_targets_are_isolated(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as first_probe, _new_probe(state) as second_probe:
        first_url = _reset_url(state, "parallel-a")
        second_url = _reset_url(state, "parallel-b")
        first, second = await asyncio.gather(
            _commit_error_document(first_probe, first_url),
            _commit_error_document(second_probe, second_url),
        )
        if first_probe.target_id == second_probe.target_id:
            raise SmokeError("parallel error probes unexpectedly shared one target")
        if first["loaderId"] == second["loaderId"]:
            raise SmokeError("parallel error Documents crossed loader identity")
        if first["contextUniqueId"] == second["contextUniqueId"]:
            raise SmokeError("parallel error Documents crossed realm identity")
        assert_equal(first["unreachableUrl"], first_url, "first target unreachable URL")
        assert_equal(second["unreachableUrl"], second_url, "second target unreachable URL")
        return {
            "first": _compact_failure(first),
            "second": _compact_failure(second),
        }


async def _commit_error_document(
    probe: ErrorDocumentProbe,
    requested_url: str,
    *,
    expected_unreachable_url: str | None = None,
    expected_request_urls: list[str] | None = None,
) -> dict[str, Any]:
    expected_unreachable_url = expected_unreachable_url or requested_url
    expected_request_urls = expected_request_urls or [requested_url]
    start = len(probe.events)
    navigation = await probe.cdp.send("Page.navigate", {"url": requested_url})
    error_text = navigation.get("errorText")
    if not isinstance(error_text, str) or not error_text:
        raise SmokeError(f"failed navigation did not return errorText: {navigation}")
    assert_equal(
        error_text,
        "net::ERR_CONNECTION_RESET",
        "failed navigation browser error text",
    )
    assert_equal(navigation.get("isDownload"), False, "failed navigation isDownload")
    loader_id = _required_string(navigation, "loaderId", "failed navigation loaderId")
    frame_id = _required_string(navigation, "frameId", "failed navigation frameId")
    assert_equal(frame_id, probe.target_id, "failed navigation target frame")

    try:
        await wait_until(
            lambda: _error_document_load_completed(
                probe.events[start:],
                loader_id,
                expected_unreachable_url,
            ),
            f"error Document load for {requested_url}",
            timeout_ms=3_000,
        )
    except SmokeError as load_error:
        try:
            await _runtime_snapshot(probe.cdp)
        except Exception as runtime_error:
            raise SmokeError(
                f"{load_error}; Runtime.evaluate after failed navigation returned: "
                f"{runtime_error}"
            ) from runtime_error
        raise
    events = probe.events[start:]
    frame_event = next(
        event
        for event in events
        if event.get("method") == "Page.frameNavigated"
        and event.get("params", {}).get("frame", {}).get("loaderId") == loader_id
    )
    frame = frame_event["params"]["frame"]
    assert_equal(frame.get("url"), ERROR_DOCUMENT_URL, "error Document frame URL")
    assert_equal(
        frame.get("unreachableUrl"),
        expected_unreachable_url,
        "error Document frame unreachable URL",
    )
    assert_equal(frame.get("securityOrigin"), "://", "error Document security origin")
    assert_equal(
        frame.get("secureContextType"),
        "InsecureScheme",
        "error Document secure context type",
    )

    request_events = [
        event
        for event in events
        if event.get("method") == "Network.requestWillBeSent"
        and event.get("params", {}).get("type") == "Document"
    ]
    request_urls = [
        event.get("params", {}).get("request", {}).get("url")
        for event in request_events
    ]
    assert_equal(request_urls, expected_request_urls, "failed navigation request URL chain")
    request_id = _required_string(
        request_events[0].get("params", {}),
        "requestId",
        "failed navigation requestId",
    )
    if any(
        event.get("params", {}).get("requestId") != request_id
        for event in request_events
    ):
        raise SmokeError(f"redirect hops did not share one requestId: {request_events}")
    correlated = [
        event
        for event in events
        if event.get("params", {}).get("requestId") == request_id
    ]
    correlated_methods = [event.get("method") for event in correlated]
    assert_equal(
        correlated_methods.count("Network.loadingFailed"),
        1,
        "failed navigation loadingFailed count",
    )
    assert_equal(
        correlated_methods.count("Network.loadingFinished"),
        1,
        "error Document late loadingFinished count",
    )
    assert_equal(
        correlated_methods.count("Network.responseReceived"),
        0,
        "failed navigation final response count",
    )
    failed = next(
        event for event in correlated if event.get("method") == "Network.loadingFailed"
    )
    assert_equal(
        failed.get("params", {}).get("errorText"),
        error_text,
        "Page.navigate and Network.loadingFailed error text",
    )
    _assert_error_document_event_order(events, request_id, loader_id)

    tree_frame = _main_frame(await probe.cdp.send("Page.getFrameTree"))
    assert_equal(tree_frame.get("loaderId"), loader_id, "error frame-tree loader")
    assert_equal(tree_frame.get("url"), ERROR_DOCUMENT_URL, "error frame-tree URL")
    assert_equal(
        tree_frame.get("unreachableUrl"),
        expected_unreachable_url,
        "error frame-tree unreachable URL",
    )
    assert_equal(
        tree_frame.get("securityOrigin"),
        "://",
        "error frame-tree security origin",
    )
    assert_equal(
        tree_frame.get("secureContextType"),
        "InsecureScheme",
        "error frame-tree secure context type",
    )

    target_info = (
        await probe.cdp.send("Target.getTargetInfo", {"targetId": probe.target_id})
    ).get("targetInfo", {})
    assert_equal(
        target_info.get("url"),
        expected_unreachable_url,
        "error Document Target URL",
    )
    history = await probe.cdp.send("Page.getNavigationHistory")
    history_index, history_entry = _current_history_entry(history)
    assert_equal(
        history_entry.get("url"),
        expected_unreachable_url,
        "error Document history URL",
    )
    snapshot = await _runtime_snapshot(probe.cdp)
    assert_equal(snapshot.get("href"), ERROR_DOCUMENT_URL, "error Runtime location")
    assert_equal(snapshot.get("origin"), "null", "error Runtime origin")
    assert_equal(snapshot.get("ready"), "complete", "error Runtime readyState")
    if not isinstance(snapshot.get("title"), str) or not snapshot["title"]:
        raise SmokeError(f"error Document title was empty: {snapshot}")

    contexts = _default_contexts(events)
    if not contexts:
        raise SmokeError(f"error Document emitted no default Runtime context: {events}")
    context = contexts[-1]
    return {
        "requestedUrl": requested_url,
        "unreachableUrl": expected_unreachable_url,
        "loaderId": loader_id,
        "contextId": context["id"],
        "contextUniqueId": context["uniqueId"],
        "requestId": request_id,
        "historyIndex": history_index,
        "snapshot": snapshot,
        "eventMethods": [event.get("method") for event in events],
        "requestUrls": request_urls,
    }


async def _commit_success_document(
    probe: ErrorDocumentProbe,
    url: str,
) -> dict[str, Any]:
    start = len(probe.events)
    navigation = await navigate_cdp(probe.cdp, url)
    loader_id = _required_string(navigation, "loaderId", "successful navigation loaderId")
    committed = next(index for index, event in enumerate(probe.events[start:], start)
                     if event.get("method") == "Page.frameNavigated"
                     and event.get("params", {}).get("frame", {}).get("id") == navigation["frameId"]
                     and event.get("params", {}).get("frame", {}).get("loaderId") == loader_id)
    await wait_until(
        lambda: any(event.get("method") == "Page.loadEventFired"
                    for event in probe.events[committed:]),
        f"successful Document load for {url} loader={loader_id}",
    )
    events = probe.events[start:]
    frame = _main_frame(await probe.cdp.send("Page.getFrameTree"))
    assert_equal(frame.get("loaderId"), loader_id, "successful frame-tree loader")
    assert_equal(frame.get("url"), url, "successful frame-tree URL")
    if "unreachableUrl" in frame:
        raise SmokeError(f"successful frame retained unreachableUrl: {frame}")
    target_info = (
        await probe.cdp.send("Target.getTargetInfo", {"targetId": probe.target_id})
    ).get("targetInfo", {})
    assert_equal(target_info.get("url"), url, "successful Target URL")
    history = await probe.cdp.send("Page.getNavigationHistory")
    history_index, history_entry = _current_history_entry(history)
    assert_equal(history_entry.get("url"), url, "successful history URL")
    contexts = _default_contexts(events)
    if not contexts:
        raise SmokeError(f"successful Document emitted no default Runtime context: {events}")
    context = contexts[-1]
    return {
        "loaderId": loader_id,
        "contextId": context["id"],
        "contextUniqueId": context["uniqueId"],
        "historyIndex": history_index,
        "snapshot": await _runtime_snapshot(probe.cdp),
    }


def _assert_error_document_event_order(
    events: list[dict[str, Any]],
    request_id: str,
    loader_id: str,
) -> None:
    request_index = _event_index(
        events,
        "Network.requestWillBeSent",
        request_id=request_id,
    )
    failed_index = _event_index(events, "Network.loadingFailed", request_id=request_id)
    frame_index = _event_index(events, "Page.frameNavigated", loader_id=loader_id)
    finished_index = _event_index(
        events,
        "Network.loadingFinished",
        request_id=request_id,
    )
    dcl_index = _event_index(events, "Page.domContentEventFired")
    load_index = _event_index(events, "Page.loadEventFired")
    if not (
        request_index
        < failed_index
        < frame_index
        < finished_index
        < dcl_index
        < load_index
    ):
        raise SmokeError(
            "error Document event order was not "
            "request -> failed -> frame commit -> finished -> DCL -> load: "
            f"{[event.get('method') for event in events]}"
        )


def _event_index(
    events: list[dict[str, Any]],
    method: str,
    *,
    request_id: str | None = None,
    loader_id: str | None = None,
) -> int:
    for index, event in enumerate(events):
        if event.get("method") != method:
            continue
        params = event.get("params", {})
        if request_id is not None and params.get("requestId") != request_id:
            continue
        if loader_id is not None and params.get("frame", {}).get("loaderId") != loader_id:
            continue
        return index
    raise SmokeError(
        f"missing {method} for requestId={request_id!r} loaderId={loader_id!r}: {events}"
    )


def _error_document_load_completed(
    events: list[dict[str, Any]],
    loader_id: str,
    unreachable_url: str,
) -> bool:
    committed = any(
        event.get("method") == "Page.frameNavigated"
        and event.get("params", {}).get("frame", {}).get("loaderId") == loader_id
        and event.get("params", {}).get("frame", {}).get("url") == ERROR_DOCUMENT_URL
        and event.get("params", {}).get("frame", {}).get("unreachableUrl")
        == unreachable_url
        for event in events
    )
    loaded = any(event.get("method") == "Page.loadEventFired" for event in events)
    return committed and loaded


async def _runtime_snapshot(cdp: Any) -> dict[str, Any]:
    value = await _evaluate_value(
        cdp,
        """
        JSON.stringify({
          href: location.href,
          origin: location.origin,
          ready: document.readyState,
          title: document.title,
          text: (document.body?.innerText || '').trim(),
          oldMarkerType: typeof globalThis.__errorDocumentOldRealmMarker,
        })
        """,
        await_promise=True,
    )
    if not isinstance(value, str):
        raise SmokeError(f"Runtime snapshot was not serialized JSON: {value!r}")
    snapshot = json.loads(value)
    if not isinstance(snapshot, dict):
        raise SmokeError(f"Runtime snapshot was not an object: {snapshot!r}")
    return snapshot


async def _evaluate_value(
    cdp: Any,
    expression: str,
    *,
    await_promise: bool = False,
) -> Any:
    response = await cdp.send(
        "Runtime.evaluate",
        {
            "expression": expression,
            "returnByValue": True,
            "awaitPromise": await_promise,
        },
    )
    if response.get("exceptionDetails"):
        raise SmokeError(f"Runtime.evaluate raised: {response['exceptionDetails']}")
    return response.get("result", {}).get("value")


def _main_frame(frame_tree: dict[str, Any]) -> dict[str, Any]:
    frame = frame_tree.get("frameTree", {}).get("frame")
    if not isinstance(frame, dict):
        raise SmokeError(f"Page.getFrameTree returned no main frame: {frame_tree}")
    return frame


def _current_history_entry(history: dict[str, Any]) -> tuple[int, dict[str, Any]]:
    index = history.get("currentIndex")
    entries = history.get("entries")
    if not isinstance(index, int) or not isinstance(entries, list):
        raise SmokeError(f"invalid Page.getNavigationHistory result: {history}")
    if index < 0 or index >= len(entries) or not isinstance(entries[index], dict):
        raise SmokeError(f"invalid current navigation history entry: {history}")
    return index, entries[index]


def _default_contexts(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    contexts: list[dict[str, Any]] = []
    for event in events:
        if event.get("method") != "Runtime.executionContextCreated":
            continue
        context = event.get("params", {}).get("context", {})
        if context.get("auxData", {}).get("isDefault") is not True:
            continue
        context_id = context.get("id")
        unique_id = context.get("uniqueId")
        if isinstance(context_id, int) and isinstance(unique_id, str) and unique_id:
            contexts.append({"id": context_id, "uniqueId": unique_id})
    return contexts


def _required_string(value: dict[str, Any], key: str, label: str) -> str:
    result = value.get(key)
    if not isinstance(result, str) or not result:
        raise SmokeError(f"{label} was missing: {value}")
    return result


def _reset_url(state: SmokeState, name: str) -> str:
    return f"{state.fixture}{RESET_ROUTE}?error-document={name}"


def _compact_failure(failure: dict[str, Any]) -> dict[str, Any]:
    return {
        "requestedUrl": failure["requestedUrl"],
        "unreachableUrl": failure["unreachableUrl"],
        "loaderId": failure["loaderId"],
        "contextId": failure["contextId"],
        "contextUniqueId": failure["contextUniqueId"],
        "requestId": failure["requestId"],
        "historyIndex": failure["historyIndex"],
        "requestUrls": failure["requestUrls"],
    }
