from __future__ import annotations

import base64
from typing import Any
from uuid import UUID
from ..diagnostics import new_cdp_session
from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..helpers import attach_cdp_event_collector

from .chromium_cdp_support import _send_cdp_expect_optional_error


async def _verify_chromium_runtime_sample(state: SmokeState) -> None:
    events = attach_cdp_event_collector(state.cdp, ["Runtime.executionContextCreated"])
    await state.cdp.send("Runtime.enable")
    await wait_until(
        lambda: any(
            event["params"].get("context", {}).get("id")
            for event in events
            if event["method"] == "Runtime.executionContextCreated"
        ),
        "Chromium Runtime.executionContextCreated sample",
    )

    value_result = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "({ answer: 42, nested: { ok: true }, list: [1, 2] })",
            "returnByValue": True,
        },
    )
    assert_equal(
        value_result.get("result", {}).get("value"),
        {"answer": 42, "nested": {"ok": True}, "list": [1, 2]},
        "Chromium Runtime.evaluate returnByValue object sample",
    )

    exception_result = await state.cdp.send(
        "Runtime.evaluate",
        {"expression": "(() => { throw new Error('chromium sample throw'); })()"},
    )
    details = exception_result.get("exceptionDetails")
    if not details or "chromium sample throw" not in str(details):
        raise SmokeError(f"Runtime.evaluate should return exceptionDetails for thrown Error: {exception_result}")
    state.record("chromium_runtime_evaluate_sample")

async def _verify_chromium_input_session_state_sample(state: SmokeState) -> None:
    page = await state.context.new_page()
    primary = None
    peer = None

    async def install_input_fixture() -> None:
        await page.evaluate(
            """() => {
              document.body.innerHTML = '<input id="field" value="">';
              window.__inputEvents = [];
              const field = document.getElementById('field');
              field.addEventListener('keydown', event => window.__inputEvents.push(`keydown:${event.key}`));
              field.addEventListener('input', () => window.__inputEvents.push(`input:${field.value}`));
              field.focus();
            }"""
        )

    async def input_state() -> dict[str, Any]:
        return await page.evaluate(
            "() => ({ value: document.getElementById('field').value, events: window.__inputEvents })"
        )

    async def dispatch_key(session: Any, key: str) -> None:
        await session.send(
            "Input.dispatchKeyEvent",
            {
                "type": "keyDown",
                "key": key,
                "code": f"Key{key.upper()}",
                "text": key,
            },
        )

    try:
        await page.goto(f"{state.fixture}/plain?chromium-input-session-state")

        await install_input_fixture()
        primary = await new_cdp_session(state.context, page)
        peer = await new_cdp_session(state.context, page)

        assert_equal(
            await peer.send("Input.setIgnoreInputEvents", {"ignore": True}),
            {},
            "Input.setIgnoreInputEvents true result",
        )
        await dispatch_key(primary, "a")
        assert_equal(
            await input_state(),
            {"value": "", "events": []},
            "one Inspector session suppresses target keyboard input",
        )

        assert_equal(
            await primary.send("Input.insertText", {"text": "b"}),
            {},
            "Input.insertText while input events are ignored",
        )
        assert_equal(
            await input_state(),
            {"value": "b", "events": ["input:b"]},
            "Input.insertText bypasses input-event ignoring like Chromium",
        )

        assert_equal(
            await primary.send("Input.setIgnoreInputEvents", {"ignore": False}),
            {},
            "peer Input.setIgnoreInputEvents false result",
        )
        await dispatch_key(primary, "d")
        assert_equal(
            await input_state(),
            {"value": "b", "events": ["input:b"]},
            "one session cannot clear another session's input-event ignore handle",
        )

        await page.goto(f"{state.fixture}/plain?chromium-input-session-navigation")

        await install_input_fixture()
        await dispatch_key(primary, "n")
        assert_equal(
            await input_state(),
            {"value": "", "events": []},
            "Input.setIgnoreInputEvents survives renderer navigation",
        )

        await peer.detach()
        peer = None
        await dispatch_key(primary, "e")
        assert_equal(
            await input_state(),
            {"value": "e", "events": ["keydown:e", "input:e"]},
            "detaching the owning session releases input-event ignoring",
        )
        assert_equal(
            await primary.send("Input.cancelDragging"),
            {},
            "Input.cancelDragging idle result",
        )
        state.record("chromium_input_session_state_sample")
    finally:
        if peer is not None:
            await peer.detach()
        if primary is not None:
            await primary.detach()
        await page.close()

async def _verify_chromium_log_domain_sample(state: SmokeState) -> None:
    page = await state.context.new_page()
    await page.goto(f"{state.fixture}/plain?chromium-log-domain")

    primary = await new_cdp_session(state.context, page)
    peer = await new_cdp_session(state.context, page)
    controls = await new_cdp_session(state.context, page)
    primary_log_events = attach_cdp_event_collector(primary, ["Log.entryAdded"])
    peer_log_events = attach_cdp_event_collector(peer, ["Log.entryAdded"])
    network_events = attach_cdp_event_collector(
        primary,
        [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
            "Network.loadingFailed",
        ],
    )

    def entries_for(events: list[dict[str, Any]], url: str) -> list[dict[str, Any]]:
        return [
            event.get("params", {}).get("entry", {})
            for event in events
            if event.get("params", {}).get("entry", {}).get("url") == url
        ]

    async def failed_fetch(url: str) -> None:
        result = await primary.send(
            "Runtime.evaluate",
            {
                "expression": f"fetch({url!r}).then(response => response.status)",
                "awaitPromise": True,
                "returnByValue": True,
            },
        )
        assert_equal(
            result.get("result", {}).get("value"),
            404,
            "failed fetch status",
        )

    async def failed_image(url: str) -> None:
        result = await primary.send(
            "Runtime.evaluate",
            {
                "expression": (
                    "(() => {"
                    "const image = document.createElement('img');"
                    f"image.src = {url!r};"
                    "document.body.appendChild(image);"
                    "})()"
                ),
            },
        )
        assert_equal(
            result.get("result", {}).get("type"),
            "undefined",
            "failed image injection result",
        )

    try:
        await primary.send("Network.enable")
        fetch_url = f"{state.fixture}/chromium-log-missing-fetch"
        await failed_fetch(fetch_url)
        await wait_until(
            lambda: any(
                event.get("params", {}).get("response", {}).get("url") == fetch_url
                for event in network_events
            ),
            "Network.responseReceived for buffered Log fetch",
        )

        assert_equal(await primary.send("Log.enable"), {}, "Log.enable result")
        buffered_entries = entries_for(primary_log_events, fetch_url)
        if len(buffered_entries) != 1:
            raise SmokeError(
                "Log.enable must synchronously replay exactly one buffered network entry before "
                f"its response resolves: {primary_log_events}"
            )
        buffered = buffered_entries[0]
        network_response = next(
            event["params"]
            for event in network_events
            if event.get("params", {}).get("response", {}).get("url") == fetch_url
        )
        assert_equal(buffered.get("source"), "network", "buffered Log entry source")
        assert_equal(buffered.get("level"), "error", "buffered Log entry level")
        assert_equal(
            buffered.get("text"),
            "Failed to load resource: the server responded with a status of 404 (Not Found)",
            "buffered Log entry text",
        )
        assert_equal(
            buffered.get("networkRequestId"),
            network_response.get("requestId"),
            "Log entry Network request identity",
        )
        timestamp = buffered.get("timestamp")
        if not isinstance(timestamp, (int, float)) or timestamp < 1_000_000_000_000:
            raise SmokeError(f"Log entry timestamp must be Unix epoch milliseconds: {buffered}")

        repeated_enable_start = len(primary_log_events)
        assert_equal(await primary.send("Log.enable"), {}, "repeated Log.enable result")
        await primary.send("Runtime.evaluate", {"expression": "0"})
        assert_equal(
            len(primary_log_events),
            repeated_enable_start,
            "repeated Log.enable must not replay storage",
        )

        assert_equal(await peer.send("Log.enable"), {}, "peer Log.enable result")
        peer_entries = entries_for(peer_log_events, fetch_url)
        if len(peer_entries) != 1:
            raise SmokeError(
                "each Inspector session must independently replay shared Log storage: "
                f"{peer_log_events}"
            )
        assert_equal(
            peer_entries[0].get("networkRequestId"),
            buffered.get("networkRequestId"),
            "peer Log replay Network request identity",
        )

        assert_equal(await peer.send("Log.disable"), {}, "peer Log.disable result")
        image_url = f"{state.fixture}/chromium-log-missing-image.png"
        primary_live_start = len(primary_log_events)
        image_network_start = len(network_events)
        await failed_image(image_url)
        try:
            await wait_until(
                lambda: bool(entries_for(primary_log_events[primary_live_start:], image_url)),
                "live Log.entryAdded for failed image",
            )
        except SmokeError as error:
            image_network_events = [
                event
                for event in network_events[image_network_start:]
                if event.get("params", {}).get("request", {}).get("url") == image_url
                or event.get("params", {}).get("response", {}).get("url") == image_url
            ]
            raise SmokeError(
                f"{error}; networkEvents={image_network_events}, "
                f"primaryLogEvents={primary_log_events[primary_live_start:]}, "
                f"peerLogEvents={peer_log_events}"
            ) from error
        image_entry = entries_for(primary_log_events[primary_live_start:], image_url)[0]
        assert_equal(image_entry.get("source"), "network", "image Log entry source")
        assert_equal(image_entry.get("level"), "error", "image Log entry level")

        assert_equal(await primary.send("Log.clear"), {}, "Log.clear result")
        peer_before_reenable = len(peer_log_events)
        assert_equal(await peer.send("Log.enable"), {}, "peer Log re-enable result")
        await peer.send("Runtime.evaluate", {"expression": "0"})
        assert_equal(
            len(peer_log_events),
            peer_before_reenable,
            "Log.clear must clear target-shared storage for peer sessions",
        )

        assert_equal(await controls.send("Log.clear"), {}, "Log.clear before enable result")
        assert_equal(
            await controls.send("Log.stopViolationsReport"),
            {},
            "Log.stopViolationsReport before enable result",
        )
        disabled_start_error = await _send_cdp_expect_optional_error(
            controls,
            "Log.startViolationsReport",
            {"config": []},
        )
        if not disabled_start_error or "Log is not enabled" not in str(disabled_start_error):
            raise SmokeError(
                "Log.startViolationsReport before Log.enable must fail like Chromium: "
                f"{disabled_start_error}"
            )
        assert_equal(await controls.send("Log.enable"), {}, "controls Log.enable result")
        invalid_params_error = await _send_cdp_expect_optional_error(
            controls,
            "Log.startViolationsReport",
            {},
        )
        if not invalid_params_error or "Invalid parameters" not in str(invalid_params_error):
            raise SmokeError(
                "Log.startViolationsReport must validate config like Chromium: "
                f"{invalid_params_error}"
            )
        assert_equal(
            await controls.send(
                "Log.startViolationsReport",
                {
                    "config": [
                        {"name": "discouragedAPIUse", "threshold": -1},
                        {"name": "handler", "threshold": 50},
                        {"name": "unknown-setting", "threshold": 0},
                    ]
                },
            ),
            {},
            "Log.startViolationsReport result",
        )
        assert_equal(
            await controls.send("Log.stopViolationsReport"),
            {},
            "Log.stopViolationsReport result",
        )

        state.record(
            "chromium_log_domain_sample",
            {
                "bufferedNetworkRequestId": buffered.get("networkRequestId"),
                "bufferedTimestamp": timestamp,
                "primaryEntryCount": len(primary_log_events),
                "peerEntryCount": len(peer_log_events),
            },
        )
    finally:
        await primary.detach()
        await peer.detach()
        await controls.detach()
        await page.close()

async def _verify_chromium_audits_domain_sample(state: SmokeState) -> None:
    page = await state.context.new_page()
    primary = None
    peer = None
    late = None
    try:
        await page.goto(f"{state.fixture}/chromium-audits-quirks-page")

        primary = await new_cdp_session(state.context, page)
        peer = await new_cdp_session(state.context, page)
        primary_events = attach_cdp_event_collector(primary, ["Audits.issueAdded"])
        peer_events = attach_cdp_event_collector(peer, ["Audits.issueAdded"])

        assert_equal(await primary.send("Audits.enable"), {}, "Audits.enable result")
        if len(primary_events) != 1:
            raise SmokeError(
                "Audits.enable must synchronously replay one buffered QuirksModeIssue before "
                f"its response resolves: {primary_events}"
            )
        quirks_issue = primary_events[0].get("params", {}).get("issue", {})
        assert_equal(quirks_issue.get("code"), "QuirksModeIssue", "Audits quirks issue code")
        quirks_details = (
            quirks_issue.get("details", {}).get("quirksModeIssueDetails", {})
        )
        frame_tree = await primary.send("Page.getFrameTree")
        root_frame = frame_tree.get("frameTree", {}).get("frame", {})
        document_node_id = quirks_details.get("documentNodeId")
        if not isinstance(document_node_id, int) or document_node_id <= 0:
            raise SmokeError(f"QuirksModeIssue must carry a backend document node id: {quirks_issue}")
        assert_equal(
            quirks_details.get("isLimitedQuirksMode"),
            False,
            "Audits quirks mode kind",
        )
        assert_equal(
            quirks_details.get("frameId"),
            root_frame.get("id"),
            "Audits quirks frame identity",
        )
        assert_equal(
            quirks_details.get("loaderId"),
            root_frame.get("loaderId"),
            "Audits quirks loader identity",
        )
        assert_equal(
            quirks_details.get("url"),
            page.url,
            "Audits quirks document URL",
        )

        primary_repeat_start = len(primary_events)
        assert_equal(
            await primary.send("Audits.enable"),
            {},
            "repeated Audits.enable result",
        )
        await primary.send("Runtime.evaluate", {"expression": "0"})
        assert_equal(
            len(primary_events),
            primary_repeat_start,
            "repeated Audits.enable must not replay storage",
        )

        assert_equal(await peer.send("Audits.enable"), {}, "peer Audits.enable result")
        if len(peer_events) != 1:
            raise SmokeError(
                "each Inspector session must independently replay Audits storage: "
                f"{peer_events}"
            )
        assert_equal(
            peer_events[0].get("params", {}).get("issue"),
            quirks_issue,
            "peer Audits replay",
        )
        assert_equal(await peer.send("Audits.disable"), {}, "peer Audits.disable result")

        await page.goto(f"{state.fixture}/chromium-audits-csp-page")

        primary_csp_start = len(primary_events)
        peer_disabled_count = len(peer_events)
        evaluate = await primary.send(
            "Runtime.evaluate",
            {
                "expression": (
                    "(() => {"
                    "const script = document.createElement('script');"
                    "script.text = 'globalThis.__auditsSmokeBlocked = true';"
                    "document.body.appendChild(script);"
                    "return globalThis.__auditsSmokeBlocked === true;"
                    "})()"
                ),
                "returnByValue": True,
            },
        )
        assert_equal(
            evaluate.get("result", {}).get("value"),
            False,
            "CSP must block the injected inline script",
        )
        csp_events = primary_events[primary_csp_start:]
        if len(csp_events) != 1:
            raise SmokeError(
                "the live Audits.issueAdded event must arrive before Runtime.evaluate resolves: "
                f"{csp_events}"
            )
        assert_equal(
            len(peer_events),
            peer_disabled_count,
            "Audits.disable must suppress live issues for only that session",
        )
        csp_issue = csp_events[0].get("params", {}).get("issue", {})
        assert_equal(
            csp_issue.get("code"),
            "ContentSecurityPolicyIssue",
            "Audits CSP issue code",
        )
        csp_details = (
            csp_issue.get("details", {}).get("contentSecurityPolicyIssueDetails", {})
        )
        assert_equal(
            csp_details.get("violatedDirective"),
            "script-src-elem",
            "Audits CSP violated directive",
        )
        assert_equal(csp_details.get("isReportOnly"), False, "Audits CSP disposition")
        assert_equal(
            csp_details.get("contentSecurityPolicyViolationType"),
            "kInlineViolation",
            "Audits CSP violation type",
        )
        violating_node_id = csp_details.get("violatingNodeId")
        if not isinstance(violating_node_id, int) or violating_node_id <= 0:
            raise SmokeError(f"CSP issue must identify the violating script node: {csp_issue}")
        source_location = csp_details.get("sourceCodeLocation")
        if source_location is not None and (
            not isinstance(source_location.get("url"), str)
            or not isinstance(source_location.get("lineNumber"), int)
            or not isinstance(source_location.get("columnNumber"), int)
        ):
            raise SmokeError(f"CSP sourceCodeLocation has an invalid CDP shape: {csp_issue}")

        peer_replay_start = len(peer_events)
        assert_equal(await peer.send("Audits.enable"), {}, "peer Audits re-enable result")
        peer_replay = peer_events[peer_replay_start:]
        if len(peer_replay) != 1:
            raise SmokeError(
                "re-enabled Audits session must replay only the current document issue storage: "
                f"{peer_replay}"
            )
        assert_equal(
            peer_replay[0].get("params", {}).get("issue", {}).get("code"),
            "ContentSecurityPolicyIssue",
            "Audits replay after navigation",
        )

        await page.goto(f"{state.fixture}/plain?chromium-audits-storage-reset")

        late = await new_cdp_session(state.context, page)
        late_events = attach_cdp_event_collector(late, ["Audits.issueAdded"])
        assert_equal(await late.send("Audits.enable"), {}, "late Audits.enable result")
        await late.send("Runtime.evaluate", {"expression": "0"})
        assert_equal(
            late_events,
            [],
            "main-frame navigation must clear target Audits issue storage",
        )

        state.record(
            "chromium_audits_domain_sample",
            {
                "quirksDocumentNodeId": document_node_id,
                "cspViolatingNodeId": violating_node_id,
                "primaryIssueCount": len(primary_events),
                "peerIssueCount": len(peer_events),
            },
        )
    finally:
        if late is not None:
            await late.detach()
        if peer is not None:
            await peer.detach()
        if primary is not None:
            await primary.detach()
        await page.close()

async def _verify_chromium_io_resolve_blob_sample(state: SmokeState) -> None:
    first = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "globalThis.__cdpSmokeBlob = new Blob(['hello world'], {type:'text/plain'})",
            "returnByValue": False,
        },
    )
    first_object_id = first.get("result", {}).get("objectId")
    if not isinstance(first_object_id, str) or not first_object_id:
        raise SmokeError(f"Runtime.evaluate should return a Blob objectId: {first}")

    first_resolution = await state.cdp.send("IO.resolveBlob", {"objectId": first_object_id})
    first_uuid = first_resolution.get("uuid")
    if not isinstance(first_uuid, str) or UUID(first_uuid).version != 4:
        raise SmokeError(f"IO.resolveBlob should return a v4 UUID: {first_resolution}")
    repeated = await state.cdp.send("IO.resolveBlob", {"objectId": first_object_id})
    assert_equal(repeated.get("uuid"), first_uuid, "IO.resolveBlob stable UUID")

    second = await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "globalThis.__cdpSmokeBinaryBlob = new Blob([new Uint8Array([0,255,65])])",
            "returnByValue": False,
        },
    )
    second_object_id = second.get("result", {}).get("objectId")
    if not isinstance(second_object_id, str) or not second_object_id:
        raise SmokeError(f"Runtime.evaluate should return a second Blob objectId: {second}")
    second_resolution = await state.cdp.send("IO.resolveBlob", {"objectId": second_object_id})
    second_uuid = second_resolution.get("uuid")
    if not isinstance(second_uuid, str) or UUID(second_uuid).version != 4:
        raise SmokeError(f"second IO.resolveBlob should return a v4 UUID: {second_resolution}")
    if second_uuid == first_uuid:
        raise SmokeError("distinct Blob objects must not share a DevTools UUID")

    first_handle = f"blob:{first_uuid}"
    first_read = await state.cdp.send("IO.read", {"handle": first_handle})
    assert_equal(first_read.get("base64Encoded"), False, "text Blob IO.read encoding")
    assert_equal(first_read.get("data"), "hello world", "text Blob IO.read data")
    assert_equal(first_read.get("eof"), True, "text Blob IO.read eof")
    await state.cdp.send("IO.close", {"handle": first_handle})

    reopened = await state.cdp.send(
        "IO.read",
        {"handle": first_handle, "offset": 0, "size": 5},
    )
    assert_equal(reopened.get("data"), "hello", "closed Blob stream reopens from backing")
    assert_equal(reopened.get("eof"), False, "reopened Blob partial read eof")
    await state.cdp.send("IO.close", {"handle": first_handle})

    second_handle = f"blob:{second_uuid}"
    binary_read = await state.cdp.send("IO.read", {"handle": second_handle})
    assert_equal(binary_read.get("base64Encoded"), True, "binary Blob IO.read encoding")
    assert_equal(
        base64.b64decode(binary_read.get("data", "")),
        b"\x00\xffA",
        "binary Blob IO.read data",
    )
    await state.cdp.send("IO.close", {"handle": second_handle})

    non_blob = await state.cdp.send(
        "Runtime.evaluate",
        {"expression": "({answer: 42})", "returnByValue": False},
    )
    non_blob_error = await _send_cdp_expect_optional_error(
        state.cdp,
        "IO.resolveBlob",
        {"objectId": non_blob.get("result", {}).get("objectId")},
    )
    if not non_blob_error or "Object id doesn't reference a Blob" not in str(non_blob_error):
        raise SmokeError(f"IO.resolveBlob non-Blob error should match Chromium: {non_blob_error}")

    invalid_error = await _send_cdp_expect_optional_error(
        state.cdp,
        "IO.resolveBlob",
        {"objectId": "not-a-valid-object-id"},
    )
    if not invalid_error or "Invalid remote object id" not in str(invalid_error):
        raise SmokeError(f"IO.resolveBlob invalid object error should match Chromium: {invalid_error}")

    released = await state.cdp.send(
        "Runtime.evaluate",
        {"expression": "new Blob(['released'])", "returnByValue": False},
    )
    released_object_id = released.get("result", {}).get("objectId")
    await state.cdp.send("Runtime.releaseObject", {"objectId": released_object_id})
    released_error = await _send_cdp_expect_optional_error(
        state.cdp,
        "IO.resolveBlob",
        {"objectId": released_object_id},
    )
    if not released_error or "Could not find object with given id" not in str(released_error):
        raise SmokeError(f"IO.resolveBlob released object error should match Chromium: {released_error}")

    non_blob_session = await new_cdp_session(state.context, state.page)
    blob_session = await new_cdp_session(state.context, state.page)
    try:
        await non_blob_session.send(
            "Runtime.evaluate",
            {"expression": "({owner: 'non-blob-session'})", "returnByValue": False},
        )
        attached_blob = await blob_session.send(
            "Runtime.evaluate",
            {"expression": "new Blob(['attached'])", "returnByValue": False},
        )
        attached_object_id = attached_blob.get("result", {}).get("objectId")
        attached_resolution = await blob_session.send(
            "IO.resolveBlob",
            {"objectId": attached_object_id},
        )
        if not attached_resolution.get("uuid"):
            raise SmokeError(f"attached IO.resolveBlob should succeed: {attached_resolution}")
        cross_session_error = await _send_cdp_expect_optional_error(
            non_blob_session,
            "IO.resolveBlob",
            {"objectId": attached_object_id},
        )
        if cross_session_error is None:
            raise SmokeError("IO.resolveBlob must unwrap objectId in the calling Inspector session")
    finally:
        await non_blob_session.detach()
        await blob_session.detach()

    state.record("chromium_io_resolve_blob_sample")


async def run_audits_sample(state: SmokeState) -> None:
    await _verify_chromium_audits_domain_sample(state)


async def run_runtime_sample(state: SmokeState) -> None:
    await _verify_chromium_runtime_sample(state)


async def run_input_sample(state: SmokeState) -> None:
    await _verify_chromium_input_session_state_sample(state)


async def run_log_sample(state: SmokeState) -> None:
    await _verify_chromium_log_domain_sample(state)


async def run_io_blob_sample(state: SmokeState) -> None:
    await _verify_chromium_io_resolve_blob_sample(state)
