from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.server import WptFixtureServer


class TemplateHeaderFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, path: str, headers=(), *, method: str = "GET"):
        connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
        try:
            connection.putrequest(method, path)
            for name, value in headers:
                connection.putheader(name, value)
            connection.endheaders()
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_required_header_lookup_is_case_insensitive_and_preserves_empty_values(self) -> None:
        (self.root / "page.sub.html").write_bytes(
            b"{{headers[referer]}}|{{headers[X-Empty]}}|{{header_or_default(X-Missing,<none>)}}"
        )
        headers = [("rEfErEr", "https://example.test/page?a=1&b=2"), ("X-Empty", "")]
        expected = b"https://example.test/page?a=1&amp;b=2||&lt;none&gt;"
        for method in ("GET", "HEAD"):
            with self.subTest(method=method):
                status, response_headers, body = self.request("/page.sub.html", headers, method=method)
                self.assertEqual(status, 200)
                self.assertEqual(body, b"" if method == "HEAD" else expected)
                self.assertEqual(response_headers["Content-Length"], str(len(expected)))

    def test_duplicate_header_fields_are_combined_in_request_order(self) -> None:
        (self.root / "page.sub.txt").write_bytes(
            b"{{headers[x-test]}}|{{header_or_default(X-Test,missing)}}"
        )
        status, _, body = self.request(
            "/page.sub.txt", [("X-Test", "first"), ("x-test", "second"), ("X-Test", "third")],
        )
        self.assertEqual(status, 200)
        self.assertEqual(body, b"first, second, third|first, second, third")

    def test_header_escaping_follows_filename_and_explicit_sub_mode(self) -> None:
        template = b"{{headers[X-Test]}}|{{header_or_default(X-Test,missing)}}"
        for name in ("page.sub.html", "page.sub.xml", "page.sub.js", "page.sub.txt", "page.txt"):
            (self.root / name).write_bytes(template)
        value = '<&"\''
        escaped = b"&lt;&amp;&quot;&#x27;"
        for path, expected in (
            ("/page.sub.html", escaped), ("/page.sub.xml", escaped),
            ("/page.sub.js", value.encode()), ("/page.sub.txt", value.encode()),
            ("/page.txt?pipe=sub", escaped), ("/page.txt?pipe=sub(html)", escaped),
            ("/page.txt?pipe=sub(none)", value.encode()),
            ("/page.sub.html?pipe=sub(none)", escaped),
        ):
            with self.subTest(path=path):
                status, _, body = self.request(path, [("X-Test", value)])
                self.assertEqual(status, 200)
                self.assertEqual(body, expected + b"|" + expected)
        self.assertEqual(self.request("/page.txt", [("X-Test", value)])[2], template)

    def test_sub_headers_use_raw_values_and_plain_headers_and_pipe_values_stay_literal(self) -> None:
        (self.root / "page.txt").write_bytes(b"body")
        (self.root / "page.txt.sub.headers").write_text(
            "X-Echo: {{headers[X-Test]}}\nX-Default: {{header_or_default(X-Test,missing)}}\n"
        )
        value = '<&"\''
        status, headers, body = self.request("/page.txt", [("X-Test", value)])
        self.assertEqual((status, body), (200, b"body"))
        self.assertEqual(headers["X-Echo"], value)
        self.assertEqual(headers["X-Default"], value)
        (self.root / "page.txt.sub.headers").unlink()
        (self.root / "page.txt.headers").write_text("X-Echo: {{headers[X-Test]}}\n")
        self.assertEqual(self.request("/page.txt", [("X-Test", value)])[1]["X-Echo"],
                         "{{headers[X-Test]}}")
        query = urlencode({"pipe": "header(X-Echo,{{headers[X-Test]}})"})
        self.assertEqual(self.request("/page.txt?" + query, [("X-Test", value)])[1]["X-Echo"],
                         "{{headers[X-Test]}}")

    def test_missing_required_headers_fail_without_closing_the_connection_abruptly(self) -> None:
        (self.root / "body.sub.txt").write_bytes(b"{{headers[X-Missing]}}")
        (self.root / "sidecar.txt").write_bytes(b"body")
        (self.root / "sidecar.txt.sub.headers").write_text("X-Echo: {{headers[X-Missing]}}\n")
        for path in ("/body.sub.txt", "/sidecar.txt"):
            for method in ("GET", "HEAD"):
                with self.subTest(path=path, method=method):
                    status, _, body = self.request(path, method=method)
                    self.assertEqual(status, 500)
                    if method == "HEAD":
                        self.assertEqual(body, b"")

    def test_request_values_are_not_interpreted_as_more_templates(self) -> None:
        (self.root / "page.sub.txt").write_bytes(
            b"{{headers[X-Test]}}|{{header_or_default(X-Test,missing)}}|{{GET[q]}}"
        )
        value = "{{GET[q]}} {{headers[X-Missing]}} {{$id:uuid()}} {{host}}"
        query_value = "{{headers[X-Missing]}}"
        status, _, body = self.request(
            "/page.sub.txt?" + urlencode({"q": query_value}), [("X-Test", value)],
        )
        self.assertEqual(status, 200)
        self.assertEqual(body, (value + "|" + value + "|" + query_value).encode())

    def test_template_header_values_decode_utf8_request_bytes(self) -> None:
        (self.root / "page.sub.html").write_bytes(b"{{headers[X-Test]}}")
        status, _, body = self.request("/page.sub.html", [("X-Test", b"caf\xc3\xa9")])
        self.assertEqual((status, body), (200, b"caf\xc3\xa9"))
        self.assertEqual(self.request("/page.sub.html", [("X-Test", b"caf\xe9")])[0], 500)


if __name__ == "__main__":
    unittest.main()
