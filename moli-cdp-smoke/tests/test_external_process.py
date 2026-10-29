from __future__ import annotations

import asyncio
import json
import os
import signal
import sys
import unittest

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.external_process import run_external_json_process
from moli_cdp_smoke.process import (
    process_exit_diagnostics,
    terminate_process_tree,
    wait_for_process_exit,
)


class ExternalProcessTests(unittest.IsolatedAsyncioTestCase):
    async def test_successful_json_process_extends_results(self) -> None:
        expected = {"name": "external-process-probe", "ok": True}
        payload = json.dumps({"ok": True, "results": [expected]})
        results: list[dict[str, object]] = []

        await run_external_json_process(
            "test",
            [sys.executable, "-c", f"print({payload!r})"],
            results,
            timeout_seconds=5,
        )

        self.assertEqual(results, [expected])

    @unittest.skipUnless(os.name == "posix", "process-group regression is POSIX-specific")
    async def test_timeout_kills_descendants_holding_output_pipes(self) -> None:
        descendant = "import time; time.sleep(4)"
        parent = (
            "import subprocess, sys, time; "
            f"subprocess.Popen([sys.executable, '-c', {descendant!r}], "
            "stdout=sys.stdout, stderr=sys.stderr); "
            "print('descendant-started', flush=True); "
            "time.sleep(30)"
        )
        loop = asyncio.get_running_loop()
        started_at = loop.time()

        with self.assertRaisesRegex(SmokeError, "test smoke timed out") as raised:
            await run_external_json_process(
                "test",
                [sys.executable, "-c", parent],
                [],
                timeout_seconds=1,
            )

        elapsed = loop.time() - started_at
        self.assertIn("descendant-started", str(raised.exception))
        self.assertLess(
            elapsed,
            2.5,
            "timeout cleanup waited for the descendant to close inherited pipes",
        )

    @unittest.skipUnless(os.name == "posix", "signal regression is POSIX-specific")
    async def test_service_cleanup_escalates_to_sigkill_with_a_bound(self) -> None:
        process = await asyncio.create_subprocess_exec(
            sys.executable,
            "-c",
            (
                "import signal, time; "
                "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                "print('ready', flush=True); "
                "time.sleep(30)"
            ),
            stdout=asyncio.subprocess.PIPE,
            start_new_session=True,
        )
        self.assertIsNotNone(process.stdout)
        self.assertEqual(await process.stdout.readline(), b"ready\n")

        stopped = await terminate_process_tree(
            process,
            terminate_timeout_seconds=0.05,
            kill_timeout_seconds=1,
        )

        self.assertTrue(stopped)
        self.assertEqual(process.returncode, -signal.SIGKILL)


class NaturalProcessExitTests(unittest.IsolatedAsyncioTestCase):
    @unittest.skipUnless(sys.platform.startswith("linux"), "diagnostic regression uses procfs")
    async def test_failed_exit_captures_owned_process_and_descendant_before_cleanup(self) -> None:
        descendant = "import time; print('ready', flush=True); time.sleep(30)"
        parent = (
            "import subprocess, sys; "
            f"child = subprocess.Popen([sys.executable, '-c', {descendant!r}]); "
            "child.wait()"
        )
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c", parent,
            stdout=asyncio.subprocess.PIPE,
            start_new_session=True,
        )
        try:
            self.assertEqual(await asyncio.wait_for(process.stdout.readline(), 5), b"ready\n")
            with self.assertRaises(TimeoutError):
                await wait_for_process_exit(process, timeout_seconds=0.02)

            diagnostics = process_exit_diagnostics(process)

            self.assertEqual(diagnostics["pid"], process.pid)
            self.assertIsNone(diagnostics["returncode"])
            self.assertIn("State", diagnostics["process"])
            self.assertTrue(diagnostics["threads"])
            self.assertEqual(diagnostics["threads"][0]["tid"], process.pid)
            self.assertEqual(len(diagnostics["descendants"]), 1)
            self.assertEqual(int(diagnostics["descendants"][0]["PPid"]), process.pid)
            self.assertIsNone(process.returncode, "diagnostics must not stop the process")
            json.dumps(diagnostics)
        finally:
            await terminate_process_tree(process)

    @unittest.skipUnless(sys.platform.startswith("linux"), "diagnostic regression uses procfs")
    async def test_diagnostics_preserve_a_reaped_process_status(self) -> None:
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c", "raise SystemExit(7)",
        )
        await wait_for_process_exit(process, timeout_seconds=5)

        diagnostics = process_exit_diagnostics(process)

        self.assertEqual(diagnostics["returncode"], 7)
        self.assertIn("FileNotFoundError", diagnostics["process"]["procfsError"])
        self.assertEqual(diagnostics["threads"], [])
        self.assertEqual(diagnostics["descendants"], [])

    @unittest.skipUnless(os.name == "posix", "inherited-pipe regression is POSIX-specific")
    async def test_parent_exit_does_not_wait_for_descendant_output_eof(self) -> None:
        descendant = "import time; time.sleep(30)"
        parent = (
            "import subprocess, sys; "
            f"subprocess.Popen([sys.executable, '-c', {descendant!r}]); "
            "print('ready', flush=True); sys.stdin.read(1)"
        )
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c", parent,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            start_new_session=True,
        )
        pipe_wait = None
        try:
            self.assertEqual(await asyncio.wait_for(process.stdout.readline(), 5), b"ready\n")
            pipe_wait = asyncio.create_task(process.wait())
            await asyncio.sleep(0)
            process.stdin.write(b"x")
            await process.stdin.drain()

            code = await wait_for_process_exit(process, timeout_seconds=5)

            self.assertEqual(code, 0)
            self.assertFalse(pipe_wait.done(), "descendant still owns stdout/stderr")
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            await asyncio.wait_for(pipe_wait if pipe_wait is not None else process.wait(), 5)

    async def test_exit_status_is_preserved(self) -> None:
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c", "raise SystemExit(7)",
        )

        self.assertEqual(await wait_for_process_exit(process, timeout_seconds=5), 7)

    async def test_deadline_does_not_terminate_a_live_process(self) -> None:
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c",
            "import time; print('ready', flush=True); time.sleep(30)",
            stdout=asyncio.subprocess.PIPE,
        )
        try:
            self.assertEqual(await asyncio.wait_for(process.stdout.readline(), 5), b"ready\n")
            with self.assertRaises(TimeoutError):
                await wait_for_process_exit(process, timeout_seconds=0.02)
            self.assertIsNone(process.returncode)
        finally:
            process.kill()
            await process.communicate()


if __name__ == "__main__":
    unittest.main()
