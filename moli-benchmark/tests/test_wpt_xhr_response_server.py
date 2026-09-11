from __future__ import annotations

import os
import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


class XhrResponseFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        (self.root / "xhr/resources").mkdir(parents=True)
        self.xml = self.root / "xhr/resources/well-formed.xml"
        self.xml.write_bytes(b"<x>caf\xc3\xa9\r\n</x>")
        os.utime(self.xml, (946684800.875, 946684800.875))
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def server(self) -> WptFixtureServer:
        return self.stack.enter_context(WptFixtureServer(self.root))

    def request(
        self, port: int, path: str, *, method: str = "GET",
    ) -> tuple[int, str, dict[str, str], bytes]:
        connection = HTTPConnection("127.0.0.1", port, timeout=2)
        try:
            connection.request(method, "/xhr/resources/" + path)
            response = connection.getresponse()
            return (
                response.status, response.reason,
                {name.lower(): value for name, value in response.getheaders()},
                response.read(),
            )
        finally:
            connection.close()

    def test_status_preserves_first_query_values_and_raw_response_bytes(self) -> None:
        server = self.server()
        cases = [
            ("", 200, "OMG", "", b""),
            ("code=402&code=200&text=caf%E9&text=ignored&type=text/xml&type=text/plain&content=%00%FF+%2B%25ff&content=ignored", 402, "caf\xe9", "text/xml", b"\x00\xff +%ff"),
            ("text=&type=&content=&content=ignored", 200, "", "", b""),
            ("code=699&text=custom&type=text/plain&content=unknown-status", 699, "custom", "text/plain", b"unknown-status"),
            ("type=application/xml;charset=windows-1252&content=%3Cx%3E%E6%A9%9F%3C%2Fx%3E", 200, "OMG", "application/xml;charset=windows-1252", b"<x>\xe6\xa9\x9f</x>"),
        ]
        for query, code, reason, content_type, expected in cases:
            with self.subTest(query=query):
                status, text, headers, body = self.request(server.port, "status.py?" + query)
                self.assertEqual((status, text, body), (code, reason, expected))
                self.assertEqual(headers["content-type"], content_type)
                self.assertEqual(headers["x-request-method"], "GET")
                self.assertEqual(headers["content-length"], str(len(expected)))
        for code in ("", "invalid"):
            with self.subTest(code=code):
                self.assertEqual(self.request(server.port, "status.py?code=" + code)[0], 500)

    def test_status_supports_custom_methods_and_head_preserves_response_metadata(self) -> None:
        server = self.server()
        for method in ("GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS", "CHICKEN"):
            with self.subTest(method=method):
                status, reason, headers, body = self.request(
                    server.port, "status.py?code=402&text=FIVE+BUCKS&type=text/xml&content=%3Cx/%3E",
                    method=method,
                )
                self.assertEqual((status, reason), (402, "FIVE BUCKS"))
                self.assertEqual(body, b"" if method == "HEAD" else b"<x/>")
                self.assertEqual(headers["content-length"], "4")
                self.assertEqual(headers["content-type"], "text/xml")
                self.assertEqual(headers["x-request-method"], method)

    def test_last_modified_uses_xml_file_contents_and_mtime_without_caching(self) -> None:
        server = self.server()
        for method in ("GET", "HEAD", "POST", "CHICKEN"):
            with self.subTest(method=method):
                status, _, headers, body = self.request(server.port, "last-modified.py", method=method)
                self.assertEqual(status, 200)
                self.assertEqual(body, b"" if method == "HEAD" else b"<x>caf\xc3\xa9\n</x>")
                self.assertEqual(headers["content-type"], "application/xml")
                self.assertEqual(headers["last-modified"], "Sat, 01 Jan 2000 00:00:00 GMT")
                self.assertEqual(headers["content-length"], str(len(b"<x>caf\xc3\xa9\n</x>")))
        self.xml.write_text("<updated/>")
        os.utime(self.xml, (946771200, 946771200))
        status, _, headers, body = self.request(server.port, "last-modified.py?ignored=1")
        self.assertEqual((status, body), (200, b"<updated/>"))
        self.assertEqual(headers["last-modified"], "Sun, 02 Jan 2000 00:00:00 GMT")
        self.xml.unlink()
        self.assertEqual(self.request(server.port, "last-modified.py")[0], 500)

    def test_response_fixtures_do_not_wait_for_unused_uploads(self) -> None:
        server = self.server()
        for path, expected in (("status.py?content=response", b"response"),
                               ("last-modified.py", b"<x>caf\xc3\xa9\n</x>")):
            for framing in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
                with self.subTest(path=path, framing=framing):
                    connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
                    try:
                        connection.putrequest("POST", "/xhr/resources/" + path)
                        connection.putheader(*framing)
                        connection.endheaders()
                        response = connection.getresponse()
                        self.assertEqual((response.status, response.read()), (200, expected))
                        self.assertEqual(response.headers["Connection"], "close")
                    finally:
                        connection.close()

    def test_case_selection_accepts_only_the_supported_response_fixture_paths(self) -> None:
        sources = {
            "xhr/status.window.js": "fetch('resources/status.py');",
            "xhr/metadata.html": "fetch('/xhr/resources/last-modified.py?ignored');",
            "xhr/nested/both.html": "fetch('../resources/status.py'); fetch('../resources/last-modified.py');",
            "xhr/unknown.html": "fetch('resources/status.py'); fetch('resources/unknown.py');",
            "xhr/suffix.html": "fetch('resources/status.py-extra');",
            "xhr/prefix.html": "fetch('/wrong/xhr/resources/last-modified.py');",
            "xhr/wrong-parent.html": "fetch('../resources/status.py');",
        }
        for path, source in sources.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            if path.endswith(".html"):
                source = (
                    '<script src="/resources/testharness.js"></script>'
                    f"<script>{source}</script>"
                )
            target.write_text(source)
        self.assertEqual([case.case_path for case in enumerate_cases(self.root, dir_prefixes=("xhr",))], [
            "xhr/metadata.html", "xhr/nested/both.html",
            "xhr/status.window.js?moli-wpt-script=window",
        ])
