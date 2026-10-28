from __future__ import annotations

from pathlib import Path
import re
import unittest


WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
CANCEL_WORKFLOW = WORKFLOW.with_name("cancel-pr-ci.yml")


class CiExecutionPolicyTests(unittest.TestCase):
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
                else:
                    self.assertNotIn("--retries", job)
                self.assertNotIn("continue-on-error:", job)
                self.assertIn("if-no-files-found: error", job)
