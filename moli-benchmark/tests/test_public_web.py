from __future__ import annotations

import unittest

from moli_benchmark.process import ProcessResult
from moli_benchmark.public_web import (
    POST_DCL_SETTLE_MILLISECONDS,
    POST_DCL_WAIT_SCRIPT,
    PUBLIC_WEB_SNAPSHOT_CONTRACT,
    TOP_SITES_ARTIFACT_POLICY,
    TOP_SITES_CLASSIFIER,
    WILD_WEB_CLASSIFIER,
    PublicWebAttempt,
    PublicWebResult,
    PublicWebScheduler,
    resolve_main_document_status,
    schedule_public_web_cases,
)


class PublicWebTests(unittest.TestCase):
    def test_post_dcl_wait_script_encodes_the_reported_snapshot_contract(self) -> None:
        self.assertEqual(POST_DCL_SETTLE_MILLISECONDS, 50)
        self.assertIn("setTimeout", POST_DCL_WAIT_SCRIPT)
        self.assertIn("}, 50)", POST_DCL_WAIT_SCRIPT)
        self.assertIn("PostDclReady", POST_DCL_WAIT_SCRIPT)
        self.assertIn("50 ms", PUBLIC_WEB_SNAPSHOT_CONTRACT)

    def test_typed_cli_fetch_readiness_timeout_is_shared_by_both_suites(self) -> None:
        stderr = b"""
Error: failed to fetch `https://example.test/`
Reason: fetch readiness timed out after 30000 ms while waiting for DOMContentLoaded
"""

        for classifier in (TOP_SITES_CLASSIFIER, WILD_WEB_CLASSIFIER):
            with self.subTest(policy=classifier.policy):
                self.assertEqual(
                    classifier.classify_output(
                        stdout=b"",
                        stderr=stderr,
                        returncode=1,
                        timed_out=False,
                    ),
                    "timeout",
                )

    def test_protocol_status_is_preferred(self) -> None:
        self.assertEqual(
            resolve_main_document_status(
                200,
                b"lifecycle target document `https://example.test/` returned 502 Bad Gateway",
                1,
            ),
            (200, "protocol"),
        )

    def test_nonzero_cli_diagnostic_supplies_main_document_status(self) -> None:
        self.assertEqual(
            resolve_main_document_status(
                None,
                b"Reason: lifecycle target document `https://example.test/` "
                b"returned 429 Too Many Requests",
                1,
            ),
            (429, "cli-diagnostic"),
        )
        self.assertEqual(
            resolve_main_document_status(
                None,
                b"HTTP request `https://example.test/` returned 404 Not Found",
                1,
            ),
            (404, "cli-diagnostic"),
        )

    def test_success_and_subresource_diagnostics_are_not_promoted(self) -> None:
        diagnostic = b"script subresource `https://cdn.example.test/a.js` returned 500"
        self.assertEqual(resolve_main_document_status(None, diagnostic, 1), (None, None))
        self.assertEqual(
            resolve_main_document_status(
                None,
                b"lifecycle target document `https://example.test/` returned 500",
                0,
            ),
            (None, None),
        )

    def test_attempt_result_owns_common_evidence_fields(self) -> None:
        attempt = PublicWebAttempt.start(
            target="chrome",
            metadata={"engine": "chrome", "driver": "cdp-dcl"},
            target_info={"available": True, "path": "/bin/chromium"},
            run=2,
            case_fields={"seed": "example"},
            url="https://example.test/",
            schedule_index=3,
            target_order_index=2,
            artifact_stem="chrome-run-2-example",
        )
        process = ProcessResult(
            command=["chromium"],
            returncode=0,
            elapsed_ms=12.5,
            stdout=b"<title>Example</title><body>Hello world</body>",
            stderr=b"",
            timed_out=False,
            resources={"peak_pss_bytes": 123},
            response_status=200,
            final_url="https://example.test/final",
        )

        result = PublicWebResult.capture(
            attempt,
            process,
            classifier=TOP_SITES_CLASSIFIER,
        )
        row = {
            **result.base_row(),
            **result.evidence_fields(policy=TOP_SITES_ARTIFACT_POLICY),
        }

        self.assertEqual(row["seed"], "example")
        self.assertEqual(row["response_status_source"], "protocol")
        self.assertEqual(row["final_url"], "https://example.test/final")
        self.assertEqual(row["title"], "Example")
        self.assertEqual(row["peak_pss_bytes"], 123)
        self.assertEqual(len(row["stdout_sha256"]), 64)

    def test_scheduler_rotates_targets_and_restores_schedule_order(self) -> None:
        scheduled = schedule_public_web_cases(("first", "second"), runs=1)
        scheduler = PublicWebScheduler[str, tuple[int, str, int]](
            ("moli", "lightpanda", "chrome"),
            parallelism=2,
        )

        results = scheduler.run(
            scheduled,
            lambda case, target, target_order_index: (
                case.schedule_index,
                target,
                target_order_index,
            ),
        )

        self.assertEqual(
            results,
            [
                (0, "moli", 1),
                (0, "lightpanda", 2),
                (0, "chrome", 3),
                (1, "lightpanda", 1),
                (1, "chrome", 2),
                (1, "moli", 3),
            ],
        )

    def test_scheduler_changes_each_site_first_target_across_runs(self) -> None:
        cases = tuple(f"site-{index}" for index in range(20))
        scheduled = schedule_public_web_cases(cases, runs=3)
        scheduler = PublicWebScheduler[str, tuple[str, int, str, int]](
            ("moli", "chrome"),
        )

        results = scheduler.run(
            scheduled,
            lambda case, target, target_order_index: (
                case.case,
                case.run,
                target,
                target_order_index,
            ),
        )
        first_targets = {
            (case, run): target
            for case, run, target, target_order_index in results
            if target_order_index == 1
        }

        for case_index, case in enumerate(cases):
            expected = (
                ("moli", "chrome", "moli")
                if case_index % 2 == 0
                else ("chrome", "moli", "chrome")
            )
            self.assertEqual(
                tuple(first_targets[(case, run)] for run in range(1, 4)),
                expected,
            )
