from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.cdp_smoke import (
    _discover_group_listing,
    _effective_cdp_smoke_groups,
    run_cdp_smoke_suite,
)
from moli_benchmark.process import ProcessResult


GROUP_LISTING = [
    {"name": "protocol", "phase": "raw", "default": True},
    {"name": "target-lifecycle", "phase": "process", "default": True},
    {"name": "core", "phase": "page", "default": True},
    {"name": "network", "phase": "page", "default": True},
    {"name": "emulation-storage", "phase": "browser", "default": True},
    {"name": "puppeteer", "phase": "external", "default": False},
]
PREFLIGHT_OK = {
    "node": {"executable": "node", "available": True},
    "puppeteer_core": {"module": "puppeteer-core", "available": True, "error": None},
}


class CdpSmokeTests(unittest.TestCase):
    def test_group_listing_comes_from_smoke_runner_json(self) -> None:
        result = ProcessResult(
            command=["uv", "run", "moli-cdp-smoke", "--list-groups"],
            returncode=0,
            elapsed_ms=10.0,
            stdout=json.dumps({"groups": GROUP_LISTING}).encode(),
            stderr=b"",
            timed_out=False,
            resources={},
        )
        commands: list[list[str]] = []

        def run_process(command: list[str], **_: object) -> ProcessResult:
            commands.append(command)
            return result

        with patch("moli_benchmark.cdp_smoke.run_process", run_process):
            groups = _discover_group_listing(["uv", "run", "moli-cdp-smoke"], 30, {})

        self.assertEqual(groups, GROUP_LISTING)
        self.assertEqual(commands[0], ["uv", "run", "moli-cdp-smoke", "--list-groups"])

    def test_formal_profile_selects_ecosystem_groups(self) -> None:
        self.assertEqual(_effective_cdp_smoke_groups("smoke", (), GROUP_LISTING), ("protocol", "target-lifecycle", "core", "network", "emulation-storage"))
        self.assertEqual(_effective_cdp_smoke_groups("formal", (), GROUP_LISTING), ("protocol", "target-lifecycle", "core", "network", "emulation-storage", "puppeteer"))
        self.assertEqual(_effective_cdp_smoke_groups("formal", ("puppeteer",), GROUP_LISTING), ("puppeteer",))

    def test_supervisor_requires_successful_complete_artifacts(self) -> None:
        groups = ("protocol", "core", "puppeteer")
        names = ("raw_cdp_runtime_evaluate_awaitpromise_fetch_without_followup", "connect_over_cdp", "puppeteer_goto_plain")
        for failure in (None, "failed-group", "missing-group", "missing-result", "invalid-result", "invalid-schema", "failed-result"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temp_dir:
                def run_process(command: list[str], **_: object) -> ProcessResult:
                    self.assertIn("--output-dir", command)
                    artifacts = Path(command[command.index("--output-dir") + 1])
                    outcomes = []
                    for group, name in zip(groups, names):
                        outcomes.append({"group": group, "status": "passed"})
                        payload = {"ok": True, "group": group, "results": [{"name": name, "ok": True}]}
                        (artifacts / f"{group}.json").write_text(json.dumps(payload))
                    if failure == "failed-group":
                        outcomes[0]["status"] = "failed"
                    elif failure == "missing-group":
                        outcomes.pop()
                    elif failure == "missing-result":
                        (artifacts / "puppeteer.json").unlink()
                    elif failure == "invalid-result":
                        (artifacts / "puppeteer.json").write_text("incomplete")
                    elif failure == "invalid-schema":
                        (artifacts / "puppeteer.json").write_text("[]")
                    elif failure == "failed-result":
                        (artifacts / "puppeteer.json").write_text(json.dumps({"ok": False, "group": "puppeteer", "results": []}))
                    (artifacts / "summary.json").write_text(json.dumps({"ok": True, "groups": outcomes}))
                    return ProcessResult(command, 0, 123.0, b"", b"", False, {})

                with (
                    patch("moli_benchmark.cdp_smoke.run_process", run_process),
                    patch("moli_benchmark.cdp_smoke._discover_group_listing", return_value=GROUP_LISTING),
                    patch("moli_benchmark.cdp_smoke._collect_preflight", return_value=PREFLIGHT_OK),
                ):
                    summary = run_cdp_smoke_suite(
                        output_dir=Path(temp_dir), moli_bin=Path("/tmp/moli"),
                        timeout_seconds=30, groups=groups, profile="formal",
                    )
                self.assertEqual(summary["ok"], failure is None)
                self.assertEqual(summary["gate_failures"] == 0, failure is None)
                if failure is None:
                    self.assertEqual(summary["total_records"], 3)
                    self.assertTrue(all(summary["client_coverage"].values()))

    def test_formal_summary_records_client_coverage(self) -> None:
        payload = {
            "ok": True,
            "results": [
                {"name": "raw_cdp_runtime_evaluate_awaitpromise_fetch_without_followup", "ok": True},
                {"name": "connect_over_cdp", "ok": True},
                {"name": "puppeteer_goto_plain", "ok": True},
            ],
        }
        result = ProcessResult(
            command=["uv", "run", "moli-cdp-smoke"],
            returncode=0,
            elapsed_ms=123.0,
            stdout=json.dumps(payload).encode(),
            stderr=b"",
            timed_out=False,
            resources={},
        )
        commands: list[list[str]] = []

        def run_process(command: list[str], **_: object) -> ProcessResult:
            commands.append(command)
            return result

        with tempfile.TemporaryDirectory() as temp_dir:
            with (
                patch("moli_benchmark.cdp_smoke.run_process", run_process),
                patch("moli_benchmark.cdp_smoke._discover_group_listing", return_value=GROUP_LISTING),
                patch("moli_benchmark.cdp_smoke._collect_preflight", return_value=PREFLIGHT_OK),
            ):
                summary = run_cdp_smoke_suite(
                    output_dir=Path(temp_dir),
                    moli_bin=Path("/tmp/moli"),
                    timeout_seconds=30,
                    groups=(),
                    profile="formal",
                    command=("python3", "-m", "moli_cdp_smoke"),
                )

        self.assertEqual(summary["profile"], "formal")
        self.assertEqual(summary["gate_failures"], 0)
        self.assertEqual(summary["client_coverage"], {"raw_cdp": True, "playwright": True, "puppeteer": True})
        self.assertEqual(summary["formal_requirements"]["puppeteer"]["ok"], True)
        self.assertEqual(summary["client_rows"][2]["client"], "puppeteer")
        self.assertEqual(summary["client_rows"][2]["record_count"], 1)
        self.assertIn("--group", commands[0])
        self.assertIn("puppeteer", commands[0])

    def test_formal_profile_fails_gate_when_puppeteer_record_is_missing(self) -> None:
        payload = {
            "ok": True,
            "results": [
                {"name": "raw_cdp_runtime_evaluate_awaitpromise_fetch_without_followup", "ok": True},
                {"name": "connect_over_cdp", "ok": True},
            ],
        }
        result = ProcessResult(
            command=["uv", "run", "moli-cdp-smoke"],
            returncode=0,
            elapsed_ms=123.0,
            stdout=json.dumps(payload).encode(),
            stderr=b"",
            timed_out=False,
            resources={},
        )

        def run_process(command: list[str], **_: object) -> ProcessResult:
            return result

        with tempfile.TemporaryDirectory() as temp_dir:
            with (
                patch("moli_benchmark.cdp_smoke.run_process", run_process),
                patch("moli_benchmark.cdp_smoke._discover_group_listing", return_value=GROUP_LISTING),
                patch("moli_benchmark.cdp_smoke._collect_preflight", return_value=PREFLIGHT_OK),
            ):
                summary = run_cdp_smoke_suite(
                    output_dir=Path(temp_dir),
                    moli_bin=Path("/tmp/moli"),
                    timeout_seconds=30,
                    groups=("protocol", "core"),
                    profile="formal",
                    command=("python3", "-m", "moli_cdp_smoke"),
                )

        self.assertEqual(summary["total_failures"], 0)
        self.assertEqual(summary["profile_failures"], 1)
        self.assertEqual(summary["gate_failures"], 1)
        self.assertEqual(summary["formal_requirements"]["puppeteer"]["ok"], False)
        self.assertEqual(summary["client_rows"][2]["failure_kind"], "no-records")

    def test_formal_profile_reports_puppeteer_dependency_missing(self) -> None:
        payload = {
            "ok": True,
            "results": [
                {"name": "raw_cdp_runtime_evaluate_awaitpromise_fetch_without_followup", "ok": True},
                {"name": "connect_over_cdp", "ok": True},
            ],
        }
        result = ProcessResult(
            command=["uv", "run", "moli-cdp-smoke"],
            returncode=0,
            elapsed_ms=123.0,
            stdout=json.dumps(payload).encode(),
            stderr=b"",
            timed_out=False,
            resources={},
        )
        preflight = {
            "node": {"executable": "node", "available": False},
            "puppeteer_core": {
                "module": "puppeteer-core",
                "available": False,
                "error": "executable `node` not found",
            },
        }

        def run_process(command: list[str], **_: object) -> ProcessResult:
            return result

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            with (
                patch("moli_benchmark.cdp_smoke.run_process", run_process),
                patch("moli_benchmark.cdp_smoke._discover_group_listing", return_value=GROUP_LISTING),
                patch("moli_benchmark.cdp_smoke._collect_preflight", return_value=preflight),
            ):
                summary = run_cdp_smoke_suite(
                    output_dir=output_dir,
                    moli_bin=Path("/tmp/moli"),
                    timeout_seconds=30,
                    groups=(),
                    profile="formal",
                    command=("python3", "-m", "moli_cdp_smoke"),
                )

            client_rows = json.loads((output_dir / "cdp-smoke" / "client-rows.json").read_text(encoding="utf-8"))
            preflight_json = json.loads((output_dir / "cdp-smoke" / "preflight.json").read_text(encoding="utf-8"))

        self.assertEqual(summary["client_rows"][2]["client"], "puppeteer")
        self.assertEqual(summary["client_rows"][2]["failure_kind"], "dependency-missing")
        self.assertEqual(client_rows["rows"][2]["failure_kind"], "dependency-missing")
        self.assertEqual(preflight_json["node"]["available"], False)


if __name__ == "__main__":
    unittest.main()
