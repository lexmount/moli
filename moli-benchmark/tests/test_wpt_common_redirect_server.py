from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.server import WptFixtureServer


COMMON_REDIRECT = "/common/redirect.py"


class CommonRedirectFixtureTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, query="", *, method="GET", headers=None, body=None, port=None, path=COMMON_REDIRECT):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=2)
        try:
            connection.request(method, path + "?" + query, body, headers or {})
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_common_redirect_cors_requires_opt_in_and_nonempty_origin(self):
        for flag in ("", "&enable-cors", "&enable-cors=false"):
            for origin in (None, "", "null", "https://caller.test"):
                with self.subTest(flag=flag, origin=origin):
                    status, headers, body = self.request(
                        "location=/target" + flag, path=COMMON_REDIRECT,
                        headers={} if origin is None else {"Origin": origin},
                    )
                    self.assertEqual((status, body), (302, b""))
                    self.assertEqual(headers["Location"], "/target")
                    allowed = bool(flag and origin)
                    self.assertEqual(headers["Access-Control-Allow-Origin"], origin if allowed else None)
                    self.assertEqual(headers["Access-Control-Allow-Credentials"], "true" if allowed else None)
                    self.assertEqual(headers["Content-Type"], "text/plain" if allowed else None)
                    self.assertIsNone(headers["Access-Control-Allow-Methods"])
                    self.assertIsNone(headers["Cache-Control"])
                    self.assertIsNone(headers["Content-Length"])

    def test_common_redirect_handles_standard_and_extension_methods(self):
        for method in ("GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "YO", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    "location=/target&status=307", method=method, path=COMMON_REDIRECT,
                    headers={"Origin": "https://caller.test"}, body=b"unused upload",
                )
                self.assertEqual((status, body), (307, b""))
                self.assertEqual(headers["Location"], "/target")
                self.assertIsNone(headers["Access-Control-Allow-Origin"])
                self.assertIsNone(headers["Timing-Allow-Origin"])

    def test_common_redirect_uses_first_parameters_and_preserves_raw_bytes(self):
        for query, status, location in [
            ("location=", 302, ""),
            ("location=/first&location=/second", 302, "/first"),
            ("location=/target&status=200", 200, "/target"),
            ("location=/target&status=404", 404, "/target"),
            ("location=/target&status=301&status=307", 301, "/target"),
            ("location=/target&status=wrong", 302, "/target"),
            ("location=/target&status=", 302, "/target"),
            ("location=/target&status=%A0307%A0", 302, "/target"),
            ("location=/target&redirect_status=307", 302, "/target"),
            ("location=/caf%E9", 302, "/café"),
        ]:
            with self.subTest(query=query):
                actual_status, headers, body = self.request(query, path=COMMON_REDIRECT)
                self.assertEqual((actual_status, body), (status, b""))
                self.assertEqual(headers.get_all("Location"), [location])
        self.assertEqual(self.request(path=COMMON_REDIRECT)[0], 500)

    def test_common_redirect_responds_before_unused_uploads_finish(self):
        for method in ("POST", "OPTIONS"):
            connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
            try:
                connection.putrequest(method, COMMON_REDIRECT + "?location=/target&enable-cors")
                connection.putheader("Content-Length", "1000000")
                connection.putheader("Origin", "https://caller.test")
                connection.endheaders()
                response = connection.getresponse()
                self.assertEqual((response.status, response.read()), (302, b""))
                self.assertEqual(response.headers["Location"], "/target")
                self.assertEqual(response.headers["Access-Control-Allow-Origin"], "https://caller.test")
                self.assertEqual(response.headers["Connection"], "close")
            finally:
                connection.close()
