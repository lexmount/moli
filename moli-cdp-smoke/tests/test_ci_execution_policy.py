from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

from moli_cdp_smoke.runner import group_listing
from moli_cdp_smoke.supervisor import parse_args

WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
CANCEL_WORKFLOW = WORKFLOW.with_name("cancel-pr-ci.yml")


class CiExecutionPolicyTests(unittest.TestCase):
    def chromium_step(self) -> str:
        step = WORKFLOW.read_text().split(
            "      - name: Run full CDP smoke on Chromium\n", 1
        )[1].split("\n      - ", 1)[0]
        return textwrap.dedent(step.split("        run: |\n", 1)[1])

    def run_chromium_step(self, groups: list[str], *, enumeration_exit: int = 0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            invocation = root / "invocation.json"
            uv = root / "uv"
            uv.write_text(f"#!{sys.executable}\n" + textwrap.dedent("""\
                import json
                import os
                from pathlib import Path
                import sys

                args = sys.argv[1:]
                assert args[:4] == ["run", "--project", "moli-cdp-smoke", "--no-sync"], args
                if args[4] == "python":
                    print(os.environ["TEST_GROUPS"])
                    sys.exit(int(os.environ["TEST_ENUMERATION_EXIT"]))
                assert args[4] == "moli-cdp-smoke", args
                Path(os.environ["TEST_INVOCATION"]).write_text(json.dumps(args[5:]))
                """))
            uv.chmod(0o755)
            result = subprocess.run(
                ["bash", "-euo", "pipefail", "-c", self.chromium_step()],
                env={**os.environ, "PATH": str(root) + os.pathsep + os.environ["PATH"],
                     "CHROMIUM_CURRENT_ORACLE_BIN": "/tmp/oracle with spaces=156/chrome",
                     "TEST_GROUPS": " ".join(groups),
                     "TEST_ENUMERATION_EXIT": str(enumeration_exit),
                     "TEST_INVOCATION": str(invocation)},
                capture_output=True, text=True, check=False,
            )
            return result, json.loads(invocation.read_text()) if invocation.exists() else None

    def test_chromium_overrides_follow_the_checked_out_group_registry(self) -> None:
        current = [group["name"] for group in group_listing()]
        for groups in (
            current,
            [name for name in current if name != "fetch-headers"],
            [name for name in current if name != "worker-loading"],
            ["worker-loading"],
            ["network"],
        ):
            with self.subTest(groups=groups):
                result, invocation = self.run_chromium_step(groups)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIsNotNone(invocation)
                with patch("moli_cdp_smoke.supervisor.group_listing", return_value=[
                    {"name": name} for name in groups
                ]):
                    args = parse_args(invocation)
                self.assertEqual(args.chromium_bin_for_group, {
                    name: "/tmp/oracle with spaces=156/chrome"
                    for name in ("dom-input", "fetch-headers", "worker-loading") if name in groups
                })
                self.assertEqual(args.group, [])
                self.assertTrue(args.strict_full_suite)
                self.assertEqual((args.repeat, args.retries, args.jobs), (2, 3, 4))

    def test_chromium_registry_failure_does_not_start_a_partial_suite(self) -> None:
        result, invocation = self.run_chromium_step(["dom-input"], enumeration_exit=42)
        self.assertEqual(result.returncode, 42, result.stderr)
        self.assertIsNone(invocation)

    def test_only_new_main_pushes_cancel_older_ci_runs(self) -> None:
        workflow = WORKFLOW.read_text()
        self.assertRegex(workflow, r"(?m)^concurrency:\n"
                         r"  group: ci-\$\{\{ github.event_name == 'push' && github.ref \|\| github.run_id \}\}\n"
                         r"  cancel-in-progress: \$\{\{ github.event_name == 'push' \}\}")

    def test_closed_prs_cancel_runs_using_the_trusted_default_branch(self) -> None:
        workflow = CANCEL_WORKFLOW.read_text()
        self.assertRegex(workflow, r"pull_request_target:\s+types: \[closed\]")
        self.assertIn("actions: write", workflow)
        self.assertIn("pull-requests: read", workflow)
        self.assertIn("persist-credentials: false", workflow)
        self.assertIn("ref: ${{ github.event.repository.default_branch }}", workflow)
        self.assertIn("require('./.github/scripts/cancel-pr-ci.cjs')", workflow)
        self.assertIn("await cancelClosedPrCi({ github, context, core })", workflow)

    def test_both_browser_jobs_require_full_runs_and_diagnostic_uploads(self) -> None:
        workflow = WORKFLOW.read_text()
        for name, engine in (("cdp-smoke", "moli"), ("cdp-smoke-chromium", "chromium")):
            with self.subTest(job=name):
                match = re.search(rf"^  {re.escape(name)}:\n(.*?)(?=^  [\w-]+:|\Z)",
                                  workflow, re.MULTILINE | re.DOTALL)
                self.assertIsNotNone(match)
                job = match.group(1)
                self.assertRegex(job, rf"--browser {engine} --strict-full-suite --repeat 2")
                if engine == "chromium":
                    self.assertIn("--retries 3", job)
                    self.assertIn("shell: bash", job)
                else:
                    self.assertNotIn("--retries", job)
                self.assertNotIn("continue-on-error:", job)
                self.assertIn("if-no-files-found: error", job)
