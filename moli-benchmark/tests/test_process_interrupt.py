from __future__ import annotations

import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from moli_benchmark.process import run_process


@unittest.skipUnless(os.name == "posix", "POSIX SIGINT")
class ProcessInterruptTests(unittest.TestCase):
    def test_interrupt_reaps_managed_child_before_propagating(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pidfile = root / "child.pid"
            child = (
                "import os,pathlib,sys,time; "
                "pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(60)"
            )
            driver = (
                "import sys; from pathlib import Path; "
                "from moli_benchmark.process import run_process\n"
                "try:\n"
                " run_process([sys.executable,'-c',sys.argv[2],sys.argv[1]], "
                "cwd=Path.cwd(), timeout_seconds=30, sample_resources=False)\n"
                "except KeyboardInterrupt:\n sys.exit(17)\n"
            )
            env = dict(os.environ, PYTHONPATH=str(Path(__file__).resolve().parents[1]))
            parent = subprocess.Popen(
                [sys.executable, "-c", driver, str(pidfile), child],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            managed_pid = None
            try:
                deadline = time.monotonic() + 5
                while not pidfile.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(pidfile.exists(), "managed child did not start")
                managed_pid = int(pidfile.read_text())
                parent.send_signal(signal.SIGINT)
                stdout, stderr = parent.communicate(timeout=5)
                self.assertEqual(parent.returncode, 17, (stdout, stderr))
                status = subprocess.run(
                    ["ps", "-o", "stat=", "-p", str(managed_pid)],
                    capture_output=True,
                    text=True,
                    check=False,
                ).stdout.strip()
                self.assertTrue(
                    not status or status.startswith("Z"),
                    "interrupted benchmark left its managed process running",
                )
            finally:
                if managed_pid is not None:
                    try:
                        os.killpg(managed_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if parent.poll() is None:
                    parent.kill()
                parent.communicate(timeout=5)

    def test_successful_command_retains_output(self) -> None:
        result = run_process(
            [sys.executable, "-c", "print('control')"],
            cwd=Path.cwd(),
            timeout_seconds=5,
            sample_resources=False,
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"control\n")
        self.assertFalse(result.timed_out)
