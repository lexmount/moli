from __future__ import annotations

import socket
import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection, IncompleteRead
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlsplit

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE_DIR = "/xhr/resources/"


class XhrNetworkErrorFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        (self.root / "xhr/resources").mkdir(parents=True)
        for name in ("bad-chunk-encoding.py", "infinite-redirects.py"):
            (self.root / "xhr/resources" / name).write_text("# must not serve Python source")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def server(self) -> WptFixtureServer:
        return self.stack.enter_context(WptFixtureServer(self.root))

    def test_bad_chunk_response_preserves_wire_framing_and_explicit_head_writes(self) -> None:
        server = self.server()
        for method in ("GET", "HEAD", "CHICKEN"):
            with self.subTest(method=method), socket.create_connection(
                ("127.0.0.1", server.port), timeout=3
            ) as connection:
                connection.sendall((
                    f"{method} {RESOURCE_DIR}bad-chunk-encoding.py HTTP/1.1\r\n"
                    f"Host: localhost:{server.port}\r\n\r\n"
                ).encode())
                with connection.makefile("rb") as response:
                    headers, body = response.read().split(b"\r\n\r\n", 1)
                self.assertTrue(headers.startswith(b"HTTP/1.1 200 "))
                fields = headers.lower().split(b"\r\n")[1:]
                for field in (
                    b"transfer-encoding: chunked", b"content-type: text/plain",
                    b"x-content-type-options: nosniff", b"connection: close",
                ):
                    self.assertIn(field, fields)
                self.assertFalse(any(field.startswith(b"content-length:") for field in fields))
                self.assertEqual(body, b"a\r\nTEST_CHUNK\r\n" * 5 + b"garbage")

    def test_http_client_can_read_partial_body_before_chunk_decoding_fails(self) -> None:
        server = self.server()
        connection = HTTPConnection("127.0.0.1", server.port, timeout=3)
        self.addCleanup(connection.close)
        # This upstream handler never reads the upload, even when it is unfinished.
        connection.putrequest("POST", RESOURCE_DIR + "bad-chunk-encoding.py")
        connection.putheader("Content-Length", "1000000")
        connection.endheaders()
        response = connection.getresponse()
        self.assertEqual(response.status, 200)
        self.assertTrue(response.chunked)
        self.assertEqual(response.read(10), b"TEST_CHUNK")
        with self.assertRaises(IncompleteRead) as failure:
            response.read()
        self.assertEqual(failure.exception.partial, b"TEST_CHUNK" * 4)

    def redirect(
        self, port: int, target: str, *, method: str = "GET", host: str | None = None,
    ) -> tuple[int, dict[str, str], bytes]:
        connection = HTTPConnection("127.0.0.1", port, timeout=3)
        try:
            connection.request(method, target, headers={"Host": host} if host else {})
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def test_infinite_redirect_query_parameters_and_status_match_upstream(self) -> None:
        server = self.server()
        base = f"http://localhost:{server.port}" + RESOURCE_DIR + "infinite-redirects.py"
        cases = [
            ("", "alternate", 302, 0),
            ("page=alternate", "default", 302, 0),
            ("page=alternate&page=default&type=301&type=302&mix=1&mix=0", "default", 302, 1),
            ("page=default&page=alternate&type=301&mix=0", "alternate", 301, 0),
            ("type=other&mix=1", "alternate", 301, 1),
            ("page=&type=&mix=", "alternate", 302, 0),
        ]
        for query, page, redirect_type, mix in cases:
            with self.subTest(query=query):
                status, headers, body = self.redirect(server.port, base + "?" + query)
                expected = f"{base}?page={page}&type={redirect_type}&mix={mix}"
                # Upstream always returns 301; `type` only controls the next URL.
                self.assertEqual(status, 301)
                self.assertEqual(headers["Location"], expected)
                self.assertIn("no-cache", headers["Cache-Control"])
                self.assertEqual(headers["Pragma"], "no-cache")
                self.assertEqual(body, ("Hello guest. You have been redirected to " + expected).encode())

    def test_infinite_redirects_keep_looping_on_the_requested_origin(self) -> None:
        server = self.server()
        for port, host in ((server.port, "www1.localhost"),
                           (server.alternate_port, f"www2.localhost:{server.alternate_port}")):
            for method in ("GET", "HEAD", "OPTIONS", "CHICKEN"):
                with self.subTest(port=port, host=host, method=method):
                    target = RESOURCE_DIR + "%69nfinite-redirects.py?mix=1"
                    authority = host if ":" in host else f"{host}:{port}"
                    for hop in range(6):
                        status, headers, body = self.redirect(port, target, method=method, host=host)
                        self.assertEqual(status, 301)
                        next_url = urlsplit(headers["Location"])
                        self.assertEqual((next_url.scheme, next_url.netloc), ("http", authority))
                        self.assertEqual(next_url.path, RESOURCE_DIR + "%69nfinite-redirects.py")
                        self.assertEqual(next_url.query, (
                            "page=alternate&type=301&mix=1" if hop % 2 == 0
                            else "page=default&type=302&mix=1"
                        ))
                        expected_body = ("Hello guest. You have been redirected to " + headers["Location"]).encode()
                        self.assertEqual(headers["Content-Length"], str(len(expected_body)))
                        self.assertEqual(body, b"" if method == "HEAD" else expected_body)
                        target = headers["Location"]

    def test_case_selection_accepts_only_the_supported_error_fixture_paths(self) -> None:
        sources = {
            "xhr/error.any.js": "fetch('resources/bad-chunk-encoding.py');",
            "xhr/loop.html": "fetch('/xhr/resources/infinite-redirects.py');",
            "xhr/nested/both.html": "fetch('../resources/infinite-redirects.py'); fetch('../resources/bad-chunk-encoding.py');",
            "xhr/unknown.html": "fetch('resources/infinite-redirects.py'); fetch('resources/unknown.py');",
            "xhr/suffix.html": "fetch('resources/bad-chunk-encoding.py-extra');",
            "xhr/prefix.html": "fetch('/wrong/xhr/resources/infinite-redirects.py');",
            "xhr/wrong-parent.html": "fetch('../resources/bad-chunk-encoding.py');",
            "xhr/fetch-handler.html": "fetch('/fetch/api/resources/bad-chunk-encoding.py');",
        }
        for path, source in sources.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            if path.endswith(".html"):
                source = '<script src="/resources/testharness.js"></script><script>' + source + "</script>"
            target.write_text(source)
        self.assertEqual([case.case_path for case in enumerate_cases(
            self.root, dir_prefixes=("xhr",), any_js_global="both",
        )], [
            "xhr/error.any.js?moli-wpt-any=dedicatedworker",
            "xhr/error.any.js?moli-wpt-any=window",
            "xhr/loop.html", "xhr/nested/both.html",
        ])
