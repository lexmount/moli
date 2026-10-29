from __future__ import annotations

import asyncio
import os
import sys
import unittest


class WorkerStackTests(unittest.IsolatedAsyncioTestCase):
    async def test_watchdog_reports_a_blocked_main_thread_before_the_worker_deadline(self) -> None:
        script = """
import time
from moli_cdp_smoke import worker
def blocked():
    time.sleep(0.2)
worker.run = blocked
worker.main()
"""
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c", script,
            env={**os.environ, "MOLI_SMOKE_WORKER_TIMEOUT_SECONDS": "0.05"},
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            _, error = await asyncio.wait_for(process.communicate(), timeout=5)
            self.assertEqual(process.returncode, 0, error.decode())
            self.assertIn(b"Timeout", error)
            self.assertIn(b"in blocked", error)
            self.assertIn(b"worker.py", error)
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()
