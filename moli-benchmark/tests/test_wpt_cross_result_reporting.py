from __future__ import annotations

from wpt_cross_test_support import *


class WptCrossResultReportingTests(WptCrossTestCase):
    def test_report_bridge_keys_results_by_initial_search_without_hash(self) -> None:
        self.assertIn(b"var initialCasePath =", BENCH_REPORT_BRIDGE)
        self.assertIn(b"case_path: initialCasePath", BENCH_REPORT_BRIDGE)
        self.assertIn(b"location.pathname + location.search", BENCH_REPORT_BRIDGE)
        self.assertNotIn(b"location.pathname + location.search + location.hash", BENCH_REPORT_BRIDGE)
    def test_report_bridge_does_not_post_incremental_payloads(self) -> None:
        self.assertIn(b"if (source === 'incremental') {", BENCH_REPORT_BRIDGE)
        incremental_start = BENCH_REPORT_BRIDGE.index(b"if (source === 'incremental') {")
        final_payload_start = BENCH_REPORT_BRIDGE.index(b"var snapshot = {")
        incremental_block = BENCH_REPORT_BRIDGE[incremental_start:final_payload_start]

        self.assertIn(b"partial_count: accumulator.tests.length", incremental_block)
        self.assertIn(b"tests: []", incremental_block)
        self.assertIn(b"return;", incremental_block)
        self.assertNotIn(b"accumulator.tests.slice()", incremental_block)
    def test_report_bridge_posts_only_bounded_final_payloads_synchronously(self) -> None:
        self.assertIn(b"if (body.length <= 60000)", BENCH_REPORT_BRIDGE)
        self.assertIn(b"xhr.open('POST', '/__bench__/result', false)", BENCH_REPORT_BRIDGE)
        self.assertLess(
            BENCH_REPORT_BRIDGE.index(b"xhr.open('POST', '/__bench__/result', false)"),
            BENCH_REPORT_BRIDGE.rindex(b"window.__bench_wpt__ = snapshot;"),
        )
    def test_report_bridge_writes_dom_payload_before_posting(self) -> None:
        self.assertIn(b"__bench_wpt_payload", BENCH_REPORT_BRIDGE)
        self.assertLess(
            BENCH_REPORT_BRIDGE.index(b"node.textContent = body;"),
            BENCH_REPORT_BRIDGE.index(b"xhr.open('POST', '/__bench__/result', false)"),
        )
    def test_report_bridge_disables_in_page_testharness_output(self) -> None:
        self.assertIn(b"output: false", BENCH_REPORT_BRIDGE)
    def test_report_bridge_applies_configured_timeout_multiplier(self) -> None:
        bridge = _bench_report_bridge(7.5)

        self.assertIn(b"timeout_multiplier: 7.5", bridge)
    def test_report_bridge_config_is_injected_without_changing_case_url(self) -> None:
        body = (
            b'<!doctype html><script src="/resources/testharness.js"></script>'
            b'<script src="/resources/testharnessreport.js"></script>'
            b"<script>test(() => location.search, 'query visible to case')</script>"
        )

        injected = _inject_bench_report_bridge_config(body, 3.0)

        self.assertIn(
            (
                b'src="/resources/testharnessreport.js?'
                + BENCH_TIMEOUT_MULTIPLIER_QUERY.encode("ascii")
                + b'=3"'
            ),
            injected,
        )
        self.assertIn(b"location.search", injected)
    def test_report_bridge_config_keeps_default_multiplier_body_unchanged(self) -> None:
        body = (
            b'<!doctype html><script src="/resources/testharness.js"></script>'
            b'<script src="/resources/testharnessreport.js"></script>'
        )

        self.assertEqual(_inject_bench_report_bridge_config(body, 1.0), body)
    def test_harness_case_key_ignores_url_fragment(self) -> None:
        self.assertEqual(
            _normalize_harness_case_key("/dom/ranges/feature.html?mode=open#frag"),
            "dom/ranges/feature.html?mode=open",
        )
    def test_report_bridge_does_not_publish_empty_done_fallback(self) -> None:
        self.assertIn(b"function canPublishDoneFallback()", BENCH_REPORT_BRIDGE)
        self.assertIn(
            b"window.__bench_wpt__ === undefined && canPublishDoneFallback()",
            BENCH_REPORT_BRIDGE,
        )
        self.assertIn(
            b"canPublishDoneFallback()) {\n            publish('done-hook-late', null);",
            BENCH_REPORT_BRIDGE,
        )
    def test_case_result_dict_records_payload_source(self) -> None:
        row = case_result_to_dict(
            CaseResult(
                case_path="example.html",
                url="http://example.test/example.html",
                status="pass",
                duration_ms=1.0,
                payload_source="completion-callback",
            )
        )

        self.assertEqual(row["payload_source"], "completion-callback")
    def test_case_result_dict_records_reftest_comparisons_and_artifacts(self) -> None:
        row = case_result_to_dict(
            CaseResult(
                case_path="css/example.html",
                url="http://example.test/css/example.html",
                status="fail",
                duration_ms=1.0,
                test_type="reftest",
                reftest_comparisons=[
                    {
                        "reference_path": "css/example-ref.html",
                        "relation": "==",
                        "passed": False,
                        "max_difference": 255,
                        "different_pixels": 10,
                    }
                ],
                artifacts={
                    "test": "artifacts/moli/example/test.png",
                    "references": [
                        {
                            "reference": "artifacts/moli/example/reference-01.png",
                            "diff": "artifacts/moli/example/diff-01.png",
                        }
                    ],
                },
            )
        )

        self.assertEqual(row["test_type"], "reftest")
        self.assertEqual(row["reftest_comparisons"][0]["relation"], "==")
        self.assertEqual(row["artifacts"]["test"], "artifacts/moli/example/test.png")
    def test_cli_runner_extracts_payload_from_stdout_html(self) -> None:
        payload = {
            "case_path": "/case.html",
            "harness": {"status": 0, "message": None},
            "tests": [{"name": "ok", "status": 0}],
            "source": "completion-callback",
        }
        stdout = (
            "<!doctype html><html><pre id=\"__bench_wpt_payload\" hidden>"
            + json.dumps(payload)
            + "</pre></html>"
        ).encode()

        self.assertEqual(_payload_from_stdout_html(stdout), payload)
    def test_cli_runner_prefers_final_stdout_payload_without_callback_wait(self) -> None:
        class UnexpectedResultsAccess:
            def wait_for_final(self, key: str, timeout: float) -> None:
                raise AssertionError("final stdout payload should skip callback wait")

            def get(self, key: str) -> None:
                raise AssertionError("final stdout payload should skip callback lookup")

        payload = {
            "case_path": "/case.html",
            "harness": {"status": 0, "message": None},
            "tests": [{"name": "ok", "status": 0}],
            "source": "completion-callback",
        }
        stdout = (
            '<!doctype html><pre id="__bench_wpt_payload" hidden>'
            + json.dumps(payload)
            + "</pre>"
        ).encode()

        result = _classify_cli_case_result(
            case_path="case.html",
            url="http://example.test/case.html",
            bridge_key="/case.html",
            fixture_server=SimpleNamespace(results=UnexpectedResultsAccess()),
            subprocess_result=_CliSubprocessResult(
                duration_ms=10.0,
                proc_error=None,
                proc_returncode=0,
                proc_stderr="",
                proc_stdout=stdout,
                wait_script_timeout=False,
            ),
            payload_grace_seconds=2.0,
            successful_process_payload_grace_seconds=8.0,
        )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.payload_source, "completion-callback")
    def test_case_result_dict_records_failure_details(self) -> None:
        row = case_result_to_dict(
            CaseResult(
                case_path="example.html",
                url="http://example.test/example.html",
                status="fail",
                duration_ms=1.0,
                failures=[
                    {
                        "name": "subtest name",
                        "status": 1,
                        "status_name": "FAIL",
                        "message": "expected true got false",
                    }
                ],
            )
        )

        self.assertEqual(
            row["failures"],
            [
                {
                    "name": "subtest name",
                    "status": 1,
                    "status_name": "FAIL",
                    "message": "expected true got false",
                }
            ],
        )
    def test_classify_payload_records_limited_failure_details(self) -> None:
        result = classify_payload(
            payload={
                "source": "completion-callback",
                "harness": {"status": 0},
                "tests": [
                    {"name": "ok", "status": 0},
                    {
                        "name": "bad",
                        "status": 1,
                        "message": "x" * 700,
                    },
                    {"name": "notrun", "status": 3},
                ],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.subtests_total, 3)
        self.assertEqual([failure["name"] for failure in result.failures], ["bad", "notrun"])
        self.assertEqual(result.failure_names, ["bad", "notrun"])
        self.assertEqual(result.failures[0]["status_name"], "FAIL")
        self.assertEqual(len(result.failures[0]["message"]), 500)
        self.assertTrue(result.failures[0]["message_truncated"])
    def test_recorded_failure_drift_reports_hidden_subtest_differences(self) -> None:
        drift = _recorded_failure_drift(
            [
                {
                    "case_path": "webcrypto/shared-fail.html",
                    "results": {
                        "moli": {
                            "status": "fail",
                            "failures": [
                                {"name": "shared", "message": "lm detail"},
                                {"name": "lm-only", "message": "bad"},
                            ],
                        },
                        "chrome": {
                            "status": "fail",
                            "failures": [
                                {"name": "shared", "message": "chrome detail"},
                                {"name": "chrome-only", "message": "bad"},
                            ],
                        },
                    },
                }
            ],
            ["moli", "chrome"],
        )

        self.assertEqual(drift["comparison_count"], 1)
        self.assertEqual(drift["primary_only_comparison_count"], 1)
        self.assertEqual(drift["peer_only_comparison_count"], 1)
        self.assertEqual(drift["message_diff_comparison_count"], 1)
        row = drift["comparisons"][0]
        self.assertEqual(row["primary_only_examples"], ["lm-only"])
        self.assertEqual(row["peer_only_examples"], ["chrome-only"])
        self.assertEqual(row["message_diff_examples"], ["shared"])
    def test_recorded_failure_drift_uses_full_failure_names_when_available(self) -> None:
        drift = _recorded_failure_drift(
            [
                {
                    "case_path": "webcrypto/large-shared-fail.html",
                    "results": {
                        "moli": {
                            "status": "fail",
                            "failures": [{"name": "recorded", "message": "same"}],
                            "failure_names": ["recorded", "late-moli-only"],
                        },
                        "chrome": {
                            "status": "fail",
                            "failures": [{"name": "recorded", "message": "same"}],
                            "failure_names": ["recorded"],
                        },
                    },
                }
            ],
            ["moli", "chrome"],
        )

        self.assertEqual(drift["comparison_count"], 1)
        self.assertEqual(drift["primary_only_comparison_count"], 1)
        self.assertEqual(drift["comparisons"][0]["primary_only_examples"], ["late-moli-only"])
    def test_classify_payload_does_not_pass_incremental_only_result(self) -> None:
        result = classify_payload(
            payload={
                "source": "incremental",
                "harness": {"status": None},
                "tests": [{"name": "observed pass", "status": 0}],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "harness-stalled")
        self.assertEqual(result.subtests_total, 1)
        self.assertEqual(result.subtests_pass, 1)
        self.assertEqual(result.payload_source, "incremental")
        self.assertIn("incremental", result.error or "")
    def test_classify_payload_does_not_pass_empty_final_result(self) -> None:
        result = classify_payload(
            payload={
                "source": "completion-callback",
                "harness": {"status": 0},
                "tests": [],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.subtests_total, 0)
        self.assertIn("without reporting any subtests", result.error or "")
    def test_classify_payload_includes_empty_harness_error_message(self) -> None:
        result = classify_payload(
            payload={
                "source": "completion-callback",
                "harness": {
                    "status": 1,
                    "message": "Unhandled rejection: cyclic wasm dependency",
                },
                "tests": [],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.harness_status, 1)
        self.assertEqual(result.subtests_total, 0)
        self.assertIn("without reporting any subtests", result.error or "")
        self.assertIn("cyclic wasm dependency", result.error or "")
    def test_classify_payload_keeps_observed_incremental_failure(self) -> None:
        result = classify_payload(
            payload={
                "source": "incremental",
                "harness": {"status": None},
                "tests": [{"name": "observed fail", "status": 1}],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.subtests_fail, 1)
        self.assertEqual(result.payload_source, "incremental")
    def test_results_store_wait_for_final_does_not_return_incremental(self) -> None:
        store = ResultsStore()
        store.put("example.html", {"source": "incremental"})

        self.assertIsNone(store.wait_for_final("example.html", timeout=0))
        self.assertEqual(store.get("example.html"), {"source": "incremental"})

    def test_testdriver_vendor_bridge_provides_computed_label(self) -> None:
        self.assertIn(b"get_computed_label", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"resolveReferenceTarget", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"data-expectedlabel", BENCH_TESTDRIVER_VENDOR_BRIDGE)
