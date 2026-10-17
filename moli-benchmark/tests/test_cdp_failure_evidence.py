from __future__ import annotations

import asyncio
import errno
import json
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

from moli_benchmark import raw_cdp, target_serve

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


if __name__ == "__main__":
    unittest.main()
