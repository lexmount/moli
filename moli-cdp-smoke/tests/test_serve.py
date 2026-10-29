from __future__ import annotations

import asyncio
import unittest
from typing import cast

from moli_cdp_smoke.serve import (
    BrowserProcess,
    _collect_process_output,
    _endpoint_from_log_line,
    wait_for_browser_endpoint,
)


class _FakeProcess:
    def __init__(self, returncode: int | None = None) -> None:
        self.returncode = returncode
        self._never_exits = asyncio.Event()

    async def wait(self) -> int:
        if self.returncode is None:
            await self._never_exits.wait()
        assert self.returncode is not None
        return self.returncode


class ServeLogTests(unittest.IsolatedAsyncioTestCase):
    def test_bound_endpoint_is_parsed_from_plain_and_ansi_logs(self) -> None:
        plain = "INFO protocol server listening addr=127.0.0.1:41827"
        ansi = (
            "\x1b[2m2026-08-24T01:02:03Z\x1b[0m "
            "\x1b[32m INFO\x1b[0m protocol server listening "
            "\x1b[3maddr\x1b[0m\x1b[2m=\x1b[0m127.0.0.1:53109"
        )

        self.assertEqual(
            _endpoint_from_log_line(plain), "http://127.0.0.1:41827"
        )
        self.assertEqual(
            _endpoint_from_log_line(ansi), "http://127.0.0.1:53109"
        )
        self.assertIsNone(_endpoint_from_log_line("unrelated addr=127.0.0.1:9"))
        self.assertIsNone(
            _endpoint_from_log_line(
                "protocol server listening addr=127.0.0.1:99999"
            )
        )

    async def test_output_collector_publishes_the_bound_endpoint(self) -> None:
        stream = asyncio.StreamReader()
        stream.feed_data(
            b"INFO protocol server listening addr=127.0.0.1:42123\n"
        )
        stream.feed_eof()
        logs: list[str] = []
        endpoint_ready: asyncio.Future[str] = (
            asyncio.get_running_loop().create_future()
        )

        await _collect_process_output(stream, logs, "stderr", endpoint_ready)

        self.assertEqual(endpoint_ready.result(), "http://127.0.0.1:42123")
        self.assertEqual(len(logs), 1)

    async def test_wait_reports_process_exit_before_the_listening_log(self) -> None:
        endpoint_ready: asyncio.Future[str] = (
            asyncio.get_running_loop().create_future()
        )
        serve = BrowserProcess(
            process=cast(asyncio.subprocess.Process, _FakeProcess(7)),
            logs=["stderr: startup failed"],
            tasks=[],
            data_dir="",
            endpoint_ready=endpoint_ready,
        )

        with self.assertRaisesRegex(RuntimeError, "exited early with 7"):
            await wait_for_browser_endpoint(serve, timeout_seconds=1)

    async def test_wait_for_listening_log_has_a_bounded_timeout(self) -> None:
        endpoint_ready: asyncio.Future[str] = (
            asyncio.get_running_loop().create_future()
        )
        serve = BrowserProcess(
            process=cast(asyncio.subprocess.Process, _FakeProcess()),
            logs=[],
            tasks=[],
            data_dir="",
            endpoint_ready=endpoint_ready,
        )

        with self.assertRaisesRegex(RuntimeError, "bound endpoint"):
            await wait_for_browser_endpoint(serve, timeout_seconds=0.01)


if __name__ == "__main__":
    unittest.main()
