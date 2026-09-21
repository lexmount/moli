from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch
from urllib.parse import quote

from moli_benchmark.wpt_cross.server import WptFixtureServer


class DirectoryFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        (self.root / "empty").mkdir()
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, path: str, *, method: str = "GET"):
        connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
        try:
            connection.request(method, path)
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_root_and_empty_directory_have_get_and_head_responses(self) -> None:
        for path in ("/", "/empty/"):
            with self.subTest(path=path):
                status, headers, body = self.request(path)
                self.assertEqual(status, 200)
                self.assertEqual(headers["Content-Type"], "text/html; charset=utf-8")
                self.assertIn(b"<!doctype html>", body)
                self.assertIn(f"Directory listing for {path}".encode(), body)
                head_status, head_headers, head_body = self.request(path, method="HEAD")
                self.assertEqual((head_status, head_body), (200, b""))
                self.assertEqual(head_headers["Content-Length"], str(len(body)))
                self.assertEqual(head_headers["Content-Type"], headers["Content-Type"])

    def test_missing_directory_slash_redirects_without_losing_query(self) -> None:
        (self.root / "space dir").mkdir()
        for method in ("GET", "HEAD"):
            with self.subTest(method=method):
                status, headers, body = self.request("/space%20dir?key=%2F", method=method)
                self.assertEqual((status, body), (301, b""))
                self.assertEqual(headers["Location"], "/space%20dir/?key=%2F")

    def test_listing_escapes_names_and_lists_index_as_an_ordinary_file(self) -> None:
        directory = self.root / "<folder&>"
        directory.mkdir()
        filename = '<tag>&"#?目次.txt'
        (directory / filename).write_bytes(b"file contents")
        (directory / "index.html").write_text("INDEX MUST NOT REPLACE THE LISTING")
        (directory / "child").mkdir()
        path = "/" + quote(directory.name, safe="") + "/"
        status, _, body = self.request(path)
        self.assertEqual(status, 200)
        self.assertIn(b"Directory listing for /&lt;folder&amp;&gt;/", body)
        self.assertIn(b'&lt;tag&gt;&amp;&quot;#?', body)
        self.assertIn(f'href="{quote(filename, safe="")}"'.encode(), body)
        self.assertIn(b'href="child/"', body)
        self.assertIn(b'href="../"', body)
        self.assertIn(b'href="index.html"', body)
        self.assertNotIn(b"INDEX MUST NOT REPLACE THE LISTING", body)
        self.assertEqual(self.request(path + quote(filename, safe=""))[2], b"file contents")

    def test_missing_paths_and_paths_outside_the_fixture_root_remain_missing(self) -> None:
        outside = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (outside / "secret.txt").write_text("outside fixture data")
        (self.root / "outside").symlink_to(outside, target_is_directory=True)
        for path in ("/missing", "/missing/", "/../", "/%2e%2e/", "/outside/",
                     "/outside/secret.txt"):
            with self.subTest(path=path):
                self.assertEqual(self.request(path)[0], 404)

    def test_explicit_index_file_keeps_its_contents_and_sidecar_headers(self) -> None:
        (self.root / "index.html").write_bytes(b"<!doctype html><p>index fixture</p>")
        (self.root / "index.html.headers").write_text("X-Fixture: index-file\n")
        status, headers, body = self.request("/index.html")
        self.assertEqual(status, 200)
        self.assertEqual(headers["X-Fixture"], "index-file")
        self.assertEqual(body, b"<!doctype html><p>index fixture</p>")


if __name__ == "__main__":
    unittest.main()
