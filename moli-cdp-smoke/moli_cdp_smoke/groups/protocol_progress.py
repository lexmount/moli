"""Session-local navigation gates and independent output during a JS dialog."""

from __future__ import annotations

import asyncio
import contextlib
import json
import threading
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Iterator

from ..assertions import SmokeError, assert_equal, record_contract
from ..raw_cdp import connect_raw_cdp, discover_target_websocket_url
import websockets
from .cdp_ordering import _Wire


@contextmanager
def _http_gates() -> Iterator[
    tuple[str, dict[str, tuple[threading.Event, threading.Event]]]
]:
    gates = {path: (threading.Event(), threading.Event()) for path in ("/a", "/b")}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            if self.path in gates:
                arrived, release = gates[self.path]
                arrived.set()
                release.wait()
            body = b"<!doctype html><title>progress-completed</title>"
            try:
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.send_header("Access-Control-Allow-Origin", "*")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            except OSError:
                pass  # A canceled request may outlive its exact Target.

        def log_message(self, *_args: Any) -> None:
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", gates
    finally:
        for _arrived, release in gates.values():
            release.set()
        server.shutdown()
        thread.join(timeout=5)
        server.server_close()


async def _page(wire: _Wire, context: str, fixture: str) -> tuple[str, str]:
    target = (
        await wire.call(
            "Target.createTarget", {"url": "about:blank", "browserContextId": context}
        )
    )["targetId"]
    session = (
        await wire.call("Target.attachToTarget", {"targetId": target, "flatten": True})
    )["sessionId"]
    for method, params in (
        ("Page.enable", {}),
        ("Page.setLifecycleEventsEnabled", {"enabled": True}),
        ("Runtime.enable", {}),
        ("Network.enable", {}),
    ):
        await wire.call(method, params, session)
    start = len(wire.messages)
    nav = await wire.call("Page.navigate", {"url": f"{fixture}/plain"}, session)
    await wire.until(
        lambda m: m.get("sessionId") == session
        and m.get("method") == "Page.lifecycleEvent"
        and m["params"].get("loaderId") == nav.get("loaderId")
        and m["params"].get("name") == "load",
        "initial load",
        after=start,
    )
    return target, session


async def _navigation(wire: _Wire, a: str, b: str, _fixture: str) -> dict[str, Any]:
    with _http_gates() as (url, gates):
        try:
            nav_a = await wire.send("Page.navigate", {"url": url + "/a"}, a)
            if not await asyncio.to_thread(gates["/a"][0].wait, 10):
                raise SmokeError("A's held request did not arrive")
            nav_b = await wire.send("Page.navigate", {"url": url + "/b"}, b)
            if not await asyncio.to_thread(gates["/b"][0].wait, 10):
                raise SmokeError("B's held request did not arrive")
            held = await wire.send(
                "Runtime.evaluate", {"expression": "73", "returnByValue": True}, a
            )
            first = await wire.send(
                "Runtime.evaluate",
                {
                    "expression": "globalThis.progressOrder=[1]; progressOrder.join(',')",
                    "returnByValue": True,
                },
                b,
            )
            second = await wire.send(
                "Runtime.evaluate",
                {
                    "expression": "progressOrder.push(2); progressOrder.join(',')",
                    "returnByValue": True,
                },
                b,
            )
            await wire.call(
                "Target.getTargets"
            )  # Proves the actor received all prior commands.
            gates["/b"][1].set()
            assert_equal(
                (await wire.response(first))["result"]["result"]["value"],
                "1",
                "B's first queued command",
            )
            assert_equal(
                (await wire.response(second))["result"]["result"]["value"],
                "1,2",
                "B's session dispatch order",
            )
            assert_equal(
                gates["/a"][1].is_set(), False, "B advances without releasing A"
            )
            observed = {
                "aRepliedBeforeRelease": wire.has_response(held),
                "bOrder": ["1", "1,2"],
            }
            gates["/a"][1].set()
            for rid in (nav_a, nav_b, held):
                await wire.response(rid)
            return observed
        finally:
            for _arrived, release in gates.values():
                release.set()


async def _dialog(wire: _Wire, a: str, b: str, fixture: str) -> dict[str, Any]:
    with _http_gates() as (url, gates):
        try:
            await wire.evaluate(
                b,
                "void (async () => {"
                f"await fetch({json.dumps(url + '/b')}); console.log('progress-console');"
                "await new Promise(resolve => setTimeout(() => { console.log('progress-timer'); resolve(); }, 0));"
                f"const f=document.createElement('iframe'); f.src={json.dumps(fixture + '/plain?progress-child')}; document.body.append(f);"
                f"await fetch({json.dumps(url + '/resource')}); console.log('progress-done');"
                "})()",
            )
            if not await asyncio.to_thread(gates["/b"][0].wait, 10):
                raise SmokeError("B's asynchronous work did not reach its start gate")
            start = len(wire.messages)
            alert = await wire.send(
                "Runtime.evaluate",
                {"expression": "alert('progress-dialog'); 42", "returnByValue": True},
                a,
            )
            await wire.until(
                lambda m: m.get("sessionId") == a
                and m.get("method") == "Page.javascriptDialogOpening",
                "A dialog opens",
                after=start,
            )
            gates["/b"][1].set()
            # No B command can accidentally flush its journal for this test.
            # Its normal unsolicited output must advance while A stays modal.
            for text in ("progress-console", "progress-timer", "progress-done"):
                await wire.until(
                    lambda m: m.get("sessionId") == b
                    and m.get("method") == "Runtime.consoleAPICalled"
                    and any(
                        arg.get("value") == text for arg in m["params"].get("args", [])
                    ),
                    text,
                    after=start,
                )
            await wire.until(
                lambda m: m.get("sessionId") == b
                and m.get("method") == "Page.frameAttached",
                "B frame attachment",
                after=start,
            )
            await wire.until(
                lambda m: m.get("sessionId") == b
                and m.get("method") == "Network.responseReceived"
                and m["params"]["response"]["url"] == url + "/resource",
                "B's response",
                after=start,
            )
            assert_equal(
                wire.has_response(alert), False, "A's alert still awaits its handler"
            )
            await wire.call("Page.handleJavaScriptDialog", {"accept": True}, a)
            assert_equal(
                (await wire.response(alert))["result"]["result"]["value"],
                42,
                "A resumes only after handling",
            )
            return {
                "unsolicitedConsole": True,
                "frameAttached": True,
                "networkResponse": True,
            }
        finally:
            for _arrived, release in gates.values():
                release.set()


async def _self_navigation(wire: _Wire, a: str, b: str, fixture: str) -> dict[str, Any]:
    target = (await wire.call("Target.getTargetInfo", session=a))["targetInfo"][
        "targetId"
    ]
    second = (
        await wire.call("Target.attachToTarget", {"targetId": target, "flatten": True})
    )["sessionId"]
    for method in ("Page.enable", "Runtime.enable", "Network.enable"):
        await wire.call(method, session=second)
    world = await wire.call(
        "Page.createIsolatedWorld",
        {
            "frameId": target,
            "worldName": "progress-self-navigation",
            "grantUniversalAccess": True,
        },
        a,
    )
    utility = await wire.call(
        "Runtime.evaluate",
        {
            "contextId": world["executionContextId"],
            "expression": "({evaluate(isFunction,returnByValue,expression,argCount,...args){let result=globalThis.eval(expression);if(isFunction===true||(isFunction!==false&&typeof result==='function'))result=result(...args.slice(0,argCount));return returnByValue?{v:result===null?'null':'undefined'}:result}})",
        },
        a,
    )
    obj = utility["result"]["objectId"]
    url = fixture + "/plain?progress-self-navigation"
    start = len(wire.messages)
    command = await wire.send(
        "Runtime.callFunctionOn",
        {
            "objectId": obj,
            "functionDeclaration": "(utilityScript,...args)=>utilityScript.evaluate(...args)",
            "arguments": [
                {"objectId": obj},
                {},
                {"value": True},
                {"value": "(url)=>window.open(url,'_self')"},
                {"value": 1},
                {"value": url},
            ],
            "returnByValue": True,
            "awaitPromise": True,
            "userGesture": True,
        },
        a,
    )
    response = await wire.response(command)
    assert_equal(
        response["result"]["result"]["value"],
        {"v": "undefined"},
        "original utility call result survives its navigation",
    )
    if any(
        m.get("sessionId") in (a, second)
        and (
            m.get("method") == "Runtime.executionContextsCleared"
            or (
                m.get("method") == "Page.frameNavigated"
                and m["params"]["frame"]["url"] == url
            )
        )
        for m in wire.messages[start : wire.index(command)]
    ):
        raise SmokeError("realm retirement overtook the original Runtime result")

    for session in (a, second):
        started = await wire.until(
            lambda m: m.get("sessionId") == session
            and m.get("method") == "Page.frameStartedNavigating"
            and m["params"].get("url") == url,
            "source Session starts its navigation",
            after=start,
        )
        navigated = await wire.until(
            lambda m: m.get("sessionId") == session
            and m.get("method") == "Page.frameNavigated"
            and m["params"]["frame"]["url"] == url,
            "source Session commits its navigation",
            after=start,
        )
        loaded = await wire.until(
            lambda m: m.get("sessionId") == session
            and m.get("method") == "Page.loadEventFired",
            "source Session loads",
            after=wire.messages.index(navigated),
        )
        if (
            not wire.messages.index(started)
            < wire.messages.index(navigated)
            < wire.messages.index(loaded)
        ):
            raise SmokeError("same-Session navigation lifecycle order changed")
        await wire.until(
            lambda m: m.get("sessionId") == session
            and m.get("method") == "Network.requestWillBeSent"
            and m["params"]["request"]["url"] == url,
            "source Session's exact Document request",
            after=start,
        )
    assert_equal(
        await wire.evaluate(b, "6*7"), 42, "unrelated Page survives self navigation"
    )
    return {
        "navigationBeforeReply": [
            m["method"]
            for m in wire.messages[start : wire.index(command)]
            if m.get("method", "").startswith(("Page.", "Network."))
        ]
    }


async def _self_close(wire: _Wire, a: str, b: str, endpoint: str) -> dict[str, Any]:
    target = (await wire.call("Target.getTargetInfo", session=a))["targetInfo"][
        "targetId"
    ]
    await wire.call("Target.setDiscoverTargets", {"discover": True})
    start = len(wire.messages)
    await wire.call(
        "Runtime.evaluate",
        {
            "expression": "globalThis.progressChild=window.open('about:blank');void 0",
            "userGesture": True,
        },
        a,
    )
    created = await wire.until(
        lambda m: m.get("method") == "Target.targetCreated"
        and m["params"]["targetInfo"].get("openerId") == target,
        "script-created close target",
        after=start,
    )
    child = created["params"]["targetInfo"]["targetId"]
    uri = await discover_target_websocket_url(endpoint, child)
    frames = []
    async with websockets.connect(uri) as page:
        await page.send(
            json.dumps(
                {
                    "id": 501,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": "window.close();true",
                        "returnByValue": True,
                    },
                }
            )
        )
        try:
            async with asyncio.timeout(10):
                async for frame in page:
                    frames.append(json.loads(frame))
        except websockets.exceptions.ConnectionClosed:
            pass  # Target retirement can end its direct transport before a reply.
    assert_equal(
        sum(m.get("id") == 501 for m in frames) <= 1,
        True,
        "a closed Page never duplicates its terminal reply",
    )
    await wire.until(
        lambda m: m.get("method") == "Target.targetDestroyed"
        and m["params"]["targetId"] == child,
        "actual child destruction",
        after=start,
    )
    assert_equal(
        sum(
            m.get("method") == "Target.targetDestroyed"
            and m["params"]["targetId"] == child
            for m in wire.messages[start:]
        ),
        1,
        "one destruction fact",
    )
    assert_equal(await wire.evaluate(a, "6*7"), 42, "opener remains usable")
    assert_equal(await wire.evaluate(b, "6*7"), 42, "independent Page remains usable")
    return {"directPageFrames": frames, "targetDestroyed": child}


async def run_protocol_progress_group(
    endpoint: str, fixture_url: str, results: list[dict[str, Any]]
) -> None:
    failures = []
    for name, isolated, scenario, resource in (
        (
            "cdp_self_navigation_result_and_sessions",
            False,
            _self_navigation,
            fixture_url,
        ),
        ("cdp_self_close_transport", False, _self_close, endpoint),
        ("cdp_navigation_session_lanes", False, _navigation, fixture_url),
        ("cdp_dialog_shared_context_output", False, _dialog, fixture_url),
        ("cdp_dialog_isolated_context_output", True, _dialog, fixture_url),
    ):
        client = await connect_raw_cdp(endpoint)
        wire, contexts, sessions = _Wire(client, name), [], []
        try:
            contexts.append(
                (await wire.call("Target.createBrowserContext"))["browserContextId"]
            )
            if isolated:
                contexts.append(
                    (await wire.call("Target.createBrowserContext"))["browserContextId"]
                )
            _target_a, a = await _page(wire, contexts[0], fixture_url)
            sessions.append(a)
            _target_b, b = await _page(wire, contexts[-1], fixture_url)
            sessions.append(b)
            observed = await scenario(wire, a, b, resource)
            for session in sessions:
                await wire.call("Page.getFrameTree", session=session)
            for context in contexts:
                await wire.call(
                    "Target.disposeBrowserContext", {"browserContextId": context}
                )
            contexts.clear()
            wire.assert_all_replied()
            record_contract(
                results,
                name,
                contract="Independent sessions progress with local navigation and modal waits.",
                source="Raw Chromium wire calibration; independent Renderer sessions and channels.",
                commands=list(
                    dict.fromkeys(method for method, _session in wire.requests.values())
                ),
                observed={**observed, "wire": wire.trace},
            )
        except Exception as error:
            results.append(
                {"name": name, "ok": False, "error": str(error), "wire": wire.trace}
            )
            failures.append(f"{name}: {error}")
        finally:
            for session in sessions:
                with contextlib.suppress(Exception):
                    async with asyncio.timeout(3):
                        await wire.call(
                            "Page.handleJavaScriptDialog", {"accept": False}, session
                        )
            for context in contexts:
                with contextlib.suppress(Exception):
                    async with asyncio.timeout(3):
                        await wire.call(
                            "Target.disposeBrowserContext",
                            {"browserContextId": context},
                        )
            await client.websocket.close()
    if failures:
        raise SmokeError("; ".join(failures))
