from __future__ import annotations


import asyncio
from contextlib import asynccontextmanager, suppress
from dataclasses import dataclass
from functools import partial
from typing import Any, AsyncIterator, Awaitable, Callable

from ..config import is_moli_browser
from ..diagnostics import new_cdp_session
from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..helpers import attach_cdp_event_collector
from ..state import SmokeState


_OLD_DOCUMENT_MARKER = "navigation-outcome-old-document"

NavigationScenario = Callable[[SmokeState], Awaitable[dict[str, Any]]]


@dataclass(frozen=True)
class NavigationOutcomeContract:
    name: str
    contract: str
    scenario: NavigationScenario


@dataclass
class NavigationProbe:
    page: Any
    cdp: Any
    events: list[dict[str, Any]]


async def run_navigation_outcomes_group(state: SmokeState) -> None:
    source = (
        "raw Page.navigate and Network observation calibrated against Debian "
        "Chromium 145.0.7632.116"
    )
    commands = [
        "Page.enable",
        "Page.setLifecycleEventsEnabled",
        "Runtime.enable",
        "Network.enable",
        "Page.navigate",
        "Runtime.evaluate",
    ]
    for item in _contracts():
        try:
            observed = await item.scenario(state)
        except Exception as error:
            state.results.append(
                {
                    "name": item.name,
                    "ok": False,
                    "contract": item.contract,
                    "source": source,
                    "commands": commands,
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
                commands=commands,
                observed=observed,
            )


def _contracts() -> tuple[NavigationOutcomeContract, ...]:
    return (
        NavigationOutcomeContract(
            "navigate_attachment_retains_active_document",
            "A text attachment reports isDownload with response metadata and leaves the active Document and realm untouched.",
            _text_attachment,
        ),
        NavigationOutcomeContract(
            "navigate_html_attachment_overrides_renderable_mime",
            "Content-Disposition attachment remains a download even when the response MIME is renderable HTML.",
            _html_attachment,
        ),
        NavigationOutcomeContract(
            "navigate_empty_attachment_is_download",
            "A zero-byte attachment still reports a download and retains the active Document.",
            _empty_attachment,
        ),
        NavigationOutcomeContract(
            "navigate_redirect_to_attachment_uses_final_response",
            "A redirect into an attachment keeps one Document request identity and exposes the final response as a download.",
            _redirect_attachment,
        ),
        NavigationOutcomeContract(
            "navigate_error_status_attachment_keeps_http_evidence",
            "isDownload remains authoritative when an attachment has an HTTP error status, while Network preserves that status and MIME for classification.",
            _error_status_attachment,
        ),
        NavigationOutcomeContract(
            "navigate_binary_mime_outcome_is_self_consistent",
            "A binary main response always exposes headers and either becomes a download or commits a lifecycle-bearing external Document without mixing the two outcomes.",
            _binary_mime,
        ),
        NavigationOutcomeContract(
            "navigate_http_error_commits_document",
            "An ordinary HTML HTTP error is a successful navigation with status evidence and a DOMContentLoaded Document, not a Page.navigate network error.",
            _http_error_document,
        ),
        *(
            NavigationOutcomeContract(
                name,
                f"A {status} response reports net::ERR_ABORTED without a download or a new Document, including Fetch interception.",
                partial(_no_content, status=status, fetch_stage=stage),
            )
            for name, status, stage in (
                ("navigate_no_content_is_not_download", 204, None),
                ("navigate_reset_content_retains_document", 205, None),
                ("navigate_fulfilled_no_content_retains_document", 204, "Request"),
                ("navigate_fulfilled_reset_content_retains_document", 205, "Request"),
                ("navigate_overridden_no_content_retains_document", 204, "Response"),
                ("navigate_overridden_reset_content_retains_document", 205, "Response"),
            )
        ),
        *(
            NavigationOutcomeContract(
                f"navigate_{name.replace('-', '_')}_attachment_is_download",
                f"An attachment remains a download even when its response status is {status}.",
                partial(
                    _successful_download,
                    route=f"/navigation-{name}-attachment",
                    expected_mime="text/plain",
                    expected_status=status,
                ),
            )
            for name, status in (("no-content", 204), ("reset-content", 205))
        ),
        NavigationOutcomeContract(
            "navigate_transport_failure_is_not_download",
            "A reset before response metadata reports a non-download navigation error and a matching Network.loadingFailed terminal.",
            _transport_failure,
        ),
        NavigationOutcomeContract(
            "navigate_fail_request_blocked_by_client_retains_document",
            "Fetch.failRequest(BlockedByClient) on a paused main-document navigation answers Page.navigate with the error, emits exactly one Network.loadingFailed terminal, and leaves the committed Document responsive.",
            _fail_request_blocked_by_client,
        ),
    )


@asynccontextmanager
async def _new_probe(state: SmokeState) -> AsyncIterator[NavigationProbe]:
    page = await state.context.new_page()
    cdp = await new_cdp_session(state.context, page)
    events = attach_cdp_event_collector(
        cdp,
        [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFailed",
            "Network.loadingFinished",
            "Page.loadEventFired",
            "Page.lifecycleEvent",
            "Runtime.executionContextsCleared",
        ],
    )
    try:
        await cdp.send("Page.enable")
        await cdp.send("Network.enable")
        await cdp.send("Runtime.enable")
        await cdp.send("Page.setLifecycleEventsEnabled", {"enabled": True})
        marker = await _evaluate_value(
            cdp,
            f"globalThis.__navigationOutcomeMarker = {_OLD_DOCUMENT_MARKER!r}",
        )
        assert_equal(marker, _OLD_DOCUMENT_MARKER, "initial Document marker")
        events.clear()
        yield NavigationProbe(page=page, cdp=cdp, events=events)
    finally:
        with suppress(Exception):
            await cdp.detach()
        with suppress(Exception):
            await page.close()


async def _text_attachment(state: SmokeState) -> dict[str, Any]:
    return await _successful_download(
        state,
        route="/download",
        expected_mime="text/plain",
    )


async def _html_attachment(state: SmokeState) -> dict[str, Any]:
    return await _successful_download(
        state,
        route="/navigation-download-html",
        expected_mime="text/html",
    )


async def _empty_attachment(state: SmokeState) -> dict[str, Any]:
    return await _successful_download(
        state,
        route="/navigation-download-empty",
        expected_mime="application/zip",
    )


async def _successful_download(
    state: SmokeState,
    *,
    route: str,
    expected_mime: str,
    expected_status: int = 200,
) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = f"{state.fixture}{route}"
        result = await probe.cdp.send("Page.navigate", {"url": url})
        _assert_download_result(result, "net::ERR_ABORTED")
        response_event = await _wait_for_document_response(probe, url)
        _assert_response(response_event, status=expected_status, mime=expected_mime, url=url)
        _assert_request_response_identity(probe.events, response_event, url)
        snapshot = await _active_document_snapshot(probe.cdp)
        _assert_retained_document(snapshot)
        _assert_no_navigation_dcl(probe.events, response_event)
        return _compact_observation(result, response_event, snapshot)


async def _redirect_attachment(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        requested_url = f"{state.fixture}/navigation-redirect-download"
        final_url = f"{state.fixture}/navigation-download-html"
        result = await probe.cdp.send("Page.navigate", {"url": requested_url})
        _assert_download_result(result, "net::ERR_ABORTED")
        response_event = await _wait_for_document_response(probe, final_url)
        _assert_response(response_event, status=200, mime="text/html", url=final_url)

        requests = _document_request_events(probe.events)
        request_urls = [_request_url(event) for event in requests]
        assert_equal(
            request_urls,
            [requested_url, final_url],
            "redirect download request chain",
        )
        request_ids = {_required_string(event["params"], "requestId") for event in requests}
        if len(request_ids) != 1:
            raise SmokeError(
                f"redirect download changed Network request identity: {request_ids}"
            )
        redirect_response = requests[-1]["params"].get("redirectResponse")
        if not isinstance(redirect_response, dict):
            raise SmokeError("redirect download omitted redirectResponse")
        assert_equal(int(redirect_response.get("status", 0)), 302, "redirect status")

        snapshot = await _active_document_snapshot(probe.cdp)
        _assert_retained_document(snapshot)
        _assert_no_navigation_dcl(probe.events, response_event)
        observed = _compact_observation(result, response_event, snapshot)
        observed["requestUrls"] = request_urls
        observed["requestId"] = next(iter(request_ids))
        return observed


async def _error_status_attachment(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = f"{state.fixture}/navigation-download-http-error"
        result = await probe.cdp.send("Page.navigate", {"url": url})
        if result.get("isDownload") is not True:
            raise SmokeError(f"HTTP-error attachment was not marked as download: {result}")
        error_text = _required_string(result, "errorText")
        if not error_text.startswith("net::ERR_"):
            raise SmokeError(f"unexpected attachment navigation errorText: {error_text}")
        response_event = await _wait_for_document_response(probe, url)
        _assert_response(response_event, status=404, mime="text/plain", url=url)
        _assert_request_response_identity(probe.events, response_event, url)
        snapshot = await _active_document_snapshot(probe.cdp)
        return _compact_observation(result, response_event, snapshot)


async def _binary_mime(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = f"{state.fixture}/chromium-resource-xhr.bin"
        result = await probe.cdp.send("Page.navigate", {"url": url})
        response_event = await _wait_for_document_response(probe, url)
        _assert_response(
            response_event,
            status=200,
            mime="application/octet-stream",
            url=url,
        )
        snapshot = await _active_document_snapshot(probe.cdp)
        if result.get("isDownload") is True:
            _assert_retained_document(snapshot)
            _assert_no_navigation_dcl(probe.events, response_event)
            mode = "download"
        else:
            if result.get("errorText"):
                raise SmokeError(f"binary external Document reported an error: {result}")
            await _wait_for_navigation_dcl(probe, response_event)
            assert_equal(snapshot.get("href"), url, "binary external Document URL")
            mode = "external-document"
        observed = _compact_observation(result, response_event, snapshot)
        observed["outcome"] = mode
        return observed


async def _http_error_document(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = f"{state.fixture}/navigation-http-error"
        result = await probe.cdp.send("Page.navigate", {"url": url})
        _assert_not_download_or_navigation_error(result)
        response_event = await _wait_for_document_response(probe, url)
        _assert_response(response_event, status=502, mime="text/html", url=url)
        await _wait_for_navigation_dcl(probe, response_event)
        snapshot = await _active_document_snapshot(probe.cdp)
        assert_equal(snapshot.get("href"), url, "HTTP error Document URL")
        if "gateway error" not in str(snapshot.get("text", "")):
            raise SmokeError(f"HTTP error body was not committed: {snapshot}")
        return _compact_observation(result, response_event, snapshot)


async def _no_content(
    state: SmokeState,
    *,
    status: int,
    fetch_stage: str | None,
) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        route = "/navigation-no-content" if status == 204 else "/navigation-reset-content"
        if fetch_stage == "Response":
            route = f"/navigation-empty?no-content-override={status}"
        url = f"{state.fixture}{route}"
        await _evaluate_value(
            probe.cdp,
            "document.body.innerHTML='<input id=retainedField value=retained>';",
        )
        history = await probe.cdp.send("Page.getNavigationHistory")
        if fetch_stage is None:
            result = await probe.cdp.send("Page.navigate", {"url": url})
        else:
            paused = asyncio.get_running_loop().create_future()
            probe.cdp.once("Fetch.requestPaused", lambda params: paused.set_result(params))
            await probe.cdp.send(
                "Fetch.enable",
                {"patterns": [{"urlPattern": url, "requestStage": fetch_stage}]},
            )
            navigation = asyncio.create_task(probe.cdp.send("Page.navigate", {"url": url}))
            try:
                request = await paused
                headers = [{"name": "Content-Type", "value": "text/plain; charset=utf-8"}]
                if fetch_stage == "Request":
                    await probe.cdp.send(
                        "Fetch.fulfillRequest",
                        {
                            "requestId": request["requestId"],
                            "responseCode": status,
                            "responseHeaders": headers,
                            "body": "",
                        },
                    )
                else:
                    await probe.cdp.send(
                        "Fetch.continueResponse",
                        {
                            "requestId": request["requestId"],
                            "responseCode": status,
                            "responsePhrase": "No Content" if status == 204 else "Reset Content",
                            "responseHeaders": headers,
                        },
                    )
                result = await navigation
            finally:
                if not navigation.done():
                    navigation.cancel()
                    with suppress(asyncio.CancelledError):
                        await navigation
        assert_equal(result.get("isDownload"), False, f"{status} isDownload")
        assert_equal(result.get("errorText"), "net::ERR_ABORTED", f"{status} navigation errorText")
        response_event = await _wait_for_document_response(probe, url)
        _assert_response(response_event, status=status, mime="text/plain", url=url)
        _assert_request_response_identity(probe.events, response_event, url)
        await wait_until(
            lambda: _loading_failure_for_url(probe.events, url) is not None,
            f"{status} main-document Network.loadingFailed",
        )
        failure = _loading_failure_for_url(probe.events, url)
        if failure is None:
            raise SmokeError(f"missing {status} main-document Network.loadingFailed")
        assert_equal(
            failure["params"].get("errorText"), "net::ERR_ABORTED", f"{status} Network errorText"
        )
        assert_equal(failure["params"].get("canceled"), True, f"{status} canceled navigation")
        request_id = response_event["params"]["requestId"]
        terminals = [
            event
            for event in probe.events
            if event["method"] in {"Network.loadingFailed", "Network.loadingFinished"}
            and event["params"].get("requestId") == request_id
        ]
        assert_equal(len(terminals), 1, f"{status} navigation terminal count")
        snapshot = await _active_document_snapshot(probe.cdp)
        _assert_retained_document(snapshot)
        _assert_no_navigation_dcl(probe.events, response_event)
        if any(event["method"] == "Runtime.executionContextsCleared" for event in probe.events):
            raise SmokeError(f"{status} cleared the retained Document's Runtime contexts")
        assert_equal(
            await _evaluate_value(probe.cdp, "document.querySelector('#retainedField')?.value"),
            "retained",
            f"{status} retained form value",
        )
        assert_equal(await probe.cdp.send("Page.getNavigationHistory"), history, f"{status} history")
        observed = _compact_observation(result, response_event, snapshot)
        observed["outcome"] = "retained-document"
        observed["fetchStage"] = fetch_stage
        return observed


async def _transport_failure(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = f"{state.fixture}/chromium-network-reset-before-response?navigation-outcome=1"
        result = await probe.cdp.send("Page.navigate", {"url": url})
        if result.get("isDownload") is True:
            raise SmokeError(f"transport failure was misreported as download: {result}")
        error_text = _required_string(result, "errorText")
        await wait_until(
            lambda: _loading_failure_for_url(probe.events, url) is not None,
            "main-document Network.loadingFailed",
        )
        failure = _loading_failure_for_url(probe.events, url)
        if failure is None:
            raise SmokeError("missing main-document Network.loadingFailed")
        assert_equal(
            failure["params"].get("errorText"),
            error_text,
            "Page.navigate and Network.loadingFailed error",
        )
        if _document_response_for_url(probe.events, url) is not None:
            raise SmokeError("reset-before-response unexpectedly exposed response metadata")
        snapshot = await _active_document_snapshot(probe.cdp)
        if snapshot.get("href") == url:
            raise SmokeError(f"transport failure committed the requested URL: {snapshot}")
        return {
            "frameId": result.get("frameId"),
            "loaderId": result.get("loaderId"),
            "isDownload": result.get("isDownload"),
            "errorText": error_text,
            "activeUrl": snapshot.get("href"),
            "requestId": failure["params"].get("requestId"),
        }


async def _fail_request_blocked_by_client(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = f"{state.fixture}/navigation-blocked-by-client"
        await _evaluate_value(
            probe.cdp,
            "document.body.innerHTML='<input id=retainedField value=retained>';",
        )
        history = await probe.cdp.send("Page.getNavigationHistory")
        paused = asyncio.get_running_loop().create_future()
        probe.cdp.once("Fetch.requestPaused", lambda params: paused.set_result(params))
        await probe.cdp.send(
            "Fetch.enable",
            {"patterns": [{"urlPattern": url, "requestStage": "Request"}]},
        )
        navigation = asyncio.create_task(probe.cdp.send("Page.navigate", {"url": url}))
        navigation_result: dict[str, Any] | None = None
        navigation_error: str | None = None
        try:
            request = await paused
            await probe.cdp.send(
                "Fetch.failRequest",
                {"requestId": request["requestId"], "errorReason": "BlockedByClient"},
            )
            try:
                navigation_result = await navigation
            except Exception as error:
                navigation_error = str(error)
        finally:
            if not navigation.done():
                navigation.cancel()
                with suppress(asyncio.CancelledError):
                    await navigation
        if is_moli_browser():
            # Moli answers a client-failed navigation with the protocol error
            # carrying the Chromium error text.
            if navigation_error is None:
                raise SmokeError(
                    f"blocked navigation was not refused by Page.navigate: {navigation_result!r}"
                )
            if "net::ERR_BLOCKED_BY_CLIENT" not in navigation_error:
                raise SmokeError(f"blocked navigation error mismatch: {navigation_error}")
        else:
            # Chromium keeps the command successful and reports the failure in
            # the Page.navigate result payload.
            if navigation_result is None:
                raise SmokeError(
                    f"Chromium blocked navigation rejected the command: {navigation_error}"
                )
            assert_equal(
                navigation_result.get("errorText"),
                "net::ERR_BLOCKED_BY_CLIENT",
                "Chromium blocked navigation errorText",
            )
            assert_equal(
                navigation_result.get("isDownload"),
                False,
                "Chromium blocked navigation isDownload",
            )
        requests = [
            event
            for event in _document_request_events(probe.events)
            if _request_url(event) == url
        ]
        if not requests:
            raise SmokeError(f"missing Document request for blocked navigation: {probe.events}")
        await wait_until(
            lambda: _loading_failure_for_url(probe.events, url) is not None,
            "blocked main-document Network.loadingFailed",
        )
        failure = _loading_failure_for_url(probe.events, url)
        if failure is None:
            raise SmokeError("missing blocked main-document Network.loadingFailed")
        assert_equal(
            failure["params"].get("errorText"),
            "net::ERR_BLOCKED_BY_CLIENT",
            "blocked Network errorText",
        )
        request_id = requests[0]["params"].get("requestId")
        terminals = [
            event
            for event in probe.events
            if event["method"] in {"Network.loadingFailed", "Network.loadingFinished"}
            and event["params"].get("requestId") == request_id
        ]
        assert_equal(len(terminals), 1, "blocked navigation terminal count")
        blocked_loader_id = requests[0]["params"].get("loaderId")
        if any(
            event.get("method") == "Page.lifecycleEvent"
            and event.get("params", {}).get("loaderId") == blocked_loader_id
            and event.get("params", {}).get("name") in {"DOMContentLoaded", "domContentLoaded"}
            for event in probe.events
        ):
            raise SmokeError("blocked navigation emitted DOMContentLoaded")
        snapshot = await _active_document_snapshot(probe.cdp)
        _assert_retained_document(snapshot)
        if any(
            event["method"] == "Runtime.executionContextsCleared" for event in probe.events
        ):
            raise SmokeError("blocking the navigation cleared the retained Document's contexts")
        assert_equal(
            await _evaluate_value(probe.cdp, "document.querySelector('#retainedField')?.value"),
            "retained",
            "blocked navigation retained form value",
        )
        assert_equal(
            await probe.cdp.send("Page.getNavigationHistory"), history, "blocked history"
        )
        if any(event["method"] == "Page.loadEventFired" for event in probe.events):
            raise SmokeError("blocked navigation published a load event")
        retained_url = snapshot.get("href")
        retained_realm = snapshot.get("marker") == _OLD_DOCUMENT_MARKER
        # The tab must stay navigable: commit an ordinary document afterwards.
        next_url = f"{state.fixture}/plain"
        next_result = await probe.cdp.send("Page.navigate", {"url": next_url})
        if next_result.get("errorText"):
            raise SmokeError(f"navigation after a blocked one failed: {next_result}")
        await wait_until(
            lambda: any(event["method"] == "Page.loadEventFired" for event in probe.events),
            "load event for the navigation after a blocked one",
        )
        load_events = [event for event in probe.events if event["method"] == "Page.loadEventFired"]
        assert_equal(len(load_events), 1, "load event count across blocked and next navigation")
        next_snapshot = await _active_document_snapshot(probe.cdp)
        assert_equal(next_snapshot.get("href"), next_url, "active Document after the next navigation")
        return {
            "requestId": request_id,
            "networkErrorText": failure["params"].get("errorText"),
            "retainedUrl": retained_url,
            "oldRealmRetained": retained_realm,
            "nextActiveUrl": next_snapshot.get("href"),
            "outcome": "retained-document-then-next-navigation",
        }


async def _wait_for_document_response(
    probe: NavigationProbe,
    url: str,
) -> dict[str, Any]:
    await wait_until(
        lambda: _document_response_for_url(probe.events, url) is not None,
        f"main-document response for {url}",
    )
    response = _document_response_for_url(probe.events, url)
    if response is None:
        raise SmokeError(f"missing main-document response for {url}")
    return response


async def _wait_for_navigation_dcl(
    probe: NavigationProbe,
    response_event: dict[str, Any],
) -> None:
    loader_id = _required_string(response_event["params"], "loaderId")
    await wait_until(
        lambda: any(
            event.get("method") == "Page.lifecycleEvent"
            and event.get("params", {}).get("loaderId") == loader_id
            and event.get("params", {}).get("name")
            in {"DOMContentLoaded", "domContentLoaded"}
            for event in probe.events
        ),
        f"DOMContentLoaded for loader {loader_id}",
    )


def _assert_download_result(result: dict[str, Any], error_text: str) -> None:
    if result.get("isDownload") is not True:
        raise SmokeError(f"Page.navigate did not report isDownload=true: {result}")
    assert_equal(result.get("errorText"), error_text, "download navigation errorText")
    _required_string(result, "frameId")


def _assert_not_download_or_navigation_error(result: dict[str, Any]) -> None:
    if result.get("isDownload") is True:
        raise SmokeError(f"ordinary Document was misreported as download: {result}")
    if result.get("errorText"):
        raise SmokeError(f"ordinary Document reported navigation error: {result}")
    _required_string(result, "frameId")
    _required_string(result, "loaderId")


def _assert_response(
    event: dict[str, Any],
    *,
    status: int,
    mime: str,
    url: str,
) -> None:
    response = event.get("params", {}).get("response")
    if not isinstance(response, dict):
        raise SmokeError(f"Network.responseReceived omitted response: {event}")
    assert_equal(int(response.get("status", 0)), status, "main-document status")
    observed_mime = str(response.get("mimeType", "")).split(";", 1)[0].lower()
    assert_equal(observed_mime, mime, "main-document MIME")
    assert_equal(response.get("url"), url, "main-document final URL")


def _assert_request_response_identity(
    events: list[dict[str, Any]],
    response_event: dict[str, Any],
    url: str,
) -> None:
    request = next(
        (event for event in _document_request_events(events) if _request_url(event) == url),
        None,
    )
    if request is None:
        raise SmokeError(f"missing Document request for {url}")
    assert_equal(
        response_event["params"].get("requestId"),
        request["params"].get("requestId"),
        "main-document request/response identity",
    )


def _assert_retained_document(snapshot: dict[str, Any]) -> None:
    assert_equal(snapshot.get("href"), "about:blank", "retained active Document URL")
    assert_equal(
        snapshot.get("marker"),
        _OLD_DOCUMENT_MARKER,
        "retained active Document realm marker",
    )


def _assert_no_navigation_dcl(
    events: list[dict[str, Any]],
    response_event: dict[str, Any],
) -> None:
    loader_id = response_event.get("params", {}).get("loaderId")
    matching = [
        event
        for event in events
        if event.get("method") == "Page.lifecycleEvent"
        and event.get("params", {}).get("loaderId") == loader_id
        and event.get("params", {}).get("name")
        in {"DOMContentLoaded", "domContentLoaded"}
    ]
    if matching:
        raise SmokeError(f"non-document navigation emitted DOMContentLoaded: {matching}")


def _document_request_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        event
        for event in events
        if event.get("method") == "Network.requestWillBeSent"
        and event.get("params", {}).get("type") == "Document"
    ]


def _document_response_for_url(
    events: list[dict[str, Any]],
    url: str,
) -> dict[str, Any] | None:
    for event in events:
        if event.get("method") != "Network.responseReceived":
            continue
        params = event.get("params", {})
        response = params.get("response", {})
        if params.get("type") == "Document" and response.get("url") == url:
            return event
    return None


def _loading_failure_for_url(
    events: list[dict[str, Any]],
    url: str,
) -> dict[str, Any] | None:
    request_ids = {
        event.get("params", {}).get("requestId")
        for event in _document_request_events(events)
        if _request_url(event) == url
    }
    for event in events:
        if (
            event.get("method") == "Network.loadingFailed"
            and event.get("params", {}).get("requestId") in request_ids
        ):
            return event
    return None


def _request_url(event: dict[str, Any]) -> str | None:
    request = event.get("params", {}).get("request")
    return request.get("url") if isinstance(request, dict) else None


async def _active_document_snapshot(cdp: Any) -> dict[str, Any]:
    value = await _evaluate_value(
        cdp,
        """
        ({
          href: location.href,
          marker: globalThis.__navigationOutcomeMarker ?? null,
          text: document.body?.textContent ?? '',
        })
        """,
    )
    if not isinstance(value, dict):
        raise SmokeError(f"unexpected active Document snapshot: {value!r}")
    return value


async def _evaluate_value(cdp: Any, expression: str) -> Any:
    response = await cdp.send(
        "Runtime.evaluate",
        {
            "expression": expression,
            "returnByValue": True,
            "awaitPromise": True,
        },
    )
    if response.get("exceptionDetails") is not None:
        raise SmokeError(f"Runtime.evaluate failed: {response['exceptionDetails']}")
    return response.get("result", {}).get("value")


def _compact_observation(
    result: dict[str, Any],
    response_event: dict[str, Any],
    snapshot: dict[str, Any],
) -> dict[str, Any]:
    params = response_event["params"]
    response = params["response"]
    return {
        "frameId": result.get("frameId"),
        "loaderId": result.get("loaderId"),
        "networkLoaderId": params.get("loaderId"),
        "requestId": params.get("requestId"),
        "isDownload": result.get("isDownload"),
        "errorText": result.get("errorText"),
        "status": int(response.get("status", 0)),
        "mimeType": response.get("mimeType"),
        "finalUrl": response.get("url"),
        "activeUrl": snapshot.get("href"),
        "oldRealmRetained": snapshot.get("marker") == _OLD_DOCUMENT_MARKER,
    }


def _required_string(value: dict[str, Any], key: str) -> str:
    field = value.get(key)
    if not isinstance(field, str) or not field:
        raise SmokeError(f"missing {key}: {value}")
    return field
