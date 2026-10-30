from __future__ import annotations

import io
import json
import os
import tempfile
import unittest
from contextlib import redirect_stderr
from copy import deepcopy
from pathlib import Path
from unittest.mock import AsyncMock, patch

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.config import moli_binary
from moli_cdp_smoke.helpers import read_io_to_eof
from moli_cdp_smoke.runner import GROUPS_BY_NAME, resolve_group_selection
from moli_cdp_smoke.serve import _endpoint_from_log_line, browser_identity, start_moli_serve
from moli_cdp_smoke.supervisor import WorkerJob, WorkerOutcome, _coverage_error, _worker_argv, parse_args, run_supervisor


class BrowserCoverageTests(unittest.IsolatedAsyncioTestCase):
    def test_group_binary_overrides_are_repeatable_and_preserve_literal_paths(self) -> None:
        args = parse_args([
            "--browser", "chromium", "--chromium-bin-for-group", "dom-input=/tmp/browser with spaces/chrome",
            "--chromium-bin-for-group", "network=/tmp/browser=other/chrome",
        ])
        self.assertEqual(args.chromium_bin_for_group, {
            "dom-input": "/tmp/browser with spaces/chrome", "network": "/tmp/browser=other/chrome",
        })

    def test_group_binary_overrides_reject_invalid_and_ambiguous_configuration(self) -> None:
        for entries, message in (
            (["dom-input"], "requires GROUP=PATH"),
            (["dom-input="], "nonempty path"),
            (["dom-input=   "], "nonempty path"),
            (["=/tmp/chrome"], "requires GROUP=PATH"),
            (["unknown-group=/tmp/chrome"], "unknown group"),
            (["dom-input=/tmp/old", "dom-input=/tmp/new"], "duplicate group"),
        ):
            options = [value for entry in entries for value in ("--chromium-bin-for-group", entry)]
            stderr = io.StringIO()
            with self.subTest(entries=entries), redirect_stderr(stderr), self.assertRaises(SystemExit) as error:
                parse_args(["--browser", "chromium", *options])
            self.assertEqual(error.exception.code, 2)
            self.assertIn(message, stderr.getvalue())
        for options in ([], ["--browser", "moli"],
                        ["--browser", "chromium", "--endpoint", "http://localhost:9222"]):
            stderr = io.StringIO()
            with self.subTest(options=options), redirect_stderr(stderr), self.assertRaises(SystemExit):
                parse_args([*options, "--chromium-bin-for-group", "dom-input=/tmp/chrome"])
            self.assertIn("requires managed --browser chromium", stderr.getvalue())

    async def test_group_binary_override_preserves_every_strict_full_suite_group(self) -> None:
        with (tempfile.TemporaryDirectory() as directory,
              patch.dict(os.environ, {name: "" for name in (
                  "MOLI_SMOKE_GROUPS", "MOLI_MULTI_PAGE_CASES", "MOLI_MULTI_PAGE_SHARD",
              )})):
            args = parse_args([
                "--browser", "chromium", "--strict-full-suite", "--jobs", "4",
                "--chromium-bin", "/tmp/legacy/chrome",
                "--chromium-bin-for-group", "dom-input=/tmp/current browser/chrome",
                "--output-dir", directory,
            ])
            seen = []

            async def worker(job, output, timeout):
                seen.append((job.group.name, job.chromium_bin))
                argv = _worker_argv(job, output / "result")
                self.assertEqual(argv[argv.index("--chromium-bin") + 1], job.chromium_bin)
                return WorkerOutcome(job, "passed", 0, 0, output / "log", output / "result", 1)

            with patch("moli_cdp_smoke.supervisor._run_worker_job", worker), redirect_stderr(io.StringIO()):
                code, summary = await run_supervisor(args)
            expected = [(group.name, "/tmp/current browser/chrome" if group.name == "dom-input"
                         else "/tmp/legacy/chrome") for group in resolve_group_selection().groups]
            self.assertEqual(code, 0)
            self.assertTrue(summary["strictFullSuite"])
            self.assertEqual(seen, expected)
            self.assertEqual([(row["group"], row["chromiumBin"]) for row in summary["groups"]], expected)

    async def test_strict_full_suite_rejects_every_partial_execution_path(self) -> None:
        for options in (
            ["--group", "network"], ["--endpoint", "http://127.0.0.1:9222"],
            ["--fail-fast"], ["--list-groups"],
        ):
            with self.subTest(options=options):
                args = parse_args(["--strict-full-suite", *options])
                with self.assertRaisesRegex(RuntimeError, "strict full-suite mode forbids"):
                    await run_supervisor(args)
        for variable in ("MOLI_SMOKE_GROUPS", "MOLI_MULTI_PAGE_CASES", "MOLI_MULTI_PAGE_SHARD"):
            with self.subTest(variable=variable), patch.dict(os.environ, {variable: "filtered"}):
                with self.assertRaisesRegex(RuntimeError, variable):
                    await run_supervisor(parse_args(["--strict-full-suite"]))

    async def test_every_repetition_runs_and_one_failure_cannot_become_a_retry_success(self) -> None:
        for engine in ("moli", "chromium"):
            for failure in ("failed", "timed_out", "crashed", "skipped"):
                with (self.subTest(engine=engine, failure=failure),
                      tempfile.TemporaryDirectory() as directory,
                      patch.dict(os.environ, {name: "" for name in (
                          "MOLI_SMOKE_GROUPS", "MOLI_MULTI_PAGE_CASES", "MOLI_MULTI_PAGE_SHARD",
                      )})):
                    expected = [(group.name, attempt)
                                for group in resolve_group_selection().groups
                                for attempt in (1, 2)]
                    seen = []

                    async def worker(job, output, timeout):
                        seen.append((job.group.name, job.attempt))
                        status = failure if len(seen) == 1 else "passed"
                        return WorkerOutcome(job, status, 0, 1 if status != "passed" else 0,
                                             output / "log", output / "result", 1)

                    args = parse_args(["--browser", engine, "--strict-full-suite", "--repeat", "2",
                                       "--jobs", "1", "--output-dir", directory])
                    with patch("moli_cdp_smoke.supervisor._run_worker_job", worker):
                        code, summary = await run_supervisor(args)
                    self.assertEqual(code, 1)
                    self.assertFalse(summary["ok"])
                    self.assertTrue(summary["strictFullSuite"])
                    self.assertEqual(seen, expected)
                    self.assertEqual(summary["groups"][0]["status"], failure)
                    self.assertEqual(summary["groups"][1]["status"], "passed")

    async def test_both_engines_schedule_every_default_group(self) -> None:
        schedules = {}
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {"MOLI_SMOKE_GROUPS": ""}):
            for engine in ("moli", "chromium"):
                seen = []

                async def worker(job, output, timeout):
                    self.assertEqual(job.browser, engine)
                    seen.append(job.group.name)
                    return WorkerOutcome(job, "passed", 0, 0, output / "log", output / "result", 1)

                args = parse_args(["--browser", engine, "--output-dir", directory])
                with patch("moli_cdp_smoke.supervisor._run_worker_job", worker):
                    code, _ = await run_supervisor(args)
                self.assertEqual(code, 0)
                schedules[engine] = seen
            self.assertEqual(schedules["chromium"], [g.name for g in resolve_group_selection().groups])
            self.assertEqual(schedules["chromium"], schedules["moli"])

    async def test_chromium_worker_cannot_launch_moli_or_invoke_its_cli(self) -> None:
        with patch.dict(os.environ, {"MOLI_SMOKE_BROWSER": "chromium"}):
            with self.assertRaisesRegex(RuntimeError, "must not invoke"):
                moli_binary()
            with self.assertRaisesRegex(RuntimeError, "must not launch"):
                await start_moli_serve()

    async def test_endpoint_identity_rejects_wrong_engine_and_non_chromium(self) -> None:
        for version, expected, message in [
            ({"Browser": "Chrome/145", "webSocketDebuggerUrl": "ws://localhost/devtools/browser/moli-browser"}, "chromium", "belongs to moli"),
            ({"Browser": "Firefox/145", "webSocketDebuggerUrl": "ws://localhost/devtools/browser/123"}, "chromium", "not a Chromium"),
        ]:
            with self.subTest(version=version), patch("urllib.request.urlopen", return_value=io.BytesIO(json.dumps(version).encode())):
                with self.assertRaisesRegex(RuntimeError, message):
                    await browser_identity("http://localhost", expected)

    def test_no_assertions_no_skip_and_no_mixed_engine_can_pass(self) -> None:
        job = WorkerJob(GROUPS_BY_NAME["network"], 1, 1, browser="chromium")
        valid = {
            "results": [{"name": "body", "ok": True}],
            "browser": {"engine": "chromium"},
            "browserLaunches": [{"engine": "chromium", "browser": {"engine": "chromium"}}],
        }
        self.assertIsNone(_coverage_error(job, valid))
        invalid = [
            {"results": []},
            {"results": [{"name": "body", "ok": False}]},
            {"results": [{"name": "body_not_applicable", "ok": True}]},
            {"results": [{"name": "body", "ok": True, "applicable": False}]},
            {"browser": {"engine": "moli"}},
            {"browserLaunches": []},
            {"browserLaunches": [{"engine": "chromium"}]},
            {"browserLaunches": [{"engine": "chromium"}, {"engine": "moli"}]},
        ]
        for changes in invalid:
            with self.subTest(changes=changes):
                payload = deepcopy(valid)
                payload.update(changes)
                self.assertIsNotNone(_coverage_error(job, payload))

    def test_engine_and_binary_are_forwarded_without_shell_interpolation(self) -> None:
        binary = "/tmp/browser with spaces/chrome"
        job = WorkerJob(GROUPS_BY_NAME["network"], 1, 1, browser="chromium", chromium_bin=binary)
        argv = _worker_argv(job, Path("/tmp/result.json"))
        self.assertEqual(argv[argv.index("--browser") + 1], "chromium")
        self.assertEqual(argv[argv.index("--chromium-bin") + 1], binary)

    def test_chromium_endpoint_requires_a_valid_bound_loopback_port(self) -> None:
        self.assertEqual(_endpoint_from_log_line("DevTools listening on ws://127.0.0.1:43210/devtools/browser/123"), "http://127.0.0.1:43210")
        for port in (0, 65536):
            self.assertIsNone(_endpoint_from_log_line(f"DevTools listening on ws://127.0.0.1:{port}/devtools/browser/123"))

    async def test_sequential_stream_consumes_the_explicit_eof_marker(self) -> None:
        client = AsyncMock()
        client.send.side_effect = [
            {"data": "response", "base64Encoded": False, "eof": False},
            {"data": "-stage body", "base64Encoded": False, "eof": False},
            {"data": "", "eof": True},
        ]
        self.assertEqual(await read_io_to_eof(client, "1"), {"data": "response-stage body", "base64Encoded": False, "eof": True})
        self.assertEqual(client.send.await_count, 3)
        for call in client.send.await_args_list:
            self.assertEqual(call.args, ("IO.read", {"handle": "1"}))

    async def test_binary_stream_decodes_chunks_before_concatenating(self) -> None:
        client = AsyncMock()
        client.send.side_effect = [
            {"data": "AP8=", "base64Encoded": True, "eof": False},
            {"data": "YQ==", "base64Encoded": True, "eof": False},
            {"data": "", "eof": True},
        ]
        self.assertEqual(await read_io_to_eof(client, "1"), {"data": "AP9h", "base64Encoded": True, "eof": True})

    async def test_malformed_stream_response_fails(self) -> None:
        client = AsyncMock()
        client.send.return_value = {"data": "body"}
        with self.assertRaises(SmokeError):
            await read_io_to_eof(client, "1")
