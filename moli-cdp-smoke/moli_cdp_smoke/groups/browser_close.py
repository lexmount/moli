from __future__ import annotations

import asyncio
import json
from typing import Any

from ..assertions import SmokeError, assert_equal, wait_until
from ..process import process_exit_diagnostics, wait_for_process_exit
from ..raw_cdp import RawCdpClient, connect_raw_cdp, connect_raw_cdp_websocket
from ..serve import browser_identity, start_browser, stop_browser, wait_for_browser_endpoint


_OMITTED = object()


class BrowserCloseError(SmokeError):
    def __init__(self, message: str, observed: dict[str, Any]) -> None:
        super().__init__(message)
        self.observed = observed


async def _command(
    client: RawCdpClient, method: str, params: dict[str, Any] | None = None,
    *, session_id: str | None = None,
) -> dict[str, Any]:
    message_id = await client.send(method, params, session_id=session_id)
    response, _ = await client.recv_until_id(message_id)
    return response["result"]


async def _close_request(
    client: RawCdpClient, *, params: Any = _OMITTED, session: Any = _OMITTED,
) -> dict[str, Any]:
    # Write the wire envelope directly to cover explicit null and malformed
    # session fields, which the normal client API deliberately omits.
    request: dict[str, Any] = {"id": 1000, "method": "Browser.close"}
    if params is not _OMITTED:
        request["params"] = params
    if session is not _OMITTED:
        request["sessionId"] = session
    await client.websocket.send(json.dumps(request))
    async with asyncio.timeout(10):
        while True:
            message = await client.recv()
            if message.get("id") == 1000:
                return message


async def _route(
    root: RawCdpClient, endpoint: str, fixture: str, scope: str,
    clients: list[RawCdpClient],
) -> tuple[RawCdpClient, Any]:
    if scope == "root":
        return root, _OMITTED
    if scope in ("browser", "nested-browser"):
        attached = await _command(root, "Target.attachToBrowserTarget")
        session = attached["sessionId"]
        if scope == "nested-browser":
            attached = await _command(root, "Target.attachToBrowserTarget", session_id=session)
            session = attached["sessionId"]
        return root, session
    if scope == "tab":
        targets = await _command(root, "Target.getTargets", {"filter": [{"type": "tab"}]})
        target_id = targets["targetInfos"][0]["targetId"]
    else:
        target = await _command(root, "Target.createTarget", {"url": fixture + "/plain"})
        target_id = target["targetId"]
    if scope == "page-socket":
        page = await connect_raw_cdp_websocket(
            endpoint.replace("http://", "ws://", 1) + "/devtools/page/" + target_id
        )
        clients.append(page)
        return page, _OMITTED
    attached = await _command(root, "Target.attachToTarget", {"targetId": target_id, "flatten": True})
    session = attached["sessionId"]
    if scope not in ("worker", "shared_worker"):
        return root, session

    async def page_ready() -> bool:
        result = await _command(root, "Runtime.evaluate", {
            "expression": f"location.href === {json.dumps(fixture + '/plain')} && document.readyState === 'complete'",
            "returnByValue": True,
        }, session_id=session)
        return result.get("result", {}).get("value") is True

    await wait_until(page_ready, "owned worker creator document")
    if scope == "worker":
        expression = """new Promise(resolve => {
          globalThis.closeWorker = new Worker('/worker.js');
          closeWorker.onmessage = () => resolve(true);
          closeWorker.postMessage('close-ready');
        })"""
    else:
        expression = """new Promise(resolve => {
          globalThis.closeWorker = new SharedWorker('/shared-worker.js', 'browser-close');
          closeWorker.port.onmessage = () => resolve(true);
          closeWorker.port.start();
          closeWorker.port.postMessage({kind: 'ready'});
        })"""
    ready = await _command(root, "Runtime.evaluate", {
        "expression": expression, "awaitPromise": True, "returnByValue": True,
    }, session_id=session)
    assert_equal(ready.get("result", {}).get("value"), True, "owned worker readiness")
    targets: dict[str, Any] = {}

    async def worker_visible() -> bool:
        nonlocal targets
        targets = await _command(root, "Target.getTargets", {"filter": [{"type": scope}]})
        return bool(targets["targetInfos"])

    await wait_until(worker_visible, "owned worker target")
    attached = await _command(root, "Target.attachToTarget", {
        "targetId": targets["targetInfos"][0]["targetId"], "flatten": True,
    })
    return root, attached["sessionId"]


async def browser_close_instance_lifecycle(endpoint: str, fixture: str) -> dict[str, Any]:
    identity = await browser_identity(endpoint, None)
    cases = [
        ("root", "root", _OMITTED),
        ("extra-params", "root", {"ignored": True}),
        ("null-params", "root", None),
        ("array-params", "root", []),
        ("browser-session", "browser", _OMITTED),
        ("nested-browser-session", "nested-browser", _OMITTED),
        ("page-session", "page", _OMITTED),
        ("page-socket", "page-socket", _OMITTED),
        ("tab-session", "tab", _OMITTED),
        ("worker-session", "worker", _OMITTED),
        ("shared-worker-session", "shared_worker", _OMITTED),
        ("unknown-session", "unknown", _OMITTED),
        ("null-session", "null", _OMITTED),
        ("foreign-session", "foreign", _OMITTED),
    ]
    observed = []
    for name, scope, params in cases:
        # Every destructive command goes to a fresh process owned by this
        # scenario. The supplied endpoint is only an oracle and a survivor.
        serving = await start_browser()
        clients: list[RawCdpClient] = []
        stage = "startup"
        child_endpoint = None
        response = None
        try:
            child_endpoint = await wait_for_browser_endpoint(serving)
            await browser_identity(child_endpoint, identity["engine"])
            root = await connect_raw_cdp(child_endpoint)
            clients.append(root)
            rejected = None
            if scope in ("unknown", "null", "foreign"):
                stage = "invalid session rejection"
                selected, session = root, _OMITTED
                if scope == "foreign":
                    attached = await _command(root, "Target.attachToBrowserTarget")
                    foreign = await connect_raw_cdp(child_endpoint)
                    clients.append(foreign)
                    rejected = await _close_request(foreign, session=attached["sessionId"])
                else:
                    rejected = await _close_request(root, session=None if scope == "null" else "missing")
                assert_equal(rejected.get("error", {}).get("code"),
                             -32600 if scope == "null" else -32001, "invalid session rejection")
                await _command(root, "Browser.getVersion")
                assert_equal(serving.process.returncode, None, "invalid command keeps instance alive")
            else:
                stage = "session setup"
                selected, session = await _route(root, child_endpoint, fixture, scope, clients)
            stage = "close acknowledgment"
            response = await _close_request(selected, params=params, session=session)
            expected: dict[str, Any] = {"id": 1000, "result": {}}
            if session is not _OMITTED:
                expected["sessionId"] = session
            assert_equal(response, expected, "close acknowledgment before EOF")
            stage = "natural process exit"
            code = await wait_for_process_exit(serving.process, timeout_seconds=10)
            assert_equal(code, 0, "natural process exit")
            observed.append({"case": name, "response": response, "exitCode": code, "rejected": rejected})
        except Exception as error:
            # Keep the pre-cleanup state: closing the clients or sending SIGTERM
            # first would conceal whether the browser itself was still alive.
            diagnostics = process_exit_diagnostics(serving.process)
            observed_failure = {
                "browser": identity,
                "completedCases": observed,
                "failedCase": {
                    "case": name, "scope": scope, "stage": stage,
                    "endpoint": child_endpoint, "closeResponse": response,
                    "process": diagnostics,
                    "connections": [
                        {"state": client.websocket.state.name,
                         "closeCode": client.websocket.close_code,
                         "closeReason": client.websocket.close_reason}
                        for client in clients
                    ],
                    "browserLogTail": serving.logs[-80:],
                },
            }
            process_state = diagnostics.get("process", {})
            raise BrowserCloseError(
                f"{name} ({stage}): {type(error).__name__}: {error}; "
                f"pid={serving.process.pid} returncode={diagnostics['returncode']} "
                f"state={process_state.get('State')} wchan={process_state.get('wchan')}",
                observed_failure,
            ) from error
        finally:
            await asyncio.gather(
                *(client.websocket.close() for client in clients), return_exceptions=True,
            )
            await stop_browser(serving)
    survivor = await connect_raw_cdp(endpoint)
    try:
        await _command(survivor, "Browser.getVersion")
    finally:
        await survivor.websocket.close()
    return {"browser": identity, "cases": observed, "independentInstanceSurvived": True}
