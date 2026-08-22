from __future__ import annotations

import unittest
from tempfile import TemporaryDirectory
from pathlib import Path

from moli_benchmark.artifacts import write_json
from moli_benchmark.html_report import _artifact_paths_by_suite, _chartjs_document, _report_payload


class HtmlReportTests(unittest.TestCase):
    def test_artifact_index_omits_absent_conditional_artifacts(self) -> None:
        paths = _artifact_paths_by_suite(
            [
                {"suite": "startup", "cache_artifacts": []},
                {"suite": "cdp-session", "total_failures": 0, "total_trace_events": 0},
                {"suite": "wild-web", "total_failures": 0, "replay_artifacts": 0},
                {"suite": "wpt", "diff": None},
            ]
        )

        self.assertNotIn("startup/cache/", paths["startup"])
        self.assertNotIn("cdp-session/traces/", paths["cdp-session"])
        self.assertNotIn("wild-web/failures/", paths["wild-web"])
        self.assertNotIn("wild-web/replay/manifest.json", paths["wild-web"])
        self.assertNotIn("wpt/diff.json", paths["wpt"])
        self.assertNotIn("wpt/diff.csv", paths["wpt"])

    def test_artifact_index_includes_present_conditional_artifacts(self) -> None:
        paths = _artifact_paths_by_suite(
            [
                {"suite": "startup", "cache_artifacts": ["startup/cache/serve-ready-run-1/drop-caches.txt"]},
                {"suite": "cdp-session", "total_failures": 0, "total_trace_events": 2},
                {"suite": "wild-web", "total_failures": 1, "replay_artifacts": 1},
                {"suite": "wpt", "diff": {"total_changes": 1}},
            ]
        )

        self.assertIn("startup/cache/", paths["startup"])
        self.assertIn("cdp-session/traces/", paths["cdp-session"])
        self.assertIn("wild-web/failures/", paths["wild-web"])
        self.assertIn("wild-web/replay/manifest.json", paths["wild-web"])
        self.assertIn("wpt/diff.json", paths["wpt"])
        self.assertIn("wpt/diff.csv", paths["wpt"])

    def test_artifact_index_includes_render_compare_details(self) -> None:
        paths = _artifact_paths_by_suite([{"suite": "render-compare", "total_failures": 0}])

        self.assertIn("render-compare/runs.json", paths["render-compare"])
        self.assertIn("render-compare/baseline-sites.csv", paths["render-compare"])

    def test_artifact_index_includes_top_sites_site_outcomes(self) -> None:
        paths = _artifact_paths_by_suite(
            [{"suite": "top-sites", "total_failures": 0}]
        )

        self.assertIn("top-sites/site-outcomes.json", paths["top-sites"])

    def test_cdp_session_trace_link_uses_nested_summary_fallback(self) -> None:
        paths = _artifact_paths_by_suite(
            [
                {
                    "suite": "cdp-session",
                    "total_failures": 0,
                    "targets": {"moli": {"console_errors": 0, "js_exceptions": 1, "network_failures": 0}},
                }
            ]
        )

        self.assertIn("cdp-session/traces/", paths["cdp-session"])

    def test_report_payload_adds_combined_web_scraping_variant_view(self) -> None:
        payload = _report_payload(
            output_dir=Path("/tmp/report"),
            versions={},
            publish_readiness=None,
            report_diff=None,
            summaries=[
                {
                    "suite": "synthetic-compare",
                    "gate_target": "moli",
                    "gate_failures": 0,
                    "cases": ["static-html"],
                    "targets": {
                        "moli": {
                            "engine": "moli",
                            "driver": "fetch",
                            "label": "moli / fetch",
                            "failures": 0,
                            "cases": {"static-html": {"elapsed_ms": {"p50": 10.0, "count": 1}, "failures": 0}},
                        }
                    },
                },
                {
                    "suite": "cdp-session",
                    "gate_target": "moli-cdp",
                    "gate_failures": 0,
                    "cases": ["static-html"],
                    "targets": {
                        "moli-cdp": {
                            "engine": "moli",
                            "driver": "cdp",
                            "label": "moli / cdp",
                            "failures": 0,
                            "cases": {"static-html": {"elapsed_ms": {"p50": 20.0, "count": 1}, "failures": 0}},
                        }
                    },
                },
            ],
        )

        combined = payload["horizontal_comparisons"][0]
        self.assertEqual(combined["suite"], "web-scraping-variants")
        self.assertEqual(list(combined["targets"]), ["moli", "moli-cdp"])
        self.assertEqual(combined["targets"]["moli-cdp"]["label"], "moli / cdp")

    def test_report_payload_embeds_render_compare_page_rows(self) -> None:
        with TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            write_json(
                output_dir / "render-compare" / "runs.json",
                [
                    {
                        "target": "moli",
                        "engine": "moli",
                        "rank": 1,
                        "domain": "https://example.test/page",
                        "url": "https://example.test/page",
                        "category": "render-match",
                        "ok": True,
                        "elapsed_ms": 12.0,
                        "peak_rss_bytes": 4096,
                        "render_quality_score": 99.0,
                        "key_phrases": ["large internal detail should be dropped"],
                    }
                ],
            )
            write_json(
                output_dir / "render-compare" / "baseline-sites.json",
                [
                    {
                        "rank": 1,
                        "domain": "https://example.test/page",
                        "url": "https://example.test/page",
                        "category": "baseline-usable",
                        "usable": True,
                        "baseline_elapsed_ms": 10.0,
                    }
                ],
            )

            payload = _report_payload(
                output_dir=output_dir,
                versions={},
                publish_readiness=None,
                report_diff=None,
                summaries=[{"suite": "render-compare", "targets": {}}],
            )

        self.assertEqual(payload["render_compare"]["run_count"], 1)
        self.assertEqual(payload["render_compare"]["baseline_site_count"], 1)
        self.assertEqual(payload["render_compare"]["runs"][0]["render_quality_score"], 99.0)
        self.assertEqual(payload["render_compare"]["runs"][0]["peak_rss_bytes"], 4096)
        self.assertNotIn("key_phrases", payload["render_compare"]["runs"][0])

    def test_chartjs_report_uses_horizontal_case_metric_bars(self) -> None:
        document = _chartjs_document(
            {
                "output_dir": "/tmp/report",
                "versions": {},
                "publish_readiness": {},
                "summaries": [
                    {
                        "suite": "synthetic-compare",
                        "cases": ["static-html"],
                        "targets": {
                            "moli": {
                                "engine": "moli",
                                "driver": "fetch",
                                "label": "moli / fetch",
                                "failures": 0,
                                "cases": {"static-html": {"elapsed_ms": {"p50": 1.0, "count": 1}, "failures": 0}},
                            }
                        },
                    }
                ],
            }
        )

        self.assertIn("indexAxis: 'y'", document)
        self.assertIn("document.getElementById('latencyChart').parentElement.style.height", document)
        self.assertIn("document.getElementById('rssChart').parentElement.style.height", document)
        self.assertIn("Memory RSS P50 by Case", document)
        self.assertIn("Web Page Request Scores", document)
        self.assertIn("function renderWebPages()", document)
        self.assertIn("render_quality_score", document)
        self.assertIn("peak_rss_bytes", document)
        self.assertIn('class="panel wide"', document)
        self.assertIn("'peak_rss_bytes'", document)
        self.assertIn("x: { beginAtZero: true, grid: { color: '#e6ebf1' }, title: { display: true, text: label } }", document)
        self.assertIn("y: { grid: { display: false }, ticks: { autoSkip: false } }", document)

    def test_chartjs_report_keeps_failure_reason_in_tooltip(self) -> None:
        document = _chartjs_document({"output_dir": "/tmp/report", "versions": {}, "summaries": []})

        self.assertIn("function statusBadge(status, tooltip)", document)
        self.assertIn('class="status-tooltip"', document)
        self.assertIn("data-tooltip=", document)
        self.assertNotIn('<span class="muted">reason</span>', document)

    def test_chartjs_report_aggregate_pass_rate_prefers_attempts_and_passes(self) -> None:
        document = _chartjs_document(
            {
                "output_dir": "/tmp/report",
                "versions": {},
                "summaries": [
                    {
                        "suite": "crawler",
                        "targets": {
                            "moli": {
                                "label": "moli",
                                "passes": 3,
                                "failures": 2,
                                "runs": 1,
                            }
                        },
                    }
                ],
            }
        )

        self.assertIn("const attempts = passes + failures;", document)
        self.assertIn("result.pages ?? result.seeds ?? result.sites ?? result.runs", document)
        self.assertIn("result.successes ?? result.categories?.success ?? result.passes", document)
        self.assertIn("counted pass rate", document)
        self.assertIn("raw pass rate", document)
        self.assertIn("successful p50 / p90 / p95 ms", document)
        self.assertIn("common p50 / p90 / p95 ms", document)
        self.assertIn("result?.successful_elapsed_ms || result?.elapsed_ms", document)
        self.assertIn("summary?.common_success?.targets?.[target]", document)
        self.assertIn("site stability", document)
        self.assertIn("HTTP status observed", document)
        self.assertIn("not repeat-validated", document)
        self.assertIn("Pairwise site outcomes", document)
        self.assertIn("SINGLE SAMPLE", document)

    def test_chartjs_report_renders_every_aggregate_summary_and_prioritizes_top_sites(
        self,
    ) -> None:
        document = _chartjs_document(
            {
                "output_dir": "/tmp/report",
                "versions": {},
                "summaries": [
                    {"suite": "wild-web", "targets": {"moli": {}}},
                    {"suite": "top-sites", "targets": {"moli": {}}},
                ],
            }
        )

        self.assertIn("function horizontalSummaries()", document)
        self.assertIn("'top-sites', 'wild-web'", document)
        self.assertIn("summaries.map(summary =>", document)
        self.assertIn('class="comparison-summary"', document)


if __name__ == "__main__":
    unittest.main()
