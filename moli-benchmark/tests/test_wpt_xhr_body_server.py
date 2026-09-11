from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


class XhrBodyFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def server(self) -> WptFixtureServer:
        return self.stack.enter_context(WptFixtureServer(self.root))

    def request(
        self, port: int, path: str, *, method: str = "GET",
        headers: tuple[tuple[str, str], ...] = (), body: bytes = b"",
    ) -> tuple[int, dict[str, str], bytes]:
        connection = HTTPConnection("127.0.0.1", port, timeout=2)
        try:
            connection.putrequest(method, "/xhr/resources/" + path)
            for name, value in headers:
                connection.putheader(name, value)
            connection.endheaders(body)
            response = connection.getresponse()
            return (
                response.status,
                {name.lower(): value for name, value in response.getheaders()},
                response.read(),
            )
        finally:
            connection.close()

    def test_content_echoes_binary_upload_and_request_metadata_for_all_methods(self) -> None:
        server = self.server()
        payload = b"\x00\xff\xc3\xa9\r\n"
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "YO", "FOO"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    server.port, "content.py?raw=%FF+%20", method=method,
                    headers=(("Content-Length", str(len(payload))),
                             ("Content-Type", "application/custom; charset=ascii")),
                    body=payload,
                )
                self.assertEqual(status, 200)
                self.assertEqual(body, b"" if method == "HEAD" else payload)
                self.assertEqual(headers["content-length"], str(len(payload)))
                self.assertEqual(headers["content-type"], "text/plain")
                self.assertEqual(headers["x-request-method"], method)
                self.assertEqual(headers["x-request-query"], "raw=%FF+%20")
                self.assertEqual(headers["x-request-content-length"], str(len(payload)))
                self.assertEqual(headers["x-request-content-type"], "application/custom; charset=ascii")

    def test_content_distinguishes_missing_and_empty_headers(self) -> None:
        server = self.server()
        for request_headers, expected in (
            ((), "NO"),
            ((("Content-Type", ""),), ""),
            ((("content-TYPE", "caf\xe9 \t"),), "caf\xe9 \t"),
            ((("Content-Type", "first"), ("content-type", "second")), "first"),
        ):
            with self.subTest(headers=request_headers):
                status, headers, body = self.request(server.port, "content.py", headers=request_headers)
                self.assertEqual((status, body), (200, b""))
                self.assertEqual(headers["x-request-query"], "NO")
                self.assertEqual(headers["x-request-content-length"], "NO")
                self.assertEqual(headers["x-request-content-type"], expected)

    def test_content_query_override_uses_first_byte_values_and_preserves_charset_label(self) -> None:
        server = self.server()
        for query, expected_type, expected_body in (
            ("content=%00%FF+a&content=ignored&response_charset_label=windows-1252", "text/plain;charset=windows-1252", b"\x00\xff a"),
            ("content=&content=ignored&response_charset_label=&response_charset_label=UTF-8", "text/plain;charset=", b""),
            ("content=x&response_charset_label=UTF-8&response_charset_label=ignored", "text/plain;charset=UTF-8", b"x"),
        ):
            with self.subTest(query=query):
                status, headers, body = self.request(server.port, "content.py?" + query)
                self.assertEqual((status, body), (200, expected_body))
                self.assertEqual(headers["content-type"], expected_type)
                self.assertEqual(headers["x-request-query"], query)

    def test_content_reads_upload_before_reusing_connection(self) -> None:
        server = self.server()
        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
        self.addCleanup(connection.close)
        for payload in (b"first", b"second"):
            connection.request("POST", "/xhr/resources/content.py", payload)
            response = connection.getresponse()
            self.assertEqual(response.status, 200)
            self.assertEqual(response.read(), payload)

    def test_echo_content_type_preserves_header_value_and_closes_connection(self) -> None:
        server = self.server()
        for value in (None, "", "Application/JSON; charset=UTF-8", "caf\xe9 \t"):
            with self.subTest(value=value):
                status, headers, body = self.request(
                    server.port, "echo-content-type.py", method="POST",
                    headers=() if value is None else (("Content-Type", value),),
                )
                self.assertEqual((status, body), (200, (value or "").encode("latin-1")))
                self.assertEqual(headers["content-type"], "text/plain")
                self.assertEqual(headers["connection"], "close")

    def test_early_responses_do_not_wait_for_overridden_or_unused_body(self) -> None:
        server = self.server()
        for path, expected in (
            ("content.py?content=override", b"override"),
            ("echo-content-type.py", b"application/test"),
            ("empty-div-utf8-html.py", b"<!DOCTYPE html><div></div>"),
        ):
            with self.subTest(path=path):
                status, headers, body = self.request(
                    server.port, path, method="POST",
                    headers=(("Content-Length", "1000000"), ("Content-Type", "application/test")),
                )
                self.assertEqual((status, body), (200, expected))
                self.assertEqual(headers["connection"], "close")

    def test_document_fixtures_preserve_encoding_labels_and_exact_bytes(self) -> None:
        server = self.server()
        for path, expected_type, expected in (
            ("win-1252-xml.py", "application/xml;charset=windows-1252", b"<\xff/>"),
            ("win-1252-html.py", "text/html;charset=windows-1252", b"\xc3\xbf"),
            ("invalid-utf8-html.py", "text/html;charset=utf-8", b"\xff"),
            ("shift-jis-html.py", "text/html;charset=shift-jis", "テスト".encode("shift-jis")),
            ("img-utf8-html.py", "text/html;charset=utf-8", b"<img>foo"),
            ("empty-div-utf8-html.py", "text/html;charset=utf-8", b"<!DOCTYPE html><div></div>"),
        ):
            for method in ("GET", "HEAD"):
                with self.subTest(path=path, method=method):
                    status, headers, body = self.request(server.port, path, method=method)
                    self.assertEqual((status, body), (200, b"" if method == "HEAD" else expected))
                    self.assertEqual(headers["content-type"], expected_type)
                    self.assertEqual(headers["content-length"], str(len(expected)))

    def test_case_selection_recognizes_body_fixtures_without_matching_unhandled_paths(self) -> None:
        sources = {
            "xhr/content.window.js": "fetch('resources/content.py');",
            "xhr/echo.window.js": "fetch('/xhr/resources/echo-content-type.py');",
            "xhr/nested/doc.window.js": "fetch('../resources/shift-jis-html.py');",
            "xhr/unknown.window.js": "fetch('resources/content.py'); fetch('resources/unknown.py');",
            "xhr/suffix.window.js": "fetch('resources/content.py2');",
            "xhr/wrong.window.js": "fetch('/other/resources/echo-content-type.py');",
        }
        for path, source in sources.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(source)
        self.assertEqual([case.case_path for case in enumerate_cases(self.root, dir_prefixes=("xhr",))], [
            "xhr/content.window.js?moli-wpt-script=window",
            "xhr/echo.window.js?moli-wpt-script=window",
            "xhr/nested/doc.window.js?moli-wpt-script=window",
        ])
