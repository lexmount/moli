from __future__ import annotations

import html
import json
import re
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest.mock import patch
from urllib.parse import unquote, urlsplit

from moli_benchmark.sampling import ResourceSampler
from moli_benchmark.sequential_navigate import summarize_navigation_resource_samples
from moli_benchmark.stress import DEFAULT_URLS, main
from moli_benchmark.stress_report import write_stress_report


MIB = 1024 * 1024


def stress_payload(binary: Path) -> dict:
    navigation_samples = [
        {
            "index": 0,
            "url": None,
            "rss_bytes": 80 * MIB,
            "pss_bytes": 75 * MIB,
            "fd_count": 20,
            "thread_count": 8,
            "process_count": 1,
            "error": None,
        }
    ]
    navigation_samples.extend(
        {
            "index": index,
            "url": url,
            "rss_bytes": (250 + index) * MIB,
            "pss_bytes": (245 + index) * MIB,
            "fd_count": 30,
            "thread_count": 12,
            "process_count": 1,
            "error": None,
        }
        for index, url in enumerate(DEFAULT_URLS, 1)
    )
    periodic_samples = [
        {
            "elapsed_ms": elapsed_ms,
            "rss_bytes": rss * MIB,
            "pss_bytes": (rss - 5) * MIB,
            "cpu_percent": cpu,
            "process_count": 1,
            "pss_process_count": 1,
            "thread_count": 12,
            "fd_count": 30,
            "capture_duration_ms": 1.0,
        }
        for elapsed_ms, rss, cpu in (
            (0.0, 80, None),
            (100.0, 250, 50.0),
            (200.0, 254, 75.0),
        )
    ]
    sampler = ResourceSampler(1)
    sampler.samples = periodic_samples
    periodic_summary = sampler.summary()
    periodic_summary["samples"] = periodic_samples
    navigation_summary = summarize_navigation_resource_samples(
        navigation_samples,
        periodic_summary,
    )
    rows = [
        {
            "index": index,
            "url": url,
            "ok": True,
            "response_ms": 100.0 + index,
            "dcl_ms": 200.0 + index,
            "load_ms": 300.0 + index,
            "elapsed_ms": 310.0 + index,
        }
        for index, url in enumerate(DEFAULT_URLS, 1)
    ]
    return {
        "schema_version": 5,
        "repository": {"commit": "a" * 40, "dirty": False},
        "engine_selection": "moli",
        "rounds": 1,
        "urls": list(DEFAULT_URLS),
        "network_diagnostics": True,
        "navigation_resource_samples": True,
        "periodic_resource_samples": True,
        "results": [
            {
                "target": "moli-cdp",
                "binary": str(binary),
                "started_at": "2026-08-12T00:00:00+00:00",
                "finished_at": "2026-08-12T00:00:02+00:00",
                "ready_ms": 20.0,
                "summary": {
                    "planned": 4,
                    "attempted": 4,
                    "observable_passes": 4,
                    "failures": 0,
                    "document_passes": 4,
                    "network_error_documents": 0,
                    "recovery_attempts": 0,
                    "recovery_passes": 0,
                    "recovery_failures": 0,
                    "order_violations": 0,
                    "network_order_violations": 0,
                    "superseded_passes": 0,
                    "aborted_after_index": None,
                },
                "navigation_resources": {
                    "samples": navigation_samples,
                    "summary": navigation_summary,
                },
                "rows": rows,
                "process": {
                    "returncode": 143,
                    "resources": periodic_summary,
                    "log_tail": [],
                },
            }
        ],
    }


class StressTests(unittest.TestCase):
    def test_default_run_retains_periodic_samples_and_generates_report(self) -> None:
        with (
            TemporaryDirectory() as temp_dir,
            patch(
                "moli_benchmark.stress.sequential_navigate_main",
                return_value=0,
            ) as navigate,
            patch(
                "moli_benchmark.stress.write_stress_report",
                return_value={"status": "pass"},
            ) as report,
        ):
            output_dir = Path(temp_dir)
            exit_code = main(["run", "--output-dir", str(output_dir)])

        self.assertEqual(exit_code, 0)
        sequential_args = navigate.call_args.args[0]
        self.assertIn("--periodic-resource-samples", sequential_args)
        self.assertIn("--navigation-resource-samples", sequential_args)
        self.assertIn("--network-diagnostics", sequential_args)
        self.assertEqual(sequential_args[sequential_args.index("--rounds") + 1], "150")
        self.assertEqual(sequential_args.count("--url"), 4)
        report.assert_called_once_with(
            output_dir / "result.json",
            output_dir / "report.html",
        )

    def test_run_rejects_navigation_count_that_cannot_complete_url_rounds(self) -> None:
        with TemporaryDirectory() as temp_dir:
            exit_code = main(
                [
                    "run",
                    "--navigations",
                    "5",
                    "--output-dir",
                    temp_dir,
                ]
            )

        self.assertEqual(exit_code, 2)

    def test_report_is_offline_and_writes_machine_readable_summary(self) -> None:
        with TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            binary = root / "moli"
            binary.write_bytes(b"test-binary")
            result_path = root / "result.json"
            report_path = root / "report.html"
            result_path.write_text(
                json.dumps(stress_payload(binary)),
                encoding="utf-8",
            )

            summary = write_stress_report(result_path, report_path)
            document = report_path.read_text(encoding="utf-8")
            saved_summary = json.loads((root / "summary.json").read_text())

        self.assertEqual(summary["status"], "pass")
        self.assertEqual(summary["navigation"]["attempted"], 4)
        self.assertEqual(saved_summary["metadata"]["binary_size_bytes"], 11)
        self.assertIn("// https://d3js.org v7.8.4", document)
        self.assertIn("RSS / PSS / CPU", document)
        self.assertNotIn("<script src=", document)
        self.assertNotIn("https://cdn.", document)

    def test_report_requires_retained_periodic_samples(self) -> None:
        with TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            binary = root / "moli"
            binary.write_bytes(b"moli")
            payload = stress_payload(binary)
            del payload["results"][0]["process"]["resources"]["samples"]
            result_path = root / "result.json"
            result_path.write_text(json.dumps(payload), encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "no periodic resource samples"):
                write_stress_report(result_path, root / "report.html")

    def test_report_links_to_result_filename_without_treating_it_as_url_syntax(self) -> None:
        with TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            binary = root / "moli"
            binary.write_bytes(b"moli")
            result_path = root / "raw" / "result #1?%done.json"
            result_path.parent.mkdir()
            result_path.write_text(json.dumps(stress_payload(binary)), encoding="utf-8")
            report_path = root / "report" / "index.html"
            write_stress_report(result_path, report_path)
            document = report_path.read_text(encoding="utf-8")
            match = re.search(r'<a href="([^"]+)">result.json</a>', document)
            self.assertIsNotNone(match)
            link = urlsplit(html.unescape(match.group(1)))
            self.assertEqual(link.fragment, "")
            self.assertEqual(link.query, "")
            self.assertEqual((report_path.parent / unquote(link.path)).resolve(), result_path.resolve())

    def test_report_refuses_to_overwrite_its_input(self) -> None:
        with TemporaryDirectory() as temp_dir:
            result_path = Path(temp_dir) / "result.json"
            result_path.write_text("{}", encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "must not overwrite"):
                write_stress_report(result_path, result_path)


if __name__ == "__main__":
    unittest.main()
