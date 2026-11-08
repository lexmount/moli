from __future__ import annotations

import asyncio
import json
import tempfile
import unittest
from pathlib import Path
from typing import Any
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from moli_cdp_smoke.assertions import wait_until
from moli_cdp_smoke.diagnostics import (
    Diagnostics, ObservedCdpSession, current_diagnostics, diagnostics_scope,
    observe_playwright_connection, record_phase, record_wire, write_artifact,
)
from moli_cdp_smoke.raw_cdp import RawCdpClient
from moli_cdp_smoke.helpers import attach_cdp_event_collector
from moli_cdp_smoke.progress import await_with_progress


class _Socket:
    def __init__(self) -> None:
        self.sent: list[dict[str, Any]] = []
        self.messages: asyncio.Queue[str] = asyncio.Queue()

    async def send(self, message: str) -> None:
        self.sent.append(json.loads(message))

    async def recv(self) -> str:
        return await self.messages.get()


class DiagnosticsTests(unittest.IsolatedAsyncioTestCase):
    async def test_failure_preserves_pending_commands_and_task_stack_before_later_cases(self) -> None:
        entered = asyncio.Event()

        async def pending_history_back() -> None:
            entered.set()
            await asyncio.Event().wait()

        with tempfile.TemporaryDirectory() as directory:
            with diagnostics_scope(Path(directory) / "worker.json") as diagnostics:
                task = asyncio.create_task(pending_history_back(), name="pending-history-back")
                await entered.wait()
                record_wire(self, "send", {"id": 17, "method": "Page.goBack"})
                with self.assertRaises(TimeoutError):
                    await await_with_progress("history-back", task, timeout_seconds=0.01)
                for index in range(150):
                    record_phase(f"later-case-{index}", "passed")
                    record_wire(self, "event", {"method": "later.event"})
            saved = json.loads((diagnostics.directory / "failure-001.json").read_text())
            self.assertEqual(saved["phases"][-1]["label"], "history-back")
            self.assertEqual(saved["phases"][-1]["status"], "failed")
            self.assertEqual(saved["pendingCommands"][0]["message"]["method"], "Page.goBack")
            stack = next(item for item in saved["taskStacks"] if item["name"] == "pending-history-back")
            self.assertIn("pending_history_back", "".join(stack["stack"]))
            checkpoint = json.loads((diagnostics.directory / "checkpoint.json").read_text())
            self.assertFalse(any(item["label"] == "history-back" for item in checkpoint["phases"]))
            self.assertIn(str(diagnostics.directory / "failure-001.json"), diagnostics.artifacts)

    async def test_lone_utf16_surrogates_round_trip_in_protocol_and_trace_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            value = "high:\ud800-low:\udfff"
            with diagnostics_scope(Path(directory) / "worker.json") as diagnostics:
                record_wire(self, "receive", {"result": {"value": value}})
                artifact = write_artifact("cpu-trace.json", {"functionName": value})
            saved = json.loads((diagnostics.directory / "checkpoint.json").read_text())
            self.assertEqual(saved["lastMessages"][0]["message"]["result"]["value"], value)
            self.assertEqual(json.loads(Path(artifact).read_text())["functionName"], value)
            self.assertEqual(diagnostics.errors, [])

    async def test_failed_trace_finalization_still_preserves_received_chunks(self) -> None:
        from pyee import EventEmitter
        from moli_cdp_smoke.groups.tracing import _verify_agent_browser_profiler_configuration

        class Session(EventEmitter):
            async def send(self, method: str, params: Any = None) -> dict[str, Any]:
                if method == "Tracing.end":
                    self.emit("Tracing.dataCollected", {"value": [{"name": "received chunk"}]})
                    raise RuntimeError("tracing finalization failed")
                return {}

        state = SimpleNamespace(
            cdp=Session(), fixture="http://fixture.test",
            page=SimpleNamespace(goto=AsyncMock(), evaluate=AsyncMock(side_effect=RuntimeError("workload failed"))),
        )
        with tempfile.TemporaryDirectory() as directory:
            with diagnostics_scope(Path(directory) / "worker.json") as diagnostics:
                with self.assertRaisesRegex(RuntimeError, "tracing finalization failed") as caught:
                    await _verify_agent_browser_profiler_configuration(state)
            self.assertIn("workload failed", str(caught.exception.__context__))
            trace = json.loads((diagnostics.directory / "tracing-cpu.json").read_text())
            self.assertEqual(trace["traceEvents"], [{"name": "received chunk"}])
            self.assertIsNone(trace["tracingComplete"])

    async def test_unhandled_callback_failure_is_retained_and_previous_handler_restored(self) -> None:
        loop = asyncio.get_running_loop()
        original = loop.get_exception_handler()
        seen: list[dict[str, Any]] = []
        previous = lambda active_loop, context: seen.append(context)
        loop.set_exception_handler(previous)
        try:
            with diagnostics_scope(None) as diagnostics:
                entered = asyncio.Event()

                def fail() -> None:
                    entered.set()
                    raise RuntimeError("unexpected protocol callback")

                loop.call_soon(fail)
                await entered.wait()
                result = diagnostics.finish()
            self.assertEqual(len(seen), 1)
            self.assertIn("RuntimeError: unexpected protocol callback", result["errors"][0])
            self.assertIs(loop.get_exception_handler(), previous)
        finally:
            loop.set_exception_handler(original)

    async def test_unhandled_background_task_failure_is_not_only_printed(self) -> None:
        loop = asyncio.get_running_loop()
        original = loop.get_exception_handler()
        loop.set_exception_handler(lambda *_: None)
        try:
            with diagnostics_scope(None) as diagnostics:
                async def fail() -> None:
                    raise RuntimeError("unobserved task failed")

                task = asyncio.create_task(fail())
                await asyncio.sleep(0)
                del task  # Deliver the actual 'exception was never retrieved' callback.
                self.assertIn("RuntimeError: unobserved task failed", diagnostics.finish()["errors"][0])
        finally:
            loop.set_exception_handler(original)

    async def test_unhandled_callback_fails_an_otherwise_successful_worker(self) -> None:
        from moli_cdp_smoke import runner

        fixture = SimpleNamespace(url="http://fixture.test", start=lambda: None, stop=lambda: None)

        async def smoke(endpoint: str, server: Any, selection: Any, results: list[Any]) -> None:
            completed = asyncio.Event()

            def fail() -> None:
                completed.set()
                raise RuntimeError("background protocol callback failed")

            asyncio.get_running_loop().call_soon(fail)
            await completed.wait()
            results.append({"name": "normal assertion", "ok": True})

        loop = asyncio.get_running_loop()
        previous = loop.get_exception_handler()
        loop.set_exception_handler(lambda *_: None)
        try:
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "worker.json"
                args = runner.parse_args(["--group", "tracing", "--browser", "chromium",
                                          "--endpoint", "http://fixture.test", "--result", str(path)])
                with (diagnostics_scope(path),
                      patch.object(runner, "FixtureServer", return_value=fixture),
                      patch.object(runner, "wait_for_cdp_server", new_callable=AsyncMock),
                      patch.object(runner, "browser_identity", new_callable=AsyncMock,
                                   return_value={"engine": "chromium"}),
                      patch.object(runner, "stop_browser", new_callable=AsyncMock),
                      patch.object(runner, "run_smoke", side_effect=smoke)):
                    code = await runner._run_worker(args)
                payload = json.loads(path.read_text())
                self.assertEqual(code, 1)
                self.assertFalse(payload["ok"])
                self.assertTrue(payload["results"][0]["ok"])
                self.assertIn("background protocol callback failed", payload["error"])
        finally:
            loop.set_exception_handler(previous)

    async def test_raw_messages_keep_wire_order_and_exact_pending_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "raw.json"
            with diagnostics_scope(result) as diagnostics:
                socket = _Socket()
                client = RawCdpClient(socket)  # type: ignore[arg-type]
                request = await client.send("Page.navigate", {"url": "http://fixture.test/"},
                                            session_id="page")
                checkpoint = diagnostics.directory / "checkpoint.json"
                await wait_until(lambda: checkpoint.exists()
                                 and json.loads(checkpoint.read_text())["messageCount"] >= 1,
                                 "diagnostic checkpoint", timeout_ms=2_000)
                saved = json.loads(checkpoint.read_text())
                self.assertEqual(saved["pendingCommands"][0]["message"]["id"], request)
                self.assertEqual(saved["pendingCommands"][0]["message"]["sessionId"], "page")
                for message in [{"method": "Page.frameNavigated", "sessionId": "page"},
                                {"id": request, "sessionId": "page", "result": {}}]:
                    socket.messages.put_nowait(json.dumps(message))
                await client.recv_until_id(request)
                client.mark_no_followup_boundary()
                diagnostics.finish()
                saved = json.loads(checkpoint.read_text())
                self.assertEqual(len(socket.sent), 1, "diagnostics sent browser commands")
                self.assertEqual([m["direction"] for m in saved["lastMessages"]],
                                 ["send", "receive", "receive"])
                self.assertEqual(saved["pendingCommands"], [])

    async def test_failure_phase_and_bounded_tail_survive_without_worker_completion(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with diagnostics_scope(Path(directory) / "worker.json") as diagnostics:
                for index in range(300):
                    record_wire(self, "receive", {"method": "event", "params": {
                        "index": index, "payload": "x" * 100_000,
                    }})
                record_phase("navigation/main-frame", "failed", RuntimeError("target crashed"))
                checkpoint = diagnostics.directory / "checkpoint.json"
                await wait_until(lambda: checkpoint.exists()
                                 and json.loads(checkpoint.read_text())["messageCount"] >= 300,
                                 "worker checkpoint", timeout_ms=2_000)
                saved = json.loads(checkpoint.read_text())
                self.assertEqual(saved["messageCount"], 300)
                self.assertLessEqual(len(saved["lastMessages"]), 128)
                self.assertEqual(saved["lastMessages"][-1]["message"]["params"]["index"], 299)
                self.assertLess(checkpoint.stat().st_size, 500_000)
                self.assertTrue(any(phase.get("error") == "RuntimeError: target crashed"
                                    for phase in saved["phases"]))
                self.assertIn("cpuCount", saved["initialResources"])

    async def test_raw_trace_artifacts_are_complete_and_outlive_scope(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            trace = {"traceEvents": [{"name": f"event-{index}"} for index in range(200)]}
            with diagnostics_scope(Path(directory) / "tracing.json"):
                path = write_artifact("cpu-trace.json", trace)
            self.assertEqual(json.loads(Path(path).read_text()), trace)

    async def test_capture_errors_are_retained_as_failures(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with diagnostics_scope(Path(directory) / "worker.json") as diagnostics:
                with patch.object(Path, "write_text", side_effect=PermissionError("read-only")):
                    self.assertIsNone(write_artifact("cpu-trace.json", {}))
                result = diagnostics.finish()
                self.assertTrue(result["errors"])
                self.assertIn("PermissionError", result["errors"][0])
                self.assertIn("read-only", result["errors"][0])

    async def test_failing_finalization_does_not_leak_the_diagnostic_scope(self) -> None:
        with patch.object(Diagnostics, "finish", side_effect=RuntimeError("serialization failed")):
            with self.assertRaisesRegex(RuntimeError, "serialization failed"):
                with diagnostics_scope(None):
                    self.assertIsNotNone(current_diagnostics())
        self.assertIsNone(current_diagnostics())

    async def test_capture_failure_makes_an_otherwise_successful_worker_fail(self) -> None:
        from moli_cdp_smoke import runner

        fixture = SimpleNamespace(url="http://fixture.test", start=lambda: None, stop=lambda: None)
        diagnostics = SimpleNamespace(finish=lambda: {"errors": ["PermissionError: read-only"]})

        async def smoke(endpoint: str, server: Any, selection: Any, results: list[Any]) -> None:
            results.append({"name": "protocol assertion", "ok": True})

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "worker.json"
            args = runner.parse_args(["--group", "tracing", "--browser", "chromium",
                                      "--endpoint", "http://fixture.test", "--result", str(path)])
            with (patch.object(runner, "FixtureServer", return_value=fixture),
                  patch.object(runner, "wait_for_cdp_server", new_callable=AsyncMock),
                  patch.object(runner, "browser_identity", new_callable=AsyncMock,
                               return_value={"engine": "chromium"}),
                  patch.object(runner, "stop_browser", new_callable=AsyncMock),
                  patch.object(runner, "run_smoke", side_effect=smoke),
                  patch.object(runner, "current_diagnostics", return_value=diagnostics)):
                code = await runner._run_worker(args)
            saved = json.loads(path.read_text())
            self.assertEqual(code, 1)
            self.assertFalse(saved["ok"])
            self.assertTrue(saved["results"][0]["ok"])
            self.assertIn("diagnostic capture failed", saved["error"])
            self.assertIn("PermissionError", saved["error"])


class _Session:
    def __init__(self) -> None:
        self.calls: list[tuple[str, Any]] = []
        self.result = {"result": {"value": 42}}
        self.error: BaseException | None = None

    async def send(self, method: str, params: Any = None) -> Any:
        self.calls.append((method, params))
        if self.error:
            raise self.error
        return self.result

    async def detach(self) -> None:
        self.calls.append(("detach", None))


class SessionObservationTests(unittest.IsolatedAsyncioTestCase):
    async def test_sdk_navigation_and_frame_events_are_observed_without_mutating_transport(self) -> None:
        sent, received = [], []
        send, receive = sent.append, received.append
        transport = SimpleNamespace(send=send, on_message=receive)
        playwright = SimpleNamespace(_impl_obj=SimpleNamespace(
            _connection=SimpleNamespace(_transport=transport),
        ))
        command = {"id": 7, "guid": "frame", "method": "goto", "params": {"url": "http://fixture.test/"}}
        event = {"guid": "frame", "method": "navigated", "params": {"url": "http://fixture.test/"}}
        reply = {"id": 7, "result": {"response": {"guid": "response"}}}
        with diagnostics_scope(None) as diagnostics:
            with observe_playwright_connection(playwright):
                transport.send(command)
                self.assertTrue(diagnostics.pending)
                transport.on_message(event)
                transport.on_message(reply)
            self.assertIs(transport.send, send)
            self.assertIs(transport.on_message, receive)
            self.assertEqual(diagnostics.pending, {})
            self.assertEqual([m["message"] for m in diagnostics.messages], [command, event, reply])
            self.assertTrue(all(m["transport"] == "playwright-driver" for m in diagnostics.messages))
        self.assertIs(sent[0], command)
        self.assertIs(received[0], event)
        self.assertIs(received[1], reply)

    async def test_sdk_transport_failure_is_preserved_and_hooks_are_restored(self) -> None:
        error = ValueError("driver write failed")

        def fail(message: Any) -> None:
            raise error

        transport = SimpleNamespace(send=fail, on_message=lambda _: None)
        playwright = SimpleNamespace(_impl_obj=SimpleNamespace(
            _connection=SimpleNamespace(_transport=transport),
        ))
        with self.assertRaises(ValueError) as caught:
            with observe_playwright_connection(playwright):
                transport.send({"id": 1, "method": "goto"})
        self.assertIs(caught.exception, error)
        self.assertIs(transport.send, fail)
        with self.assertRaisesRegex(RuntimeError, "Playwright transport changed"):
            with observe_playwright_connection(SimpleNamespace()):
                self.fail("unsupported SDK must not disable diagnosis silently")

    async def test_multiple_scenario_listeners_do_not_duplicate_the_protocol_evidence(self) -> None:
        class Session(_Session):
            def __init__(self) -> None:
                super().__init__()
                self.handlers: dict[str, list[Any]] = {}

            def on(self, method: str, handler: Any) -> None:
                self.handlers.setdefault(method, []).append(handler)

            def remove_listener(self, method: str, handler: Any) -> None:
                self.handlers[method].remove(handler)

            def emit(self, method: str, params: Any) -> None:
                for handler in list(self.handlers.get(method, [])):
                    handler(params)

        original = Session()
        session = ObservedCdpSession(original)
        with diagnostics_scope(None) as diagnostics:
            first = attach_cdp_event_collector(session, ["Page.lifecycleEvent"])
            peer = attach_cdp_event_collector(session, ["Page.lifecycleEvent"])
            original.emit("Page.lifecycleEvent", {"loaderId": "first"})
            self.assertEqual(len(first), 1)
            self.assertEqual(len(peer), 1)
            self.assertEqual(len(diagnostics.messages), 1)
            first.close()
            original.emit("Page.lifecycleEvent", {"loaderId": "second"})
            self.assertEqual(len(first), 1)
            self.assertEqual(len(peer), 2)
            self.assertEqual(len(diagnostics.messages), 2)
            await session.detach()
            peer.close()
            self.assertEqual(original.handlers["Page.lifecycleEvent"], [])

    async def test_sdk_calls_keep_their_original_result_and_exception(self) -> None:
        original = _Session()
        session = ObservedCdpSession(original)
        with diagnostics_scope(None) as diagnostics:
            self.assertIs(await session.send("Runtime.evaluate", {"expression": "21 * 2"}),
                          original.result)
            error = ValueError("protocol error")
            original.error = error
            with self.assertRaises(ValueError) as caught:
                await session.send("Page.navigate", {"url": "http://fixture.test/"})
            self.assertIs(caught.exception, error)
            await session.detach()
            self.assertEqual(len(original.calls), 3)
            self.assertEqual(diagnostics.pending, {})
            self.assertEqual([row["direction"] for row in diagnostics.messages],
                             ["send", "receive"] * 3)
            self.assertTrue(all(row["transport"] == "playwright" for row in diagnostics.messages))


if __name__ == "__main__":
    unittest.main()
