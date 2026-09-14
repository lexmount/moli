from __future__ import annotations

import asyncio
import io
import json
import os
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

from moli_cdp_smoke.runner import GROUPS_BY_NAME
from moli_cdp_smoke.supervisor import (
    WorkerJob,
    WorkerOutcome,
    _run_worker_job,
    parse_args,
    run_supervisor,
)


class SupervisorWorkerTests(unittest.IsolatedAsyncioTestCase):
    async def test_worker_result_is_read_from_its_isolated_file(self) -> None:
        group = GROUPS_BY_NAME["protocol"]
        job = WorkerJob(group=group, attempt=1, repeat=1)
        with tempfile.TemporaryDirectory() as temporary:
            output_dir = Path(temporary)

            def fake_argv(_: WorkerJob, result_path: Path) -> list[str]:
                payload = json.dumps(
                    {
                        "ok": True,
                        "group": "protocol",
                        "results": [{"name": "probe", "ok": True}],
                    }
                )
                return [
                    sys.executable,
                    "-c",
                    (
                        "from pathlib import Path; import sys; "
                        f"Path(sys.argv[1]).write_text({payload!r}, encoding='utf-8'); "
                        "print('worker-log')"
                    ),
                    str(result_path),
                ]

            with patch("moli_cdp_smoke.supervisor._worker_argv", fake_argv):
                outcome = await _run_worker_job(job, output_dir, 5)

            self.assertTrue(outcome.passed)
            self.assertEqual(outcome.scenario_count, 1)
            self.assertEqual(outcome.log_path.read_text().strip(), "worker-log")

    async def test_worker_wall_clock_timeout_is_bounded(self) -> None:
        group = GROUPS_BY_NAME["protocol"]
        job = WorkerJob(group=group, attempt=1, repeat=1)
        with tempfile.TemporaryDirectory() as temporary:
            output_dir = Path(temporary)
            stale_result = output_dir / "protocol.json"
            stale_result.write_text(
                json.dumps(
                    {
                        "ok": True,
                        "group": "protocol",
                        "results": [{"name": "stale", "ok": True}],
                    }
                ),
                encoding="utf-8",
            )

            def hanging_argv(_: WorkerJob, __: Path) -> list[str]:
                return [sys.executable, "-c", "import time; time.sleep(30)"]

            loop = asyncio.get_running_loop()
            started_at = loop.time()
            with patch("moli_cdp_smoke.supervisor._worker_argv", hanging_argv):
                outcome = await _run_worker_job(job, output_dir, 0.05)

            self.assertEqual(outcome.status, "timed_out")
            self.assertEqual(outcome.scenario_count, 0)
            self.assertFalse(stale_result.exists())
            self.assertLess(loop.time() - started_at, 2)

    async def test_worker_timeout_preserves_streamed_moli_output(self) -> None:
        group = GROUPS_BY_NAME["protocol"]
        job = WorkerJob(group=group, attempt=1, repeat=1)
        with tempfile.TemporaryDirectory() as temporary:
            output_dir = Path(temporary)

            script = """
import os
import time

assert os.environ["MOLI_SMOKE_TRACE_BG"] == "1"
print("[moli serve stderr] renderer reached diagnostic checkpoint", flush=True)
time.sleep(30)
"""

            def hanging_argv(_: WorkerJob, __: Path) -> list[str]:
                return [sys.executable, "-c", script]

            with patch("moli_cdp_smoke.supervisor._worker_argv", hanging_argv):
                outcome = await _run_worker_job(job, output_dir, 0.05)

            self.assertEqual(outcome.status, "timed_out")
            self.assertIn(
                "[moli serve stderr] renderer reached diagnostic checkpoint",
                outcome.log_path.read_text(encoding="utf-8"),
            )

    @unittest.skipUnless(
        sys.platform.startswith("linux"),
        "descendant process-state regression uses Linux /proc",
    )
    async def test_worker_timeout_signals_descendants_in_its_process_group(self) -> None:
        group = GROUPS_BY_NAME["protocol"]
        job = WorkerJob(group=group, attempt=1, repeat=1)
        with tempfile.TemporaryDirectory() as temporary:
            output_dir = Path(temporary)
            child_ready = output_dir / "child-ready"
            child = f"""
import os
import time
from pathlib import Path

Path({str(child_ready)!r}).write_text(str(os.getpid()), encoding="utf-8")
while True:
    time.sleep(1)
"""
            parent = f"""
import subprocess
import sys
import time
from pathlib import Path

subprocess.Popen([sys.executable, "-c", {child!r}])
ready = Path({str(child_ready)!r})
while not ready.exists():
    time.sleep(0.01)
time.sleep(30)
"""

            def descendant_argv(_: WorkerJob, __: Path) -> list[str]:
                return [sys.executable, "-c", parent]

            with patch("moli_cdp_smoke.supervisor._worker_argv", descendant_argv):
                outcome = await _run_worker_job(job, output_dir, 0.5)

            self.assertEqual(outcome.status, "timed_out")
            self.assertTrue(child_ready.exists())
            child_pid = int(child_ready.read_text(encoding="utf-8"))
            process_state = Path(f"/proc/{child_pid}/stat")
            for _ in range(100):
                if not process_state.exists():
                    break
                if process_state.read_text(encoding="utf-8").split()[2] == "Z":
                    break
                await asyncio.sleep(0.01)
            else:
                self.fail(f"worker descendant {child_pid} survived process-group cleanup")


class SupervisorSchedulingTests(unittest.IsolatedAsyncioTestCase):
    async def test_worker_error_is_reported_even_when_absent_from_log(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = parse_args(["--group", "protocol", "--output-dir", temporary])
            error = (
                "CDPSession.send: Protocol error (Emulation.setCPUThrottlingRate): "
                "CPU throttling is not supported"
            )

            async def failed_worker(
                job: WorkerJob, output_dir: Path, _: float
            ) -> WorkerOutcome:
                log_path = output_dir / f"{job.file_stem}.log"
                log_path.write_text("group failed: Error\n", encoding="utf-8")
                return WorkerOutcome(
                    job=job,
                    status="failed",
                    duration_seconds=0,
                    exit_code=1,
                    log_path=log_path,
                    result_path=output_dir / f"{job.file_stem}.json",
                    scenario_count=0,
                    error=error,
                )

            stderr = io.StringIO()
            with (
                patch("moli_cdp_smoke.supervisor._run_worker_job", failed_worker),
                redirect_stderr(stderr),
            ):
                exit_code, summary = await run_supervisor(args)

            self.assertEqual(exit_code, 1)
            self.assertFalse(summary["ok"])
            self.assertIn(error, stderr.getvalue())
            self.assertIn("group failed: Error", stderr.getvalue())

    async def test_jobs_bound_allows_isolated_workers_to_run_in_parallel(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = parse_args(
                [
                    "--group",
                    "protocol",
                    "--group",
                    "puppeteer",
                    "--jobs",
                    "2",
                    "--output-dir",
                    temporary,
                ]
            )
            both_started = asyncio.Event()
            active = 0
            maximum_active = 0

            async def fake_run(
                job: WorkerJob,
                output_dir: Path,
                _: float,
            ) -> WorkerOutcome:
                nonlocal active, maximum_active
                active += 1
                maximum_active = max(maximum_active, active)
                if active == 2:
                    both_started.set()
                await asyncio.wait_for(both_started.wait(), timeout=1)
                active -= 1
                return WorkerOutcome(
                    job=job,
                    status="passed",
                    duration_seconds=0,
                    exit_code=0,
                    log_path=output_dir / f"{job.file_stem}.log",
                    result_path=output_dir / f"{job.file_stem}.json",
                    scenario_count=1,
                )

            with patch("moli_cdp_smoke.supervisor._run_worker_job", fake_run):
                exit_code, summary = await run_supervisor(args)

            self.assertEqual(exit_code, 0)
            self.assertTrue(summary["ok"])
            self.assertEqual(maximum_active, 2)

    async def test_fixed_port_is_rejected_for_parallel_isolated_workers(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = parse_args(
                [
                    "--group",
                    "protocol",
                    "--group",
                    "puppeteer",
                    "--jobs",
                    "2",
                    "--output-dir",
                    temporary,
                ]
            )

            with patch.dict(os.environ, {"MOLI_CDP_PORT": "9333"}), self.assertRaisesRegex(
                RuntimeError, "cannot be shared"
            ):
                await run_supervisor(args)


if __name__ == "__main__":
    unittest.main()
