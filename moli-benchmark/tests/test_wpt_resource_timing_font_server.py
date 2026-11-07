import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from threading import Event
from unittest.mock import patch

from moli_benchmark.wpt_cross.server import WptFixtureServer


FONT_PATH = "/resource-timing/resources/cors-ahem.py"


class ResourceTimingFontFixtureTests(unittest.TestCase):
    def setUp(self):
        stack = ExitStack()
        self.addCleanup(stack.close)
        self.root = Path(stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        handler = self.root / FONT_PATH.lstrip("/")
        handler.parent.mkdir(parents=True)
        handler.write_text("raise AssertionError('never expose the Python handler')")
        (self.root / "fonts").mkdir()
        self.font = b"\x00\x01\x00\x00the requested Ahem font bytes\x00\xff"
        (self.root / "fonts/Ahem.ttf").write_bytes(self.font)
        stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))
        self.server = stack.enter_context(WptFixtureServer(self.root))

    def request(self, *, method="GET", etag=None, alternate=False, query="unique=opaque-cache-key"):
        connection = HTTPConnection(
            "127.0.0.1",
            self.server.alternate_port if alternate else self.server.port,
            timeout=2,
        )
        try:
            headers = {"Origin": "http://remote.test"}
            if etag is not None:
                headers["If-None-Match"] = etag
            connection.request(method, FONT_PATH + "?" + query, headers=headers)
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_font_body_and_permission_headers_match_the_upstream_handler(self):
        for method in ("GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "YO", "chicken"):
            for alternate in (False, True):
                with self.subTest(method=method, alternate=alternate):
                    status, headers, body = self.request(method=method, alternate=alternate)
                    self.assertEqual(status, 200)
                    self.assertEqual(body, self.font if method != "HEAD" else b"")
                    self.assertEqual(headers["Content-Type"], "font/truetype")
                    self.assertEqual(headers["Content-Length"], str(len(self.font)))
                    self.assertEqual(headers.get_all("Cache-Control"), ["public, max-age=86400"])
                    self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
                    self.assertEqual(headers["Timing-Allow-Origin"], "*")
                    self.assertEqual(headers["ETag"], "123abc")

    def test_conditional_response_leaves_cached_permissions_to_the_client(self):
        for method in ("GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "YO", "chicken"):
            for etag in ("123abc", "different", '"123abc"', "W/123abc"):
                with self.subTest(method=method, etag=etag):
                    status, headers, body = self.request(method=method, etag=etag)
                    if etag == "123abc":
                        self.assertEqual((status, body), (304, b""))
                        self.assertEqual(headers["X-HTTP-STATUS"], "304")
                        # The original handler deliberately omits these on 304.
                        # Supplying them would avoid testing cache header reuse.
                        for name in (
                            "Timing-Allow-Origin", "Access-Control-Allow-Origin",
                            "Cache-Control", "Content-Type", "ETag",
                        ):
                            self.assertIsNone(headers[name], name)
                    else:
                        self.assertEqual(status, 200)
                        self.assertEqual(body, self.font if method != "HEAD" else b"")
                        self.assertEqual(headers["Timing-Allow-Origin"], "*")

    def test_handler_does_not_wait_for_unused_request_bodies(self):
        for method in ("POST", "OPTIONS", "chicken"):
            with self.subTest(method=method):
                connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
                try:
                    connection.putrequest(method, FONT_PATH)
                    connection.putheader("Content-Length", "1000000")
                    connection.endheaders()
                    response = connection.getresponse()
                    self.assertEqual(response.status, 200)
                    self.assertEqual(response.read(), self.font)
                finally:
                    connection.close()

    def test_response_pipes_override_the_dynamic_font_response(self):
        status, headers, body = self.request(query=(
            "pipe=status(404)|header(Timing-Allow-Origin,http://caller.test)"
            "|header(Content-Type,application/octet-stream)"
        ))
        self.assertEqual((status, body), (404, self.font))
        self.assertEqual(headers.get_all("Timing-Allow-Origin"), ["http://caller.test"])
        self.assertEqual(headers.get_all("Content-Type"), ["application/octet-stream"])
        self.assertEqual(headers["ETag"], "123abc")
        self.assertEqual(self.request(query="pipe=unknown()")[:1], (500,))

    def test_trickle_sends_font_headers_before_waiting_for_the_body(self):
        entered, released = Event(), Event()
        delays = []

        def hold_body(seconds):
            delays.append(seconds)
            entered.set()
            released.wait(2)

        connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
        try:
            with patch("moli_benchmark.wpt_cross.server.time.sleep", side_effect=hold_body):
                connection.request("GET", FONT_PATH + "?pipe=trickle(d1)")
                response = connection.getresponse()
                self.assertEqual(response.status, 200)
                self.assertEqual(response.headers["Timing-Allow-Origin"], "*")
                self.assertTrue(entered.wait(2), "body delay must begin after headers")
                released.set()
                self.assertEqual(response.read(), self.font)
            self.assertEqual(delays, [1.0])
        finally:
            released.set()
            connection.close()
