from __future__ import annotations

import argparse
import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from moli_benchmark.cli import (
    _amiibo_limit,
    _amiibo_modes,
    _amiibo_pools,
    _finish_report,
    _report_date_output_dir,
    _report_output_dir,
    _run_runs,
    _run_suites,
    _startup_idle_seconds,
    _startup_include_cdp_first_page,
    _startup_include_cdp_warm_pages,
    _startup_exit_code,
    _startup_runs,
    _startup_warm_pages,
    build_parser,
)
from moli_benchmark.config import FORMAL_RESULTS_ROOT
from moli_benchmark.startup import FORMAL_STARTUP_IDLE_SECONDS, FORMAL_STARTUP_RUNS


class CliAmiiboProfileTests(unittest.TestCase):
    def test_startup_formal_profile_defaults_to_full_workflow(self) -> None:
        args = argparse.Namespace(
            startup_profile="formal",
            runs=None,
            include_cdp_first_page=False,
            include_cdp_warm_pages=False,
            cdp_warm_pages=10,
            idle_seconds=None,
        )

        self.assertEqual(_startup_runs(args), FORMAL_STARTUP_RUNS)
        self.assertTrue(_startup_include_cdp_first_page(args))
        self.assertTrue(_startup_include_cdp_warm_pages(args))
        self.assertEqual(_startup_warm_pages(args), 10)
        self.assertEqual(_startup_idle_seconds(args), FORMAL_STARTUP_IDLE_SECONDS)

    def test_startup_explicit_values_override_formal_defaults(self) -> None:
        args = argparse.Namespace(
            startup_profile="formal",
            runs=3,
            include_cdp_first_page=False,
            include_cdp_warm_pages=False,
            cdp_warm_pages=4,
            idle_seconds=[5.0, 1.0, 5.0],
        )

        self.assertEqual(_startup_runs(args), 3)
        self.assertTrue(_startup_include_cdp_first_page(args))
        self.assertTrue(_startup_include_cdp_warm_pages(args))
        self.assertEqual(_startup_warm_pages(args), 4)
        self.assertEqual(_startup_idle_seconds(args), (5.0, 1.0))

    def test_startup_exit_uses_formal_gate_failures_only_for_formal_profile(self) -> None:
        self.assertEqual(_startup_exit_code({"profile": "smoke", "total_failures": 0, "gate_failures": 3}), 0)
        self.assertEqual(_startup_exit_code({"profile": "formal", "total_failures": 0, "gate_failures": 1}), 1)

    def test_smoke_profile_defaults_to_small_amiibo_workload(self) -> None:
        args = argparse.Namespace(amiibo_profile="smoke", pool=None, amiibo_mode=None, limit=None)

        self.assertEqual(_amiibo_pools(args), (1,))
        self.assertEqual(_amiibo_modes(args), ("session",))
        self.assertEqual(_amiibo_limit(args), 5)

    def test_formal_profile_defaults_to_full_amiibo_matrix(self) -> None:
        args = argparse.Namespace(amiibo_profile="formal", pool=None, amiibo_mode=None, limit=None)

        self.assertEqual(_amiibo_pools(args), (1, 2, 5, 10, 25, 100))
        self.assertEqual(_amiibo_modes(args), ("session", "process"))
        self.assertEqual(_amiibo_limit(args), 0)

    def test_explicit_amiibo_values_override_profile_defaults(self) -> None:
        args = argparse.Namespace(amiibo_profile="formal", pool=[5, 1, 5], amiibo_mode=["process"], limit=7)

        self.assertEqual(_amiibo_pools(args), (5, 1))
        self.assertEqual(_amiibo_modes(args), ("process",))
        self.assertEqual(_amiibo_limit(args), 7)

    def test_report_date_uses_formal_results_directory(self) -> None:
        self.assertEqual(_report_date_output_dir("2026-05-07"), FORMAL_RESULTS_ROOT / "2026-05-07")

    def test_report_date_rejects_non_iso_date(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "expected YYYY-MM-DD"):
            _report_date_output_dir("05-07-2026")
        with self.assertRaisesRegex(RuntimeError, "expected YYYY-MM-DD"):
            _report_date_output_dir("20260507")

    def test_report_output_dir_keeps_explicit_output_without_report_date(self) -> None:
        output_dir = Path("/tmp/moli-benchmark-smoke")
        args = argparse.Namespace(output_dir=output_dir, report_date=None)

        self.assertEqual(_report_output_dir(args), output_dir)

    def test_report_output_dir_prefers_report_date_for_formal_artifact(self) -> None:
        args = argparse.Namespace(output_dir=Path("/tmp/ignored"), report_date="2026-05-07")

        self.assertEqual(_report_output_dir(args), FORMAL_RESULTS_ROOT / "2026-05-07")

    def test_run_smoke_profile_keeps_existing_default_suites(self) -> None:
        args = argparse.Namespace(profile="smoke", suite=None, runs=None)

        self.assertEqual(_run_suites(args), ("startup", "synthetic"))
        self.assertEqual(_run_runs(args), 5)

    def test_run_horizontal_profile_defaults_to_compare_and_cdp_session(self) -> None:
        args = argparse.Namespace(profile="horizontal", suite=None, runs=None)

        self.assertEqual(_run_suites(args), ("synthetic-compare", "cdp-session"))
        self.assertEqual(_run_runs(args), 10)

    def test_run_explicit_suite_and_runs_override_profile_defaults(self) -> None:
        args = argparse.Namespace(profile="horizontal", suite=["synthetic-compare"], runs=3)

        self.assertEqual(_run_suites(args), ("synthetic-compare",))
        self.assertEqual(_run_runs(args), 3)

    def test_wpt_runner_arguments_are_nextest_only(self) -> None:
        parser = build_parser()

        args = parser.parse_args(["wpt", "--runner", "nextest", "--no-run"])
        self.assertEqual(args.runner, "nextest")

        run_args = parser.parse_args(["run", "--suite", "wpt", "--wpt-runner", "nextest", "--wpt-no-run"])
        self.assertEqual(run_args.wpt_runner, "nextest")

        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as wpt_error:
            parser.parse_args(["wpt", "--runner", "cargo", "--no-run"])
        self.assertEqual(wpt_error.exception.code, 2)

        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as run_error:
            parser.parse_args(["run", "--suite", "wpt", "--wpt-runner", "cargo", "--wpt-no-run"])
        self.assertEqual(run_error.exception.code, 2)

    def test_agent_episode_defaults_match_the_canonical_local_benchmark(self) -> None:
        args = build_parser().parse_args(["agent-episode"])

        self.assertIsNone(args.target)
        self.assertEqual(args.runs, 1)
        self.assertEqual(args.workers, 1)
        self.assertEqual(args.parallelism, 1)
        self.assertEqual(args.step_dwell_ms, 14_000)
        self.assertEqual(args.sample_interval_ms, 500)

    def test_agent_episode_has_no_stress_or_live_profile(self) -> None:
        parser = build_parser()
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(["agent-episode", "--profile", "stress"])
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(["agent-episode", "--live"])

    def test_fingerprint_is_an_explicit_native_observational_command(self) -> None:
        from moli_benchmark.fingerprint.cli import command

        args = build_parser().parse_args(["fingerprint", "--site", "creepjs", "--engine", "moli"])
        self.assertIs(args.func, command)
        self.assertEqual(args.site, ["creepjs"])
        self.assertEqual(args.engine, ["moli"])
        self.assertEqual(args.runs, 1)
        self.assertNotIn("fingerprint", _run_suites(argparse.Namespace(profile="smoke", suite=None)))
        for unsupported in ["--headless", "--disable-gpu", "--user-agent", "--retry"]:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                build_parser().parse_args(["fingerprint", unsupported])

    def test_finish_report_builds_readiness_after_top_level_artifacts_exist(self) -> None:
        summaries = [
            {
                "suite": "synthetic-matrix",
                "profile": "formal",
                "gate_failures": 0,
                "formal_gate_rows": [
                    {"gate": "profile", "ok": True},
                    {"gate": "runs", "ok": True},
                    {"gate": "repeats", "ok": True},
                    {"gate": "concurrency_levels", "ok": True},
                    {"gate": "cases", "ok": True},
                    {"gate": "workload_failures", "ok": True},
                    {"gate": "stability_failures", "ok": True},
                ],
                "targets": {"moli": {}, "lightpanda": {}, "chrome": {}, "obscura": {}},
            },
            {
                "suite": "startup",
                "profile": "formal",
                "total_failures": 0,
                "gate_failures": 0,
                "formal_gate_rows": [
                    {"gate": "profile", "ok": True},
                    {"gate": "runs", "ok": True},
                    {"gate": "cdp-first-page", "ok": True},
                    {"gate": "cdp-warm-pages", "ok": True},
                    {"gate": "idle-footprint", "ok": True},
                    {"gate": "workload-failures", "ok": True},
                ],
            },
            {
                "suite": "wpt",
                "cases": 1,
                "total_failures": 0,
                "summary": {"unexpected_fail": 0, "skip": 0, "known_fail": 0},
            },
            {
                "suite": "cdp-smoke",
                "profile": "formal",
                "total_failures": 0,
                "gate_failures": 0,
                "client_coverage": {"raw_cdp": True, "playwright": True, "puppeteer": True},
                "client_rows": [
                    {"client": "raw_cdp", "gate_ok": True},
                    {"client": "playwright", "gate_ok": True},
                    {"client": "puppeteer", "gate_ok": True},
                ],
            },
            {"suite": "amiibo-crawler", "profile": "formal", "gate_failures": 0},
            {
                "suite": "wild-web",
                "gate_failures": 0,
                "seeds": ["zhihu-home", "toutiao-home"],
                "targets": {"moli": {"extraction_failures": 0}},
            },
        ]
        target_matrix = {
            "moli": {"available": True},
            "lightpanda": {"available": True},
            "chrome": {"available": True},
            "obscura": {"available": True},
        }
        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            (output_dir / "environment.json").write_text("{}\n", encoding="utf-8")
            (output_dir / "versions.json").write_text("{}\n", encoding="utf-8")
            _finish_report(output_dir=output_dir, moli_bin=None, target_matrix=target_matrix, summaries=summaries)
            readiness = json.loads((output_dir / "publish-readiness.json").read_text(encoding="utf-8"))

        top_level = next(check for check in readiness["checks"] if check["name"] == "top-level artifacts")
        self.assertTrue(top_level["ok"])
        self.assertEqual(readiness["status"], "publishable")

    def test_finish_report_writes_top_level_diff_when_baseline_is_provided(self) -> None:
        target_matrix = {"moli": {"available": True}}
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            baseline_dir = root / "baseline"
            output_dir = root / "current"
            baseline_dir.mkdir()
            output_dir.mkdir()
            (output_dir / "environment.json").write_text("{}\n", encoding="utf-8")
            (output_dir / "versions.json").write_text("{}\n", encoding="utf-8")
            (baseline_dir / "summary.json").write_text(
                json.dumps({"suites": [{"suite": "startup", "total_failures": 2}]}) + "\n",
                encoding="utf-8",
            )

            _finish_report(
                output_dir=output_dir,
                moli_bin=None,
                target_matrix=target_matrix,
                summaries=[{"suite": "startup", "total_failures": 1}],
                baseline_report=baseline_dir,
            )

            diff = json.loads((output_dir / "report-diff.json").read_text(encoding="utf-8"))
            report_data = json.loads((output_dir / "report-data.json").read_text(encoding="utf-8"))
            summary_md = (output_dir / "summary.md").read_text(encoding="utf-8")
            index_html = (output_dir / "index.html").read_text(encoding="utf-8")
            diff_csv_exists = (output_dir / "report-diff.csv").exists()

        self.assertEqual(diff["summary"]["total_failures_delta"], -1)
        self.assertEqual(report_data["report_diff"]["summary"]["total_failures_delta"], -1)
        self.assertTrue(diff_csv_exists)
        self.assertIn("Previous report diff", summary_md)
        self.assertIn("Previous Report Diff", index_html)
        self.assertIn("cdn.jsdelivr.net/npm/chart.js", index_html)
        self.assertIn("report-data", index_html)


if __name__ == "__main__":
    unittest.main()
