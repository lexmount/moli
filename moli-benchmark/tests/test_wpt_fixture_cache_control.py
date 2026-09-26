import tempfile
import unittest
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

from moli_benchmark.wpt_cross.server import WptFixtureServer


class WptFixtureCacheControlTests(unittest.TestCase):
    def test_explicit_cache_control_precedes_fixture_default(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources/testharness.js").write_text("// testharness")
            (root_path / "module.json").write_text('{"value": 1}')
            sidecar = root_path / "module.json.headers"
            with WptFixtureServer(root_path) as server:
                for header, pipe, expected in (
                    (None, "", ["no-store"]),
                    ("max-age=600", "", ["max-age=600"]),
                    ("", "", [""]),
                    (None, "header(Cache-Control,max-age=30)", ["max-age=30"]),
                    (None, "header(cache-control,public)", ["public"]),
                ):
                    if header is None:
                        sidecar.unlink(missing_ok=True)
                    else:
                        sidecar.write_text("cAcHe-CoNtRoL: " + header + "\n")
                    for method in ("GET", "HEAD"):
                        with self.subTest(header=header, pipe=pipe, method=method):
                            url = server.base_url + "/module.json?" + urlencode({"pipe": pipe})
                            with urlopen(Request(url, method=method), timeout=2) as response:
                                self.assertEqual(response.status, 200)
                                self.assertEqual(response.headers.get_all("Cache-Control"), expected)
                                self.assertEqual(response.read(), b"" if method == "HEAD" else b'{"value": 1}')
