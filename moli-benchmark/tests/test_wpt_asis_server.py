from __future__ import annotations

import socket
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.server import WptFixtureServer


class WptAsIsFixtureTests(unittest.TestCase):
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

    def raw_response(self, target: str, method: str = "GET") -> bytes:
        with socket.create_connection(("127.0.0.1", self.server.port), timeout=2) as connection:
            connection.sendall(
                f"{method} {target} HTTP/1.1\r\nHost: localhost\r\n\r\n".encode("ascii")
            )
            parts = []
            while chunk := connection.recv(4096):
                parts.append(chunk)
            return b"".join(parts)

    def test_preserves_header_only_responses_and_framing_bytes(self) -> None:
        responses = [
            b"HTTP/1.1 200 YAYAYAYA\nfoo-TEST: 1\nFOO-test: 2\n__Custom: token\n",
            b"HTTP/1.0 200 NANANA\nCONTENT-LENGTH:  0\ncontent-length:\t 0\n",
            b"HTTP/1.1 202 Giraffe\r\nContent-Length: 999\r\n\r\nContent",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n0\r\n\r\n",
            b"HTTP/1.1 699 \xff\nX-Bytes: \xa0\xc3\xbf\x85 \t\nHEYA: \x0b\x0c\n\n\x00\xff\x80",
            b"not an HTTP response\x00\xff",
        ]
        for index, response in enumerate(responses):
            with self.subTest(index=index):
                (self.root / "response.asis").write_bytes(response)
                self.assertEqual(self.raw_response("/response.asis"), response)

    def test_bypasses_headers_substitution_and_response_pipes(self) -> None:
        response = b"HTTP/1.1 202 Giraffe\nX-Test: PASS\n\n{{host}}\xff"
        (self.root / "response.sub.asis").write_bytes(response)
        (self.root / "response.sub.asis.headers").write_text("X-Sidecar: ignored\n")
        for query in (
            "", "?pipe=header(X-Test,FAIL)", "?pipe=status(201)",
            "?pipe=slice(null,2)", "?pipe=sub", "?pipe=trickle(1:d2:r2)",
        ):
            with self.subTest(query=query):
                self.assertEqual(self.raw_response("/response.sub.asis" + query), response)

    def test_raw_writer_also_bypasses_head_body_suppression(self) -> None:
        response = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbody"
        (self.root / "response.asis").write_bytes(response)
        self.assertEqual(self.raw_response("/response.asis", "HEAD"), response)
