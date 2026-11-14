from __future__ import annotations

import asyncio
import tempfile
import unittest
from pathlib import Path
from typing import cast
from unittest.mock import AsyncMock, patch

from moli_cdp_smoke.serve import (
    BrowserProcess,
    _collect_process_output,
    _endpoint_from_log_line,
    _wait_for_chromium_port_file,
    wait_for_browser_endpoint,
)


class _FakeProcess:
    def __init__(self, returncode: int | None = None) -> None:
        self.pid = 999_999_999
        self.returncode = returncode
        self._never_exits = asyncio.Event()

    async def wait(self) -> int:
        if self.returncode is None:
            await self._never_exits.wait()
        assert self.returncode is not None
        return self.returncode


class ServeLogTests(unittest.IsolatedAsyncioTestCase):
    async def test_chromium_discovers_and_verifies_its_port_without_a_log(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "DevToolsActivePort").write_text(
                "42123\n/devtools/browser/browser-id", encoding="utf-8",
            )
            serve = BrowserProcess(
                process=cast(asyncio.subprocess.Process, _FakeProcess()), logs=[], tasks=[],
                data_dir=directory, endpoint_ready=asyncio.get_running_loop().create_future(),
                engine="chromium",
            )
            identity = {"engine": "chromium", "version": {"Browser": "Chrome/156"}}
            launches = [{"pid": serve.process.pid, "engine": "chromium"}]
            with (patch("moli_cdp_smoke.serve.wait_for_cdp_server", new_callable=AsyncMock) as probe,
                  patch("moli_cdp_smoke.serve.browser_identity", new_callable=AsyncMock,
                        return_value=identity) as discover,
                  patch("moli_cdp_smoke.serve.BROWSER_LAUNCHES", launches)):
                endpoint = await wait_for_browser_endpoint(serve, timeout_seconds=1)
            self.assertEqual(endpoint, "http://127.0.0.1:42123")
            probe.assert_awaited_once_with(endpoint, serve)
            discover.assert_awaited_once_with(endpoint, "chromium")
            self.assertEqual(launches[0]["browser"], identity)
            self.assertEqual(launches[0]["endpointDiscovery"], "DevToolsActivePort")
            self.assertFalse(serve.endpoint_ready.done())

    async def test_chromium_port_file_waits_for_complete_publication(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "DevToolsActivePort"
            path.write_text("42123\n", encoding="utf-8")
            task = asyncio.create_task(_wait_for_chromium_port_file(directory))
            try:
                await asyncio.sleep(0)
                self.assertFalse(task.done())
                path.write_text("42123\n/devtools/browser/browser-id", encoding="utf-8")
                self.assertEqual(await asyncio.wait_for(task, 1), "http://127.0.0.1:42123")
            finally:
                task.cancel()
                await asyncio.gather(task, return_exceptions=True)

    async def test_invalid_chromium_port_files_remain_failures(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            for contents in ["0\n/devtools/browser/id", "65536\n/devtools/browser/id",
                             "-1\n/devtools/browser/id", "42123\nhttp://example.com/id"]:
                with self.subTest(contents=contents):
                    (Path(directory) / "DevToolsActivePort").write_text(contents, encoding="utf-8")
                    with self.assertRaisesRegex(RuntimeError, "invalid Chromium DevToolsActivePort"):
                        await asyncio.wait_for(_wait_for_chromium_port_file(directory), 1)

    async def test_failed_startup_preserves_process_and_port_file_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            serve = BrowserProcess(
                process=cast(asyncio.subprocess.Process, _FakeProcess()), logs=[], tasks=[],
                data_dir=directory, endpoint_ready=asyncio.get_running_loop().create_future(),
                engine="chromium",
            )
            launches = [{"pid": serve.process.pid, "engine": "chromium"}]
            with (patch("moli_cdp_smoke.serve.process_exit_diagnostics",
                        return_value={"pid": serve.process.pid, "returncode": None}),
                  patch("moli_cdp_smoke.serve.BROWSER_LAUNCHES", launches)):
                with self.assertRaisesRegex(RuntimeError, "bound endpoint"):
                    await wait_for_browser_endpoint(serve, timeout_seconds=0.01)
            snapshot = launches[0]["startupFailure"]
            self.assertIsNone(snapshot["returncode"])
            self.assertIn("FileNotFoundError", snapshot["devToolsActivePortError"])

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
