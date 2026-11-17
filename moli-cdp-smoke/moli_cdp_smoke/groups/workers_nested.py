"""Nested Worker Inspector replies, startup detach and failed script lifetimes."""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from typing import TYPE_CHECKING, Any

from ..assertions import SmokeError, assert_equal, wait_until
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
        await probe.command("Target.setDiscoverTargets", {"discover": True})
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
        outer_attachment = (await probe.event("Target.attachedToTarget", page))[
            "params"
        ]
        outer = outer_attachment["sessionId"]
        await probe.auto_attach(outer, True)
        await probe.command("Network.enable", session=outer)
        await probe.command("Runtime.runIfWaitingForDebugger", session=outer)
        boot = await probe.command(
            "Runtime.evaluate",
            {"expression": "outerBoot", "awaitPromise": True, "returnByValue": True},
            page,
        )
        assert_equal(boot["result"]["value"], "ready", "outer Worker executed")
        failed = case in ("script-failure", "failed-script-detach")
        loading = case in ("late-auto-attach", "close-loading")
        if case == "late-auto-attach":
            await probe.auto_attach(outer, False)
        gate_path = path if failed else "/worker.js"
        if failed or loading:
            fixture.hold(gate_path, before_headers=case != "close-loading")
        script_url = fixture.origin + (path if failed else "/worker.js?inner-" + case)
        known_targets: set[str] = set()
        if case == "close-loading":
            existing = await probe.command("Target.getTargets")
            known_targets.update(t["targetId"] for t in existing["targetInfos"])
            known_targets.update(
                m["params"]["targetInfo"]["targetId"]
                for m in probe.messages
                if m.get("method") == "Target.targetCreated"
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
        if failed or loading:
            # Chromium announces the creator's request before response headers,
            # but can expose the provisional child target only after headers.
            request = await probe.wait(
                lambda m: (
                    m.get("method") == "Network.requestWillBeSent"
                    and m.get("sessionId") == outer
                    and m["params"]["request"]["url"] == script_url
                )
            )
            if case == "late-auto-attach":
                await probe.auto_attach(outer, True)
                assert_equal(
                    [
                        m
                        for m in probe.messages
                        if m.get("method") == "Target.attachedToTarget"
                        and m.get("sessionId") == outer
                    ],
                    [],
                    "a loading child must not attach before its response headers",
                )
            elif case == "close-loading":
                await wait_until(
                    lambda: gate_path in fixture.sent_headers,
                    "script headers received while body is held",
                )
                created = await probe.wait(
                    lambda m: (
                        m.get("method") == "Target.targetCreated"
                        and m["params"]["targetInfo"]["type"] == "worker"
                        and m["params"]["targetInfo"]["targetId"] not in known_targets
                    )
                )
                loading_target = created["params"]["targetInfo"]["targetId"]
                close_id = await client.send(
                    "Target.closeTarget", {"targetId": loading_target}
                )
                closed = await probe.wait(lambda m: m.get("id") == close_id)
                assert_equal(
                    closed.get("error"),
                    {
                        "code": -32602,
                        "message": "Specified target doesn't support closing",
                    },
                    "close is rejected while the Worker is still loading",
                )
                targets = await probe.command("Target.getTargets")
                assert_equal(
                    any(
                        t["targetId"] == loading_target for t in targets["targetInfos"]
                    ),
                    True,
                    "failed close retains loading target",
                )
            fixture.release(gate_path)
        attached = await probe.event("Target.attachedToTarget", outer)
        inner = attached["params"]["sessionId"]
        assert_equal(
            attached["params"]["waitingForDebugger"], True, "nested startup pause"
        )
        if failed:
            assert request is not None
            await probe.command("Network.enable", session=inner)
            await probe.response(create, outer)
            if case == "failed-script-detach":
                await probe.auto_attach(outer, False)
                await probe.event("Target.detachedFromTarget", outer, sessionId=inner)
                child_target = attached["params"]["targetInfo"]["targetId"]
                await probe.wait(
                    lambda m: (
                        m.get("method") == "Target.targetDestroyed"
                        and m["params"]["targetId"] == child_target
                    )
                )
                targets = await probe.command("Target.getTargets")
                assert_equal(
                    any(t["targetId"] == child_target for t in targets["targetInfos"]),
                    False,
                    "failed Worker is retired when its last debugger leaves",
                )
                state.record("native_nested_worker_" + case, {"events": probe.messages})
                return
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
        elif case == "close-running":
            await probe.response(create, outer)
            await probe.command("Runtime.runIfWaitingForDebugger", session=inner)
            boot = await probe.command(
                "Runtime.evaluate",
                {
                    "expression": "childBoot",
                    "awaitPromise": True,
                    "returnByValue": True,
                },
                outer,
            )
            assert_equal(
                boot["result"]["value"], "booted", "Worker runs before closeTarget"
            )
            child_target = attached["params"]["targetInfo"]["targetId"]
            close_id = await client.send(
                "Target.closeTarget", {"targetId": child_target}
            )
            closed = await probe.wait(lambda m: m.get("id") == close_id)
            assert_equal(
                closed.get("error"),
                {"code": -32602, "message": "Specified target doesn't support closing"},
                "running DedicatedWorker cannot be closed through CDP",
            )
            result = await probe.command(
                "Runtime.evaluate", {"expression": "40+2", "returnByValue": True}, inner
            )
            assert_equal(
                result["result"]["value"],
                42,
                "rejected close leaves Worker execution alive",
            )
            targets = await probe.command("Target.getTargets")
            assert_equal(
                any(t["targetId"] == child_target for t in targets["targetInfos"]),
                True,
                "rejected close retains running target",
            )
        elif not loading:
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
        if not failed:
            if loading:
                await probe.response(create, outer)
                await probe.command("Runtime.runIfWaitingForDebugger", session=inner)
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
            if case == "late-auto-attach":
                assert_equal(
                    len(
                        [
                            m
                            for m in probe.messages
                            if m.get("method") == "Target.attachedToTarget"
                            and m.get("sessionId") == outer
                        ]
                    ),
                    1,
                    "exactly one child attachment after loading",
                )
        state.record("native_nested_worker_" + case, {"events": probe.messages})
    finally:
        for gate_path in (path, "/worker.js"):
            if gate_path in fixture.gates:
                fixture.release(gate_path)
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
            "late-auto-attach",
            "close-loading",
            "close-running",
            "failed-script-detach",
        ):
            await check_nested_worker_protocol(state, fixture, case, endpoint)
    finally:
        await stop_browser(server)
