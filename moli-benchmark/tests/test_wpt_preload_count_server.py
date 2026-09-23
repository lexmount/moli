from __future__ import annotations

import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


COUNT_PATH = "/preload/resources/preload-count.py"


class PreloadCountFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        (self.root / "preload/resources").mkdir(parents=True)
        (self.root / COUNT_PATH.lstrip("/")).write_text("# Do not serve Python source")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, query: str, *, method="GET", path=COUNT_PATH, port=None, body=None):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=5)
        try:
            connection.request(method, path + "?" + query, body)
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def assert_count(self, count: int, **kwargs) -> None:
        status, headers, body = self.request("action=result", **kwargs)
        expected = f"preloadCount = {count};".encode("ascii")
        self.assertEqual(status, 200)
        self.assertEqual(headers.get_all("Content-Type"), ["text/javascript"])
        self.assertEqual(headers["Content-Length"], str(len(expected)))
        self.assertIsNone(headers["Cache-Control"])
        self.assertEqual(body, b"" if kwargs.get("method") == "HEAD" else expected)

    def test_counts_failed_resource_requests_and_results_consume_the_count(self) -> None:
        self.assert_count(0)
        for action in ("image", "font", "", "RESULT"):
            status, headers, body = self.request("action=" + action)
            self.assertEqual((status, body), (404, b"No entry is found"))
            self.assertIsNone(headers["Content-Type"])
            self.assertIsNone(headers["Cache-Control"])
        self.assert_count(4)
        self.assert_count(0)

    def test_query_keys_and_first_value_match_upstream(self) -> None:
        for query in ("action=&action=result", "action=image&action=result", "act%69on=image"):
            self.assertEqual(self.request(query)[0], 404)
        status, _, body = self.request("action=result&action=image")
        self.assertEqual((status, body), (200, b"preloadCount = 3;"))
        self.assert_count(0)
        for query in ("", "Action=result"):
            self.request("action=image")
            self.assertEqual(self.request(query)[0], 500)
            self.assert_count(0)

    def test_all_methods_use_query_parameters_and_head_still_consumes(self) -> None:
        methods = ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "YO", "chicken")
        for method in methods:
            with self.subTest(method=method):
                status, headers, body = self.request(
                    "action=image", method=method, body=b"action=result",
                )
                self.assertEqual(status, 404)
                self.assertEqual(headers["Content-Length"], str(len(b"No entry is found")))
                self.assertEqual(body, b"" if method == "HEAD" else b"No entry is found")
        self.assert_count(len(methods), method="HEAD")
        self.assert_count(0)
        for method in methods:
            self.request("action=image")
            self.assert_count(1, method=method, body=b"action=image")
            self.assert_count(0)

    def test_parallel_requests_share_origins_but_not_fixture_servers(self) -> None:
        with ThreadPoolExecutor(max_workers=32) as pool:
            responses = list(pool.map(
                lambda index: self.request(
                    "action=image", port=self.server.port if index % 2 else self.server.alternate_port,
                ),
                range(32),
            ))
        self.assertTrue(all(status == 404 for status, _, _ in responses))
        other = self.stack.enter_context(WptFixtureServer(self.root))
        self.assert_count(0, port=other.port)
        self.assert_count(32, port=self.server.alternate_port)
        self.assert_count(0)

    def test_unused_uploads_do_not_delay_the_response(self) -> None:
        for method in ("POST", "PUT", "OPTIONS", "chicken"):
            connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
            try:
                connection.putrequest(method, COUNT_PATH + "?action=image")
                connection.putheader("Content-Length", "1000000")
                connection.endheaders()
                response = connection.getresponse()
                self.assertEqual((response.status, response.read()), (404, b"No entry is found"))
                self.assertEqual(response.headers["Connection"], "close")
            finally:
                connection.close()
        self.assert_count(4)

    def test_only_the_exact_counter_path_is_handled_and_discovered(self) -> None:
        for path in (COUNT_PATH + "2", "/other" + COUNT_PATH, "/PRELOAD/resources/preload-count.py"):
            self.assertEqual(self.request("action=result", path=path)[0], 404)
        cases = {
            "absolute.html": (COUNT_PATH, True),
            "relative.html": ("resources/preload-count.py", True),
            "dot-relative.html": ("./resources/preload-count.py", True),
            "bare.html": ("preload-count.py", False),
            "wrong-relative.html": ("../resources/preload-count.py", False),
            "wrong-root.html": ("/other" + COUNT_PATH, False),
            "suffix.html": (COUNT_PATH + ".extra", False),
        }
        for name, (reference, _) in cases.items():
            (self.root / "preload" / name).write_text(
                '<script src="/resources/testharness.js"></script>'
                f'<script>fetch("{reference}?action=result")</script>'
            )
        (self.root / "preload/unknown.html").write_text(
            '<script src="/resources/testharness.js"></script>'
            f'<script>fetch("{COUNT_PATH}?action=result"); fetch("/unsupported.py")</script>'
        )
        discovered = enumerate_cases(self.root, dir_prefixes=("preload",))
        self.assertEqual(
            sorted(case.case_path for case in discovered),
            sorted("preload/" + name for name, (_, allowed) in cases.items() if allowed),
        )


if __name__ == "__main__":
    unittest.main()
