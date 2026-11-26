import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, patch
from urllib.parse import urlencode
from urllib.request import urlopen
from uuid import uuid4

from moli_benchmark.wpt_cross.engine import EngineDriver
from moli_benchmark.wpt_cross.runner import run_engine_on_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


class ClearSiteDataCacheFixtureTests(unittest.TestCase):
    def test_cache_fixture_headers_and_first_request_state(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "resources").mkdir()
            (root / "resources" / "testharness.js").write_text("", encoding="utf-8")
            query = urlencode({
                "response": "json",
                "cache_helper": str(uuid4()),
                "cache": "",
                "clear_first": "cache",
            })
            with WptFixtureServer(root) as server:
                url = f"{server.base_url}/clear-site-data/support/clear-site-data-cache.py?{query}"
                for expected in ('"cache"', None):
                    with urlopen(url, timeout=3) as response:
                        self.assertEqual(response.headers["Cache-Control"], "public, max-age=31536000, immutable")
                        self.assertEqual(response.headers.get("Clear-Site-Data"), expected)
                        self.assertEqual(response.headers.get_content_type(), "application/json")
                        self.assertEqual(len(json.load(response)), 1)

    def test_moli_runner_provisions_an_ephemeral_http_cache_for_cache_cases(self) -> None:
        driver = EngineDriver(
            name="moli", binary_env_var="MOLI_BIN", default_binary_names=("moli",),
            build_command=lambda binary, port, profile: [str(binary), str(port)],
            version_args=("--version",),
        )
        with patch("moli_benchmark.wpt_cross.runner._run_async", new_callable=AsyncMock) as launch:
            run_engine_on_cases(
                driver=driver,
                cases=[("clear-site-data/clear-cache.https.html", "https://fixture.test/")],
            )
        selected = launch.call_args.kwargs["driver"]
        self.assertTrue(selected.needs_profile_dir)
        profile = Path("/profile")
        self.assertEqual(
            selected.build_command(Path("moli"), 8000, profile)[-2:],
            ["--http-cache-dir", str(profile / "http-cache")],
        )
