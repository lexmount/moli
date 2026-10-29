from __future__ import annotations

import io
import unittest
from pathlib import Path
from unittest.mock import patch

from moli_benchmark import config
from moli_benchmark.config import reserve_port
from moli_benchmark.target_serve import start_target_serve, stop_target_serve


class _FakeProcess:
    pid = 12345

    def __init__(self) -> None:
        self.returncode: int | None = None
        self.stdout = io.BytesIO()
        self.stderr = io.BytesIO()

    def poll(self) -> int | None:
        return self.returncode

    def terminate(self) -> None:
        self.returncode = -15

    def wait(self, timeout: float | None = None) -> int | None:
        del timeout
        return self.returncode


class _FakeSampler:
    def __init__(self, pid: int) -> None:
        self.pid = pid
        self.samples = [{"elapsed_ms": 1.0, "rss_bytes": 42}]

    def start(self) -> None:
        return None

    def export_samples(self):
        return list(self.samples)

    def stop(self) -> dict[str, int]:
        return {"pid": self.pid}


class TargetServeTests(unittest.TestCase):
    def test_reserve_port_uses_server_range_and_holds_process_lease(self) -> None:
        lease = reserve_port()
        try:
            low, high = config._server_port_range()
            self.assertGreaterEqual(lease.port, low)
            self.assertLessEqual(lease.port, high)
            self.assertIn(lease.port, config._RESERVED_PORTS)

            lease.release_socket()
            self.assertIn(lease.port, config._RESERVED_PORTS)
        finally:
            port = lease.port
            lease.close()
        self.assertNotIn(port, config._RESERVED_PORTS)

    def test_start_target_serve_keeps_port_leased_until_stop(self) -> None:
        process = _FakeProcess()

        with (
            patch("moli_benchmark.target_serve.subprocess.Popen", return_value=process),
            patch("moli_benchmark.target_serve.ResourceSampler", _FakeSampler),
            patch("moli_benchmark.target_serve.probe_url", return_value=True),
        ):
            handle = start_target_serve("moli-cdp", Path("/bin/moli"), 1.0)
            port = int(handle.endpoint.rsplit(":", 1)[1])
            self.assertIn(port, config._RESERVED_PORTS)
            self.assertIn(str(port), handle.command)
            self.assertNotIn("--layout", handle.command)
            self.assertNotIn("--resource", handle.command)

            stop_target_serve(handle)
            self.assertNotIn(port, config._RESERVED_PORTS)

    def test_start_target_serve_enables_layout_and_all_resource_fetch_for_full_moli(
        self,
    ) -> None:
        process = _FakeProcess()

        with (
            patch("moli_benchmark.target_serve.subprocess.Popen", return_value=process),
            patch("moli_benchmark.target_serve.ResourceSampler", _FakeSampler),
            patch("moli_benchmark.target_serve.probe_url", return_value=True),
        ):
            handle = start_target_serve("moli-full-cdp", Path("/bin/moli"), 1.0)
            self.assertIn("--layout", handle.command)
            self.assertIn("--resource", handle.command)

            stop_target_serve(handle)

    def test_start_target_serve_passes_extra_args_to_chrome(self) -> None:
        process = _FakeProcess()

        with (
            patch("moli_benchmark.target_serve.subprocess.Popen", return_value=process),
            patch("moli_benchmark.target_serve.ResourceSampler", _FakeSampler),
            patch("moli_benchmark.target_serve.probe_url", return_value=True),
        ):
            handle = start_target_serve(
                "chrome-cdp",
                Path("/bin/chromium"),
                1.0,
                ("--user-agent=BenchmarkUA", "--disable-features=Example"),
            )
            self.assertIn("--user-agent=BenchmarkUA", handle.command)
            self.assertIn("--disable-features=Example", handle.command)
            self.assertLess(
                handle.command.index("--user-agent=BenchmarkUA"),
                handle.command.index("about:blank"),
            )

            stop_target_serve(handle)

    def test_stop_target_serve_can_retain_periodic_resource_samples(self) -> None:
        process = _FakeProcess()

        with (
            patch("moli_benchmark.target_serve.subprocess.Popen", return_value=process),
            patch("moli_benchmark.target_serve.ResourceSampler", _FakeSampler),
            patch("moli_benchmark.target_serve.probe_url", return_value=True),
        ):
            handle = start_target_serve("moli-cdp", Path("/bin/moli"), 1.0)
            stopped = stop_target_serve(handle, include_resource_samples=True)

        self.assertEqual(
            stopped["resources"]["samples"],
            [{"elapsed_ms": 1.0, "rss_bytes": 42}],
        )


if __name__ == "__main__":
    unittest.main()
