from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.process import ProcessResult
from moli_benchmark.public_web import POST_DCL_WAIT_SCRIPT
from moli_benchmark.top_sites import (
    COMPOSITE_TOP_SITES_SOURCES,
    DEFAULT_TOP_SITES_SOURCE,
    TOP_SITES_FIXTURE_ROOT,
    TOP_SITES_PROFILES,
    TOP_SITES_SOURCES,
    _build_site_outcomes,
    _classify,
    _default_top_sites_parallelism,
    _elapsed_failure_reached_timeout,
    _extract_title_and_text,
    _ok_categories,
    _pairwise_site_comparisons,
    _rotated_target_order,
    _site_unreachable_exclusions,
    _top_command_for_target,
    _top_sites_target_metadata,
    load_top_sites_entries,
    parse_top_sites_list,
    resolve_top_sites_source,
    run_top_sites_suite,
)


class TopSitesTests(unittest.TestCase):
    def test_target_order_rotates_per_site_group(self) -> None:
        targets = ("moli", "lightpanda", "chrome")

        self.assertEqual(_rotated_target_order(targets, 0), targets)
        self.assertEqual(
            _rotated_target_order(targets, 1),
            ("lightpanda", "chrome", "moli"),
        )
        self.assertEqual(
            _rotated_target_order(targets, 2),
            ("chrome", "moli", "lightpanda"),
        )

    def test_site_unreachable_exclusion_requires_transport_failure_from_every_target(
        self,
    ) -> None:
        rows = [
            {
                "target": "moli",
                "domain": "dead.example",
                "ok": False,
                "category": "network-error",
                "failure_kind": "network-error",
            },
            {
                "target": "chrome",
                "domain": "dead.example",
                "ok": False,
                "category": "process-error",
                "failure_kind": "process-error",
            },
        ]

        self.assertEqual(
            _site_unreachable_exclusions(rows, ("moli", "chrome")),
            {},
        )
        rows[1]["category"] = "timeout"
        rows[1]["failure_kind"] = "timeout"
        self.assertEqual(
            _site_unreachable_exclusions(rows, ("moli", "chrome")),
            {"dead.example": "site-unreachable"},
        )

    def test_site_outcomes_keep_header_only_binary_observations_neutral(self) -> None:
        outcomes = _build_site_outcomes(
            [
                {
                    "target": "chrome",
                    "rank": 1,
                    "domain": "document.example",
                    "url": "https://document.example/file.pdf",
                    "run": 1,
                    "ok": False,
                    "comparable": False,
                    "category": "binary-response-headers",
                    "response_status": 200,
                }
            ]
        )

        self.assertEqual(outcomes[0]["outcome"], "not-comparable")
        self.assertEqual(outcomes[0]["attempts"], 0)
        self.assertEqual(outcomes[0]["raw_attempts"], 1)
        self.assertEqual(outcomes[0]["non_comparable_attempts"], 1)
        self.assertIsNone(outcomes[0]["pass_rate_percent"])

    def test_site_outcomes_separate_repeat_stability_from_attempt_passes(self) -> None:
        rows: list[dict[str, object]] = []
        for run, left_ok, right_ok in (
            (1, True, False),
            (2, True, False),
            (3, True, False),
        ):
            rows.extend(
                [
                    {
                        "target": "left",
                        "rank": 1,
                        "domain": "stable.example",
                        "url": "https://stable.example/",
                        "run": run,
                        "ok": left_ok,
                        "category": "success-content",
                        "response_status": 200,
                    },
                    {
                        "target": "right",
                        "rank": 1,
                        "domain": "stable.example",
                        "url": "https://stable.example/",
                        "run": run,
                        "ok": right_ok,
                        "category": "http-error",
                        "response_status": 500,
                    },
                    {
                        "target": "left",
                        "rank": 2,
                        "domain": "flaky.example",
                        "url": "https://flaky.example/",
                        "run": run,
                        "ok": run != 2,
                        "category": "success-content" if run != 2 else "timeout",
                        "response_status": 200 if run != 2 else None,
                    },
                    {
                        "target": "right",
                        "rank": 2,
                        "domain": "flaky.example",
                        "url": "https://flaky.example/",
                        "run": run,
                        "ok": True,
                        "category": "success-content",
                        "response_status": 200,
                    },
                ]
            )

        outcomes = _build_site_outcomes(rows)  # type: ignore[arg-type]
        comparisons = _pairwise_site_comparisons(
            outcomes,
            ("left", "right"),
            runs=3,
        )
        by_target_rank = {
            (row["target"], row["rank"]): row for row in outcomes
        }

        self.assertEqual(by_target_rank[("left", 1)]["outcome"], "all-pass")
        self.assertEqual(by_target_rank[("right", 1)]["outcome"], "all-fail")
        self.assertEqual(by_target_rank[("left", 2)]["outcome"], "flaky")
        self.assertAlmostEqual(
            by_target_rank[("left", 2)]["pass_rate_percent"],
            200 / 3,
        )
        self.assertTrue(comparisons[0]["repeat_validated"])
        self.assertEqual(comparisons[0]["left_all_pass_right_all_fail"], 1)
        self.assertEqual(comparisons[0]["flaky_or_incomplete"], 1)
        self.assertEqual(
            comparisons[0]["left_only_sites"],
            [{"rank": 1, "domain": "stable.example"}],
        )

    def test_builtin_seed_lists_are_benchmark_local(self) -> None:
        for source in TOP_SITES_SOURCES:
            resolved, path = resolve_top_sites_source(source, None)
            self.assertEqual(resolved, source)
            self.assertTrue(path.is_relative_to(TOP_SITES_FIXTURE_ROOT))
            self.assertEqual(path.suffix, ".csv")
            self.assertTrue(path.is_file(), f"missing benchmark seed fixture: {path}")

        for source in COMPOSITE_TOP_SITES_SOURCES:
            resolved, path = resolve_top_sites_source(source, None)
            self.assertEqual(resolved, source)
            self.assertEqual(path.parent, TOP_SITES_FIXTURE_ROOT)

    def test_parse_top_sites_list_reads_top_100_section(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "list.md"
            path.write_text(
                "# header\n\n"
                "## Method Notes\n- skip\n\n"
                "## Top 100\n\n"
                "1. `zhihu.com` — Q&A\n"
                "2. `tieba.baidu.com` — forum\n\n"
                "## Other\n"
                "1. `ignored.com` — should not appear\n",
                encoding="utf-8",
            )
            entries = parse_top_sites_list(path)
        self.assertEqual(entries, [(1, "zhihu.com"), (2, "tieba.baidu.com")])

    def test_parse_top_sites_list_uses_first_top_section(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "list.md"
            path.write_text(
                "## Top 100\n\n"
                "1. `first.example` — intended seed\n\n"
                "## Top 10 Most Common Errors\n\n"
                "1. `wrong-1.example`\n"
                "2. `wrong-2.example`\n",
                encoding="utf-8",
            )
            entries = parse_top_sites_list(path)
        self.assertEqual(entries, [(1, "first.example")])

    def test_parse_top_sites_list_reads_csv(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "list.csv"
            path.write_text(
                "rank,target\n"
                "1,example.com\n"
                '2,"https://example.com/search?q=one,two"\n',
                encoding="utf-8",
            )
            entries = parse_top_sites_list(path)
        self.assertEqual(
            entries,
            [(1, "example.com"), (2, "https://example.com/search?q=one,two")],
        )

    def test_legacy_encoding_source_is_registered_and_parses(self) -> None:
        source, path = resolve_top_sites_source("legacy-encoding", None)
        entries = parse_top_sites_list(path)

        self.assertEqual(source, "legacy-encoding")
        self.assertGreaterEqual(len(entries), 6)
        self.assertIn(
            (1, "https://www.aozora.gr.jp/cards/000081/files/456_15050.html"),
            entries,
        )
        self.assertIn("legacy-encoding", TOP_SITES_SOURCES)

    def test_top_command_for_moli_uses_domcontentloaded(self) -> None:
        command = _top_command_for_target("moli", Path("/bin/moli"), "https://example.test/", 12.0)
        self.assertEqual(command[0], "/bin/moli")
        self.assertNotIn("--layout", command)
        self.assertNotIn("--resource", command)
        self.assertIn("--wait-until", command)
        self.assertEqual(command[command.index("--wait-until") + 1], "domcontentloaded")
        self.assertEqual(command[command.index("--wait-script") + 1], POST_DCL_WAIT_SCRIPT)
        self.assertEqual(command[command.index("--timeout") + 1], "12000")
        self.assertEqual(command[command.index("--http-timeout") + 1], "12000")
        self.assertEqual(command[-1], "https://example.test/")

    def test_top_command_for_moli_full_enables_layout_and_all_resource_fetch(self) -> None:
        command = _top_command_for_target("moli-full", Path("/bin/moli"), "https://example.test/", 12.0)
        self.assertEqual(command[0], "/bin/moli")
        self.assertIn("--layout", command)
        self.assertIn("--resource", command)
        self.assertLess(command.index("--layout"), command.index("--dump"))
        self.assertLess(command.index("--resource"), command.index("--dump"))
        self.assertEqual(command[command.index("--wait-until") + 1], "domcontentloaded")
        self.assertEqual(command[command.index("--wait-script") + 1], POST_DCL_WAIT_SCRIPT)
        self.assertEqual(command[-1], "https://example.test/")

    def test_top_command_for_moli_omits_page_waits_for_explicit_pdf(self) -> None:
        command = _top_command_for_target(
            "moli",
            Path("/bin/moli"),
            "https://example.test/report.PDF?download=1",
            12.0,
        )

        self.assertNotIn("--wait-until", command)
        self.assertNotIn("--wait-script", command)
        self.assertEqual(command[command.index("--timeout") + 1], "12000")
        self.assertEqual(command[command.index("--http-timeout") + 1], "12000")
        self.assertEqual(command[-1], "https://example.test/report.PDF?download=1")

    def test_top_command_for_lightpanda_aligns_wait_and_http_timeouts(self) -> None:
        command = _top_command_for_target("lightpanda", Path("/bin/lightpanda"), "https://example.test/", 12.0)
        self.assertEqual(command[0], "/bin/lightpanda")
        self.assertIn("--wait-until", command)
        self.assertEqual(command[command.index("--wait-until") + 1], "domcontentloaded")
        self.assertEqual(command[command.index("--wait-script") + 1], POST_DCL_WAIT_SCRIPT)
        self.assertEqual(command[command.index("--wait-ms") + 1], "12000")
        self.assertEqual(command[command.index("--http-timeout") + 1], "12000")
        self.assertEqual(command[command.index("--terminate-ms") + 1], "12000")
        self.assertEqual(command[-1], "https://example.test/")

    def test_top_sites_chrome_uses_cdp_dcl_metadata(self) -> None:
        self.assertEqual(_top_sites_target_metadata("chrome")["driver"], "cdp-dcl")
        self.assertEqual(_top_sites_target_metadata("chrome")["label"], "chrome / cdp-dcl")
        self.assertEqual(_top_sites_target_metadata("moli-full-cdp")["driver"], "cdp-dcl")
        self.assertEqual(
            _top_sites_target_metadata("moli-full-cdp")["label"],
            "moli full / cdp-dcl",
        )
        with self.assertRaisesRegex(RuntimeError, "CDP DCL runner"):
            _top_command_for_target("chrome", Path("/bin/chromium"), "https://example.test/", 12.0)
        with self.assertRaisesRegex(RuntimeError, "CDP target"):
            _top_command_for_target("moli-full-cdp", Path("/bin/moli"), "https://example.test/", 12.0)

    def test_top_sites_accepts_moli_cdp_dcl_target(self) -> None:
        list_md = "## Top 100\n\n1. `example.test` — synthetic cdp row\n"

        def fake_cdp_dump(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=["/bin/moli", "serve", "--port", "12345"],
                returncode=0,
                elapsed_ms=12.0,
                stdout=b"<html><body>" + b"x" * 1024 + b"</body></html>",
                stderr=b"",
                timed_out=False,
                resources={"peak_pss_bytes": 4096, "peak_rss_bytes": 8192},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch("moli_benchmark.top_sites.run_served_cdp_dcl_dump", side_effect=fake_cdp_dump):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/moli"}},
                    targets=("moli-cdp",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli-cdp",
                    parallelism=1,
                    limit_override=1,
                )

        self.assertEqual(summary["gate_failures"], 0)
        self.assertEqual(summary["targets"]["moli-cdp"]["driver"], "cdp-dcl")
        self.assertEqual(summary["targets"]["moli-cdp"]["label"], "moli / cdp-dcl")
        self.assertEqual(summary["targets"]["moli-cdp"]["passes"], 1)

    def test_top_sites_accepts_moli_full_cdp_dcl_target(self) -> None:
        list_md = "## Top 100\n\n1. `example.test` — synthetic cdp row\n"

        def fake_cdp_dump(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=[
                    "/bin/moli",
                    "serve",
                    "--layout",
                    "--resource",
                    "--port",
                    "12345",
                ],
                returncode=0,
                elapsed_ms=12.0,
                stdout=b"<html><body>" + b"x" * 1024 + b"</body></html>",
                stderr=b"",
                timed_out=False,
                resources={"peak_pss_bytes": 4096, "peak_rss_bytes": 8192},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch("moli_benchmark.top_sites.run_served_cdp_dcl_dump", side_effect=fake_cdp_dump):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/moli"}},
                    targets=("moli-full-cdp",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli-full-cdp",
                    parallelism=1,
                    limit_override=1,
                )

        self.assertEqual(summary["gate_failures"], 0)
        self.assertEqual(summary["targets"]["moli-full-cdp"]["driver"], "cdp-dcl")
        self.assertEqual(summary["targets"]["moli-full-cdp"]["label"], "moli full / cdp-dcl")
        self.assertEqual(summary["targets"]["moli-full-cdp"]["passes"], 1)

    def test_top_sites_accepts_obscura_cdp_dcl_target(self) -> None:
        list_md = "## Top 100\n\n1. `example.test` — synthetic cdp row\n"

        def fake_cdp_dump(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=["/bin/obscura", "serve", "--port", "12345"],
                returncode=0,
                elapsed_ms=12.0,
                stdout=b"<html><body>" + b"x" * 1024 + b"</body></html>",
                stderr=b"",
                timed_out=False,
                resources={"peak_pss_bytes": 4096, "peak_rss_bytes": 8192},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch(
                "moli_benchmark.top_sites.run_served_cdp_dcl_dump",
                side_effect=fake_cdp_dump,
            ) as cdp_dump:
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"obscura": {"available": True, "path": "/bin/obscura"}},
                    targets=("obscura-cdp",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="obscura-cdp",
                    parallelism=1,
                    limit_override=1,
                )

        cdp_dump.assert_called_once()
        self.assertEqual(cdp_dump.call_args.args[:2], ("obscura-cdp", Path("/bin/obscura")))
        self.assertEqual(summary["gate_failures"], 0)
        self.assertEqual(summary["targets"]["obscura-cdp"]["driver"], "cdp-dcl")
        self.assertEqual(summary["targets"]["obscura-cdp"]["label"], "obscura / cdp-dcl")
        self.assertEqual(summary["targets"]["obscura-cdp"]["passes"], 1)

    def test_classify_buckets_thin_responses(self) -> None:
        self.assertEqual(_classify(b"x" * 16, b"", 0, False, 256), "app-shell-only")
        self.assertEqual(_classify(b"x" * 1024, b"", 0, False, 256), "success-content")
        self.assertEqual(_classify(b"", b"", 0, False, 256), "empty-response")
        self.assertEqual(_classify(b"", b"err", 1, False, 256), "process-error")
        self.assertEqual(_classify(b"", b"HTTP request `https://example.test/` returned 403 Forbidden", 1, False, 256), "blocked-or-forbidden")
        self.assertEqual(_classify(b"", b"HTTP request `https://example.test/` returned 404 Not Found", 1, False, 256), "not-found")
        self.assertEqual(_classify(b"", b"", None, True, 256), "timeout")

    def test_classify_recognizes_lightpanda_fatal_fetch_timeout(self) -> None:
        stderr = (
            b'$scope=app $level=fatal $msg="fetch error" err=Timeout '
            b"url=https://example.test/data.json"
        )

        self.assertEqual(_classify(b"", stderr, 0, False, 256), "timeout")

    def test_classify_preserves_cli_main_document_status_and_dns_reason(self) -> None:
        self.assertEqual(
            _classify(
                b"",
                b"Reason: lifecycle target document `https://example.test/` "
                b"returned 429 Too Many Requests",
                1,
                False,
                256,
            ),
            "http-error",
        )
        self.assertEqual(
            _classify(
                b"",
                b"Reason: lifecycle target document `https://example.test/` "
                b"returned 401 Unauthorized",
                1,
                False,
                256,
            ),
            "blocked-or-forbidden",
        )
        self.assertEqual(
            _classify(
                b"",
                b"Reason: failed to resolve example.test:443: failed to lookup "
                b"address information",
                1,
                False,
                256,
            ),
            "network-error",
        )

    def test_classify_uses_main_document_status_before_body_size(self) -> None:
        large_error_body = (
            b"<html><head><title>upstream response</title></head><body>"
            + b"error details " * 100
            + b"</body></html>"
        )

        for status in (400, 500, 502):
            with self.subTest(status=status):
                self.assertEqual(
                    _classify(
                        large_error_body,
                        b"",
                        0,
                        False,
                        256,
                        response_status=status,
                    ),
                    "http-error",
                )
        self.assertEqual(
            _classify(
                large_error_body,
                b"",
                0,
                False,
                256,
                response_status=403,
            ),
            "blocked-or-forbidden",
        )
        self.assertEqual(
            _classify(
                large_error_body,
                b"",
                0,
                False,
                256,
                response_status=404,
            ),
            "not-found",
        )

    def test_classify_infers_http_error_documents_when_cli_hides_status(self) -> None:
        cases = [
            (
                b"<html><head><title>Error: 500 | Service</title></head>"
                b"<body>There was a technical error.</body></html>",
                "http-error",
            ),
            (
                b"<html><head><title>502 Bad Gateway</title></head>"
                b"<body>Origin unavailable.</body></html>",
                "http-error",
            ),
            (
                b"<html><head><title></title></head><body><pre>"
                b'{"error":{"message":"invalid query"}}'
                b"</pre></body></html>",
                "http-error",
            ),
            (
                b"<html><head><title>ResearchGate - Temporarily Unavailable</title></head>"
                b"<body>Access restricted due to unusual activity.</body></html>",
                "blocked-or-forbidden",
            ),
        ]

        for html, expected in cases:
            with self.subTest(expected=expected, html=html):
                self.assertEqual(_classify(html, b"", 0, False, 256), expected)

    def test_classify_distinguishes_tls_verification_from_human_verification(self) -> None:
        tls_error = (
            b"<html><body><h1>Navigation failed</h1>"
            b"<p>Reason: PeerFailedVerification</p></body></html>"
        )
        human_verification = (
            "<html><body>环境异常 当前环境异常，完成验证后即可继续访问。去验证</body></html>".encode()
        )

        self.assertEqual(_classify(tls_error, b"", 0, False, 256), "network-error")
        self.assertEqual(
            _classify(human_verification, b"", 0, False, 256),
            "captcha-or-verification",
        )

    def test_classify_recognizes_chinese_not_found_document(self) -> None:
        html = (
            "<html><head><title>出错了！</title></head>"
            "<body>404 很抱歉，你访问的页面不存在或暂时无法访问！</body></html>"
        ).encode()

        self.assertEqual(_classify(html, b"", 0, False, 256), "not-found")

    def test_visible_text_excludes_document_head(self) -> None:
        snapshot = _extract_title_and_text(
            b"<html><head><title>A very long title</title></head>"
            b"<body>short body</body></html>"
        )

        self.assertEqual(snapshot["title"], "A very long title")
        self.assertEqual(snapshot["text_sample"], "short body")
        self.assertEqual(snapshot["text_length"], len("short body"))

    def test_classify_treats_browser_network_error_pages_as_failures(self) -> None:
        html = (
            b"<html><head><title>www.example.test</title></head><body>"
            b"This site can't be reached ERR_NAME_NOT_RESOLVED"
            + b"x" * 512
            + b"</body></html>"
        )
        self.assertEqual(_classify(html, b"", 0, False, 256), "network-error")

    def test_classify_treats_browser_privacy_interstitials_as_network_errors(
        self,
    ) -> None:
        html = (
            b"<html><head><title>Privacy error</title></head><body>"
            b"Your connection is not private. net::ERR_CERT_AUTHORITY_INVALID "
            + b"x" * 512
            + b"</body></html>"
        )
        self.assertEqual(_classify(html, b"", 0, False, 256), "network-error")

    def test_classify_treats_cli_network_errors_as_failures(self) -> None:
        stderr = b"curl request failed for https://example.test/: [6] Could not resolve host"
        self.assertEqual(_classify(b"", stderr, 1, False, 256), "network-error")

    def test_classify_ignores_subresource_network_logs_when_main_content_loaded(self) -> None:
        html = (
            b"<html><head><title>Example Article</title></head><body>"
            + b"real article content " * 80
            + b"</body></html>"
        )
        stderr = b"ssl_client_socket_impl.cc handshake failed; net_error -101"
        self.assertEqual(_classify(html, stderr, 0, False, 256), "success-content")

    def test_classify_treats_cli_wait_until_deadline_as_timeout(self) -> None:
        stderr = (
            b"WARN fetch_allow_http_error_with_wait_until timed out url=https://example.test/ "
            b"wait_until=DomContentLoaded timeout_ms=60000 stage=DomContentLoaded\n"
            b"Error: failed to fetch `https://example.test`\n"
            b"Caused by: fetch allow-http-error wait_until DomContentLoaded timed out after 60000 ms"
        )
        self.assertEqual(_classify(b"", stderr, 1, False, 256), "timeout")

    def test_classify_treats_cli_document_wait_until_deadline_as_timeout(self) -> None:
        stderr = (
            b"WARN fetch_document_allow_http_error_with_wait_until timed out "
            b"url=https://example.test/ wait_until=DomContentLoaded timeout_ms=30000 "
            b"stage=DomContentLoaded\n"
            b"Error: failed to fetch `https://example.test`\n"
            b"Caused by:\n"
            b"    fetch document allow-http-error wait_until DomContentLoaded "
            b"timed out after 30000 ms for `https://example.test/`"
        )
        self.assertEqual(_classify(b"", stderr, 1, False, 256), "timeout")

    def test_classify_treats_cli_wait_until_deadline_without_allow_http_error_as_timeout(
        self,
    ) -> None:
        stderr = (
            b"Error: failed to fetch `https://example.test`\n"
            b"Caused by: fetch wait_until DomContentLoaded timed out after 60000 ms"
        )
        self.assertEqual(_classify(b"", stderr, 1, False, 256), "timeout")

    def test_classify_treats_lightpanda_navigation_operation_timeout_as_timeout(self) -> None:
        html = (
            b'<!DOCTYPE html><html><head><meta charset="utf-8"></head><body>'
            b"<h1>Navigation failed</h1><p>Reason: OperationTimedout</p>"
            b"</body></html>"
        )
        stderr = b'$scope=frame $level=error $msg="navigate failed" err=OperationTimedout type=root'
        self.assertEqual(_classify(html, stderr, 0, False, 256), "timeout")

    def test_classify_treats_lightpanda_navigation_transport_pages_as_network_errors(
        self,
    ) -> None:
        for reason in ("CouldntResolveHost", "Http2Stream"):
            with self.subTest(reason=reason):
                html = (
                    "<!doctype html><body><h1>Navigation failed</h1>"
                    f"<p>Reason: {reason}</p></body>"
                ).encode()
                stderr = f'$msg="navigate failed" err={reason} type=root'.encode()
                self.assertEqual(
                    _classify(html, stderr, 0, False, 256),
                    "network-error",
                )

    def test_classify_ignores_lightpanda_subresource_operation_timeout_when_main_content_loaded(
        self,
    ) -> None:
        html = (
            b"<html><head><title>Example Article</title></head><body>"
            + b"real article content " * 80
            + b"</body></html>"
        )
        stderr = b'$scope=http $level=warn $msg="script fetch error" err=OperationTimedout'
        self.assertEqual(_classify(html, stderr, 0, False, 256), "success-content")

    def test_classify_ignores_blocked_terms_outside_title(self) -> None:
        html = b"<html><head><title>example article</title></head><body>blocked " + b"x" * 512 + b"</body></html>"
        self.assertEqual(_classify(html, b"", 0, False, 256), "success-content")

    def test_classify_ignores_article_titles_about_403(self) -> None:
        html = (
            b"<html><head><title>understanding http 403 errors</title></head><body>"
            + b"x" * 512
            + b"</body></html>"
        )
        self.assertEqual(_classify(html, b"", 0, False, 256), "success-content")

    def test_classify_treats_blocked_titles_as_blocked(self) -> None:
        titles = [
            "request blocked",
            "403 forbidden",
            "forbidden",
            "access restricted",
            "请求的内容被WAF拦截",
        ]
        for title in titles:
            with self.subTest(title=title):
                html = (
                    f"<html><head><title>{title}</title></head><body>".encode("utf-8")
                    + b"x" * 512
                    + b"</body></html>"
                )
                self.assertEqual(_classify(html, b"", 0, False, 256), "blocked-or-forbidden")

    def test_classify_detects_challenge_and_login_pages(self) -> None:
        captcha = b"<html><title>Verify</title><body>captcha verification required</body></html>"
        login = b"<html><title>Sign in</title><body>please log in</body></html>"
        chinese_login = (
            "<html><title>登录</title><body>账号登录 密码登录 手机号 短信验证码 登录 注册</body></html>".encode(
                "utf-8"
            )
        )
        shell = b"<html><body><div id=\"root\"></div><script>webpackJsonp=[]</script></body></html>"
        self.assertEqual(_classify(captcha, b"", 0, False, 256), "captcha-or-verification")
        self.assertEqual(_classify(login, b"", 0, False, 256), "login-wall")
        self.assertEqual(_classify(chinese_login, b"", 0, False, 256), "login-wall")
        self.assertEqual(_classify(shell, b"", 0, False, 16), "app-shell-only")

    def test_classify_rejects_robot_and_access_denied_false_positives(self) -> None:
        robot = (
            b"<html><body>JavaScript is disabled. In order to continue, we need to verify "
            b"that you're not a robot. This requires JavaScript. Enable JavaScript and then reload the page.</body></html>"
        )
        access_denied = (
            b"<html><head><title>adidas</title></head><body>Reference Error: 0.123. "
            b"Unfortunately we are unable to give you access to our site at this time. "
            b"A security issue was automatically identified.</body></html>"
        )
        bot_check = b"<html><head><title>Bot check</title></head><body>Javascript is needed to access this site</body></html>"
        aws_waf = b"<html><head><title></title><script>AwsWafIntegration.getToken()</script></head><body></body></html>"
        aliyun_waf = (
            b"<html><head><title></title><meta name=\"aliyun_waf_bb\" content=\"token\"></head>"
            b"<body><textarea>"
            + b"x" * 512
            + b"</textarea><script>window._waf_bd8ce2ce37='token'</script></body></html>"
        )
        vercel_checkpoint = b"<html><head><title>Vercel Security Checkpoint</title></head><body>We're verifying your browser</body></html>"
        bot_or_not = b"<html><head><title>Bot or Not?</title></head><body>We can't tell if you're a human or a bot.</body></html>"
        self.assertEqual(_classify(robot, b"", 0, False, 256), "captcha-or-verification")
        self.assertEqual(_classify(access_denied, b"", 0, False, 256), "blocked-or-forbidden")
        self.assertEqual(_classify(bot_check, b"", 0, False, 256), "js-challenge")
        self.assertEqual(_classify(aws_waf, b"", 0, False, 256), "js-challenge")
        self.assertEqual(_classify(aliyun_waf, b"", 0, False, 256), "js-challenge")
        self.assertEqual(_classify(vercel_checkpoint, b"", 0, False, 256), "js-challenge")
        self.assertEqual(_classify(bot_or_not, b"", 0, False, 256), "captcha-or-verification")

    def test_classify_recognizes_safeline_and_privacy_tile_challenges(self) -> None:
        safeline = (
            b"<html><body>Security Detection Powered By SafeLine WAF "
            + b"Confirm " * 100
            + b"</body></html>"
        )
        privacy_tiles = (
            b"<html><body>Select the 2 matching tiles A B C D Submit "
            b"Powered and protected by Privacy "
            + b"challenge " * 100
            + b"</body></html>"
        )

        self.assertEqual(
            _classify(safeline, b"", 0, False, 256),
            "blocked-or-forbidden",
        )
        self.assertEqual(
            _classify(privacy_tiles, b"", 0, False, 256),
            "captcha-or-verification",
        )

    def test_classify_treats_c2wf_probe_shell_as_js_challenge(self) -> None:
        probe_v1 = b"""<!DOCTYPE html><html><head>
  <meta charset="UTF-8">
  <script>
    var buid = "fffffffffffffffffff"
  </script>
  <script src="/C2WF946J0/probe.js?v=vc1jasc"></script>
</head><body></body></html>"""
        probe_v3 = b'<html><head><script src="/C2WF946J0/probev3.js" r="m"></script></head></html>'

        self.assertEqual(_classify(probe_v1, b"", 0, False, 256), "js-challenge")
        self.assertEqual(_classify(probe_v3, b"", 0, False, 256), "js-challenge")

    def test_classify_treats_tencent_chaos_shell_as_js_challenge(self) -> None:
        chaos_shell = b"""<!DOCTYPE html><html><head></head><body>
<script>
var __TENCENT_CHAOS_VM=function(){return function(){}};
window.solveChallenge("payload", "token#hash");
</script></body></html>"""
        edgeone_sdk_shell = b"""<!DOCTYPE html><html><head>
<script src="https://captcha.eo.gtimg.com/TEOJsChallengeSdk.js"></script>
</head><body><script>
var __EO_JSCHALLENGE_VM=function(){return function(){}};
if(window.EOJsChallengeSDK){new window.EOJsChallengeSDK({callback:function(){}}).start();}
</script></body></html>"""

        self.assertEqual(_classify(chaos_shell, b"", 0, False, 256), "js-challenge")
        self.assertEqual(_classify(edgeone_sdk_shell, b"", 0, False, 256), "js-challenge")

    def test_classify_rejects_not_found_and_empty_visible_text(self) -> None:
        not_found = (
            b"<html><head><title>Page not found - Example</title></head><body>"
            b"THIS PAGE CANNOT BE FOUND. The page you requested is missing."
            + b"x" * 512
            + b"</body></html>"
        )
        file_not_found = (
            b"<html><head><title>File not found - GitHub</title></head><body>"
            b"File not found"
            + b"x" * 512
            + b"</body></html>"
        )
        empty_visible = b"<html><head><title></title></head><body><script>" + b"x" * 1024 + b"</script></body></html>"
        self.assertEqual(_classify(not_found, b"", 0, False, 256), "not-found")
        self.assertEqual(_classify(file_not_found, b"", 0, False, 256), "not-found")
        self.assertEqual(_classify(empty_visible, b"", 0, False, 256), "app-shell-only")

    def test_classify_rejects_login_form_shells_even_when_large(self) -> None:
        login_shell = (
            b"<html><head><title>Example</title></head><body>"
            b"Login to your account Email/Username Your password is a required field. "
            b"Forgot password? Create a Free Account "
            + b"navigation " * 200
            + b"</body></html>"
        )
        self.assertEqual(_classify(login_shell, b"", 0, False, 256), "login-wall")

    def test_classify_accepts_contentful_shell_pages_with_login_nav(self) -> None:
        html = (
            b"<html><head><title>bilibili</title></head><body>"
            b"<div id=\"app\"></div>"
            + (
                "首页 番剧 直播 游戏中心 下载客户端 登录 "
                "万事俱备 宁缺桑桑开启长案生活 71.9万 3223 06:38 "
                "你为什么要当兵 小孩哥的回答火遍全网 央视军事 "
                "云南的水果像极了AI生成 这个假期也来云南感受一下 "
                "外国女友第一次来到上海 科技数码 资讯 美食 小剧场 "
                "春日遛娃高能整活现场 不感兴趣 将减少此类内容推荐 "
                "添加至稍后再看 热门推荐 影视 娱乐 知识 生活经验 "
            ).encode("utf-8") * 5
            + b"</body></html>"
        )
        self.assertEqual(_classify(html, b"", 0, False, 256), "success-content")

    def test_classify_accepts_short_search_homepage_with_sign_in_nav(self) -> None:
        html = (
            b"<html><head><title>Search - Microsoft Bing</title></head><body>"
            b"Images Videos Translate Maps Academic Dictionary MSN Online Games "
            b"Microsoft 365 Outlook Word Excel PowerPoint OneNote Sway OneDrive "
            b"Calendar People Get to Know Bing Domestic International "
            b"Get the Bing Wallpaper app A journey through time Sign in "
            b"Privacy and Cookies Legal Advertise About our ads Help Feedback "
            + b"result " * 30
            + b"</body></html>"
        )
        self.assertEqual(_classify(html, b"", 0, False, 256), "success-content")

    def test_classify_treats_large_pdf_as_success_binary_content(self) -> None:
        pdf = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n" + b"x" * 1024
        self.assertEqual(_classify(pdf, b"", 0, False, 256), "success-binary-content")
        self.assertIn("success-binary-content", _ok_categories())

    def test_classify_keeps_partial_raw_binary_body_timeout_as_network_error(
        self,
    ) -> None:
        stderr = (
            b"Error: failed to fetch `https://example.test/file.pdf`\n"
            b"Caused by:\n"
            b"    0: failed to read raw document body for `https://example.test/file.pdf`\n"
            b"    1: curl request failed for https://example.test/file.pdf\n"
            b"    2: [28] Timeout was reached (Operation timed out after 30000 milliseconds "
            b"with 3276266 out of 12159912 bytes received)\n"
        )
        self.assertEqual(
            _classify(b"", stderr, 1, False, 256),
            "network-error",
        )
        self.assertNotIn("success-binary-main-resource", _ok_categories())

    def test_classify_marks_cdp_binary_headers_as_not_a_body_success(self) -> None:
        self.assertEqual(
            _classify(
                b"",
                b"",
                0,
                False,
                256,
                response_status=200,
                main_document_body_capture="response-headers-only",
            ),
            "binary-response-headers",
        )
        self.assertNotIn("binary-response-headers", _ok_categories())

    def test_classify_keeps_raw_binary_header_timeout_as_network_error(self) -> None:
        stderr = (
            b"Error: failed to fetch `https://example.test/file.pdf`\n"
            b"Caused by:\n"
            b"    0: failed to read raw document body for `https://example.test/file.pdf`\n"
            b"    1: curl request failed for https://example.test/file.pdf\n"
            b"    2: [28] Timeout was reached (Operation timed out after 30000 milliseconds)\n"
        )
        self.assertEqual(_classify(b"", stderr, 1, False, 256), "network-error")

    def test_elapsed_failure_timeout_floor_tracks_benchmark_deadline(self) -> None:
        self.assertTrue(_elapsed_failure_reached_timeout(29_000.0, 30.0))
        self.assertFalse(_elapsed_failure_reached_timeout(28_999.0, 30.0))
        self.assertTrue(_elapsed_failure_reached_timeout(950.0, 1.0))
        self.assertFalse(_elapsed_failure_reached_timeout(949.0, 1.0))

    def test_run_top_sites_suite_writes_summary_and_failure_artifact(self) -> None:
        list_md = (
            "## Top 100\n\n"
            "1. `pass.example` — synthetic pass row\n"
            "2. `fail.example` — synthetic fail row\n"
        )

        def fake_run(command: list[str], **kwargs: object) -> ProcessResult:
            url = command[-1]
            stdout = b"<html><body>" + b"x" * 1024 + b"</body></html>" if "pass.example" in url else b""
            returncode = 0 if "pass.example" in url else 1
            return ProcessResult(
                command=command,
                returncode=returncode,
                elapsed_ms=42.0,
                stdout=stdout,
                stderr=b"" if returncode == 0 else b"boom",
                timed_out=False,
                resources={"peak_pss_bytes": 1024, "peak_rss_bytes": 2048},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch("moli_benchmark.top_sites.run_process", fake_run):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/echo"}},
                    targets=("moli",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli",
                    parallelism=1,
                    chrome_parallelism=2,
                    limit_override=2,
                )
            failure_files = sorted(p.name for p in (output_dir / "top-sites" / "failures").iterdir())
            summary_payload = (output_dir / "top-sites" / "summary.json").read_text(encoding="utf-8")
            run_rows = json.loads((output_dir / "top-sites" / "runs.json").read_text(encoding="utf-8"))
            site_outcomes = json.loads(
                (output_dir / "top-sites" / "site-outcomes.json").read_text(
                    encoding="utf-8"
                )
            )

        self.assertEqual(summary["site_count"], 2)
        self.assertEqual(summary["gate_failures"], 1)
        self.assertEqual(summary["targets"]["moli"]["passes"], 1)
        self.assertEqual(summary["targets"]["moli"]["failures"], 1)
        self.assertEqual(summary["targets"]["moli"]["raw_observations"], 2)
        self.assertEqual(summary["targets"]["moli"]["raw_comparable_attempts"], 2)
        self.assertEqual(summary["targets"]["moli"]["raw_passes"], 1)
        self.assertEqual(summary["targets"]["moli"]["raw_failures"], 1)
        self.assertEqual(summary["targets"]["moli"]["raw_pass_rate_percent"], 50.0)
        self.assertEqual(summary["targets"]["moli"]["pass_rate_percent"], 50.0)
        self.assertEqual(summary["targets"]["moli"]["elapsed_ms"]["count"], 2)
        self.assertEqual(
            summary["targets"]["moli"]["successful_elapsed_ms"]["count"],
            1,
        )
        self.assertEqual(summary["common_success"]["attempts"], 1)
        self.assertEqual(
            summary["common_success"]["targets"]["moli"]["elapsed_ms"]["count"],
            1,
        )
        self.assertEqual(summary["targets"]["moli"]["unique_sites"], 2)
        self.assertEqual(summary["targets"]["moli"]["all_pass_sites"], 1)
        self.assertEqual(summary["targets"]["moli"]["all_fail_sites"], 1)
        self.assertEqual(summary["targets"]["moli"]["flaky_sites"], 0)
        self.assertFalse(summary["repeat_validated"])
        self.assertEqual(summary["targets"]["moli"]["failure_kinds"], {"process-error": 1})
        self.assertEqual(summary["targets"]["moli"]["categories"]["success-content"], 1)
        self.assertEqual(summary["targets"]["moli"]["peak_rss_bytes"]["max"], 2048.0)
        self.assertEqual(summary["chrome_parallelism"], 2)
        self.assertEqual(summary["schedule"], "site-paired-rotating-target-order")
        self.assertEqual(summary["scheduled_site_groups"], 2)
        self.assertEqual(run_rows[0]["command"][0], "/bin/echo")
        self.assertEqual(run_rows[0]["peak_rss_bytes"], 2048)
        self.assertEqual(run_rows[0]["schedule_index"], 0)
        self.assertEqual(run_rows[0]["target_order_index"], 1)
        self.assertTrue(run_rows[0]["started_at"].endswith("Z"))
        self.assertTrue(run_rows[0]["finished_at"].endswith("Z"))
        self.assertEqual(len(run_rows[0]["stdout_sha256"]), 64)
        self.assertEqual(len(run_rows[0]["stderr_sha256"]), 64)
        self.assertTrue(run_rows[0]["stdout_head"].startswith("<html>"))
        self.assertEqual(len(site_outcomes), 2)
        self.assertEqual(
            {row["outcome"] for row in site_outcomes},
            {"all-pass", "all-fail"},
        )
        self.assertIn("moli-run-1-rank002-fail.example.json", failure_files)
        self.assertIn("\"profile\": \"quick\"", summary_payload)

    def test_run_top_sites_suite_records_chrome_http_error_status(self) -> None:
        list_md = "## Top 100\n\n1. `error.example` — synthetic HTTP error row\n"

        def fake_chrome_run(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=["/bin/chromium", "https://error.example/"],
                returncode=0,
                elapsed_ms=995.0,
                stdout=(
                    b"<html><head><title>upstream response</title></head><body>"
                    + b"error details " * 100
                    + b"</body></html>"
                ),
                stderr=b"",
                timed_out=False,
                resources={},
                response_status=502,
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch(
                "moli_benchmark.top_sites.run_chrome_dcl_dump",
                side_effect=fake_chrome_run,
            ):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"chrome": {"available": True, "path": "/bin/chromium"}},
                    targets=("chrome",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="chrome",
                    parallelism=1,
                    chrome_parallelism=1,
                    limit_override=1,
                )
            rows = json.loads(
                (output_dir / "top-sites" / "runs.json").read_text(encoding="utf-8")
            )

        self.assertEqual(rows[0]["response_status"], 502)
        self.assertEqual(rows[0]["category"], "http-error")
        self.assertEqual(rows[0]["response_status_source"], "protocol")
        self.assertEqual(rows[0]["classification_basis"], "protocol-http-status")
        self.assertFalse(rows[0]["ok"])
        self.assertEqual(summary["targets"]["chrome"]["failure_kinds"], {"http-error": 1})

    def test_run_top_sites_suite_excludes_header_only_binary_from_denominator(
        self,
    ) -> None:
        list_md = (
            "## Top 100\n\n"
            "1. `https://document.example/file.pdf` — synthetic PDF row\n"
        )

        def fake_chrome_run(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=["/bin/chromium", "https://document.example/file.pdf"],
                returncode=0,
                elapsed_ms=50.0,
                stdout=b"",
                stderr=b"",
                timed_out=False,
                resources={},
                response_status=200,
                response_mime_type="application/pdf",
                main_document_body_capture="response-headers-only",
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch(
                "moli_benchmark.top_sites.run_chrome_dcl_dump",
                side_effect=fake_chrome_run,
            ):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={
                        "chrome": {"available": True, "path": "/bin/chromium"}
                    },
                    targets=("chrome",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="chrome",
                    parallelism=1,
                    chrome_parallelism=1,
                    limit_override=1,
                )
            rows = json.loads(
                (output_dir / "top-sites" / "runs.json").read_text(
                    encoding="utf-8"
                )
            )

        self.assertEqual(rows[0]["category"], "binary-response-headers")
        self.assertFalse(rows[0]["comparable"])
        self.assertNotIn("failure_artifact", rows[0])
        self.assertEqual(summary["gate_failures"], 0)
        self.assertEqual(summary["targets"]["chrome"]["attempts"], 0)
        self.assertEqual(summary["targets"]["chrome"]["raw_comparable_attempts"], 0)
        self.assertEqual(summary["targets"]["chrome"]["raw_non_comparable_attempts"], 1)
        self.assertEqual(
            summary["targets"]["chrome"]["non_comparable_attempts"],
            1,
        )
        self.assertEqual(summary["targets"]["chrome"]["failures"], 0)
        self.assertEqual(summary["common_success"]["attempts"], 0)

    def test_run_top_sites_suite_records_cli_diagnostic_http_status(self) -> None:
        list_md = "## Top 100\n\n1. `error.example` — synthetic CLI HTTP error row\n"

        def fake_run(command: list[str], **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=command,
                returncode=1,
                elapsed_ms=12.0,
                stdout=b"",
                stderr=(
                    b"Error: failed to fetch `https://error.example/`\n"
                    b"Reason: lifecycle target document `https://error.example/` "
                    b"returned 502 Bad Gateway"
                ),
                timed_out=False,
                resources={},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch("moli_benchmark.top_sites.run_process", fake_run):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/moli"}},
                    targets=("moli",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli",
                    parallelism=1,
                    limit_override=1,
                )
            rows = json.loads(
                (output_dir / "top-sites" / "runs.json").read_text(
                    encoding="utf-8"
                )
            )

        self.assertEqual(rows[0]["response_status"], 502)
        self.assertEqual(rows[0]["response_status_source"], "cli-diagnostic")
        self.assertEqual(rows[0]["category"], "http-error")
        self.assertEqual(
            rows[0]["classification_basis"],
            "cli-diagnostic-http-status",
        )
        self.assertEqual(summary["targets"]["moli"]["status_observed_attempts"], 1)
        self.assertEqual(
            summary["targets"]["moli"]["response_status_sources"],
            {"cli-diagnostic": 1},
        )

    def test_run_top_sites_suite_promotes_deadline_adjacent_failures_to_timeout(
        self,
    ) -> None:
        list_md = "## Top 100\n\n1. `slow-fail.example` — synthetic slow fail row\n"

        def fake_run(command: list[str], **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=command,
                returncode=1,
                elapsed_ms=29_100.0,
                stdout=b"",
                stderr=b"curl request failed: could not resolve host",
                timed_out=False,
                resources={},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch("moli_benchmark.top_sites.run_process", fake_run):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/echo"}},
                    targets=("moli",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=30.0,
                    gate_target="moli",
                    parallelism=1,
                    limit_override=1,
                )
            rows = json.loads((output_dir / "top-sites" / "runs.json").read_text(encoding="utf-8"))

        self.assertEqual(rows[0]["category"], "timeout")
        self.assertEqual(rows[0]["failure_kind"], "timeout")
        self.assertEqual(summary["targets"]["moli"]["failure_kinds"], {"timeout": 1})

    def test_run_top_sites_suite_excludes_cross_target_unreachable_sites(self) -> None:
        list_md = (
            "## Top 100\n\n"
            "1. `pass.example` — synthetic pass row\n"
            "2. `dead.example` — synthetic unreachable row\n"
            "3. `blocked.example` — synthetic reachable blocked row\n"
        )

        def fake_run(command: list[str], **kwargs: object) -> ProcessResult:
            url = command[-1]
            if "pass.example" in url:
                stdout = b"<html><body>" + b"x" * 1024 + b"</body></html>"
                return ProcessResult(
                    command=command,
                    returncode=0,
                    elapsed_ms=10.0,
                    stdout=stdout,
                    stderr=b"",
                    timed_out=False,
                    resources={},
                )
            if "dead.example" in url:
                return ProcessResult(
                    command=command,
                    returncode=1,
                    elapsed_ms=10.0,
                    stdout=b"",
                    stderr=b"curl request failed for https://dead.example/: [6] Could not resolve host",
                    timed_out=False,
                    resources={},
                )
            return ProcessResult(
                command=command,
                returncode=0,
                elapsed_ms=10.0,
                stdout=b"<html><head><title>Access Restricted</title></head><body>" + b"x" * 1024 + b"</body></html>",
                stderr=b"",
                timed_out=False,
                resources={},
            )

        def fake_chrome_run(binary: Path, url: str, **kwargs: object) -> ProcessResult:
            return fake_run([str(binary), url])

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with (
                patch("moli_benchmark.top_sites.run_process", fake_run),
                patch("moli_benchmark.top_sites.run_chrome_dcl_dump", fake_chrome_run),
            ):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={
                        "moli": {"available": True, "path": "/bin/moli"},
                        "chrome": {"available": True, "path": "/bin/chromium"},
                    },
                    targets=("moli", "chrome"),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli",
                    parallelism=1,
                    limit_override=3,
                )
            raw_runs = (output_dir / "top-sites" / "raw-runs.csv").read_text(encoding="utf-8")
            run_rows = json.loads(
                (output_dir / "top-sites" / "runs.json").read_text(
                    encoding="utf-8"
                )
            )

        self.assertEqual(summary["site_count"], 3)
        self.assertEqual(summary["counted_site_count"], 2)
        self.assertEqual(summary["excluded_site_count"], 1)
        self.assertEqual(summary["excluded_sites"], [{"domain": "dead.example", "reason": "site-unreachable"}])
        self.assertEqual(summary["gate_failures"], 1)
        self.assertEqual(summary["targets"]["moli"]["raw_sites"], 3)
        self.assertEqual(summary["targets"]["moli"]["sites"], 2)
        self.assertEqual(summary["targets"]["moli"]["excluded_runs"], 1)
        self.assertEqual(summary["targets"]["moli"]["passes"], 1)
        self.assertEqual(summary["targets"]["moli"]["failures"], 1)
        self.assertIn("site-unreachable", raw_runs)
        order_by_target_rank = {
            (row["target"], row["rank"]): row["target_order_index"]
            for row in run_rows
        }
        self.assertEqual(order_by_target_rank[("moli", 1)], 1)
        self.assertEqual(order_by_target_rank[("chrome", 1)], 2)
        self.assertEqual(order_by_target_rank[("chrome", 2)], 1)
        self.assertEqual(order_by_target_rank[("moli", 2)], 2)

    def test_run_top_sites_suite_keeps_single_target_unreachable_sites_counted(self) -> None:
        list_md = "## Top 100\n\n1. `dead.example` — synthetic unreachable row\n"

        def fake_run(command: list[str], **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=command,
                returncode=1,
                elapsed_ms=10.0,
                stdout=b"",
                stderr=b"curl request failed for https://dead.example/: [6] Could not resolve host",
                timed_out=False,
                resources={},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch("moli_benchmark.top_sites.run_process", fake_run):
                summary = run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/moli"}},
                    targets=("moli",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli",
                    parallelism=1,
                    limit_override=1,
                )

        self.assertEqual(summary["counted_site_count"], 1)
        self.assertEqual(summary["excluded_site_count"], 0)
        self.assertEqual(summary["gate_failures"], 1)
        self.assertEqual(summary["targets"]["moli"]["sites"], 1)
        self.assertEqual(summary["targets"]["moli"]["failures"], 1)

    def test_default_parallelism_falls_back_when_affinity_fails(self) -> None:
        with (
            patch("moli_benchmark.top_sites.os.sched_getaffinity", side_effect=OSError("denied"), create=True),
            patch("moli_benchmark.top_sites.os.cpu_count", return_value=7),
        ):
            self.assertEqual(_default_top_sites_parallelism(), 7)

    def test_default_parallelism_caps_large_cpu_counts(self) -> None:
        with patch("moli_benchmark.top_sites.os.sched_getaffinity", return_value=set(range(31)), create=True):
            self.assertEqual(_default_top_sites_parallelism(), 8)

    def test_failure_artifact_filename_sanitizes_custom_url_domains(self) -> None:
        list_md = "## Top 100\n\n1. `https://evil.example/a/../b?q=x:y` — synthetic fail row\n"

        def fake_run(command: list[str], **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=command,
                returncode=1,
                elapsed_ms=1.0,
                stdout=b"",
                stderr=b"boom",
                timed_out=False,
                resources={},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            list_path = output_dir / "top.md"
            list_path.write_text(list_md, encoding="utf-8")
            with patch("moli_benchmark.top_sites.run_process", fake_run):
                run_top_sites_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/echo"}},
                    targets=("moli",),
                    profile="quick",
                    list_path=list_path,
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli",
                    parallelism=1,
                    limit_override=1,
                )
            failure_files = [path.relative_to(output_dir / "top-sites" / "failures") for path in (output_dir / "top-sites" / "failures").rglob("*")]

        self.assertTrue(failure_files)
        self.assertTrue(all(len(path.parts) == 1 for path in failure_files))
        self.assertTrue(all(".." not in path.name for path in failure_files))
        self.assertTrue(all("/" not in path.name and ":" not in path.name for path in failure_files))

    def test_profiles_include_quick_and_full(self) -> None:
        self.assertIn("quick", TOP_SITES_PROFILES)
        self.assertIn("full", TOP_SITES_PROFILES)
        self.assertIn("webfetch", TOP_SITES_PROFILES)
        self.assertEqual(TOP_SITES_PROFILES["quick"]["limit"], 20)
        self.assertEqual(TOP_SITES_PROFILES["full"]["limit"], 100)
        self.assertEqual(TOP_SITES_PROFILES["webfetch"]["limit"], 300)

    def test_default_source_is_chinese_community(self) -> None:
        self.assertEqual(DEFAULT_TOP_SITES_SOURCE, "chinese-community")
        self.assertIn("chinese-community", TOP_SITES_SOURCES)
        self.assertIn("global", TOP_SITES_SOURCES)
        self.assertIn("webfetch-longtail", TOP_SITES_SOURCES)
        self.assertIn("render-quality", TOP_SITES_SOURCES)
        self.assertIn("webfetch-mix", COMPOSITE_TOP_SITES_SOURCES)

    def test_chinese_default_picks_top_20_and_top_100(self) -> None:
        resolved, _ = resolve_top_sites_source("chinese-community", None)
        self.assertEqual(resolved, "chinese-community")
        entries, labels = load_top_sites_entries(resolved, None)
        domains = {domain for _, domain in entries}
        self.assertEqual(len(entries), 100, "chinese-community list should yield top 100 entries")
        self.assertEqual(entries[0][0], 1)
        self.assertEqual(entries[19][0], 20)
        self.assertEqual(entries[-1][0], 100)
        self.assertIn("www.huxiu.com", domains)
        self.assertEqual(labels, ["chinese-community:chinese-community-top100-websites.csv"])

    def test_global_source_returns_english_world_sites(self) -> None:
        resolved, _ = resolve_top_sites_source("global", None)
        entries, labels = load_top_sites_entries(resolved, None)
        self.assertEqual(len(entries), 100, "global list should yield 100 entries")
        domains = {domain for _, domain in entries}
        self.assertIn("google.com", domains)
        self.assertIn("github.com", domains)
        self.assertIn("wikipedia.org", domains)
        self.assertNotIn("baidu.com", domains, "global list must not contain Chinese top sites")
        self.assertEqual(labels, ["global:global-top-websites-seed-list.csv"])

    def test_mixed_source_interleaves_cn_and_global(self) -> None:
        resolved, list_source = resolve_top_sites_source("mixed", None)
        entries, labels = load_top_sites_entries(resolved, None)
        domains = [domain for _, domain in entries]
        self.assertIn("zhihu.com", domains)
        self.assertIn("google.com", domains)
        first_cn_idx = next(i for i, d in enumerate(domains) if d == "zhihu.com")
        first_gl_idx = next(i for i, d in enumerate(domains) if d == "google.com")
        self.assertLess(abs(first_cn_idx - first_gl_idx), 3, "mixed list should interleave sources")
        self.assertEqual(list_source.name, "mixed-top-websites")
        self.assertEqual(len(labels), 2)

    def test_webfetch_longtail_source_returns_observed_url_paths(self) -> None:
        resolved, _ = resolve_top_sites_source("webfetch-longtail", None)
        entries, labels = load_top_sites_entries(resolved, None)
        urls = [domain for _, domain in entries]
        self.assertEqual(len(entries), 159)
        self.assertTrue(all(url.startswith(("http://", "https://")) for url in urls))
        self.assertIn(
            "https://www.reddit.com/r/MachineLearning/comments/1aqxhol/d_thoughts_on_llama_3/",
            urls,
        )
        self.assertNotIn("https://github.com/golang/go/blob/master/doc/effective_go.md", urls)
        self.assertEqual(labels, ["webfetch-longtail:webfetch-longtail-seed-list.csv"])

    def test_render_quality_source_returns_curated_url_paths(self) -> None:
        resolved, _ = resolve_top_sites_source("render-quality", None)
        entries, labels = load_top_sites_entries(resolved, None)
        urls = [domain for _, domain in entries]
        self.assertEqual(len(entries), 12)
        self.assertTrue(all(url.startswith(("http://", "https://")) for url in urls))
        self.assertIn(
            "https://www.feishu.cn/hc/zh-CN/articles/485770964672-管理者查看企业-ai-功能使用数据",
            urls,
        )
        self.assertEqual(labels, ["render-quality:render-quality-seed-list.csv"])

    def test_webfetch_mix_keeps_top_site_mix_then_appends_longtail(self) -> None:
        resolved, list_source = resolve_top_sites_source("webfetch-mix", None)
        entries, labels = load_top_sites_entries(resolved, None)
        domains = [domain for _, domain in entries]
        top_site_segment = domains[:100]
        longtail_segment = domains[100:]
        self.assertEqual(resolved, "webfetch-mix")
        self.assertEqual(list_source.name, "webfetch-mix-websites")
        self.assertEqual(len(entries), 259)
        self.assertIn("zhihu.com", top_site_segment)
        self.assertIn("google.com", top_site_segment)
        self.assertTrue(all(url.startswith(("http://", "https://")) for url in longtail_segment))
        self.assertIn("https://www.cell.com/cell/fulltext/S0092-8674(24)00500-9", longtail_segment)
        self.assertNotIn(
            "https://www.usatoday.com/story/tech/news/2024/02/15/openai-sora-text-to-video/72601000007/",
            longtail_segment,
        )
        self.assertEqual([rank for rank, _ in entries], list(range(1, len(entries) + 1)))
        self.assertEqual(
            labels,
            [
                "chinese-community:chinese-community-top100-websites.csv",
                "global:global-top-websites-seed-list.csv",
                "webfetch-longtail:webfetch-longtail-seed-list.csv",
            ],
        )

    def test_unknown_source_raises(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "unknown top-sites source"):
            resolve_top_sites_source("nonexistent", None)
        with self.assertRaisesRegex(RuntimeError, "unknown top-sites source"):
            load_top_sites_entries("nonexistent", None)


if __name__ == "__main__":
    unittest.main()
