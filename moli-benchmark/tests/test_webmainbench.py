from __future__ import annotations

import hashlib
import json
import signal
import ssl
import sys
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.request import HTTPSHandler, ProxyHandler, build_opener

from moli_benchmark import webmainbench as bench
from moli_benchmark.process import ProcessResult, run_process

PAGE = bench.Case(
    "11111111-1111-4111-8111-111111111111", "https://example.org/", "<p>Page</p>"
)
REDIRECT = bench.Case(
    bench.REDIRECT_CASE_ID,
    "http://langrensha.163.com/",
    "<script>location='https://langrensha.163.com/'</script>",
)
DNS_ERROR = (
    b"Error: failed to fetch `https://127.0.0.1:1234/case/page.html`\n"
    b"Reason: failed to resolve langrensha.163.com:443: failed to lookup address information: "
    b"Temporary failure in name resolution\n"
)


def process_result(**changes: object) -> ProcessResult:
    return replace(
        ProcessResult(
            command=[],
            returncode=0,
            elapsed_ms=1,
            stdout=b"# Page\n",
            stderr=b"",
            timed_out=False,
            resources={},
        ),
        **changes,
    )


class WebMainBenchTests(unittest.TestCase):
    def test_expected_redirect_is_matched_by_case_and_error(self) -> None:
        result = process_result(returncode=1, stdout=b"", stderr=DNS_ERROR)
        self.assertEqual(bench.classify(REDIRECT, result), "expected_failure")
        self.assertEqual(bench.classify(PAGE, result), "failure")
        self.assertEqual(
            bench.classify(
                REDIRECT, replace(result, stderr=b"Reason: unrelated error\n")
            ),
            "failure",
        )
        self.assertEqual(bench.classify(REDIRECT, process_result()), "success")

    def test_exception_never_hides_panic_crash_or_timeout(self) -> None:
        base = process_result(returncode=1, stdout=b"", stderr=DNS_ERROR)
        for result, expected in [
            (
                replace(
                    base,
                    stderr=b"thread 'render_runtime' panicked at tokenizer/mod.rs:254:9:\n"
                    + DNS_ERROR,
                ),
                "panic",
            ),
            (replace(base, returncode=-signal.SIGSEGV), "crash"),
            (replace(base, timed_out=True), "timeout"),
            (
                replace(
                    base,
                    stderr=b"Reason: fetch readiness timed out after 45000 ms while waiting for Done\n",
                ),
                "timeout",
            ),
        ]:
            with self.subTest(expected=expected):
                self.assertEqual(bench.classify(REDIRECT, result), expected)
        self.assertEqual(
            bench.classify(
                PAGE,
                process_result(
                    stderr=b"thread '<unnamed>' panicked at parser.rs:12:3:\n"
                ),
            ),
            "panic",
        )

    def test_success_requires_nonempty_markdown(self) -> None:
        self.assertEqual(
            bench.classify(PAGE, process_result(stdout=b" \n\t")), "empty_output"
        )
        self.assertEqual(bench.classify(PAGE, process_result()), "success")

    def test_gate_requires_each_case_not_just_a_success_count(self) -> None:
        cases = [PAGE, REDIRECT]
        valid = [
            {"track_id": PAGE.track_id, "status": "success"},
            {"track_id": REDIRECT.track_id, "status": "expected_failure"},
        ]
        with patch.object(bench, "CASE_COUNT", 2):
            self.assertEqual(bench.health_issues(cases, valid), [])
            self.assertTrue(bench.health_issues(cases, valid[:1]))
            self.assertTrue(bench.health_issues(cases, [valid[0], valid[0]]))
            changed_failure = [
                {"track_id": PAGE.track_id, "status": "failure"},
                {"track_id": REDIRECT.track_id, "status": "success"},
            ]
            self.assertTrue(bench.health_issues(cases, changed_failure))

    def test_dataset_drift_is_rejected_even_when_a_file_is_cached(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "dataset.jsonl"
            path.write_text("{}\n", encoding="utf-8")
            with patch.object(bench, "urlopen") as download:
                with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                    bench.download_dataset(path)
                download.assert_not_called()

    def test_dataset_requires_complete_unique_ids(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "dataset.jsonl"
            rows = [
                {"track_id": case.track_id, "url": case.url, "html": case.html}
                for case in [PAGE, REDIRECT]
            ]
            for data, valid in [
                (rows, True),
                (rows[:1], False),
                ([rows[0], rows[0]], False),
            ]:
                path.write_text(
                    "".join(json.dumps(row) + "\n" for row in data), encoding="utf-8"
                )
                with (
                    patch.object(bench, "DATASET_SHA256", bench.file_sha256(path)),
                    patch.object(bench, "CASE_COUNT", 2),
                ):
                    if valid:
                        self.assertEqual(bench.load_cases(path), [PAGE, REDIRECT])
                    else:
                        with self.assertRaisesRegex(ValueError, "unique cases"):
                            bench.load_cases(path)

    def test_runner_refuses_host_network_before_changing_interfaces(self) -> None:
        links = json.dumps([{"ifname": "lo"}, {"ifname": "eth0"}]).encode()
        with (
            patch.object(bench.subprocess, "check_output", return_value=links),
            patch.object(bench.subprocess, "run") as run,
        ):
            with self.assertRaisesRegex(RuntimeError, "network namespace"):
                bench.require_loopback_namespace()
            run.assert_not_called()

    def test_https_fixture_preserves_malformed_html_and_inline_scripts(self) -> None:
        page = replace(
            PAGE,
            html='<meta name=“viewport“ content=“width=device-width“>\n<script>document.write("中文")</script>',
        )
        with bench.fixture_server([page]) as (origin, cert):
            context = ssl.create_default_context(cafile=str(cert))
            client = build_opener(ProxyHandler({}), HTTPSHandler(context=context))
            with client.open(
                f"{origin}/case/{page.track_id}.html?query=1", timeout=5
            ) as response:
                self.assertEqual(response.status, 200)
                self.assertEqual(response.read(), page.html.encode("utf-8"))
            for path in ["/missing.js", f"/elsewhere/{page.track_id}.html"]:
                with self.subTest(path=path):
                    with self.assertRaises(HTTPError) as error:
                        client.open(origin + path, timeout=5)
                    self.assertEqual(error.exception.code, 404)
                    error.exception.close()
        self.assertFalse(cert.exists())

    def test_process_watchdog_fails_even_if_partial_output_exists(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = run_process(
                [
                    sys.executable,
                    "-c",
                    'import time; print("# Partial output", flush=True); time.sleep(30)',
                ],
                cwd=Path(temporary),
                timeout_seconds=1,
                sample_resources=False,
            )
        self.assertTrue(result.timed_out)
        self.assertIn(b"Partial output", result.stdout)
        self.assertEqual(bench.classify(PAGE, result), "timeout")

    def test_failed_page_retains_evidence_and_does_not_skip_remaining_cases(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "run"
            with (
                patch.object(bench, "load_cases", return_value=[PAGE, REDIRECT]),
                patch.object(bench, "CASE_COUNT", 2),
                patch.object(bench, "require_loopback_namespace"),
                patch.object(
                    bench,
                    "run_process",
                    side_effect=[
                        process_result(
                            returncode=1, stdout=b"", stderr=b"Reason: regression\n"
                        ),
                        process_result(),
                    ],
                ),
                patch("builtins.print"),
            ):
                passed = bench.replay(
                    Path(sys.executable), Path("unused"), output, "a" * 40
                )
            self.assertFalse(passed)
            report = json.loads((output / "summary.json").read_text())
            self.assertEqual(report["completed"], 2)
            self.assertEqual(report["counts"], {"failure": 1, "success": 1})
            records = [
                json.loads(line)
                for line in (output / "results.jsonl").read_text().splitlines()
            ]
            self.assertEqual(
                [row["track_id"] for row in records], [PAGE.track_id, REDIRECT.track_id]
            )
            self.assertEqual(
                records[0]["html_sha256"],
                hashlib.sha256(PAGE.html.encode()).hexdigest(),
            )
            self.assertEqual(
                (output / f"{PAGE.track_id}.stderr").read_bytes(),
                b"Reason: regression\n",
            )
            self.assertEqual(
                (output / f"{REDIRECT.track_id}.md").read_bytes(), b"# Page\n"
            )
            self.assertIn("**FAIL**", (output / "summary.md").read_text())
            self.assertEqual(list(output.glob("*.pem")), [])

    def test_interrupted_suite_keeps_partial_results_and_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "run"
            with (
                patch.object(bench, "load_cases", return_value=[PAGE, REDIRECT]),
                patch.object(bench, "CASE_COUNT", 2),
                patch.object(bench, "require_loopback_namespace"),
                patch.object(
                    bench,
                    "run_process",
                    side_effect=[process_result(), OSError("cannot launch process")],
                ),
                patch("builtins.print"),
                self.assertLogs(bench.LOGGER, level="ERROR"),
            ):
                passed = bench.replay(
                    Path(sys.executable), Path("unused"), output, "a" * 40
                )
            self.assertFalse(passed)
            report = json.loads((output / "summary.json").read_text())
            self.assertEqual(report["completed"], 1)
            self.assertIn(
                "Infrastructure error: OSError: cannot launch process", report["issues"]
            )
            self.assertEqual(
                len((output / "results.jsonl").read_text().splitlines()), 1
            )
            self.assertIn("**FAIL**", (output / "summary.md").read_text())


if __name__ == "__main__":
    unittest.main()
