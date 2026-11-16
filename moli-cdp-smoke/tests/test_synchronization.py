from __future__ import annotations

import asyncio
import gc
import json
import sys
import unittest
import weakref
from types import SimpleNamespace
from typing import Any

from moli_cdp_smoke.assertions import SmokeError, wait_until
from moli_cdp_smoke.helpers import attach_cdp_event_collector, observe_worker_network


class WaitDeadlineTests(unittest.IsolatedAsyncioTestCase):
    async def test_deadline_includes_a_blocked_async_predicate(self) -> None:
        async def blocked() -> bool:
            await asyncio.Event().wait()
            return False

        with self.assertRaisesRegex(SmokeError, "blocked command.*10ms"):
            await asyncio.wait_for(wait_until(blocked, "blocked command", timeout_ms=10), 0.25)

    async def test_poll_interval_cannot_extend_the_deadline(self) -> None:
        with self.assertRaisesRegex(SmokeError, "unobserved event.*10ms"):
            await asyncio.wait_for(wait_until(lambda: False, "unobserved event",
                                              timeout_ms=10, interval_ms=1_000), 0.25)

    async def test_real_predicate_errors_are_not_retried_or_reclassified(self) -> None:
        for error in [TimeoutError("command failed"), ValueError("bad response")]:
            calls = 0

            async def fail() -> bool:
                nonlocal calls
                calls += 1
                raise error

            with self.subTest(error=error), self.assertRaises(type(error)) as caught:
                await wait_until(fail, "response")
            self.assertIs(caught.exception, error)
            self.assertEqual(calls, 1)

    async def test_cancellation_is_preserved_and_releases_the_predicate(self) -> None:
        entered, exited = asyncio.Event(), asyncio.Event()

        async def blocked() -> bool:
            entered.set()
            try:
                await asyncio.Event().wait()
            finally:
                exited.set()
            return False

        pending = asyncio.create_task(wait_until(blocked, "canceled wait"))
        await entered.wait()
        pending.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await pending
        self.assertTrue(exited.is_set())


class _Emitter:
    def __init__(self) -> None:
        self.handlers: dict[str, list[Any]] = {}

    def on(self, method: str, handler: Any) -> None:
        self.handlers.setdefault(method, []).append(handler)

    def remove_listener(self, method: str, handler: Any) -> None:
        self.handlers[method].remove(handler)

    def emit(self, method: str, params: Any) -> None:
        for handler in list(self.handlers.get(method, [])):
            handler(params)


class _Payload:
    pass


class _WorkerCdp(_Emitter):
    def __init__(self) -> None:
        super().__init__()
        self.commands: list[tuple[str, dict[str, Any]]] = []
        self.children: asyncio.Queue[tuple[str, dict[str, Any]]] = asyncio.Queue()

    async def send(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        self.commands.append((method, params))
        if method == "Target.sendMessageToTarget":
            self.children.put_nowait((params["sessionId"], json.loads(params["message"])))
        return {}

    async def child_command(self) -> tuple[str, dict[str, Any]]:
        return await asyncio.wait_for(self.children.get(), timeout=1)

    def child_message(self, session: str, message: dict[str, Any]) -> None:
        self.emit("Target.receivedMessageFromTarget", {
            "sessionId": session, "message": json.dumps(message),
        })

    def attach(self, session: str, kind: str = "worker") -> None:
        self.emit("Target.attachedToTarget", {
            "sessionId": session, "targetInfo": {"type": kind},
        })


class WorkerNetworkObserverTests(unittest.IsolatedAsyncioTestCase):
    async def test_enable_ack_precedes_startup_and_forwards_to_the_requested_sink(self) -> None:
        client, events = _WorkerCdp(), []
        state = SimpleNamespace(cdp=client, subresource_events=[])
        async with observe_worker_network(state, events):
            client.attach("PAGE", "page")
            self.assertTrue(client.children.empty())
            client.attach("WORKER")
            session, enable = await client.child_command()
            self.assertEqual((session, enable["method"]), ("WORKER", "Network.enable"))
            self.assertTrue(client.children.empty(), "startup raced Network.enable acknowledgement")
            client.child_message(session, {"id": enable["id"], "result": {}})
            resumed_session, resume = await client.child_command()
            self.assertEqual((resumed_session, resume["method"]),
                             (session, "Runtime.runIfWaitingForDebugger"))
            client.child_message(session, {"id": resume["id"], "result": {}})
            client.child_message(session, {
                "method": "Network.requestWillBeSentExtraInfo", "params": {"requestId": "REQUEST"},
            })
            client.child_message(session, {"method": "Runtime.consoleAPICalled", "params": {}})
            self.assertEqual(events, [{
                "method": "Network.requestWillBeSentExtraInfo",
                "params": {"requestId": "REQUEST"}, "sessionId": session,
            }])
            self.assertEqual(state.subresource_events, [])
        self.assertFalse(client.commands[-1][1]["autoAttach"])
        self.assertFalse(any(client.handlers.values()), "Worker listeners outlived their scope")

    async def test_session_scoped_replies_do_not_resume_another_worker(self) -> None:
        client = _WorkerCdp()
        state = SimpleNamespace(cdp=client, subresource_events=[])
        async with observe_worker_network(state):
            client.attach("FIRST")
            client.attach("SECOND")
            first, second = await client.child_command(), await client.child_command()
            self.assertEqual([command[1]["method"] for command in [first, second]],
                             ["Network.enable", "Network.enable"])
            for session, enable in [second, first]:
                client.child_message(session, {"id": enable["id"], "result": {}})
                resumed_session, resume = await client.child_command()
                self.assertEqual(resumed_session, session)
                self.assertEqual(resume["method"], "Runtime.runIfWaitingForDebugger")
                client.child_message(session, {"id": resume["id"], "result": {}})
                client.child_message(session, {
                    "method": "Network.responseReceivedExtraInfo", "params": {"statusCode": 401},
                })
            self.assertEqual([event["sessionId"] for event in state.subresource_events],
                             [second[0], first[0]])

    async def test_failed_enable_keeps_startup_blocked_and_cleans_up(self) -> None:
        client = _WorkerCdp()
        state = SimpleNamespace(cdp=client, subresource_events=[])
        with self.assertRaisesRegex(SmokeError, "worker Network.enable failed"):
            async with observe_worker_network(state):
                client.attach("WORKER")
                session, enable = await client.child_command()
                client.child_message(session, {
                    "id": enable["id"], "error": {"code": -32000, "message": "retired"},
                })
        self.assertTrue(client.children.empty(), "failed Network.enable released startup")
        self.assertFalse(client.commands[-1][1]["autoAttach"])
        self.assertFalse(any(client.handlers.values()))


class EventCollectorLifetimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_explicit_scope_stops_collecting_even_when_results_remain_owned(self) -> None:
        client = _Emitter()
        with attach_cdp_event_collector(client, ["event"]) as events:
            client.emit("event", {"id": 1})
        events.close()
        client.emit("event", {"id": 2})
        self.assertEqual(events, [{"method": "event", "params": {"id": 1}}])
        self.assertEqual(client.handlers["event"], [])

    async def test_finished_scenario_releases_events_and_listeners(self) -> None:
        client, payload = _Emitter(), _Payload()
        observer = attach_cdp_event_collector(client, ["event"])
        reference = weakref.ref(payload)
        client.emit("event", payload)
        self.assertIs(observer[0]["params"], payload)
        del observer, payload
        gc.collect()
        self.assertIsNone(reference(), "client retained a finished scenario's event payloads")
        await asyncio.sleep(0)
        self.assertEqual(client.handlers["event"], [])

    async def test_live_observers_keep_independent_event_order_and_payloads(self) -> None:
        client = _Emitter()
        first = attach_cdp_event_collector(client, ["first", "second"])
        peer = attach_cdp_event_collector(client, ["second"])
        gc.collect()
        client.emit("first", {"id": 1})
        client.emit("second", {"id": 2})
        self.assertEqual([event["params"]["id"] for event in first], [1, 2])
        self.assertEqual([event["params"]["id"] for event in peer], [2])
        del first
        gc.collect()
        client.emit("second", {"id": 3})
        self.assertEqual([event["params"]["id"] for event in peer], [2, 3])

    async def test_gc_inside_the_emitter_lock_does_not_reenter_it(self) -> None:
        script = """
import asyncio, gc
from pyee import EventEmitter
from moli_cdp_smoke.helpers import attach_cdp_event_collector
async def run():
    gc.disable()
    emitter = EventEmitter()
    events = attach_cdp_event_collector(emitter, ['event'])
    events.append({'cycle': events})
    del events
    with emitter._lock:
        gc.collect()
    await asyncio.sleep(0)
    assert emitter.listeners('event') == []
asyncio.run(run())
"""
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c", script, stderr=asyncio.subprocess.PIPE,
        )
        try:
            _, error = await asyncio.wait_for(process.communicate(), timeout=5)
            self.assertEqual(process.returncode, 0, error.decode())
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()


if __name__ == "__main__":
    unittest.main()
