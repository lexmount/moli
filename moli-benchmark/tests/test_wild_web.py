from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.process import ProcessResult
from moli_benchmark.public_web import POST_DCL_WAIT_SCRIPT
from moli_benchmark.wild_web import (
    _classify,
    _extract_page_snapshot,
    _failure_kind,
    _rotated_target_order,
    _wild_command_for_target,
    _wild_web_target_metadata,
    _wild_web_extraction_failures,
    run_wild_web_suite,
)


class WildWebTests(unittest.TestCase):
    def test_target_order_rotates_per_seed(self) -> None:
        targets = ("moli", "lightpanda", "chrome")

        self.assertEqual(_rotated_target_order(targets, 0), targets)
        self.assertEqual(
            _rotated_target_order(targets, 1),
            ("lightpanda", "chrome", "moli"),
        )

    def test_obscura_wild_web_command_uses_fetch_timeout_seconds(self) -> None:
        command = _wild_command_for_target("obscura", Path("/bin/obscura"), "https://example.test/", 7.9)

        self.assertEqual(command[:4], ["/bin/obscura", "fetch", "--dump", "html"])
        self.assertIn("--wait-until", command)
        self.assertEqual(command[command.index("--wait") + 1], "0")
        self.assertIn("--timeout", command)
        self.assertEqual(command[command.index("--timeout") + 1], "7")
        self.assertEqual(command[-1], "https://example.test/")

    def test_lightpanda_wild_web_command_aligns_fetch_timeouts(self) -> None:
        command = _wild_command_for_target("lightpanda", Path("/bin/lightpanda"), "https://example.test/", 30.0)

        self.assertEqual(command[:4], ["/bin/lightpanda", "fetch", "--dump", "html"])
        self.assertEqual(
            command[command.index("--wait-until") + 1],
            "domcontentloaded",
        )
        self.assertEqual(
            command[command.index("--wait-script") + 1],
            POST_DCL_WAIT_SCRIPT,
        )
        self.assertEqual(command[command.index("--wait-ms") + 1], "30000")
        self.assertEqual(command[command.index("--http-timeout") + 1], "30000")
        self.assertEqual(command[command.index("--terminate-ms") + 1], "30000")
        self.assertNotIn("--wait_until", command)
        self.assertNotIn("--wait_ms", command)
        self.assertNotIn("--http_timeout", command)
        self.assertEqual(command[-1], "https://example.test/")

    def test_moli_wild_web_command_uses_the_same_dcl_boundary(self) -> None:
        command = _wild_command_for_target(
            "moli",
            Path("/bin/moli"),
            "https://example.test/",
            30.0,
        )

        self.assertEqual(
            command[command.index("--wait-until") + 1],
            "domcontentloaded",
        )
        self.assertEqual(
            command[command.index("--wait-script") + 1],
            POST_DCL_WAIT_SCRIPT,
        )
        self.assertEqual(command[command.index("--timeout") + 1], "30000")
        self.assertEqual(command[command.index("--http-timeout") + 1], "30000")

    def test_wild_web_chrome_uses_cdp_dcl_metadata(self) -> None:
        self.assertEqual(_wild_web_target_metadata("chrome")["driver"], "cdp-dcl")
        self.assertEqual(_wild_web_target_metadata("chrome")["label"], "chrome / cdp-dcl")
        self.assertEqual(_wild_web_target_metadata("obscura-cdp")["driver"], "cdp-dcl")
        self.assertEqual(_wild_web_target_metadata("obscura-cdp")["label"], "obscura / cdp-dcl")
        with self.assertRaisesRegex(RuntimeError, "CDP DCL runner"):
            _wild_command_for_target("chrome", Path("/bin/chromium"), "https://example.test/", 7.9)

    def test_wild_web_accepts_obscura_cdp_dcl_target(self) -> None:
        def fake_cdp_dump(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=["/bin/obscura", "serve", "--port", "12345"],
                returncode=0,
                elapsed_ms=12.0,
                stdout="<!doctype html><title>知乎</title><body>知乎 首页 内容 推荐 回答 专栏 热榜 发现 等你来答</body>".encode(),
                stderr=b"",
                timed_out=False,
                resources={"peak_pss_bytes": 123},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            with patch(
                "moli_benchmark.wild_web.run_served_cdp_dcl_dump",
                side_effect=fake_cdp_dump,
            ) as cdp_dump:
                summary = run_wild_web_suite(
                    output_dir=Path(temp_dir),
                    target_matrix={"obscura": {"available": True, "path": "/bin/obscura"}},
                    targets=("obscura-cdp",),
                    seeds=("zhihu-home",),
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="obscura-cdp",
                )

        cdp_dump.assert_called_once()
        self.assertEqual(cdp_dump.call_args.args[:2], ("obscura-cdp", Path("/bin/obscura")))
        self.assertEqual(summary["gate_failures"], 0)
        self.assertEqual(summary["targets"]["obscura-cdp"]["driver"], "cdp-dcl")

    def test_extraction_assertions_accept_expected_seed_keywords(self) -> None:
        snapshot = _extract_page_snapshot(
            """
            <!doctype html>
            <title>知乎 - 有问题，就会有答案</title>
            <body><main>知乎 首页 内容 推荐 回答 专栏 热榜 发现 等你来答</main><script>hidden()</script></body>
            """.encode()
        )

        self.assertEqual(snapshot["title"], "知乎 - 有问题，就会有答案")
        self.assertIn("知乎 首页", snapshot["text_sample"])
        self.assertEqual(_wild_web_extraction_failures("zhihu-home", snapshot), [])

    def test_seed_brand_is_required_in_title_but_not_repeated_in_body(self) -> None:
        snapshot = _extract_page_snapshot(
            b"<!doctype html><title>Baidu</title><body>"
            + b"useful navigation content " * 20
            + b"</body>"
        )

        self.assertEqual(_wild_web_extraction_failures("baidu-home", snapshot), [])

    def test_extraction_assertions_report_missing_business_fields(self) -> None:
        snapshot = _extract_page_snapshot(b"<!doctype html><title>Example</title><body>tiny</body>")

        self.assertEqual(
            _wild_web_extraction_failures("toutiao-home", snapshot),
            ["title-keyword-mismatch", "short-body-text"],
        )
        self.assertEqual(_failure_kind("success", ["title-keyword-mismatch"]), "extraction-failure")

    def test_snapshot_does_not_count_title_as_body_text(self) -> None:
        snapshot = _extract_page_snapshot(
            b"<!doctype html><title>A long title</title><body>short body</body>"
        )

        self.assertEqual(snapshot["title"], "A long title")
        self.assertEqual(snapshot["text_sample"], "short body")
        self.assertEqual(snapshot["text_length"], len("short body"))

    def test_classify_ignores_blocked_markers_inside_scripts_when_visible_content_exists(
        self,
    ) -> None:
        html = (
            b"<!doctype html><title>bilibili</title><body>"
            b"<main>bilibili homepage video feed</main>"
            b"<script>window.__data__ = { code: 403, label: 'blocked' };</script>"
            b"</body>"
        )

        self.assertEqual(_classify(html, b"", 0, False), "success")

    def test_classify_detects_visible_blocked_pages(self) -> None:
        html = b"<!doctype html><title>403 Forbidden</title><body>request blocked</body>"

        self.assertEqual(_classify(html, b"", 0, False), "blocked")

    def test_classify_rejects_contentful_http_error_response(self) -> None:
        html = (
            b"<!doctype html><title>upstream response</title><body>"
            + b"error details " * 100
            + b"</body>"
        )

        self.assertEqual(
            _classify(html, b"", 0, False, response_status=500),
            "http-error",
        )
        self.assertEqual(_failure_kind("http-error", []), "http-error")

    def test_classify_infers_cli_error_documents_and_network_failures(self) -> None:
        gateway = b"<!doctype html><title>502 Bad Gateway</title><body>origin unavailable</body>"
        api_error = (
            b"<!doctype html><title></title><body><pre>"
            b'{"error":{"message":"invalid query"}}'
            b"</pre></body>"
        )
        tls_error = b"<!doctype html><body>Navigation failed Reason: PeerFailedVerification</body>"
        chinese_verification = "<!doctype html><body>环境异常，完成验证后即可继续访问</body>".encode()

        self.assertEqual(_classify(gateway, b"", 0, False), "http-error")
        self.assertEqual(_classify(api_error, b"", 0, False), "http-error")
        self.assertEqual(_classify(tls_error, b"", 0, False), "network-error")
        self.assertEqual(_failure_kind("network-error", []), "network-error")
        self.assertEqual(
            _classify(chinese_verification, b"", 0, False),
            "challenge",
        )

    def test_classify_recognizes_lightpanda_transport_and_waf_documents(self) -> None:
        dns = (
            b"<!doctype html><body><h1>Navigation failed</h1>"
            b"<p>Reason: CouldntResolveHost</p></body>"
        )
        http2 = (
            b"<!doctype html><body><h1>Navigation failed</h1>"
            b"<p>Reason: Http2Stream</p></body>"
        )
        safeline = (
            b"<!doctype html><body>Security Detection Powered By SafeLine WAF"
            b"</body>"
        )
        privacy = (
            b"<!doctype html><body>Select the 2 matching tiles Submit "
            b"Powered and protected by Privacy</body>"
        )

        self.assertEqual(_classify(dns, b"", 0, False), "network-error")
        self.assertEqual(_classify(http2, b"", 0, False), "network-error")
        self.assertEqual(_classify(safeline, b"", 0, False), "blocked")
        self.assertEqual(_classify(privacy, b"", 0, False), "challenge")

    def test_classify_recognizes_lightpanda_fatal_fetch_timeout(self) -> None:
        stderr = (
            b'$scope=app $level=fatal $msg="fetch error" err=Timeout '
            b"url=https://example.test/"
        )

        self.assertEqual(_classify(b"", stderr, 0, False), "timeout")

    def test_classify_preserves_cli_main_document_status_and_dns_reason(self) -> None:
        self.assertEqual(
            _classify(
                b"",
                b"Reason: lifecycle target document `https://example.test/` "
                b"returned 500 Internal Server Error",
                1,
                False,
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
            ),
            "blocked",
        )
        self.assertEqual(
            _classify(
                b"",
                b"Reason: failed to resolve example.test:443: failed to lookup "
                b"address information",
                1,
                False,
            ),
            "network-error",
        )

    def test_run_wild_web_suite_writes_failure_snapshots(self) -> None:
        def run_process(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=["fake"],
                returncode=0,
                elapsed_ms=12.0,
                stdout=b"<!doctype html><title>Example</title><body>tiny</body>",
                stderr=b"",
                timed_out=False,
                resources={"peak_pss_bytes": 123},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            with patch("moli_benchmark.wild_web.run_process", run_process):
                summary = run_wild_web_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/echo"}},
                    targets=("moli",),
                    seeds=("toutiao-home",),
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli",
                )

            failures_dir = output_dir / "wild-web" / "failures"
            failure_files = sorted(path.name for path in failures_dir.iterdir())
            rows = json.loads(
                (output_dir / "wild-web" / "runs.json").read_text(
                    encoding="utf-8"
                )
            )

        self.assertEqual(summary["gate_failures"], 1)
        self.assertEqual(summary["targets"]["moli"]["failure_kinds"], {"extraction-failure": 1})
        self.assertEqual(summary["targets"]["moli"]["extraction_failures"], 2)
        self.assertEqual(summary["schedule"], "site-paired-rotating-target-order")
        self.assertEqual(summary["scheduled_site_groups"], 1)
        self.assertEqual(rows[0]["schedule_index"], 0)
        self.assertEqual(rows[0]["target_order_index"], 1)
        self.assertTrue(rows[0]["started_at"].endswith("Z"))
        self.assertEqual(len(rows[0]["stdout_sha256"]), 64)
        self.assertEqual(len(rows[0]["stderr_sha256"]), 64)
        self.assertIn("moli-run-1-toutiao-home.json", failure_files)
        self.assertIn("moli-run-1-toutiao-home.stdout.html", failure_files)

    def test_acceptable_login_result_does_not_count_as_failure_kind(self) -> None:
        def run_process(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=["fake"],
                returncode=0,
                elapsed_ms=12.0,
                stdout="<!doctype html><title>知乎</title><body>知乎 首页 登录 内容 推荐 回答 专栏</body>".encode(),
                stderr=b"",
                timed_out=False,
                resources={"peak_pss_bytes": 123},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            with patch("moli_benchmark.wild_web.run_process", run_process):
                summary = run_wild_web_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/echo"}},
                    targets=("moli",),
                    seeds=("zhihu-home",),
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli",
                )

        self.assertEqual(summary["gate_failures"], 0)
        self.assertEqual(summary["targets"]["moli"]["categories"], {"login": 1})
        self.assertEqual(summary["targets"]["moli"]["failure_kinds"], {})
        self.assertEqual(summary["targets"]["moli"]["elapsed_ms"]["count"], 1)
        self.assertEqual(
            summary["targets"]["moli"]["successful_elapsed_ms"]["count"],
            1,
        )
        self.assertEqual(summary["common_success"]["attempts"], 1)

    def test_capture_replay_writes_manifest_for_successful_rows(self) -> None:
        def run_process(*args: object, **kwargs: object) -> ProcessResult:
            return ProcessResult(
                command=["fake"],
                returncode=0,
                elapsed_ms=12.0,
                stdout="<!doctype html><title>知乎</title><body>知乎 首页 内容 推荐 回答 专栏 热榜 发现 等你来答</body>".encode(),
                stderr=b"",
                timed_out=False,
                resources={"peak_pss_bytes": 123},
            )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            with patch("moli_benchmark.wild_web.run_process", run_process):
                summary = run_wild_web_suite(
                    output_dir=output_dir,
                    target_matrix={"moli": {"available": True, "path": "/bin/echo"}},
                    targets=("moli",),
                    seeds=("zhihu-home",),
                    runs=1,
                    timeout_seconds=1.0,
                    gate_target="moli",
                    capture_replay=True,
                )

            replay_manifest = output_dir / "wild-web" / "replay" / "manifest.json"
            replay_html = output_dir / "wild-web" / "replay" / "moli-run-1-zhihu-home.html"
            replay_manifest_exists = replay_manifest.exists()
            replay_html_exists = replay_html.exists()

        self.assertEqual(summary["gate_failures"], 0)
        self.assertTrue(summary["replay_capture"])
        self.assertEqual(summary["replay_artifacts"], 1)
        self.assertTrue(replay_manifest_exists)
        self.assertTrue(replay_html_exists)


if __name__ == "__main__":
    unittest.main()
