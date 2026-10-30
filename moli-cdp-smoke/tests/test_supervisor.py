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
from unittest.mock import AsyncMock, patch

from moli_cdp_smoke.runner import GROUPS_BY_NAME
from moli_cdp_smoke.supervisor import (
    WorkerJob,
    WorkerOutcome,
    _run_worker_job,
    parse_args,
    run_supervisor,
)


class SupervisorWorkerTests(unittest.IsolatedAsyncioTestCase):
    async def test_failed_scenario_is_reported_when_worker_has_no_top_level_error(self) -> None:
        job = WorkerJob(group=GROUPS_BY_NAME["browser-semantics"], attempt=1, repeat=1)
        for worker_error in (None, "worker traceback takes precedence"):
            with self.subTest(worker_error=worker_error), tempfile.TemporaryDirectory() as temporary:
                payload = {
                    "ok": False,
                    "group": "browser-semantics",
                    "results": [{
                        "name": "cdp_contract_event_source_runtime_and_network",
                        "ok": False,
                        "errorType": "SmokeError",
                        "error": "timed out waiting for EventSource Network terminal",
                    }],
                }
                if worker_error is not None:
                    payload["error"] = worker_error

                def fake_argv(_: WorkerJob, result_path: Path) -> list[str]:
                    return [sys.executable, "-c", (
                        "from pathlib import Path; import sys; "
                        f"Path(sys.argv[1]).write_text({json.dumps(payload)!r}); "
                        "print('later scenario passed'); sys.exit(1)"
                    ), str(result_path)]

                with patch("moli_cdp_smoke.supervisor._worker_argv", fake_argv):
                    outcome = await _run_worker_job(job, Path(temporary), 5)
                self.assertEqual(outcome.status, "failed")
                if worker_error is not None:
                    self.assertEqual(outcome.error, worker_error)
                else:
                    self.assertIn("cdp_contract_event_source_runtime_and_network", outcome.error)
                    self.assertIn("SmokeError", outcome.error)
                    self.assertIn("timed out waiting for EventSource", outcome.error)

    async def test_worker_result_is_read_from_its_isolated_file(self) -> None:
        group = GROUPS_BY_NAME["protocol"]
        job = WorkerJob(group=group, attempt=1, repeat=1, browser="chromium", chromium_bin="requested-chrome")
        with tempfile.TemporaryDirectory() as temporary:
            output_dir = Path(temporary)

            def fake_argv(_: WorkerJob, result_path: Path) -> list[str]:
                payload = json.dumps(
                    {
                        "ok": True,
                        "group": "protocol",
                        "results": [{"name": "probe", "ok": True}],
                        "browser": {"engine": "chromium"},
                        "browserLaunches": [{"engine": "chromium", "browser": {"engine": "chromium"},
                                             "command": ["/resolved/oracle/chrome", "--headless=new"]}],
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
            self.assertEqual(outcome.summary()["chromiumBin"], "/resolved/oracle/chrome")

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


class SupervisorRetryTests(unittest.IsolatedAsyncioTestCase):
    async def test_failed_chromium_group_retries_in_fresh_workers_and_keeps_each_execution(self) -> None:
        for failures in (0, 1, 3, 4):
            with self.subTest(failures=failures), tempfile.TemporaryDirectory() as temporary:
                output = Path(temporary)
                counter = output / "counter"
                args = parse_args([
                    "--browser", "chromium", "--group", "protocol", "--retries", "3",
                    "--output-dir", temporary,
                ])

                def fake_argv(job: WorkerJob, result_path: Path) -> list[str]:
                    script = f"""
import json
import os
import sys
from pathlib import Path

counter = Path({str(counter)!r})
execution = int(counter.read_text()) + 1 if counter.exists() else 1
counter.write_text(str(execution))
passed = execution > {failures}
result = Path(sys.argv[1])
diagnostics = result.with_suffix('.diagnostics')
diagnostics.mkdir()
(diagnostics / 'checkpoint.json').write_text(json.dumps({{'execution': execution}}))
payload = {{
    'ok': passed,
    'group': {job.group.name!r},
    'browser': {{'engine': 'chromium', 'pid': os.getpid()}},
    'browserLaunches': [{{'engine': 'chromium', 'browser': {{'engine': 'chromium'}}}}],
    'results': [{{'name': 'probe', 'ok': passed}}],
}}
if not passed:
    payload['error'] = f'failure on execution {{execution}}'
result.write_text(json.dumps(payload))
print(f'execution={{execution}}')
sys.exit(0 if passed else 1)
"""
                    return [sys.executable, "-c", script, str(result_path)]

                stderr = io.StringIO()
                with patch("moli_cdp_smoke.supervisor._worker_argv", fake_argv), redirect_stderr(stderr):
                    exit_code, summary = await run_supervisor(args)

                executions = min(failures + 1, 4)
                passed = failures < 4
                flaky = 0 < failures < 4
                self.assertEqual(exit_code, 0 if passed else 1)
                self.assertEqual(summary["ok"], passed)
                self.assertEqual(summary["flakyGroups"], int(flaky))
                self.assertEqual(int(counter.read_text()), executions)
                group = summary["groups"][0]
                self.assertEqual(group["flaky"], flaky)
                self.assertEqual(group["retriesUsed"], executions - 1)
                attempts = group.get("executions", [group])
                self.assertEqual(len(attempts), executions)
                pids = []
                for index, attempt in enumerate(attempts, 1):
                    self.assertEqual(Path(attempt["log"]).read_text().strip(), f"execution={index}")
                    result = Path(attempt["result"])
                    payload = json.loads(result.read_text())
                    pids.append(payload["browser"]["pid"])
                    self.assertEqual(payload["ok"], index > failures)
                    self.assertEqual(
                        json.loads((result.with_suffix(".diagnostics") / "checkpoint.json").read_text()),
                        {"execution": index},
                    )
                    if index <= failures:
                        self.assertEqual(attempt["error"], f"failure on execution {index}")
                self.assertEqual(len(set(pids)), executions)
                self.assertEqual(group["status"], "passed" if passed else "failed")
                self.assertEqual("FLAKY protocol" in stderr.getvalue(), flaky)
                self.assertEqual(json.loads((output / "summary.json").read_text()), summary)

    async def test_retries_only_rerun_failed_group_and_keep_repetitions_independent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = parse_args([
                "--browser", "chromium", "--group", "protocol,network", "--repeat", "2",
                "--retries", "3", "--jobs", "2", "--output-dir", temporary,
            ])
            seen: dict[tuple[str, int], int] = {}

            async def worker(job: WorkerJob, output: Path, _: float) -> WorkerOutcome:
                key = (job.group.name, job.attempt)
                seen[key] = seen.get(key, 0) + 1
                passed = job.group.name == "network" or seen[key] > 1
                return WorkerOutcome(
                    job, "passed" if passed else "timed_out", 1, 0 if passed else 1,
                    output / "log", output / "result", 1,
                )

            with patch("moli_cdp_smoke.supervisor._run_worker_job", worker), redirect_stderr(io.StringIO()):
                code, summary = await run_supervisor(args)
            self.assertEqual(code, 0)
            self.assertEqual(seen, {("protocol", 1): 2, ("protocol", 2): 2,
                                    ("network", 1): 1, ("network", 2): 1})
            self.assertEqual(summary["flakyGroups"], 2)
            self.assertEqual([group["durationSeconds"] for group in summary["groups"]], [2, 2, 1, 1])

    async def test_retry_option_rejects_moli_and_shared_browsers(self) -> None:
        for options in ([], ["--browser", "moli"],
                        ["--browser", "chromium", "--endpoint", "http://localhost:9222"]):
            with self.subTest(options=options), self.assertRaisesRegex(RuntimeError, "managed --browser chromium"):
                await run_supervisor(parse_args([*options, "--retries", "3"]))
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parse_args(["--retries", "-1"])

    async def test_cancellation_never_starts_a_retry(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            args = parse_args([
                "--browser", "chromium", "--group", "protocol", "--retries", "3",
                "--output-dir", temporary,
            ])
            worker = AsyncMock(side_effect=asyncio.CancelledError)
            with patch("moli_cdp_smoke.supervisor._run_worker_job", worker), self.assertRaises(asyncio.CancelledError):
                await run_supervisor(args)
            self.assertEqual(worker.await_count, 1)


if __name__ == "__main__":
    unittest.main()
