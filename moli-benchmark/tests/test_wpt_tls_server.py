import base64
import hashlib
import socket
import ssl
import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection, HTTPSConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.engine import build_driver
from moli_benchmark.wpt_cross.server import WptFixtureServer, _substitute_wpt_template_variables
from moli_benchmark.wpt_cross.pipes import WptPipeError


class WptTlsFixtureTests(unittest.TestCase):
    def setUp(self):
        stack = ExitStack()
        self.addCleanup(stack.close)
        root = Path(stack.enter_context(tempfile.TemporaryDirectory()))
        (root / "resources").mkdir()
        (root / "resources/testharness.js").write_text("// test fixture")
        (root / "probe.sub.html").write_text(
            "{{location[scheme]}}|{{location[port]}}|{{location[server]}}|"
            "{{ports[http][0]}}|{{ports[http][1]}}|"
            "{{ports[https][0]}}|{{ports[https][1]}}|"
            "{{ports[ws][0]}}|{{ports[wss][0]}}"
        )
        (root / "probe.sub.html.sub.headers").write_text(
            "X-Fixture-Origin: {{location[server]}}\n"
        )
        stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = stack.enter_context(WptFixtureServer(root))
        self.context = ssl.create_default_context(cafile=str(self.server.tls.ca_certificate))

    def test_http_and_https_ports_and_request_templates_are_independent(self):
        server = self.server
        ports = [server.port, server.alternate_port, server.https_port, server.alternate_https_port]
        self.assertEqual(len(set(ports)), 4)
        for scheme, port in [("http", server.port), ("http", server.alternate_port),
                             ("https", server.https_port), ("https", server.alternate_https_port)]:
            with self.subTest(scheme=scheme, port=port):
                if scheme == "https":
                    connection = HTTPSConnection("127.0.0.1", port, context=self.context, timeout=3)
                else:
                    connection = HTTPConnection("127.0.0.1", port, timeout=3)
                self.addCleanup(connection.close)
                connection.request("GET", "/probe.sub.html")
                response = connection.getresponse()
                self.assertEqual(response.status, 200)
                origin = f"{scheme}://127.0.0.1:{port}"
                self.assertEqual(response.headers["X-Fixture-Origin"], origin)
                self.assertEqual(response.read().decode(), "|".join(map(str, [
                    scheme, port, origin, server.port, server.alternate_port,
                    server.https_port, server.alternate_https_port, server.port, server.https_port,
                ])))

    def test_https_rejects_untrusted_authority_and_wrong_hostname(self):
        with self.assertRaises(ssl.SSLCertVerificationError):
            with socket.create_connection(("127.0.0.1", self.server.https_port), timeout=3) as raw:
                ssl.create_default_context().wrap_socket(raw, server_hostname="localhost")
        with self.assertRaises(ssl.SSLCertVerificationError):
            with socket.create_connection(("127.0.0.1", self.server.https_port), timeout=3) as raw:
                self.context.wrap_socket(raw, server_hostname="unrelated.invalid")

    def test_wss_echo_uses_verified_tls_for_a_named_fixture_origin(self):
        name = "www2.web-platform.localhost"
        with socket.create_connection(("127.0.0.1", self.server.https_port), timeout=3) as raw:
            with self.context.wrap_socket(raw, server_hostname=name) as connection:
                self.assertIn(("DNS", name), connection.getpeercert()["subjectAltName"])
                key = base64.b64encode(b"a local WPT test").decode()
                connection.sendall((
                    f"GET /echo HTTP/1.1\r\nHost: {name}:{self.server.https_port}\r\n"
                    "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                    f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
                ).encode())
                response = b""
                while not response.endswith(b"\r\n\r\n"):
                    chunk = connection.recv(1)
                    self.assertTrue(chunk, "TLS peer closed during the WebSocket handshake")
                    response += chunk
                self.assertTrue(response.startswith(b"HTTP/1.1 101 "), response)
                expected = base64.b64encode(hashlib.sha1(
                    (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()
                ).digest())
                self.assertIn(b"Sec-WebSocket-Accept: " + expected, response)
                payload = "WPT TLS echo".encode()
                mask = b"WPT!"
                connection.sendall(bytes([0x81, 0x80 | len(payload)]) + mask + bytes(
                    byte ^ mask[index % 4] for index, byte in enumerate(payload)
                ))
                header = b""
                while len(header) < 2:
                    chunk = connection.recv(2 - len(header))
                    self.assertTrue(chunk, "TLS peer closed before the WebSocket echo")
                    header += chunk
                self.assertEqual(header, bytes([0x81, len(payload)]))
                echoed = b""
                while len(echoed) < len(payload):
                    chunk = connection.recv(len(payload) - len(echoed))
                    self.assertTrue(chunk, "TLS peer closed during the WebSocket echo")
                    echoed += chunk
                self.assertEqual(echoed, payload)

    def test_case_transport_follows_the_filename_flag_not_query_text(self):
        server = self.server
        self.assertEqual(server.url_for_case("a.https.html?x=1"), server.https_base_url + "/a.https.html?x=1")
        self.assertEqual(server.url_for_case("a.html?url=b.https.html"), server.base_url + "/a.html?url=b.https.html")

    def test_https_variables_cannot_silently_reuse_an_http_listener(self):
        with self.assertRaisesRegex(WptPipeError, "TLS listener"):
            _substitute_wpt_template_variables(b"https://localhost:{{ports[https][0]}}/", port=8000)
        with self.assertRaisesRegex(WptPipeError, "TLS listener"):
            _substitute_wpt_template_variables(b"wss://localhost:{{ports[wss][0]}}/", port=8000)

    def test_browser_trust_is_limited_to_the_fixture_authority_or_key(self):
        server = self.server
        original = build_driver("moli")
        moli = original.with_fixture_tls(server.tls)
        command = moli.build_command(Path("/bin/moli"), 9222, None)
        self.assertEqual(command[-2:], ["--ca-cert", str(server.tls.ca_certificate)])
        self.assertNotIn("--insecure", command)
        self.assertNotIn("--ca-cert", original.build_command(Path("/bin/moli"), 9222, None))
        chrome = build_driver("chrome").with_fixture_tls(server.tls)
        command = chrome.build_command(Path("/bin/chrome"), 9222, Path("/tmp/profile"))
        self.assertIn("--ignore-certificate-errors-spki-list=" + server.tls.server_spki_sha256, command)
        self.assertNotIn("--ignore-certificate-errors", command)
