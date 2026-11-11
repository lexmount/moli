from __future__ import annotations


import asyncio
import base64
import json
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
        "raw Page.navigate and Network observation calibrated against managed Chromium"
    )
    commands = [
        "Page.enable",
        "Page.setLifecycleEventsEnabled",
        "Runtime.enable",
        "Network.enable",
        "Page.navigate",
        "Runtime.evaluate",
        "Fetch.enable",
        "Fetch.failRequest",
        "Fetch.continueResponse",
        "Fetch.fulfillRequest",
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
        *(
            NavigationOutcomeContract(
                f"navigate_fail_request_{name}_{stage.lower()}",
                f"Fetch.failRequest({reason}) at {stage} reports one request failure, "
                + ("retains the committed Document for cancellation" if reason == "Aborted"
                   else "commits a responsive error Document")
                + ", and permits a subsequent successful navigation.",
                partial(_fail_request, stage=stage, reason=reason, error_text=error_text),
            )
            for name, reason, error_text in (
                ("blocked_by_client", "BlockedByClient", "net::ERR_BLOCKED_BY_CLIENT"),
                ("timed_out", "TimedOut", "net::ERR_TIMED_OUT"),
                ("aborted", "Aborted", "net::ERR_ABORTED"),
            )
            for stage in ("Request", "Response")
        ),
        *(
            NavigationOutcomeContract(
                f"navigate_redirect_fail_request_{name}_response",
                "A response-stage failure after a redirect commits an error Document "
                "whose unreachable URL and history identify the final request.",
                partial(_fail_request, stage="Response", reason=reason,
                        error_text=error_text, redirect=True),
            )
            for name, reason, error_text in (
                ("blocked_by_client", "BlockedByClient", "net::ERR_BLOCKED_BY_CLIENT"),
                ("timed_out", "TimedOut", "net::ERR_TIMED_OUT"),
            )
        ),
        *(
            NavigationOutcomeContract(
                f"navigate_mixed_redirect_request_metadata_{action}",
                "Each request in a POST/307/POST/303/GET chain reports its own "
                "method, headers and body before the final response is resumed.",
                partial(_mixed_redirect_request_metadata, action=action),
            )
            for action in ("fail", "continue", "fulfill")
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
            "Network.requestWillBeSentExtraInfo",
            "Network.responseReceived",
            "Network.loadingFailed",
            "Network.loadingFinished",
            "Page.loadEventFired",
            "Page.frameNavigated",
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


async def _fail_request(
    state: SmokeState, *, stage: str, reason: str, error_text: str, redirect: bool = False
) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        url = (f"{state.fixture}/redirect-start" if redirect
               else f"{state.fixture}/plain?failed-navigation={reason}-{stage}")
        failed_url = f"{state.fixture}/redirect-final" if redirect else url
        await _evaluate_value(
            probe.cdp, "document.body.innerHTML='<input id=retainedField value=retained>'"
        )
        history = await probe.cdp.send("Page.getNavigationHistory")
        paused = asyncio.get_running_loop().create_future()
        probe.cdp.once("Fetch.requestPaused", lambda params: paused.set_result(params))
        await probe.cdp.send(
            "Fetch.enable",
            {"patterns": [{"urlPattern": failed_url, "resourceType": "Document", "requestStage": stage}]},
        )
        navigation = asyncio.create_task(probe.cdp.send("Page.navigate", {"url": url}))
        result: dict[str, Any] | None = None
        try:
            request = await paused
            assert_equal(request["request"]["url"], failed_url, "paused request URL")
            assert_equal(
                "responseStatusCode" in request, stage == "Response", "Fetch pause stage"
            )
            await probe.cdp.send(
                "Fetch.failRequest", {"requestId": request["requestId"], "errorReason": reason}
            )
            try:
                result = await navigation
            except Exception as error:
                # The existing Moli cancellation reply is a protocol error.
                # Failed requests must use the ordinary error-Document result.
                if not (reason == "Aborted" and is_moli_browser() and "Aborted" in str(error)):
                    raise
                error_text = "Aborted"
        finally:
            if not navigation.done():
                navigation.cancel()
                with suppress(asyncio.CancelledError):
                    await navigation
        if result is not None:
            assert_equal(result.get("errorText"), error_text, "failed navigation errorText")
            assert_equal(result.get("isDownload"), False, "failed navigation isDownload")
        await wait_until(
            lambda: _loading_failure_for_url(probe.events, failed_url) is not None,
            "failed main-document Network.loadingFailed",
        )
        failure = _loading_failure_for_url(probe.events, failed_url)
        if failure is None:
            raise SmokeError("missing main-document Network.loadingFailed")
        assert_equal(failure["params"].get("errorText"), error_text, "Network failure errorText")
        requests = [event for event in _document_request_events(probe.events)
                    if _request_url(event) == failed_url]
        assert_equal(len(requests), 1, "failed navigation Document request count")
        request_id = requests[0]["params"].get("requestId")
        chain = [event for event in _document_request_events(probe.events)
                 if event["params"].get("requestId") == request_id]
        assert_equal([_request_url(event) for event in chain],
                     [url, failed_url] if redirect else [url],
                     "failed navigation redirect chain")

        if reason == "Aborted":
            snapshot = await _active_document_snapshot(probe.cdp)
            _assert_retained_document(snapshot)
            assert_equal(
                await _evaluate_value(probe.cdp, "document.querySelector('#retainedField')?.value"),
                "retained", "cancelled navigation retained form value",
            )
            assert_equal(await probe.cdp.send("Page.getNavigationHistory"), history,
                         "cancelled navigation history")
            if any(event["method"] in {"Page.frameNavigated", "Page.loadEventFired",
                                       "Runtime.executionContextsCleared"}
                   for event in probe.events):
                raise SmokeError("cancellation replaced or loaded the committed Document")
            outcome = "retained-document"
        else:
            await wait_until(
                lambda: any(event["method"] == "Page.loadEventFired" for event in probe.events),
                "error Document load",
            )
            snapshot = await _active_document_snapshot(probe.cdp)
            assert_equal(snapshot.get("href"), "chrome-error://chromewebdata/",
                         "error Document location")
            assert_equal(snapshot.get("marker"), None, "error Document replaces the old realm")
            assert_equal(
                await _evaluate_value(probe.cdp, "document.querySelector('#retainedField')?.value ?? null"),
                None, "error Document replaces the old form",
            )
            frame = next(
                event["params"]["frame"] for event in probe.events
                if event["method"] == "Page.frameNavigated"
                and event["params"]["frame"].get("loaderId") == result.get("loaderId")
            )
            assert_equal(frame.get("url"), "chrome-error://chromewebdata/", "error frame URL")
            assert_equal(frame.get("unreachableUrl"), failed_url, "error frame unreachable URL")
            frame_tree = await probe.cdp.send("Page.getFrameTree")
            assert_equal(frame_tree["frameTree"]["frame"].get("unreachableUrl"), failed_url,
                         "error frame tree unreachable URL")
            failed_history = await probe.cdp.send("Page.getNavigationHistory")
            assert_equal(
                failed_history["entries"][failed_history["currentIndex"]].get("url"),
                failed_url, "error Document history URL",
            )
            assert_equal(
                sum(event["method"] == "Page.loadEventFired" for event in probe.events),
                1, "error Document load count",
            )
            outcome = "error-document"

        correlated = [event for event in probe.events
                      if event["params"].get("requestId") == request_id]
        assert_equal(
            sum(event["method"] == "Network.loadingFailed" for event in correlated),
            1, "failed request loadingFailed count",
        )
        # Chromium publishes loadingFinished for the browser-owned error
        # Document after reporting the original request's loadingFailed.
        assert_equal(
            sum(event["method"] == "Network.loadingFinished" for event in correlated),
            int(reason != "Aborted"), "error Document loadingFinished count",
        )

        start = len(probe.events)
        next_url = f"{state.fixture}/plain?after-failure={reason}-{stage}"
        next_result = await probe.cdp.send("Page.navigate", {"url": next_url})
        _assert_not_download_or_navigation_error(next_result)
        await wait_until(
            lambda: any(event["method"] == "Page.frameNavigated"
                        and event["params"]["frame"].get("loaderId") == next_result["loaderId"]
                        for event in probe.events[start:]),
            "next Document commit",
        )
        committed = next(
            index for index, event in enumerate(probe.events[start:], start)
            if event["method"] == "Page.frameNavigated"
            and event["params"]["frame"].get("loaderId") == next_result["loaderId"]
        )
        await wait_until(
            lambda: any(event["method"] == "Page.loadEventFired"
                        for event in probe.events[committed:]),
            "next Document load",
        )
        assert_equal(
            sum(event["method"] == "Page.loadEventFired" for event in probe.events[start:]),
            1, "next Document load count",
        )
        next_snapshot = await _active_document_snapshot(probe.cdp)
        assert_equal(next_snapshot.get("href"), next_url, "next active Document URL")
        assert_equal(await _evaluate_value(probe.cdp, "6 * 7"), 42, "responsive next Document")
        return {
            "reason": reason, "fetchStage": stage, "requestId": request_id,
            "requestedUrl": url, "failedUrl": failed_url,
            "networkErrorText": error_text, "activeUrlAfterFailure": snapshot.get("href"),
            "oldRealmRetained": snapshot.get("marker") == _OLD_DOCUMENT_MARKER,
            "nextActiveUrl": next_snapshot.get("href"), "outcome": outcome,
        }


async def _mixed_redirect_request_metadata(state: SmokeState, *, action: str) -> dict[str, Any]:
    async with _new_probe(state) as probe:
        urls = [f"{state.fixture}/navigation-hop/{hop}?case={action}"
                for hop in ("start", "middle", "final")]
        paused = asyncio.get_running_loop().create_future()
        probe.cdp.once("Fetch.requestPaused", lambda params: paused.set_result(params))
        await probe.cdp.send("Fetch.enable", {"patterns": [{
            "urlPattern": urls[-1], "resourceType": "Document", "requestStage": "Response",
        }]})
        await _evaluate_value(probe.cdp, f"""(() => {{
          const form = document.createElement('form');
          form.method = 'POST'; form.enctype = 'text/plain'; form.action = {json.dumps(urls[0])};
          const input = document.createElement('input'); input.name = 'probe'; input.value = 'body';
          form.append(input); document.body.append(form); setTimeout(() => form.submit(), 0); return true;
        }})()""")
        request = await paused
        try:
            assert_equal(request["request"]["url"], urls[-1], "mixed redirect response URL")
            assert_equal(request.get("responseStatusCode"), 200, "mixed redirect response status")
            headers = {item["name"].lower(): item["value"] for item in request["responseHeaders"]}
            observed = json.loads(headers["x-navigation-requests"])
            assert_equal([item["method"] for item in observed], ["POST", "POST", "GET"],
                         "server redirect request methods")
            assert_equal([item["body"] for item in observed], ["probe=body\r\n", "probe=body\r\n", ""],
                         "server redirect request bodies")
            events = _document_request_events(probe.events)
            assert_equal([_request_url(event) for event in events], urls, "mixed redirect request URLs")
            assert_equal(len({event["params"]["requestId"] for event in events}), 1,
                         "mixed redirect request identity count")
            request_id = events[0]["params"]["requestId"]
            def request_extra_info() -> list[dict[str, Any]]:
                return [event["params"]["headers"] for event in probe.events
                        if event["method"] == "Network.requestWillBeSentExtraInfo"
                        and event["params"].get("requestId") == request_id]
            await wait_until(lambda: len(request_extra_info()) == len(observed),
                             "mixed redirect request ExtraInfo")
            for wire_headers, actual in zip(request_extra_info(), observed, strict=True):
                wire_headers = {name.lower(): value for name, value in wire_headers.items()}
                assert_equal(wire_headers.get("content-type", ""), actual["contentType"],
                             f"{actual['url']} request wire Content-Type")
            for event, actual in zip(events, observed, strict=True):
                reported = event["params"]["request"]
                label = actual["url"]
                assert_equal(reported["method"], actual["method"], f"{label} request method")
                reported_headers = {name.lower(): value for name, value in reported["headers"].items()}
                # Chromium can retain the original logical Content-Type on
                # the final GET. ExtraInfo above describes the wire headers.
                if actual["method"] == "POST":
                    assert_equal(reported_headers.get("content-type", ""), actual["contentType"],
                                 f"{label} request Content-Type")
                assert_equal(reported.get("postData", ""), actual["body"], f"{label} request body")
                assert_equal(reported.get("hasPostData", False), bool(actual["body"]),
                             f"{label} request hasPostData")

        finally:
            params = {"requestId": request["requestId"]}
            if action == "fail":
                await probe.cdp.send("Fetch.failRequest", {**params, "errorReason": "BlockedByClient"})
            elif action == "continue":
                await probe.cdp.send("Fetch.continueResponse", params)
            else:
                await probe.cdp.send("Fetch.fulfillRequest", {**params, "responseCode": 200,
                    "responseHeaders": [{"name": "Content-Type", "value": "text/html"}],
                    "body": base64.b64encode(b"<!doctype html><main>fulfilled</main>").decode()})
        await wait_until(lambda: any(event["method"] == "Page.loadEventFired" for event in probe.events),
                         "mixed redirect Document load")
        frame = (await probe.cdp.send("Page.getFrameTree"))["frameTree"]["frame"]
        if action == "fail":
            assert_equal(frame.get("unreachableUrl"), urls[-1], "mixed redirect unreachable URL")
            history = await probe.cdp.send("Page.getNavigationHistory")
            assert_equal(history["entries"][history["currentIndex"]]["url"], urls[-1],
                         "mixed redirect failed history URL")
        else:
            assert_equal(frame["url"], urls[-1], "mixed redirect committed URL")
            assert_equal(await _evaluate_value(probe.cdp, "document.body.textContent.trim()"),
                         "fulfilled" if action == "fulfill" else "redirect final",
                         "mixed redirect Document content")
        assert_equal(await _evaluate_value(probe.cdp, "6 * 7"), 42, "mixed redirect responsive Document")
        return {"action": action, "requests": observed, "finalUrl": urls[-1]}


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
