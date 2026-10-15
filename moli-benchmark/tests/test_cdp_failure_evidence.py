from __future__ import annotations

import argparse
import ast
import asyncio
import errno
import hashlib
import http.server
import json
import tempfile
import time
import traceback
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from moli_benchmark import raw_cdp, realtime_workload, sampling, target_serve

EXCEPTION_EVENT = {
    "method": "Runtime.exceptionThrown",
    "params": {"exceptionDetails": {"text": "observed before command failure"}},
}
BINDING_EVENT = {"method": "Runtime.bindingCalled", "params": {"payload": "observed"}}


class FakeWebSocket:
    def __init__(self, incoming, *, close_error=None):
        self.incoming = list(incoming)
        self.close_error = close_error
        self.sent = []
        self.read_count = 0
        self.close_count = 0

    async def send(self, message):
        self.sent.append(json.loads(message))

    async def recv(self):
        self.read_count += 1
        if not self.incoming:
            raise AssertionError("unexpected extra receive")
        item = self.incoming.pop(0)
        if isinstance(item, BaseException):
            raise item
        return json.dumps(item)

    async def close(self):
        self.close_count += 1
        if self.close_error is not None:
            raise self.close_error


def probe_function(filename, name, namespace):
    """Compile the actual local function body with only fake external I/O."""
    path = Path(__file__).resolve().parents[1] / "scripts" / filename
    tree = ast.parse(path.read_text())
    node = next(
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.AsyncFunctionDef) and node.name == name
    )
    module = ast.Module(body=[node], type_ignores=[])
    exec(  # noqa: S102 -- Only local AST; caller injects fake engine/network/procfs.
        compile(ast.fix_missing_locations(module), str(path), "exec"), namespace
    )
    return namespace[name]


def probe_namespace(client):
    async def connect(_endpoint):
        return client

    return {
        "asyncio": asyncio,
        "time": time,
        "traceback": traceback,
        "error_evidence": sampling.error_evidence,
        "start_target_serve": Mock(
            return_value=SimpleNamespace(
                process=SimpleNamespace(pid=10),
                command=["fake-engine", "serve"],
                ready_ms=1,
                root_identity={"identity": "10:100"},
                endpoint="unused",
            )
        ),
        "clock_ticks_evidence": lambda: {"ticks_per_second": 100},
        "affinity_snapshot": lambda *_args: {"boundary_complete": True},
        "connect_raw_cdp": connect,
        "finish_target_serve_run": target_serve.finish_target_serve_run,
        "MODES": {"Realtime": ("--realtime",)},
    }


class SequentialCdpEvidenceTests(unittest.IsolatedAsyncioTestCase):
    async def test_success_returns_same_ordered_events_without_extra_receive(self):
        response = {"id": 7, "result": {"value": 42}}
        websocket = FakeWebSocket([EXCEPTION_EVENT, BINDING_EVENT, response])
        reply, seen = await raw_cdp.RawCdpClient(websocket).recv_until_id(7)
        self.assertEqual(reply, response)
        self.assertEqual(seen, [EXCEPTION_EVENT, BINDING_EVENT, response])
        self.assertEqual(websocket.read_count, 3)

    async def test_cdp_error_retains_events_and_error_response(self):
        failed_reply = {"id": 7, "error": {"code": -32000, "message": "rejected"}}
        websocket = FakeWebSocket([EXCEPTION_EVENT, BINDING_EVENT, failed_reply])
        with self.assertRaises(raw_cdp.RawCdpError) as caught:
            await raw_cdp.RawCdpClient(websocket).recv_until_id(
                7, method="Runtime.evaluate"
            )
        self.assertIs(type(caught.exception), raw_cdp.RawCdpError)
        self.assertEqual(
            caught.exception.cdp_receive_evidence,
            {
                "message_id": 7,
                "method": "Runtime.evaluate",
                "messages": [EXCEPTION_EVENT, BINDING_EVENT, failed_reply],
            },
        )
        self.assertEqual(websocket.read_count, 3)
        self.assertIn("rejected", str(caught.exception))

    async def test_transport_timeout_and_cancel_preserve_original_exception_object(
        self,
    ):
        for failure in (
            EOFError("closed after event"),
            TimeoutError("receive timed out after event"),
            asyncio.CancelledError("receive canceled after event"),
        ):
            with self.subTest(kind=type(failure).__name__):
                websocket = FakeWebSocket([EXCEPTION_EVENT, failure])
                with self.assertRaises(type(failure)) as caught:
                    await raw_cdp.RawCdpClient(websocket).recv_until_id(
                        7, method="Runtime.evaluate"
                    )
                self.assertIs(caught.exception, failure)
                self.assertEqual(
                    failure.cdp_receive_evidence["messages"], [EXCEPTION_EVENT]
                )
                self.assertEqual(websocket.read_count, 2)

    async def test_deadline_expiry_retains_event_without_another_receive(self):
        # A fake monotonic clock expires after the first received event; no sleep.
        loop = SimpleNamespace(time=Mock(side_effect=[0, 0, 11]))
        client = raw_cdp.RawCdpClient(FakeWebSocket([]))

        async def receive():
            return EXCEPTION_EVENT

        async def direct_wait(awaitable, *, timeout):
            self.assertEqual(timeout, 10)
            return await awaitable

        client.recv = receive
        with (
            patch.object(raw_cdp.asyncio, "get_running_loop", return_value=loop),
            patch.object(raw_cdp.asyncio, "wait_for", side_effect=direct_wait),
            self.assertRaises(raw_cdp.RawCdpTimeoutError) as caught,
        ):
            await client.recv_until_id(7, method="Runtime.evaluate")
        self.assertEqual(
            caught.exception.cdp_receive_evidence["messages"], [EXCEPTION_EVENT]
        )

    async def test_each_actual_probe_command_preserves_failure_and_observed_events(
        self,
    ):
        for filename in ("probe-realtime-layout.py", "probe-layout-lifetime.py"):
            for failure in (
                EOFError("closed"),
                TimeoutError("timed out"),
                asyncio.CancelledError("canceled"),
            ):
                with self.subTest(probe=filename, kind=type(failure).__name__):
                    websocket = FakeWebSocket([EXCEPTION_EVENT, BINDING_EVENT, failure])
                    record = {"runtime_errors": []}
                    namespace = {
                        "client": raw_cdp.RawCdpClient(websocket),
                        "record": record,
                        "errors": record["runtime_errors"],
                        "events": [],
                        "error_evidence": sampling.error_evidence,
                    }
                    command = probe_function(filename, "command", namespace)
                    with self.assertRaises(type(failure)) as caught:
                        await command(
                            "Runtime.evaluate", {"expression": "1"}, "session"
                        )
                    self.assertIs(caught.exception, failure)
                    self.assertEqual(record["runtime_errors"], [EXCEPTION_EVENT])
                    detail = record["cdp_command_errors"][0]
                    self.assertEqual(detail["method"], "Runtime.evaluate")
                    self.assertEqual(detail["message_id"], 1)
                    self.assertEqual(detail["session_id"], "session")
                    self.assertEqual(detail["error"]["type"], type(failure).__name__)
                    self.assertEqual(
                        detail["receive"]["messages"], [EXCEPTION_EVENT, BINDING_EVENT]
                    )
                    self.assertEqual(websocket.read_count, 3)
                    self.assertEqual(len(websocket.sent), 1)

    async def test_failed_send_keeps_error_without_fabricating_receive_evidence(self):
        failure = OSError(errno.EPIPE, "send failed")

        async def failed_send(*_args, **_kwargs):
            raise failure

        record = {"runtime_errors": []}
        namespace = {
            "client": SimpleNamespace(send=failed_send),
            "record": record,
            "events": [],
            "error_evidence": sampling.error_evidence,
        }
        command = probe_function("probe-realtime-layout.py", "command", namespace)
        with self.assertRaises(OSError) as caught:
            await command("Target.createTarget")
        self.assertIs(caught.exception, failure)
        self.assertEqual(record["runtime_errors"], [])
        detail = record["cdp_command_errors"][0]
        self.assertIsNone(detail["message_id"])
        self.assertIsNone(detail["receive"])
        self.assertEqual(detail["error"]["errno"], errno.EPIPE)


class CancelledCleanupEvidenceTests(unittest.IsolatedAsyncioTestCase):
    async def test_cancelled_close_runs_real_stop_order_and_preserves_primary_error(
        self,
    ):
        cancellation = asyncio.CancelledError("close canceled")
        websocket = FakeWebSocket([], close_error=cancellation)
        calls = []
        process = SimpleNamespace(pid=10, returncode=None)
        process.poll = lambda: process.returncode

        def seal():
            self.assertIsNone(process.returncode)
            calls.append("seal")
            return {"sealed": True, "thread_alive_after_stop": False}

        def terminate(_process):
            calls.append("terminate")
            process.returncode = 143
            return {"exited": True, "returncode": 143}

        thread = SimpleNamespace(join=lambda timeout: None, is_alive=lambda: False)
        handle = SimpleNamespace(
            process=process,
            root_identity={"identity": "10:100"},
            sampler=SimpleNamespace(
                stop=seal, export_samples=lambda: [{"sequence": 1}]
            ),
            logs=[],
            log_threads=[thread, thread],
            log_state={
                name: {"eof": True, "error": None} for name in ("stdout", "stderr")
            },
            port_lease=SimpleNamespace(close=lambda: calls.append("release")),
            temp_dir=None,
        )
        record = {"ok": False, "error": "original workload failure"}
        with (
            patch.object(
                target_serve,
                "_stat_read",
                return_value={"identity": "10:100", "state": "S"},
            ),
            patch.object(target_serve, "_terminate_process", side_effect=terminate),
            self.assertRaises(asyncio.CancelledError) as caught,
        ):
            await target_serve.finish_target_serve_run(
                SimpleNamespace(websocket=websocket), handle, record
            )
        self.assertIs(caught.exception, cancellation)
        self.assertEqual(calls, ["seal", "terminate", "release"])
        self.assertEqual(record["error"], "original workload failure")
        self.assertEqual(record["cleanup_errors"][0]["type"], "CancelledError")
        self.assertEqual(record["server"]["resources"]["samples"], [{"sequence": 1}])
        self.assertTrue(record["server"]["cleanup_complete"])
        self.assertFalse(record["ok"])
        self.assertFalse(record["safe_to_continue"])

    async def test_stop_error_does_not_replace_close_cancellation_or_primary_error(
        self,
    ):
        cancellation = asyncio.CancelledError("close canceled")
        record = {"ok": False, "error": "original workload failure"}
        with (
            patch.object(
                target_serve,
                "stop_target_serve",
                side_effect=OSError(errno.EIO, "stop failed"),
            ) as stop,
            self.assertRaises(asyncio.CancelledError) as caught,
        ):
            await target_serve.finish_target_serve_run(
                SimpleNamespace(websocket=FakeWebSocket([], close_error=cancellation)),
                object(),
                record,
            )
        self.assertIs(caught.exception, cancellation)
        stop.assert_called_once()
        self.assertEqual(record["error"], "original workload failure")
        self.assertEqual(
            [item["action"] for item in record["cleanup_errors"]],
            ["websocket_close", "stop_target_serve"],
        )
        self.assertEqual(record["cleanup_errors"][1]["errno"], errno.EIO)
        self.assertFalse(record["safe_to_continue"])

    async def test_lifetime_finally_saves_primary_and_close_cancel_after_stop(self):
        cancellation = asyncio.CancelledError("close canceled")
        client = raw_cdp.RawCdpClient(
            FakeWebSocket(
                [EXCEPTION_EVENT, EOFError("original receive failure")],
                close_error=cancellation,
            )
        )
        one_run = probe_function(
            "probe-layout-lifetime.py", "one_run", probe_namespace(client)
        )
        record, saved = {}, []
        with (
            patch.object(
                target_serve, "stop_target_serve", return_value={"marker": "stopped"}
            ) as stop,
            self.assertRaises(asyncio.CancelledError) as caught,
        ):
            await one_run(
                Path("/unused"),
                "Realtime",
                "unused",
                SimpleNamespace(cpu=None),
                record,
                lambda: saved.append(json.loads(json.dumps(record))),
            )
        self.assertIs(caught.exception, cancellation)
        stop.assert_called_once()
        self.assertEqual(len(saved), 1)
        self.assertIn("original receive failure", saved[0]["error"])
        self.assertEqual(saved[0]["runtime_errors"], [EXCEPTION_EVENT])
        self.assertEqual(saved[0]["cleanup_errors"][0]["type"], "CancelledError")
        self.assertEqual(saved[0]["stopped"]["marker"], "stopped")
        self.assertIn("elapsed_seconds", saved[0])
        self.assertFalse(saved[0]["safe_to_continue"])

    async def test_workload_cancellation_is_saved_and_propagates_from_both_probes(self):
        for filename in ("probe-realtime-layout.py", "probe-layout-lifetime.py"):
            with self.subTest(probe=filename):
                cancellation = asyncio.CancelledError("original receive canceled")
                client = raw_cdp.RawCdpClient(
                    FakeWebSocket([EXCEPTION_EVENT, cancellation])
                )
                one_run = probe_function(filename, "one_run", probe_namespace(client))
                record, saved = {}, []
                with (
                    patch.object(
                        target_serve, "stop_target_serve", return_value={}
                    ) as stop,
                    self.assertRaises(asyncio.CancelledError) as caught,
                ):
                    if "lifetime" in filename:
                        await one_run(
                            Path("/unused"),
                            "Realtime",
                            "unused",
                            SimpleNamespace(cpu=None),
                            record,
                            lambda record=record, saved=saved: saved.append(
                                dict(record)
                            ),
                        )
                    else:
                        await one_run(
                            Path("/unused"),
                            "Realtime",
                            "unused",
                            None,
                            1024,
                            1,
                            record=record,
                        )
                self.assertIs(caught.exception, cancellation)
                stop.assert_called_once()
                self.assertIn("original receive canceled", record["error"])
                self.assertEqual(record["runtime_errors"], [EXCEPTION_EVENT])
                self.assertEqual(
                    record["cdp_command_errors"][0]["error"]["type"], "CancelledError"
                )
                self.assertFalse(record["ok"])
                if "lifetime" in filename:
                    self.assertEqual(len(saved), 1)

    async def test_startup_cancellation_evidence_survives_before_handle_return(self):
        for filename in ("probe-realtime-layout.py", "probe-layout-lifetime.py"):
            with self.subTest(probe=filename):
                cancellation = asyncio.CancelledError("startup canceled")
                cancellation.evidence = {
                    "startup_stage": "start_sampler",
                    "marker": "cleaned",
                }
                namespace = probe_namespace(None)
                namespace["start_target_serve"] = Mock(side_effect=cancellation)
                one_run = probe_function(filename, "one_run", namespace)
                record, saved = {}, []
                with (
                    patch.object(target_serve, "stop_target_serve") as stop,
                    self.assertRaises(asyncio.CancelledError) as caught,
                ):
                    if "lifetime" in filename:
                        await one_run(
                            Path("/unused"),
                            "Realtime",
                            "unused",
                            SimpleNamespace(cpu=None),
                            record,
                            lambda record=record, saved=saved: saved.append(
                                dict(record)
                            ),
                        )
                    else:
                        await one_run(
                            Path("/unused"),
                            "Realtime",
                            "unused",
                            None,
                            1024,
                            1,
                            record=record,
                        )
                self.assertIs(caught.exception, cancellation)
                stop.assert_not_called()
                key = "stopped" if "lifetime" in filename else "server"
                self.assertEqual(record[key]["marker"], "cleaned")
                self.assertFalse(record["ok"])
                if "lifetime" in filename:
                    self.assertEqual(saved[0][key], cancellation.evidence)

    async def test_mode_main_saves_owned_sample_and_fixture_cleanup_on_close_cancel(
        self,
    ):
        cancellation = asyncio.CancelledError("close canceled")
        client = raw_cdp.RawCdpClient(
            FakeWebSocket(
                [EXCEPTION_EVENT, EOFError("original receive failure")],
                close_error=cancellation,
            )
        )
        namespace = probe_namespace(client)
        probe_function("probe-realtime-layout.py", "one_run", namespace)
        main = probe_function("probe-realtime-layout.py", "main", namespace)
        calls = []
        server = SimpleNamespace(
            server_port=1234,
            serve_forever=lambda: self.fail("must not launch an HTTP server"),
            shutdown=lambda: calls.append("fixture_shutdown"),
            server_close=lambda: calls.append("fixture_close"),
        )
        thread = SimpleNamespace(
            start=lambda: calls.append("fake_thread_start"),
            join=lambda timeout: calls.append("fixture_join"),
            is_alive=lambda: False,
        )
        namespace.update(
            __doc__="fake I/O validation of actual producer main",
            __file__=str(Path(__file__)),
            CONTAINERS=realtime_workload.CONTAINERS,
            argparse=argparse,
            hashlib=hashlib,
            json=json,
            Path=Path,
            http=SimpleNamespace(
                server=SimpleNamespace(
                    BaseHTTPRequestHandler=http.server.BaseHTTPRequestHandler,
                    ThreadingHTTPServer=lambda *_args: server,
                )
            ),
            threading=SimpleNamespace(Thread=lambda **_kwargs: thread),
            os=SimpleNamespace(
                environ={},
                sched_getaffinity=lambda _: {0},
                getloadavg=lambda: (0, 0, 0),
            ),
            platform=SimpleNamespace(platform=lambda: "fake platform"),
            fixture=lambda *_args, **_kwargs: b"fake HTML",
            instrumentation_manifest=lambda *_args: {},
            sys=SimpleNamespace(
                modules={
                    name: SimpleNamespace(__file__=str(Path(__file__)))
                    for name in (
                        "moli_benchmark.raw_cdp",
                        "moli_benchmark.realtime_workload",
                    )
                }
            ),
            stop_fixture_server=target_serve.stop_fixture_server,
        )
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            binary = directory / "fake-binary"
            binary.write_bytes(b"this file is never executed")
            output = directory / "raw.json"
            args = SimpleNamespace(
                baseline=binary,
                candidate=binary,
                baseline_sha="old",
                candidate_sha="new",
                output=output,
                runs=2,
                nodes=1024,
                container="blocks",
                positioned_blocks=False,
                idle_seconds=1,
                cpu=None,
            )
            with (
                patch.object(argparse.ArgumentParser, "parse_args", return_value=args),
                patch.object(
                    target_serve,
                    "stop_target_serve",
                    return_value={"marker": "stopped"},
                ) as stop,
                self.assertRaises(asyncio.CancelledError) as caught,
            ):
                await main()
            self.assertIs(caught.exception, cancellation)
            stop.assert_called_once()
            manifest = json.loads(output.read_text())
        self.assertEqual(len(manifest["samples"]), 1)
        sample = manifest["samples"][0]
        self.assertEqual(
            (sample["build"], sample["repetition"], sample["mode"]),
            ("baseline", 0, "Mock"),
        )
        self.assertIn("original receive failure", sample["error"])
        self.assertEqual(sample["runtime_errors"], [EXCEPTION_EVENT])
        self.assertEqual(sample["cleanup_errors"][0]["type"], "CancelledError")
        self.assertEqual(sample["server"]["marker"], "stopped")
        self.assertFalse(sample["safe_to_continue"])
        self.assertTrue(manifest["fixture_server_cleanup"]["complete"])
        self.assertEqual(
            calls,
            ["fake_thread_start", "fixture_shutdown", "fixture_close", "fixture_join"],
        )


if __name__ == "__main__":
    unittest.main()
