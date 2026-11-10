from __future__ import annotations

import socket
import ssl

from wpt_cross_test_support import *


class WptCrossTlsTests(WptCrossTestCase):
    def test_eof_delimited_https_body_ends_with_tls_close_notify(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text("// harness", encoding="utf-8")
            with WptFixtureServer(root_path) as server:
                context = ssl.create_default_context(cafile=server.tls.ca_certificate)
                with socket.create_connection(("127.0.0.1", server.https_port), timeout=2) as connection:
                    with context.wrap_socket(
                        connection, server_hostname="localhost", suppress_ragged_eofs=False,
                    ) as secure:
                        secure.sendall(
                            b"GET /fetch/api/resources/trickle.py?count=2&ms=0 HTTP/1.1\r\n"
                            b"Host: localhost\r\nConnection: close\r\n\r\n"
                        )
                        chunks = []
                        while chunk := secure.recv(64 * 1024):
                            chunks.append(chunk)
                headers, body = b"".join(chunks).split(b"\r\n\r\n", 1)
                self.assertTrue(headers.startswith(b"HTTP/1.0 200"))
                self.assertNotIn(b"Content-Length:", headers)
                self.assertEqual(body, b"TEST_TRICKLE\n" * 2)
