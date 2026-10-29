from __future__ import annotations

import asyncio
import json
import urllib.request
from dataclasses import dataclass
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..config import is_moli_browser
from ..navigation import navigate as navigate_document
from ..raw_cdp import RawCdpClient, connect_raw_cdp


@dataclass
class RawPageSession:
    session_id: str
    browser_context_id: str


async def run_raw_protocol_group(endpoint: str, fixture: str, results: list[dict[str, Any]]) -> None:
    client = await connect_raw_cdp(endpoint)
    page_session: RawPageSession | None = None
    try:
        page_session = await _create_page_session(client, f"{fixture}/plain")
        await _runtime_unique_context_id_roundtrip(client, page_session.session_id, results)
        await _large_runtime_result_roundtrip(client, page_session.session_id, results)
        await _javascript_dialog_protocol_shape(client, page_session.session_id, results)
        await _browser_get_version_keeps_session_route(client, page_session.session_id, results)
        await _navigation_suspension_command_routing_matches_chromium(
            client,
            page_session.session_id,
            fixture,
            results,
        )
        await _debugger_pause_precedes_next_evaluate(
            client,
            page_session.session_id,
            results,
        )
        await _debugger_step_out_preserves_resume_pause_order(
            client,
            page_session.session_id,
            results,
        )
        await _get_response_body_error_keeps_session_route(client, page_session.session_id, results)
        await _app_manifest_network_lifecycle_matches_chromium(
            client,
            page_session.session_id,
            fixture,
            results,
        )
        await _evaluate_without_followup(
            client,
            page_session.session_id,
            "fetch('/api').then(response => response.text())",
            "fixture api body",
            "raw_cdp_runtime_evaluate_awaitpromise_fetch_without_followup",
            "raw Runtime.evaluate awaitPromise fetch result",
            results,
        )
        await _evaluate_without_followup(
            client,
            page_session.session_id,
            "new Promise(resolve => setTimeout(() => resolve('timer-only done'), 25))",
            "timer-only done",
            "raw_cdp_runtime_evaluate_timer_only_without_followup",
            "raw Runtime.evaluate awaitPromise pure timer result",
            results,
        )
        await _evaluate_without_followup(
            client,
            page_session.session_id,
            """
            new Promise((resolve, reject) => {
              setTimeout(() => {
                fetch('/api')
                  .then(response => response.text())
                  .then(resolve, reject);
              }, 0);
            })
            """,
            "fixture api body",
            "raw_cdp_runtime_evaluate_timer_fetch_without_followup",
            "raw Runtime.evaluate awaitPromise timer fetch result",
            results,
        )
        await _evaluate_without_followup(
            client,
            page_session.session_id,
            """
            new Promise((resolve, reject) => {
              const url = new URL('/ws-echo', location.href);
              url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
              const socket = new WebSocket(url.href, 'smoke');
              const timeout = setTimeout(() => {
                try { socket.close(); } catch (_) {}
                reject(new Error('websocket timeout'));
              }, 5000);
              socket.onopen = () => socket.send('raw protocol websocket');
              socket.onmessage = event => {
                clearTimeout(timeout);
                const data = event.data;
                socket.close(1000, 'done');
                resolve(data);
              };
              socket.onerror = () => {
                clearTimeout(timeout);
                reject(new Error('websocket error'));
              };
            })
            """,
            "echo:raw protocol websocket",
            "raw_cdp_runtime_evaluate_websocket_without_followup",
            "raw Runtime.evaluate awaitPromise websocket result",
            results,
        )
        await _shared_worker_profiler_session_state(
            client,
            page_session.session_id,
            results,
        )
    finally:
        if page_session is not None:
            client.clear_no_followup_boundary()
            try:
                dispose_id = await client.send(
                    "Target.disposeBrowserContext",
                    {"browserContextId": page_session.browser_context_id},
                )
                await client.recv_until_id(dispose_id, timeout=3)
            except Exception:
                pass
        await client.websocket.close()


async def _runtime_unique_context_id_roundtrip(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    disable_id = await client.send("Runtime.disable", session_id=session_id)
    disable_response, _ = await client.recv_until_id(disable_id, timeout=5)
    if "error" in disable_response:
        raise SmokeError(f"Runtime.disable failed before unique-context probe: {disable_response['error']}")

    enable_id = await client.send("Runtime.enable", session_id=session_id)
    enable_response, seen = await client.recv_until_id(enable_id, timeout=5)
    if "error" in enable_response:
        raise SmokeError(f"Runtime.enable failed before unique-context probe: {enable_response['error']}")
    context_event = _find_session_event(
        seen,
        session_id,
        "Runtime.executionContextCreated",
    )
    if context_event is None:
        context_event = await _recv_until_session_event(
            client,
            session_id,
            "Runtime.executionContextCreated",
            "raw page Runtime.executionContextCreated",
        )
    unique_context_id = context_event.get("params", {}).get("context", {}).get("uniqueId")
    if not isinstance(unique_context_id, str) or not unique_context_id:
        raise SmokeError(f"page execution context missing uniqueId: {context_event}")

    evaluate_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": "21 * 2",
            "objectGroup": "console",
            "includeCommandLineAPI": True,
            "silent": False,
            "returnByValue": False,
            "generatePreview": True,
            "userGesture": True,
            "awaitPromise": False,
            "replMode": True,
            "allowUnsafeEvalBlockedByCSP": True,
            "uniqueContextId": unique_context_id,
        },
        session_id=session_id,
    )
    evaluate_response, _ = await client.recv_until_id(evaluate_id, timeout=5)
    if "error" in evaluate_response:
        raise SmokeError(f"Runtime.evaluate with emitted uniqueId failed: {evaluate_response['error']}")
    assert_equal(
        evaluate_response.get("result", {}).get("result", {}).get("value"),
        42,
        "raw Runtime.evaluate emitted uniqueContextId result",
    )

    call_id = await client.send(
        "Runtime.callFunctionOn",
        {
            "functionDeclaration": "function(a, b) { return a * b; }",
            "arguments": [{"value": 6}, {"value": 7}],
            "returnByValue": True,
            "uniqueContextId": unique_context_id,
        },
        session_id=session_id,
    )
    call_response, _ = await client.recv_until_id(call_id, timeout=5)
    if "error" in call_response:
        raise SmokeError(f"Runtime.callFunctionOn with emitted uniqueId failed: {call_response['error']}")
    assert_equal(
        call_response.get("result", {}).get("result", {}).get("value"),
        42,
        "raw Runtime.callFunctionOn emitted uniqueContextId result",
    )
    record(results, "raw_cdp_runtime_emitted_unique_context_id_roundtrip")


async def _large_runtime_result_roundtrip(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    evaluate_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": """
            Array.from({length: 2048}, (_, index) => ({
              index,
              text: `row-${index}-中文-\\\"-\\\\-${"x".repeat(256)}`
            }))
            """,
            "returnByValue": True,
        },
        session_id=session_id,
    )
    response, _ = await client.recv_until_id(evaluate_id, timeout=10)
    if "error" in response:
        raise SmokeError(f"large Runtime.evaluate failed: {response['error']}")
    assert_equal(response.get("sessionId"), session_id, "large Runtime.evaluate session route")
    value = response.get("result", {}).get("result", {}).get("value")
    if not isinstance(value, list) or len(value) != 2048:
        raise SmokeError(
            "large Runtime.evaluate returned the wrong candidate array shape: "
            f"type={type(value).__name__}, length={len(value) if isinstance(value, list) else None}"
        )
    assert_equal(value[0].get("index"), 0, "large Runtime.evaluate first index")
    assert_equal(value[-1].get("index"), 2047, "large Runtime.evaluate last index")
    assert_equal(
        value[1024].get("text"),
        f"row-1024-中文-\"-\\-{'x' * 256}",
        "large Runtime.evaluate escaped Unicode payload",
    )

    followup_id = await client.send(
        "Runtime.evaluate",
        {"expression": "6 * 7", "returnByValue": True},
        session_id=session_id,
    )
    followup, _ = await client.recv_until_id(followup_id, timeout=5)
    assert_equal(
        followup.get("result", {}).get("result", {}).get("value"),
        42,
        "Runtime.evaluate after large result",
    )
    record(
        results,
        "raw_cdp_large_runtime_result_roundtrip",
        {"entries": len(value)},
    )


async def _debugger_pause_precedes_next_evaluate(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    # This is the sequence observed against local Chromium and used by V8's
    # inspector tests: acknowledge Debugger.pause, pause before the next
    # Runtime.evaluate can finish, then let Debugger.resume release it.
    enable_id = await client.send("Debugger.enable", session_id=session_id)
    await client.recv_until_id(enable_id, timeout=5)

    pause_id = await client.send("Debugger.pause", session_id=session_id)
    pause_response, pause_seen = await client.recv_until_id(pause_id, timeout=5)
    if "error" in pause_response:
        raise SmokeError(f"Debugger.pause failed: {pause_response['error']}")
    if _find_session_event(pause_seen, session_id, "Debugger.paused") is not None:
        raise SmokeError("Debugger.paused must not precede the Debugger.pause response")

    evaluate_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": "globalThis.__rawDebuggerPauseProbe = 1",
            "returnByValue": True,
        },
        session_id=session_id,
    )
    paused: dict[str, Any] | None = None
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 5.0
    while paused is None:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out waiting for Debugger.paused after Runtime.evaluate; "
                f"seen={seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == evaluate_id:
            raise SmokeError(
                "Runtime.evaluate completed before Debugger.paused after Debugger.pause"
            )
        if (
            message.get("sessionId") == session_id
            and message.get("method") == "Debugger.paused"
        ):
            paused = message

    call_frames = paused.get("params", {}).get("callFrames")
    if not isinstance(call_frames, list) or not call_frames:
        raise SmokeError(f"Debugger.paused should include author call frames: {paused}")

    resume_id = await client.send("Debugger.resume", session_id=session_id)
    resume_response: dict[str, Any] | None = None
    evaluate_response: dict[str, Any] | None = None
    saw_resumed = False
    deadline = asyncio.get_running_loop().time() + 5.0
    while resume_response is None or evaluate_response is None or not saw_resumed:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out finishing Debugger.resume/Runtime.evaluate; "
                f"resume={resume_response} evaluate={evaluate_response} "
                f"resumed={saw_resumed} seen={seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == resume_id:
            resume_response = message
        elif message.get("id") == evaluate_id:
            evaluate_response = message
        elif (
            message.get("sessionId") == session_id
            and message.get("method") == "Debugger.resumed"
        ):
            saw_resumed = True

    if "error" in resume_response:
        raise SmokeError(f"Debugger.resume failed: {resume_response['error']}")
    if "error" in evaluate_response:
        raise SmokeError(f"paused Runtime.evaluate failed: {evaluate_response['error']}")
    assert_equal(
        evaluate_response.get("result", {}).get("result", {}).get("value"),
        1,
        "raw Runtime.evaluate result after Debugger.resume",
    )

    disable_id = await client.send("Debugger.disable", session_id=session_id)
    await client.recv_until_id(disable_id, timeout=5)
    record(results, "raw_cdp_debugger_pause_precedes_next_evaluate")


async def _debugger_step_out_preserves_resume_pause_order(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    # Chromium resumes the current pause before entering the caller's step
    # pause. A timer callback with no JavaScript caller is not deterministic:
    # the step is then observed by the next script that happens to run. Keep
    # this contract anchored to an explicit inner/caller pair.
    enable_id = await client.send("Debugger.enable", session_id=session_id)
    await client.recv_until_id(enable_id, timeout=5)

    evaluate_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": """
            setTimeout(function outerTimer() {
              function inner() {
                debugger;
                return 40;
              }
              globalThis.__rawStepOutDone = inner() + 2;
            }, 25);
            'armed'
            """,
            "returnByValue": True,
        },
        session_id=session_id,
    )
    await client.recv_until_id(evaluate_id, timeout=5)
    first_pause = await _recv_until_session_event(
        client,
        session_id,
        "Debugger.paused",
        "initial nested-function debugger pause",
    )
    first_frames = first_pause.get("params", {}).get("callFrames", [])
    if not first_frames or first_frames[0].get("functionName") != "inner":
        raise SmokeError(f"initial stepOut pause should be in inner(): {first_pause}")

    step_id = await client.send("Debugger.stepOut", session_id=session_id)
    sequence: list[str] = []
    step_response: dict[str, Any] | None = None
    resumed: dict[str, Any] | None = None
    second_pause: dict[str, Any] | None = None
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 5.0
    while step_response is None or resumed is None or second_pause is None:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out waiting for stepOut response/resumed/step pause; "
                f"sequence={sequence} seen={seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == step_id:
            step_response = message
            sequence.append("response")
        elif (
            message.get("sessionId") == session_id
            and message.get("method") == "Debugger.resumed"
        ):
            resumed = message
            sequence.append("resumed")
        elif (
            message.get("sessionId") == session_id
            and message.get("method") == "Debugger.paused"
        ):
            second_pause = message
            sequence.append("paused")

    if "error" in step_response:
        raise SmokeError(f"Debugger.stepOut failed: {step_response['error']}")
    if sequence != ["response", "resumed", "paused"]:
        raise SmokeError(
            "Debugger.stepOut must respond, resume, then enter the caller step pause; "
            f"sequence={sequence}"
        )
    if second_pause.get("params", {}).get("reason") != "step":
        raise SmokeError(f"second stepOut pause should use reason=step: {second_pause}")
    second_frames = second_pause.get("params", {}).get("callFrames", [])
    if not second_frames or second_frames[0].get("functionName") != "outerTimer":
        raise SmokeError(f"stepOut should pause in outerTimer(): {second_pause}")

    resume_id = await client.send("Debugger.resume", session_id=session_id)
    await client.recv_until_id(resume_id, timeout=5)
    done_id = await client.send(
        "Runtime.evaluate",
        {"expression": "globalThis.__rawStepOutDone", "returnByValue": True},
        session_id=session_id,
    )
    done_response, _ = await client.recv_until_id(done_id, timeout=5)
    assert_equal(
        done_response.get("result", {}).get("result", {}).get("value"),
        42,
        "raw Debugger.stepOut caller completion",
    )

    disable_id = await client.send("Debugger.disable", session_id=session_id)
    await client.recv_until_id(disable_id, timeout=5)
    record(results, "raw_cdp_debugger_step_out_resume_pause_order")


async def _javascript_dialog_protocol_shape(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    no_dialog_id = await client.send(
        "Page.handleJavaScriptDialog",
        {"accept": True},
        session_id=session_id,
    )
    no_dialog_response, _ = await _recv_until_id_allow_error(client, no_dialog_id, timeout=5)
    error = no_dialog_response.get("error", {})
    assert_equal(
        error.get("code"),
        -32602,
        "raw Page.handleJavaScriptDialog no-dialog error code",
    )
    assert_equal(
        error.get("message"),
        "No dialog is showing",
        "raw Page.handleJavaScriptDialog no-dialog message",
    )

    evaluate_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": "prompt('raw prompt?', 'default prompt')",
            "returnByValue": True,
        },
        session_id=session_id,
    )
    opening: dict[str, Any] | None = None
    deadline = asyncio.get_running_loop().time() + 5.0
    seen: list[dict[str, Any]] = []
    while opening is None:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for prompt opening; seen={seen[-20:]}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == evaluate_id:
            raise SmokeError(
                "Runtime.evaluate(prompt) completed before the modal prompt was handled"
            )
        if (
            message.get("sessionId") == session_id
            and message.get("method") == "Page.javascriptDialogOpening"
        ):
            opening = message

    params = opening.get("params", {})
    assert_equal(params.get("type"), "prompt", "raw Page.javascriptDialogOpening prompt type")
    assert_equal(params.get("message"), "raw prompt?", "raw Page.javascriptDialogOpening prompt message")
    assert_equal(
        params.get("defaultPrompt"),
        "default prompt",
        "raw Page.javascriptDialogOpening default prompt",
    )
    assert_equal(
        params.get("hasBrowserHandler"),
        not is_moli_browser(),
        "raw Page.javascriptDialogOpening native browser handler",
    )
    if not isinstance(params.get("frameId"), str) or not params["frameId"]:
        raise SmokeError(f"missing prompt frameId in {opening}")

    handle_id = await client.send(
        "Page.handleJavaScriptDialog",
        {"accept": True, "promptText": "typed prompt"},
        session_id=session_id,
    )
    handle_response: dict[str, Any] | None = None
    evaluate_response: dict[str, Any] | None = None
    closed: dict[str, Any] | None = None
    deadline = asyncio.get_running_loop().time() + 5.0
    while handle_response is None or evaluate_response is None or closed is None:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out waiting for handled prompt completion; "
                f"handle={handle_response} evaluate={evaluate_response} "
                f"closed={closed} seen={seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == handle_id:
            handle_response = message
        elif message.get("id") == evaluate_id:
            evaluate_response = message
        elif (
            message.get("sessionId") == session_id
            and message.get("method") == "Page.javascriptDialogClosed"
        ):
            closed = message

    if "error" in handle_response:
        raise SmokeError(f"Page.handleJavaScriptDialog(prompt) failed: {handle_response['error']}")
    if "error" in evaluate_response:
        raise SmokeError(f"Runtime.evaluate(prompt) failed: {evaluate_response['error']}")
    assert_equal(
        evaluate_response.get("result", {}).get("result", {}).get("value"),
        "typed prompt",
        "raw Runtime.evaluate prompt return value",
    )
    closed_params = closed.get("params", {})
    assert_equal(closed_params.get("result"), True, "raw Page.javascriptDialogClosed result")
    assert_equal(
        closed_params.get("userInput"),
        "typed prompt",
        "raw Page.javascriptDialogClosed userInput",
    )
    assert_equal(
        closed_params.get("frameId"),
        params.get("frameId"),
        "raw Page.javascriptDialogClosed frameId",
    )
    record(results, "raw_cdp_javascript_dialog_protocol_shape")


async def _get_response_body_error_keeps_session_route(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    network_enable_id = await client.send("Network.enable", session_id=session_id)
    await client.recv_until_id(network_enable_id, timeout=5)
    body_id = await client.send(
        "Network.getResponseBody",
        {"requestId": "REQ-does-not-exist"},
        session_id=session_id,
    )
    response, _ = await _recv_until_id_allow_error(client, body_id, timeout=5)
    assert_equal(
        response.get("sessionId"),
        session_id,
        "raw Network.getResponseBody error session route",
    )
    error = response.get("error", {})
    assert_equal(
        error.get("code"),
        -32000,
        "raw Network.getResponseBody missing body error code",
    )
    assert_equal(
        error.get("message"),
        "No resource with given identifier found",
        "raw Network.getResponseBody missing body error message",
    )
    record(results, "raw_cdp_network_get_response_body_error_session_route")


async def _app_manifest_network_lifecycle_matches_chromium(
    client: RawCdpClient,
    session_id: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    page_url = f"{fixture}/chromium-app-manifest-valid/page"
    manifest_url = f"{fixture}/chromium-app-manifests/app.webmanifest"
    await navigate_document(client, session_id, {"url": page_url})

    manifest_id = await client.send("Page.getAppManifest", session_id=session_id)
    request_id: str | None = None
    saw_response = False
    saw_terminal = False
    saw_command_response = False
    seen = []
    deadline = asyncio.get_running_loop().time() + 10.0
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for app manifest response; seen={seen[-20:]}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("sessionId") == session_id:
            params = message.get("params", {})
            if (
                message.get("method") == "Network.requestWillBeSent"
                and params.get("type") == "Manifest"
                and params.get("request", {}).get("url") == manifest_url
            ):
                request_id = params.get("requestId")
            elif (
                message.get("method") == "Network.responseReceived"
                and params.get("requestId") == request_id
                and params.get("type") == "Manifest"
            ):
                saw_response = True
            elif (
                message.get("method") == "Network.loadingFinished"
                and params.get("requestId") == request_id
            ):
                saw_terminal = True
            elif (
                message.get("method") == "Network.loadingFailed"
                and params.get("requestId") == request_id
            ):
                raise SmokeError(f"manifest Network request failed: {message}")
        if message.get("id") == manifest_id:
            if "error" in message:
                raise SmokeError(f"Page.getAppManifest failed: {message['error']}")
            if request_id is None:
                raise SmokeError(
                    "manifest Network.requestWillBeSent must precede the "
                    f"Page.getAppManifest response; seen={seen[-20:]}"
                )
            assert_equal(
                message.get("result", {}).get("url"),
                manifest_url,
                "raw Page.getAppManifest URL",
            )
            saw_command_response = True
        if saw_command_response and saw_response and saw_terminal:
            break
    record(results, "raw_cdp_app_manifest_network_lifecycle")


async def _browser_get_version_keeps_session_route(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    version_id = await client.send("Browser.getVersion", session_id=session_id)
    response, _ = await client.recv_until_id(version_id, timeout=5)
    assert_equal(
        response.get("sessionId"),
        session_id,
        "raw Browser.getVersion target session route",
    )
    result = response.get("result", {})
    product = result.get("product")
    user_agent = result.get("userAgent")
    product_major = (
        product.removeprefix("Chrome/").split(".", 1)[0]
        if isinstance(product, str) and product.startswith("Chrome/")
        else None
    )
    if (
        product_major is None
        or not isinstance(user_agent, str)
        or f"Chrome/{product_major}." not in user_agent
    ):
        raise SmokeError(f"raw Browser.getVersion product mismatch: {response}")
    record(results, "raw_cdp_browser_get_version_session_route")


def _read_fixture_json(url: str) -> dict[str, Any]:
    with urllib.request.urlopen(url, timeout=2) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if not isinstance(payload, dict):
        raise SmokeError(f"fixture response was not an object: {payload!r}")
    return payload


async def _navigation_suspension_command_routing_matches_chromium(
    client: RawCdpClient,
    session_id: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    gate = f"{fixture}/navigation-suspension-gate"
    await asyncio.to_thread(_read_fixture_json, f"{gate}/reset")
    performance_enable_id = await client.send("Performance.enable", session_id=session_id)
    await client.recv_until_id(performance_enable_id, timeout=5)
    dom_enable_id = await client.send("DOM.enable", session_id=session_id)
    await client.recv_until_id(dom_enable_id, timeout=5)

    navigation_url = f"{fixture}/navigation-suspension-gated-document"
    navigate_id = await client.send(
        "Page.navigate",
        {"url": navigation_url},
        session_id=session_id,
    )
    deadline = asyncio.get_running_loop().time() + 5.0
    while True:
        status = await asyncio.to_thread(_read_fixture_json, f"{gate}/status")
        if status.get("requestSeen") is True:
            break
        if asyncio.get_running_loop().time() >= deadline:
            raise SmokeError(f"navigation suspension fixture saw no request: {status}")
        # smoke-timing: poll the fixture gate proving navigation reached the server.
        await asyncio.sleep(0.01)

    debugger_id = await client.send("Debugger.enable", session_id=session_id)
    document_id = await client.send(
        "DOM.getDocument",
        {"depth": 1},
        session_id=session_id,
    )
    evaluate_id = await client.send(
        "Runtime.evaluate",
        {"expression": "document.URL", "returnByValue": True},
        session_id=session_id,
    )
    metrics_id = await client.send("Performance.getMetrics", session_id=session_id)
    terminate_id = await client.send("Runtime.terminateExecution", session_id=session_id)
    version_id = await client.send("Browser.getVersion")

    immediate_ids = {metrics_id, terminate_id, version_id}
    suspended_ids = {navigate_id, debugger_id, document_id, evaluate_id}
    immediate_responses: dict[int, dict[str, Any]] = {}
    before_commit: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 5.0
    try:
        while immediate_responses.keys() != immediate_ids:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise SmokeError(
                    "timed out waiting for navigation-suspension bypass commands; "
                    f"responses={immediate_responses} seen={before_commit[-20:]}"
                )
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
            before_commit.append(message)
            message_id = message.get("id")
            if message_id in suspended_ids:
                raise SmokeError(
                    "renderer main-thread command completed before navigation commit: "
                    f"{message}"
                )
            if message_id in immediate_ids:
                if "error" in message:
                    raise SmokeError(
                        "Chromium IO-route command failed during navigation suspension: "
                        f"{message}"
                    )
                immediate_responses[message_id] = message

        await asyncio.to_thread(_read_fixture_json, f"{gate}/release")
        suspended_responses: dict[int, dict[str, Any]] = {}
        saw_load = False
        after_commit: list[dict[str, Any]] = []
        deadline = asyncio.get_running_loop().time() + 10.0
        while suspended_responses.keys() != suspended_ids or not saw_load:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise SmokeError(
                    "timed out waiting for navigation-suspension resume; "
                    f"responses={suspended_responses} load={saw_load} "
                    f"seen={after_commit[-20:]}"
                )
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
            after_commit.append(message)
            message_id = message.get("id")
            if message_id in suspended_ids:
                if "error" in message:
                    raise SmokeError(
                        f"suspended command failed after navigation commit: {message}"
                    )
                suspended_responses[message_id] = message
            if (
                message.get("sessionId") == session_id
                and message.get("method") == "Page.loadEventFired"
            ):
                saw_load = True

        document_url = (
            suspended_responses[document_id]
            .get("result", {})
            .get("root", {})
            .get("documentURL")
        )
        assert_equal(document_url, navigation_url, "resumed DOM.getDocument URL")
        evaluated_url = (
            suspended_responses[evaluate_id]
            .get("result", {})
            .get("result", {})
            .get("value")
        )
        assert_equal(evaluated_url, navigation_url, "resumed Runtime.evaluate URL")
        record(
            results,
            "raw_cdp_navigation_suspension_chromium_io_routing",
            {
                "immediate": [
                    "Browser.getVersion",
                    "Performance.getMetrics",
                    "Runtime.terminateExecution",
                ],
                "resumed": [
                    "Debugger.enable",
                    "DOM.getDocument",
                    "Runtime.evaluate",
                ],
            },
        )
    finally:
        await asyncio.to_thread(_read_fixture_json, f"{gate}/release")
        try:
            debugger_disable_id = await client.send("Debugger.disable", session_id=session_id)
            await client.recv_until_id(debugger_disable_id, timeout=5)
        except Exception:
            pass
        try:
            performance_disable_id = await client.send("Performance.disable", session_id=session_id)
            await client.recv_until_id(performance_disable_id, timeout=5)
        except Exception:
            pass


async def _recv_until_id_allow_error(
    client: RawCdpClient,
    message_id: int,
    *,
    timeout: float,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for CDP response id={message_id}; seen={seen[-20:]}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == message_id:
            return message, seen


async def _evaluate_without_followup(
    client: RawCdpClient,
    session_id: str,
    expression: str,
    expected_value: Any,
    result_name: str,
    assertion_name: str,
    results: list[dict[str, Any]],
) -> None:
    command_count_before_await = client.command_count
    evaluate_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": expression,
            "awaitPromise": True,
            "returnByValue": True,
        },
        session_id=session_id,
    )
    client.mark_no_followup_boundary()
    try:
        response, seen = await client.recv_until_id(evaluate_id, timeout=10)
    finally:
        client.clear_no_followup_boundary()
    result = response.get("result", {}).get("result", {})
    assert_equal(result.get("value"), expected_value, assertion_name)
    assert_equal(client.command_count, command_count_before_await + 1, "raw CDP command count after awaitPromise evaluate")
    record(results, result_name, {"messagesDuringAwait": len(seen)})


async def _shared_worker_profiler_session_state(
    client: RawCdpClient,
    page_session_id: str,
    results: list[dict[str, Any]],
) -> None:
    auto_attach_id = await client.send(
        "Target.setAutoAttach",
        {
            "autoAttach": True,
            "waitForDebuggerOnStart": False,
            "flatten": True,
        },
    )
    await client.recv_until_id(auto_attach_id, timeout=5)

    create_worker_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": """
                new Promise((resolve, reject) => {
                  const worker = new SharedWorker('/shared-worker.js?raw-profiler');
                  const timer = setTimeout(() => reject(new Error('shared worker ready timeout')), 5000);
                  worker.port.onmessage = event => {
                    clearTimeout(timer);
                    resolve(event.data && event.data.ready === true);
                  };
                  worker.port.start();
                  worker.port.postMessage({ kind: 'ready' });
                })
            """,
            "awaitPromise": True,
            "returnByValue": True,
        },
        session_id=page_session_id,
    )

    shared_worker_session_id: str | None = None
    shared_worker_target_id: str | None = None
    shared_worker_url: str | None = None
    saw_worker_ready = False
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 10.0
    while shared_worker_session_id is None or not saw_worker_ready:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for shared worker profiler target; seen={seen[-20:]}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == create_worker_id:
            if "error" in message:
                raise SmokeError(f"creating SharedWorker failed: {message['error']}")
            value = message.get("result", {}).get("result", {}).get("value")
            assert_equal(value, True, "raw shared worker ready result")
            saw_worker_ready = True
        if message.get("method") == "Target.attachedToTarget":
            params = message.get("params", {})
            target_info = params.get("targetInfo", {})
            if target_info.get("type") == "shared_worker":
                session_id = params.get("sessionId")
                if not isinstance(session_id, str) or not session_id:
                    raise SmokeError(f"shared worker attached event missing sessionId: {message}")
                shared_worker_session_id = session_id
                target_id = target_info.get("targetId")
                if not isinstance(target_id, str) or not target_id:
                    raise SmokeError(f"shared worker attached event missing targetId: {message}")
                shared_worker_target_id = target_id
                url = target_info.get("url")
                if not isinstance(url, str) or "/shared-worker.js?raw-profiler" not in url:
                    raise SmokeError(f"shared worker attached event missing expected URL: {message}")
                shared_worker_url = url

    assert shared_worker_session_id is not None
    assert shared_worker_target_id is not None
    assert shared_worker_url is not None
    targets_id = await client.send("Target.getTargets")
    targets, _ = await client.recv_until_id(targets_id, timeout=5)
    target_infos = targets.get("result", {}).get("targetInfos", [])
    if not any(
        target.get("targetId") == shared_worker_target_id
        and target.get("type") == "shared_worker"
        and target.get("url") == shared_worker_url
        for target in target_infos
    ):
        raise SmokeError(f"Target.getTargets should include shared worker target: {targets}")

    runtime_enable_id = await client.send("Runtime.enable", session_id=shared_worker_session_id)
    _, runtime_seen = await client.recv_until_id(runtime_enable_id, timeout=5)
    context_event = _find_session_event(
        runtime_seen,
        shared_worker_session_id,
        "Runtime.executionContextCreated",
    )
    if context_event is None:
        context_event = await _recv_until_session_event(
            client,
            shared_worker_session_id,
            "Runtime.executionContextCreated",
            "raw shared worker Runtime.executionContextCreated",
        )
    context = context_event.get("params", {}).get("context", {})
    context_id = context.get("id")
    if not isinstance(context_id, int) or context_id <= 0:
        raise SmokeError(f"shared worker execution context missing a valid id: {context_event}")
    # Runtime.ExecutionContextDescription.auxData is optional embedder data.
    # Chromium currently omits it for a SharedWorker target, so the portable
    # contract is the routed context identity and successful context evaluation.
    unique_context_id = context.get("uniqueId")
    if not isinstance(unique_context_id, str) or not unique_context_id:
        raise SmokeError(f"shared worker execution context missing uniqueId: {context_event}")

    console_event = _find_session_event(
        runtime_seen,
        shared_worker_session_id,
        "Runtime.consoleAPICalled",
        text="shared-worker-smoke-ready",
    )
    if console_event is None:
        console_event = await _recv_until_session_event(
            client,
            shared_worker_session_id,
            "Runtime.consoleAPICalled",
            "raw shared worker Runtime.consoleAPICalled",
            text="shared-worker-smoke-ready",
        )
    assert_equal(
        console_event.get("params", {}).get("type"),
        "log",
        "raw shared worker console event type",
    )

    profiler_enable_id = await client.send("Profiler.enable", session_id=shared_worker_session_id)
    await client.recv_until_id(profiler_enable_id, timeout=5)
    sampling_id = await client.send(
        "Profiler.setSamplingInterval",
        {"interval": 100},
        session_id=shared_worker_session_id,
    )
    await client.recv_until_id(sampling_id, timeout=5)
    start_id = await client.send("Profiler.start", session_id=shared_worker_session_id)
    await client.recv_until_id(start_id, timeout=5)
    try:
        work_id = await client.send(
            "Runtime.evaluate",
            {
                "expression": """
                    (() => {
                      function rawSharedWorkerProfilerWork() {
                        let total = 0;
                        for (let i = 0; i < 250000; ++i)
                          total += Math.sqrt(i + 1);
                        return total > 0;
                      }
                      return rawSharedWorkerProfilerWork();
                    })()
                """,
                "uniqueContextId": unique_context_id,
                "returnByValue": True,
            },
            session_id=shared_worker_session_id,
        )
        work, _ = await client.recv_until_id(work_id, timeout=10)
        work_result = work.get("result", {}).get("result", {})
        assert_equal(work_result.get("value"), True, "raw shared worker profiler work result")

        stop_id = await client.send("Profiler.stop", session_id=shared_worker_session_id)
        stop, _ = await client.recv_until_id(stop_id, timeout=10)
    finally:
        disable_id = await client.send("Profiler.disable", session_id=shared_worker_session_id)
        await client.recv_until_id(disable_id, timeout=5)

    profile = stop.get("result", {}).get("profile") or {}
    names = _profile_function_names(profile)
    if "rawSharedWorkerProfilerWork" not in names:
        raise SmokeError(
            "shared worker Profiler.stop should include sampled worker frame; "
            f"names={sorted(names)}"
        )
    record(
        results,
        "raw_cdp_shared_worker_target_runtime_log_profiler",
        {"targetId": shared_worker_target_id},
    )


def _find_session_event(
    messages: list[dict[str, Any]],
    session_id: str,
    method: str,
    *,
    text: str | None = None,
) -> dict[str, Any] | None:
    for message in messages:
        if message.get("sessionId") != session_id or message.get("method") != method:
            continue
        if text is None or _console_event_has_text(message, text):
            return message
    return None


async def _recv_until_session_event(
    client: RawCdpClient,
    session_id: str,
    method: str,
    label: str,
    *,
    text: str | None = None,
) -> dict[str, Any]:
    deadline = asyncio.get_running_loop().time() + 5
    seen: list[dict[str, Any]] = []
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for {label}; seen={seen[-20:]}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("sessionId") == session_id and message.get("method") == method:
            if text is None or _console_event_has_text(message, text):
                return message


def _console_event_has_text(message: dict[str, Any], text: str) -> bool:
    args = message.get("params", {}).get("args", [])
    return any(isinstance(arg, dict) and arg.get("value") == text for arg in args)


async def _create_page_session(client: RawCdpClient, url: str) -> RawPageSession:
    create_context_id = await client.send("Target.createBrowserContext")
    create_context, _ = await client.recv_until_id(create_context_id)
    browser_context_id = create_context.get("result", {}).get("browserContextId")
    if not isinstance(browser_context_id, str) or not browser_context_id:
        raise SmokeError(f"missing browserContextId in {create_context}")

    create_target_id = await client.send(
        "Target.createTarget",
        {"browserContextId": browser_context_id, "url": "about:blank"},
    )
    create_target, _ = await client.recv_until_id(create_target_id)
    target_id = create_target.get("result", {}).get("targetId")
    if not isinstance(target_id, str) or not target_id:
        raise SmokeError(f"missing targetId in {create_target}")

    attach_id = await client.send(
        "Target.attachToTarget",
        {"targetId": target_id, "flatten": True},
    )
    attach, _ = await client.recv_until_id(attach_id)
    session_id = attach.get("result", {}).get("sessionId")
    if not isinstance(session_id, str) or not session_id:
        raise SmokeError(f"missing sessionId in {attach}")

    runtime_enable_id = await client.send("Runtime.enable", session_id=session_id)
    await client.recv_until_id(runtime_enable_id)
    page_enable_id = await client.send("Page.enable", session_id=session_id)
    await client.recv_until_id(page_enable_id)

    await navigate_document(client, session_id, {"url": url})

    return RawPageSession(session_id=session_id, browser_context_id=browser_context_id)


def _profile_function_names(profile: dict[str, Any]) -> set[str]:
    names: set[str] = set()
    for node in profile.get("nodes") or []:
        call_frame = node.get("callFrame") or {}
        function_name = call_frame.get("functionName")
        if isinstance(function_name, str):
            names.add(function_name)
    return names
