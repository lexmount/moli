from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE_DIR = "/xhr/resources/"


class XhrUrlFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        (self.root / "resources/testharnessreport.js").write_text("// report")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def server(self, *, keep_alive: bool = False) -> WptFixtureServer:
        server = WptFixtureServer(self.root)
        if keep_alive:
            server.httpd.RequestHandlerClass.protocol_version = "HTTP/1.1"
        return self.stack.enter_context(server)

    def request(
        self, port: int, target: str, *, method: str = "GET",
        headers: dict[str, str] | None = None,
    ) -> tuple[int, str, dict[str, str], bytes]:
        connection = HTTPConnection("127.0.0.1", port, timeout=2)
        try:
            connection.request(method, target, headers=headers or {})
            response = connection.getresponse()
            return (
                response.status, response.reason,
                {name.lower(): value for name, value in response.getheaders()},
                response.read(),
            )
        finally:
            connection.close()

    def test_request_uri_preserves_raw_path_and_query(self) -> None:
        server = self.server()
        for path in (
            "requri.py", "requri.py?", "requri.py?help=",
            "requri.py?%23foobar", "%72equri.py?a=%2f%23&b=+%20",
        ):
            with self.subTest(path=path):
                target = RESOURCE_DIR + path
                status, _, headers, body = self.request(server.port, target)
                self.assertEqual((status, body), (200, target.encode()))
                self.assertNotIn("content-type", headers)

        for query in ("full", "full=", "full=0", "full&full=ignored"):
            with self.subTest(query=query):
                target = RESOURCE_DIR + "requri.py?" + query
                for host, authority in (
                    ("example.test:1234", "example.test:1234"),
                    ("example.test", f"example.test:{server.port}"),
                ):
                    status, _, _, body = self.request(
                        server.port, target, headers={"Host": host}
                    )
                    self.assertEqual(
                        (status, body), (200, f"http://{authority}{target}".encode())
                    )

        target = "http://example.test:1234" + RESOURCE_DIR + "requri.py?full"
        status, _, _, body = self.request(server.port, target)
        self.assertEqual((status, body), (200, target.encode()))

    def test_fixtures_accept_custom_methods_and_head_omits_only_body(self) -> None:
        server = self.server()
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "YO", "CHICKEN"):
            for resource, status, expected in (
                ("requri.py?x=%23", 200, b"/xhr/resources/requri.py?x=%23"),
                ("redirect.py", 302, b"TEST"),
            ):
                with self.subTest(method=method, resource=resource):
                    actual, _, headers, body = self.request(
                        server.port, RESOURCE_DIR + resource, method=method
                    )
                    self.assertEqual(actual, status)
                    self.assertEqual(body, b"" if method == "HEAD" else expected)
                    self.assertEqual(headers["content-length"], str(len(expected)))
                    self.assertNotIn("content-type", headers)
        status, _, _, _ = self.request(server.port, "/unhandled.py", method="CHICKEN")
        self.assertEqual(status, 501)

    def test_redirect_status_location_and_first_query_values(self) -> None:
        server = self.server()
        cases = [
            ("", 302, RESOURCE_DIR + "redirect.py?followed"),
            ("code=307&code=301&location=first&location=second", 307, "first"),
            ("location=%252Ftarget%253Fx%253D1%2523fragment", 302, "/target?x=1#fragment"),
            ("location=redirect.py%3Flocation%3Dcontent.py&code=308", 308,
             "redirect.py?location=content.py&code=308"),
            ("location=", 302, ""),
            (urlencode({"location": "javascript:invalid"}), 302, "javascript:invalid"),
            (urlencode({"location": "http://example.test/target?q=1"}), 302,
             "http://example.test/target?q=1"),
        ]
        cases.extend((f"code={code}", code, RESOURCE_DIR + "redirect.py?followed")
                     for code in (200, 300, 301, 303, 399))
        for query, code, location in cases:
            with self.subTest(query=query):
                status, reason, headers, body = self.request(
                    server.port, RESOURCE_DIR + "redirect.py?" + query
                )
                self.assertEqual((status, reason, body), (code, "WEBSRT MARKETING", b"TEST"))
                self.assertEqual(headers["location"], location)
                self.assertNotIn("content-type", headers)

    def test_redirect_followed_response_and_delay_match_upstream(self) -> None:
        server = self.server()
        _, _, redirect_headers, _ = self.request(server.port, RESOURCE_DIR + "redirect.py")
        status, _, headers, body = self.request(server.port, redirect_headers["location"])
        self.assertEqual((status, body), (200, b"MAGIC HAPPENED"))
        # Upstream spells this header "Content:Type"; it is not Content-Type.
        self.assertEqual(headers["content"], "Type: text/plain")
        self.assertNotIn("content-type", headers)
        self.assertNotIn("location", headers)
        # Once followed, the parsed location is no longer used as a header.
        status, _, headers, body = self.request(
            server.port, RESOURCE_DIR + "redirect.py?followed&location=%E2%82%AC"
        )
        self.assertEqual((status, body), (200, b"MAGIC HAPPENED"))
        self.assertNotIn("location", headers)
        with patch("moli_benchmark.wpt_cross.server.time.sleep") as sleep:
            for query, method in (
                ("delay=250&delay=1000", "GET"),
                ("delay=250&followed=", "GET"),
                ("delay=250", "HEAD"),
            ):
                self.request(server.port, RESOURCE_DIR + "redirect.py?" + query, method=method)
                sleep.assert_called_once_with(0.25)
                sleep.reset_mock()
        for query in (
            "code=invalid", "code=", "delay=invalid", "delay=nan", "delay=inf",
            "delay=-1", "location=%FF", "location=%250D%250A",
            "code=invalid&followed",
        ):
            with self.subTest(query=query):
                status, _, _, _ = self.request(server.port, RESOURCE_DIR + "redirect.py?" + query)
                self.assertEqual(status, 500)

    def test_response_can_arrive_before_upload_body(self) -> None:
        server = self.server(keep_alive=True)
        for resource, expected in (
            ("redirect.py", (302, b"TEST")),
            ("requri.py", (200, b"/xhr/resources/requri.py")),
        ):
            for framing in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
                with self.subTest(resource=resource, framing=framing):
                    connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
                    try:
                        connection.putrequest("POST", RESOURCE_DIR + resource)
                        connection.putheader(*framing)
                        connection.endheaders()
                        # No upload bytes have been sent when we read the response.
                        response = connection.getresponse()
                        self.assertEqual((response.status, response.read()), expected)
                        self.assertEqual(response.headers["Connection"], "close")
                        self.assertTrue(response.will_close)
                    finally:
                        connection.close()

    def test_bodyless_requests_can_reuse_connection(self) -> None:
        server = self.server(keep_alive=True)
        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
        try:
            connection.request("GET", RESOURCE_DIR + "redirect.py")
            redirect = connection.getresponse()
            self.assertEqual((redirect.status, redirect.read()), (302, b"TEST"))
            first_socket = connection.sock
            self.assertIsNotNone(first_socket)
            connection.request("CHICKEN", redirect.headers["Location"])
            followed = connection.getresponse()
            self.assertEqual((followed.status, followed.read()), (200, b"MAGIC HAPPENED"))
            self.assertIs(connection.sock, first_socket)
        finally:
            connection.close()

    def test_case_selection_resolves_only_supported_xhr_resource_paths(self) -> None:
        sources = {
            "xhr/absolute.html": "fetch('/xhr/resources/requri.py?full');",
            "xhr/relative.html": "fetch('resources/requri.py'); fetch('./resources/redirect.py');",
            "xhr/nested/parent.html": "fetch('../resources/redirect.py');",
            "xhr/unsupported.html": "fetch('resources/requri.py'); fetch('resources/other.py');",
            "xhr/suffix.html": "fetch('resources/redirect.py-extra');",
            "xhr/script-suffix.html": "fetch('resources/redirect.py.js');",
            "xhr/prefix.html": "fetch('/wrong/xhr/resources/redirect.py');",
            "xhr/wrong-parent.html": "fetch('../resources/redirect.py');",
            "xhr/wrong-root.html": "fetch('redirect.py');",
            "xhr/nested/wrong-relative.html": "fetch('resources/redirect.py');",
        }
        for path, script in sources.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(
                '<script src="/resources/testharness.js"></script>'
                '<script src="/resources/testharnessreport.js"></script>'
                f"<script>{script}</script>"
            )
        selected = enumerate_cases(self.root, dir_prefixes=("xhr",))
        self.assertEqual([case.case_path for case in selected], [
            "xhr/absolute.html", "xhr/nested/parent.html", "xhr/relative.html",
        ])
