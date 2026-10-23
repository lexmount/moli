from __future__ import annotations

import os
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from moli_benchmark.serve import _terminate_process


@unittest.skipUnless(os.name == "posix", "POSIX process groups")
class ServeProcessGroupTests(unittest.TestCase):
    def test_stopping_wrapper_terminates_its_browser_child(self) -> None:
        # Model /usr/bin/time -v, which wraps the measured browser process.
        with tempfile.TemporaryDirectory() as directory:
            child_pid_file = Path(directory) / "child.pid"
            child_code = (
                "import os,pathlib,signal,sys,time; "
                "signal.signal(signal.SIGTERM, signal.SIG_IGN); "
                "pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(60)"
            )
            wrapper_code = (
                "import subprocess,sys,pathlib; "
                "child=subprocess.Popen([sys.executable,'-c',sys.argv[2],sys.argv[1]]); "
                "child.wait()"
            )
            wrapper = subprocess.Popen(
                [sys.executable, "-c", wrapper_code, str(child_pid_file), child_code],
                start_new_session=True,
            )
            try:
                deadline = time.monotonic() + 5
                while not child_pid_file.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(child_pid_file.exists(), "wrapper did not start")
                child_pid = int(child_pid_file.read_text())
                self.assertTrue(_terminate_process(wrapper))
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline:
                    status = subprocess.run(
                        ["ps", "-o", "stat=", "-p", str(child_pid)],
                        capture_output=True, text=True, check=False,
                    ).stdout.strip()
                    if not status or status.startswith("Z"):
                        break
                    time.sleep(0.01)
                self.assertTrue(not status or status.startswith("Z"),
                                "benchmark browser survived its wrapper teardown")
            finally:
                try:
                    os.killpg(wrapper.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                wrapper.wait(timeout=5)

    def test_stopping_direct_process_remains_supported(self) -> None:
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"],
                                 start_new_session=True)
        try:
            self.assertTrue(_terminate_process(child))
            self.assertIsNotNone(child.poll())
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)
