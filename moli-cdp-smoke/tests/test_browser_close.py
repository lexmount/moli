from __future__ import annotations

import asyncio
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from moli_cdp_smoke.groups import browser_close, target_semantics
from moli_cdp_smoke.process import terminate_process_tree, wait_for_process_exit


class BrowserCloseDiagnosticsTests(unittest.IsolatedAsyncioTestCase):
    async def test_acknowledgment_without_exit_still_fails_and_preserves_pre_cleanup_state(self) -> None:
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c", "import time; print('ready', flush=True); time.sleep(30)",
            stdout=asyncio.subprocess.PIPE,
            start_new_session=True,
        )
        await asyncio.wait_for(process.stdout.readline(), 5)
        serving = SimpleNamespace(process=process, logs=["stderr: shutdown checkpoint"])
        socket = SimpleNamespace(
            state=SimpleNamespace(name="OPEN"), close_code=None, close_reason=None,
            close=AsyncMock(),
        )
        root = SimpleNamespace(websocket=socket)
        acknowledgment = {"id": 1000, "result": {}}
        identity = {"engine": "chromium"}

        async def bounded_exit(child, *, timeout_seconds):
            self.assertEqual(timeout_seconds, 10, "natural exit keeps the original deadline")
            return await wait_for_process_exit(child, timeout_seconds=0.02)

        async def stop(child):
            socket.close.assert_awaited_once()
            await terminate_process_tree(child.process)

        try:
            with (
                patch.object(browser_close, "browser_identity", AsyncMock(return_value=identity)),
                patch.object(browser_close, "start_browser", AsyncMock(return_value=serving)),
                patch.object(browser_close, "wait_for_browser_endpoint", AsyncMock(return_value="http://child")),
                patch.object(browser_close, "connect_raw_cdp", AsyncMock(return_value=root)),
                patch.object(browser_close, "_close_request", AsyncMock(return_value=acknowledgment)),
                patch.object(browser_close, "wait_for_process_exit", bounded_exit),
                patch.object(browser_close, "stop_browser", stop),
            ):
                with self.assertRaisesRegex(browser_close.BrowserCloseError, "root .*natural process exit.*pid=") as raised:
                    await browser_close.browser_close_instance_lifecycle("http://survivor", "http://fixture")

            observed = raised.exception.observed
            failed = observed["failedCase"]
            self.assertEqual(failed["closeResponse"], acknowledgment)
            self.assertEqual(failed["stage"], "natural process exit")
            self.assertEqual(failed["process"]["pid"], process.pid)
            self.assertIsNone(failed["process"]["returncode"], "snapshot precedes forced cleanup")
            self.assertEqual(failed["connections"][0]["state"], "OPEN")
            self.assertEqual(failed["browserLogTail"], serving.logs)
            self.assertEqual(observed["completedCases"], [])
            self.assertIsNotNone(process.returncode, "failed scenario still cleans up its child")
        finally:
            await terminate_process_tree(process)

    async def test_worker_result_retains_structured_failure_evidence(self) -> None:
        observed = {"completedCases": [{"case": "root", "exitCode": 0}],
                    "failedCase": {"case": "nested-browser-session", "stage": "natural process exit"}}
        error = browser_close.BrowserCloseError("natural process exit timed out", observed)
        scenario = AsyncMock(side_effect=error)
        contract = target_semantics.RawSemanticContract("close", "contract", "Chromium", ["Browser.close"], scenario)
        results = []

        with patch.object(target_semantics, "_raw_semantic_contracts", return_value=(contract,)):
            await target_semantics.run_target_semantics_group("http://child", "http://fixture", results)

        self.assertFalse(results[0]["ok"])
        self.assertEqual(results[0]["observed"], observed)
        self.assertEqual(results[0]["error"], str(error))


if __name__ == "__main__":
    unittest.main()
