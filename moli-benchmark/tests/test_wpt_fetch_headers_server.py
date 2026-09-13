from __future__ import annotations

import socket
import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE = "/fetch/api/resources/inspect-headers.py"


class FetchHeaderFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, method: str, query: str, headers: dict[str, str]):
        connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
        try:
            connection.request(method, RESOURCE + "?" + query, headers=headers)
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_reflects_headers_for_standard_and_extension_methods(self) -> None:
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "patcH", "DELETE",
                       "OPTIONS", "YO", "chicken", "*"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    method, "headers=x-value|empty|missing", {
                        "X-Value": "x\x01\t\x1f\x80\xffx", "Empty": "",
                    },
                )
                self.assertEqual((status, body), (200, b""))
                self.assertEqual(headers["Content-Type"], "text/plain")
                self.assertEqual(headers["x-request-x-value"], "x\x01\t\x1f\x80\xffx")
                self.assertEqual(headers["x-request-empty"], "")
                self.assertIsNone(headers["x-request-missing"])
                self.assertFalse(any(name.lower().startswith("access-control-") for name in headers))

    def test_cors_and_status_pipe_work_for_each_method(self) -> None:
        for method in ("GET", "HEAD", "POST", "PUT", "OPTIONS", "YO", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    method, "headers=x-test&cors&allow_headers=X-Test&pipe=status(202)", {
                        "Origin": "https://caller.test", "X-Test": "value",
                    },
                )
                self.assertEqual((status, body), (202, b""))
                self.assertEqual(headers["x-request-x-test"], "value")
                self.assertEqual(headers["Access-Control-Allow-Origin"], "https://caller.test")
                self.assertEqual(headers["Access-Control-Allow-Credentials"], "true")
                self.assertEqual(headers["Access-Control-Allow-Methods"], "GET, POST, HEAD")
                self.assertEqual(headers["Access-Control-Allow-Headers"], "X-Test")
                self.assertEqual(headers["Access-Control-Expose-Headers"], "x-request-x-test")

    def test_inspection_does_not_wait_for_unread_uploads(self) -> None:
        for framing in (b"Content-Length: 10", b"Transfer-Encoding: chunked"):
            with self.subTest(framing=framing), socket.create_connection(
                ("127.0.0.1", self.server.port), timeout=2
            ) as connection:
                connection.sendall(
                    b"POST " + RESOURCE.encode() + b"?headers=x-test HTTP/1.1\r\n"
                    b"Host: localhost\r\nX-Test: value\r\n" + framing + b"\r\n\r\n"
                )
                with connection.makefile("rb") as stream:
                    response = stream.read()
                head, body = response.split(b"\r\n\r\n", 1)
                self.assertTrue(head.startswith(b"HTTP/1.0 200 OK\r\n"))
                self.assertIn(b"x-request-x-test: value\r\n", head + b"\r\n")
                self.assertIn(b"Connection: close\r\n", head + b"\r\n")
                self.assertEqual(body, b"")

    def test_can_send_and_receive_all_253_header_values(self) -> None:
        request_headers = {
            f"val{byte}": "x" + chr(byte) + "x"
            for byte in range(256) if byte not in (0, 10, 13)
        }
        for method in ("GET", "POST"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    method, "headers=" + "|".join(request_headers), request_headers
                )
                self.assertEqual((status, body), (200, b""))
                for name, value in request_headers.items():
                    self.assertEqual(headers["x-request-" + name], value, name)

    def test_header_count_and_line_length_remain_bounded(self) -> None:
        for request_headers in (
            {f"val{index}": "value" for index in range(512)},
            {"X-Long": "x" * (64 * 1024)},
        ):
            with self.subTest(header_count=len(request_headers)):
                status, _, _ = self.request("GET", "", request_headers)
                self.assertEqual(status, 431)


if __name__ == "__main__":
    unittest.main()
