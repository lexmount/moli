"""Nested Worker Inspector replies, startup detach and failed script lifetimes."""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from typing import TYPE_CHECKING, Any

from ..assertions import SmokeError, assert_equal
from ..raw_cdp import connect_raw_cdp
from ..serve import start_browser, stop_browser, wait_for_browser_endpoint

if TYPE_CHECKING:
    from ..state import SmokeState
    from .workers_native import _Fixture


class _Probe:
    def __init__(self, client: Any) -> None:
        self.client = client
        self.messages: list[dict[str, Any]] = []

    async def wait(self, predicate: Callable[[dict[str, Any]], bool]) -> dict[str, Any]:
        for message in self.messages:
            if predicate(message):
                return message
        async with asyncio.timeout(10):
            while True:
                message = await self.client.recv()
                self.messages.append(message)
                if predicate(message):
                    return message

    async def response(
        self, message_id: int, session: str | None = None
    ) -> dict[str, Any]:
        response = await self.wait(lambda m: m.get("id") == message_id)
        assert_equal(
            response.get("sessionId"), session, "reply owning flattened session"
        )
        if "error" in response:
            raise SmokeError(f"nested Worker command failed: {response}")
        return response.get("result", {})

    async def command(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        session: str | None = None,
    ) -> dict[str, Any]:
        message_id = await self.client.send(method, params, session_id=session)
        return await self.response(message_id, session)

    async def event(self, method: str, session: str, **params: Any) -> dict[str, Any]:
        return await self.wait(
            lambda m: (
                m.get("method") == method
                and m.get("sessionId") == session
                and all(m.get("params", {}).get(k) == v for k, v in params.items())
            )
        )

    async def auto_attach(self, session: str, enabled: bool) -> None:
        params: dict[str, Any] = {
            "autoAttach": enabled,
            "waitForDebuggerOnStart": enabled,
            "flatten": True,
        }
        if enabled:
            params["filter"] = [{"type": "worker", "exclude": False}, {"exclude": True}]
        await self.command("Target.setAutoAttach", params, session)


async def check_nested_worker_protocol(
    state: SmokeState, fixture: _Fixture, case: str, endpoint: str | None = None
) -> None:
    client = await connect_raw_cdp(endpoint or state.endpoint)
    probe = _Probe(client)
    target = None
    path = "/missing-nested-worker.js"
    try:
        target = (await probe.command("Target.createTarget", {"url": "about:blank"}))[
            "targetId"
        ]
        page = (
            await probe.command(
                "Target.attachToTarget", {"targetId": target, "flatten": True}
            )
        )["sessionId"]
        await probe.command("Page.enable", session=page)
        await probe.command("Network.enable", session=page)
        await probe.command("Page.navigate", {"url": fixture.origin + "/page"}, page)
        await probe.wait(
            lambda m: (
                m.get("method") == "Page.frameNavigated"
                and m.get("sessionId") == page
                and m["params"]["frame"]["url"] == fixture.origin + "/page"
            )
        )
        await probe.auto_attach(page, True)
        await probe.command(
            "Runtime.evaluate",
            {
                "expression": f"""
            globalThis.descendantBoot = new Promise(resolve => globalThis.resolveDescendantBoot = resolve);
            globalThis.outerBoot = new Promise(resolve => {{
              globalThis.outerWorker = new Worker({(fixture.origin + "/worker.js?nested-" + case)!r});
              outerWorker.onmessage = e => {{ if(e.data.ready) resolve('ready'); if(e.data.childBoot) resolveDescendantBoot('booted'); }};
            }}); 'created';
        """
            },
            page,
        )
        outer = (await probe.event("Target.attachedToTarget", page))["params"][
            "sessionId"
        ]
        await probe.auto_attach(outer, True)
        await probe.command("Network.enable", session=outer)
        await probe.command("Runtime.runIfWaitingForDebugger", session=outer)
        boot = await probe.command(
            "Runtime.evaluate",
            {"expression": "outerBoot", "awaitPromise": True, "returnByValue": True},
            page,
        )
        assert_equal(boot["result"]["value"], "ready", "outer Worker executed")
        if case == "script-failure":
            fixture.hold(path, before_headers=True)
        script_url = fixture.origin + (
            path if case == "script-failure" else "/worker.js?inner-" + case
        )
        create = await client.send(
            "Runtime.evaluate",
            {
                "expression": f"""
            globalThis.childBoot = new Promise(resolve => {{
              globalThis.probeChild = new Worker({script_url!r});
              probeChild.onmessage = e => {{ if(e.data.ready) {{resolve('booted');postMessage({{childBoot:true}});}} }};
              probeChild.onerror = () => resolve('error');
            }}); 'created';
        """
            },
            session_id=outer,
        )
        request = None
        if case == "script-failure":
            # Chromium announces the creator's request before response headers,
            # but can expose the provisional child target only after headers.
            request = await probe.wait(
                lambda m: (
                    m.get("method") == "Network.requestWillBeSent"
                    and m.get("sessionId") == outer
                    and m["params"]["request"]["url"] == script_url
                )
            )
            fixture.release(path)
        attached = await probe.event("Target.attachedToTarget", outer)
        inner = attached["params"]["sessionId"]
        assert_equal(
            attached["params"]["waitingForDebugger"], True, "nested startup pause"
        )
        if case == "script-failure":
            assert request is not None
            await probe.command("Network.enable", session=inner)
            await probe.response(create, outer)
            await probe.command("Runtime.runIfWaitingForDebugger", session=inner)
            response = await probe.event(
                "Network.responseReceived",
                inner,
                requestId=request["params"]["requestId"],
            )
            assert_equal(
                response["params"]["response"]["status"],
                404,
                "failed main script status",
            )
            await probe.event(
                "Network.loadingFailed", inner, requestId=request["params"]["requestId"]
            )
            extra = await probe.event(
                "Network.responseReceivedExtraInfo",
                page,
                requestId=request["params"]["requestId"],
            )
            assert_equal(
                extra["params"]["statusCode"],
                404,
                "nested main script ExtraInfo belongs to root Page",
            )
        elif case == "flattened-response":
            await probe.response(create, outer)
            result = await probe.command(
                "Runtime.evaluate", {"expression": "1+1", "returnByValue": True}, inner
            )
            assert_equal(
                result["result"]["value"], 2, "flattened child Inspector response"
            )
            await probe.command("Runtime.runIfWaitingForDebugger", session=inner)
        else:
            await probe.response(create, outer)
            if case == "disable-auto-attach":
                await probe.auto_attach(outer, False)
            elif case == "detach-parent-session":
                await probe.command(
                    "Target.detachFromTarget", {"sessionId": outer}, page
                )
            else:
                await probe.command(
                    "Target.detachFromTarget", {"sessionId": inner}, outer
                )
            await probe.event(
                "Target.detachedFromTarget",
                page if case == "detach-parent-session" else outer,
                sessionId=outer if case == "detach-parent-session" else inner,
            )
        if case != "script-failure":
            boot = await probe.command(
                "Runtime.evaluate",
                {
                    "expression": "descendantBoot"
                    if case == "detach-parent-session"
                    else "childBoot",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
                page if case == "detach-parent-session" else outer,
            )
            assert_equal(
                boot["result"]["value"],
                "booted",
                "child executes after startup barrier release",
            )
        state.record("native_nested_worker_" + case, {"events": probe.messages})
    finally:
        if case == "script-failure":
            fixture.release(path)
        state.subresource_events.extend(m for m in probe.messages if "method" in m)
        if target:
            await probe.command("Target.closeTarget", {"targetId": target})
        await client.websocket.close()


async def run_nested_worker_protocol_checks(
    state: SmokeState, fixture: _Fixture
) -> None:
    # Playwright's own auto-attacher can resume a startup-paused child. These
    # checks need exclusive debugger ownership, including when that owner leaves.
    server = await start_browser()
    try:
        endpoint = await wait_for_browser_endpoint(server)
        for case in (
            "flattened-response",
            "disable-auto-attach",
            "detach-session",
            "detach-parent-session",
            "script-failure",
        ):
            await check_nested_worker_protocol(state, fixture, case, endpoint)
    finally:
        await stop_browser(server)
