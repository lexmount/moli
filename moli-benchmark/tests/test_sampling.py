from __future__ import annotations

import threading
import unittest
from unittest.mock import patch

from moli_benchmark.sampling import ResourceSampler, snapshot_resources


class ResourceSamplingTests(unittest.TestCase):
    def test_snapshot_does_not_report_partial_process_tree_totals(self) -> None:
        def stat(pid):
            return {"identity": f"{pid}:100", "status": "ok", "total_ticks": 0}

        def read(path):
            raw = (
                "VmRSS: 1 kB\nThreads: 2"
                if str(path).endswith("status")
                else "Pss: 1 kB"
            )
            if str(path) == "/proc/11/smaps_rollup":
                return {"status": "os_error", "error": {"errno": 2}, "raw": None}
            if str(path) == "/proc/11/status":
                raw = "VmRSS: 1 kB"
            return {"status": "ok", "raw": raw}

        with (
            patch(
                "moli_benchmark.sampling._process_tree_evidence",
                return_value=([10, 11], {10: stat(10), 11: stat(11)}, {}),
            ),
            patch("moli_benchmark.sampling._text_read", side_effect=read),
            patch("moli_benchmark.sampling._stat_read", side_effect=stat),
            patch(
                "moli_benchmark.sampling._fd_read",
                return_value={"status": "ok", "value": 3},
            ),
            patch("moli_benchmark.sampling._read_cpu_percent", return_value=12.5),
        ):
            sample = snapshot_resources(10)

        self.assertIsNone(sample["pss_bytes"])
        self.assertEqual(sample["pss_process_count"], 1)
        self.assertEqual(sample["rss_bytes"], 2048)
        self.assertIsNone(sample["thread_count"])
        self.assertEqual(sample["fd_count"], 6)

    def test_summary_records_sampler_health_and_complete_pss_coverage(self) -> None:
        sampler = ResourceSampler(1, interval_seconds=0.5)
        sampler.samples = [
            {
                "elapsed_ms": 0,
                "pss_bytes": 100,
                "rss_bytes": 120,
                "cpu_percent": None,
                "process_count": 1,
                "pss_process_count": 1,
                "thread_count": 2,
                "fd_count": 3,
                "capture_duration_ms": 4,
            },
            {
                "elapsed_ms": 510,
                "pss_bytes": 110,
                "rss_bytes": 130,
                "cpu_percent": 25,
                "process_count": 1,
                "pss_process_count": 1,
                "thread_count": 2,
                "fd_count": 3,
                "capture_duration_ms": 5,
            },
        ]

        summary = sampler.summary()

        self.assertTrue(summary["pss_complete"])
        self.assertEqual(summary["peak_pss_bytes"], 110)
        self.assertEqual(summary["average_cpu_percent"], 25)
        self.assertEqual(summary["observed_interval_ms"]["average"], 510)

    def test_sampler_surfaces_observer_failure(self) -> None:
        attempted = threading.Event()

        def fail_snapshot(*_args: object, **_kwargs: object) -> dict[str, object]:
            attempted.set()
            raise RuntimeError("observer broke")

        sampler = ResourceSampler(123, interval_seconds=0.1)
        with patch(
            "moli_benchmark.sampling.snapshot_resources",
            side_effect=fail_snapshot,
        ):
            with patch(
                "moli_benchmark.sampling.clock_ticks_evidence",
                return_value={"ticks_per_second": 100},
            ):
                sampler.start()
            self.assertTrue(attempted.wait(timeout=1))
            summary = sampler.stop()

        self.assertEqual(summary["observer_error"], "RuntimeError: observer broke")
        self.assertFalse(summary["thread_alive_after_stop"])


if __name__ == "__main__":
    unittest.main()
