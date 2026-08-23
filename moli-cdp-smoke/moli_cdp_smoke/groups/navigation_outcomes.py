from __future__ import annotations

from contextlib import asynccontextmanager, suppress
from dataclasses import dataclass
from typing import Any, AsyncIterator, Awaitable, Callable

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
        NavigationOutcomeContract(
            "navigate_no_content_is_not_download",
            "A 204 response is never inferred to be a download merely because Chromium reports net::ERR_ABORTED.",
            _no_content,
        ),
        NavigationOutcomeContract(
            "navigate_transport_failure_is_not_download",
            "A reset before response metadata reports a non-download navigation error and a matching Network.loadingFailed terminal.",
            _transport_failure,
        ),
    )


@asynccontextmanager
async def _new_probe(state: SmokeState) -> AsyncIterator[NavigationProbe]:
    page = await state.context.new_page()
    cdp = await state.context.new_cdp_session(page)
    events = attach_cdp_event_collector(
        cdp,
        [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFailed",
            "Page.lifecycleEvent",
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
) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = f"{state.fixture}{route}"
        result = await probe.cdp.send("Page.navigate", {"url": url})
        _assert_download_result(result, "net::ERR_ABORTED")
        response_event = await _wait_for_document_response(probe, url)
        _assert_response(response_event, status=200, mime=expected_mime, url=url)
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


async def _no_content(state: SmokeState) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = f"{state.fixture}/navigation-no-content"
        result = await probe.cdp.send("Page.navigate", {"url": url})
        if result.get("isDownload") is True:
            raise SmokeError(f"204 response was misreported as a download: {result}")
        response_event = await _wait_for_document_response(probe, url)
        _assert_response(response_event, status=204, mime="text/plain", url=url)
        snapshot = await _active_document_snapshot(probe.cdp)
        error_text = result.get("errorText")
        if error_text:
            assert_equal(error_text, "net::ERR_ABORTED", "204 navigation errorText")
            _assert_retained_document(snapshot)
            outcome = "retained-document"
        else:
            assert_equal(snapshot.get("href"), url, "204 committed Document URL")
            outcome = "committed-document"
        observed = _compact_observation(result, response_event, snapshot)
        observed["outcome"] = outcome
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
        raise SmokeError(f"download navigation emitted DOMContentLoaded: {matching}")


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
