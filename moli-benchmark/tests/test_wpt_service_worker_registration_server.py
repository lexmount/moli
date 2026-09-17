from __future__ import annotations


import socket
import tempfile

import unittest


from contextlib import ExitStack

from http.client import HTTPConnection, IncompleteRead
from pathlib import Path
from threading import Event

from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


DIRECTORY = "service-workers/service-worker"
RESOURCES = "/" + DIRECTORY + "/resources/"


class ServiceWorkerRegistrationFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        for name in ("testharness.js", "testharnessreport.js"):
            (self.root / "resources" / name).write_text("// harness")
        resources = self.root / DIRECTORY / "resources"
        resources.mkdir(parents=True)
        for name in (
            "mime-type-worker.py", "import-mime-type-worker.py", "malformed-worker.py",
            "invalid-chunked-encoding.py", "invalid-chunked-encoding-with-flush.py",
        ):
            (resources / name).write_text("# Python source must not be sent as a script")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def server(self) -> WptFixtureServer:
        return self.stack.enter_context(WptFixtureServer(self.root))


    def request(
        self, port: int, resource: str, query: str = "", *, method: str = "GET",
    ) -> tuple[int, list[tuple[str, str]], bytes]:
        connection = HTTPConnection("127.0.0.1", port, timeout=5)
        try:
            path = resource if resource.startswith("/") else RESOURCES + resource
            connection.request(method, path + "?" + query)
            response = connection.getresponse()
            return (
                response.status,
                [(name.lower(), value) for name, value in response.getheaders()],
                response.read(),
            )
        finally:
            connection.close()


    def test_registration_mime_handlers_preserve_missing_empty_and_raw_values(self) -> None:
        server = self.server()
        for query, mime in (
            ("", None), ("mime=", ""), ("mime=text%2Fjavascript", "text/javascript"),
            ("mime=text%2Fplain&mime=ignored", "text/plain"),
            ("mime=application%2Fjavascript%3B+charset%3Dutf-8", "application/javascript; charset=utf-8"),
            ("mime=%FF%27%26", "\xff'&"),
        ):
            for method in ("GET", "HEAD", "POST", "CUSTOM"):
                with self.subTest(query=query, method=method):
                    status, headers, body = self.request(server.port, "mime-type-worker.py", query, method=method)
                    self.assertEqual((status, body), (200, b""))
                    self.assertEqual(dict(headers).get("content-type"), mime)
                    self.assertNotIn("cache-control", dict(headers))
                    status, headers, body = self.request(server.alternate_port, "import-mime-type-worker.py", query, method=method)
                    suffix = b"?mime=" + mime.encode("latin-1") if mime is not None else b""
                    expected = b"importScripts('./mime-type-worker.py" + suffix + b"');"
                    self.assertEqual((status, body), (200, b"" if method == "HEAD" else expected))
                    self.assertEqual(dict(headers)["content-type"], "application/javascript")
                    self.assertEqual(dict(headers)["content-length"], str(len(expected)))
                    self.assertNotIn("cache-control", dict(headers))


    def test_malformed_worker_selects_on_the_complete_undecoded_query(self) -> None:
        server = self.server()
        for query, expected in (
            ("parse-error", b"var foo = function() {;"),
            ("caught-exception", b"try { throw new Error; } catch(e) {}"),
            ("import-malformed-script", b'importScripts("malformed-worker.py?parse-error");'),
            ("instantiation-error-and-top-level-await", b'import nonexistent from "./imported-module-script.js"; await Promise.resolve(1);'),
        ):
            for method in ("GET", "HEAD", "OPTIONS"):
                with self.subTest(query=query, method=method):
                    status, headers, body = self.request(server.port, "%6dalformed-worker.py", query, method=method)
                    self.assertEqual((status, body), (200, b"" if method == "HEAD" else expected))
                    self.assertEqual(dict(headers)["content-type"], "application/javascript")
                    self.assertEqual(dict(headers)["content-length"], str(len(expected)))
                    self.assertNotIn("cache-control", dict(headers))
        for query in ("", "unknown", "parse%2Derror", "parse-error=", "parse-error&ignored"):
            with self.subTest(query=query):
                self.assertEqual(self.request(server.port, "malformed-worker.py", query)[0], 500)


    def test_invalid_chunked_responses_keep_raw_bytes_and_distinct_head_behavior(self) -> None:
        server = self.server()
        with patch.object(server._stopping, "wait", return_value=False):
            for delayed in (False, True):
                resource = "invalid-chunked-encoding" + ("-with-flush" if delayed else "") + ".py"
                for method in ("GET", "HEAD", "CUSTOM"):
                    with self.subTest(delayed=delayed, method=method), socket.create_connection(
                        ("127.0.0.1", server.port), timeout=3,
                    ) as connection:
                        connection.sendall((
                            f"{method} {RESOURCES}{resource} HTTP/1.1\r\n"
                            f"Host: localhost:{server.port}\r\n\r\n"
                        ).encode())
                        with connection.makefile("rb") as response:
                            head, body = response.read().split(b"\r\n\r\n", 1)
                        fields = head.lower().split(b"\r\n")[1:]
                        self.assertTrue(head.startswith(b"HTTP/1.1 200 "))
                        self.assertIn(b"content-type: application/javascript", fields)
                        self.assertIn(b"transfer-encoding: chunked", fields)
                        self.assertEqual(b"content-length: 6" in fields, not delayed)
                        self.assertEqual(body, b"" if method == "HEAD" and not delayed else b"XX\r\n\r\n")
                connection = HTTPConnection("127.0.0.1", server.port, timeout=3)
                try:
                    connection.request("GET", RESOURCES + resource)
                    response = connection.getresponse()
                    self.assertTrue(response.chunked)
                    with self.assertRaises(IncompleteRead):
                        response.read()
                finally:
                    connection.close()


    def test_invalid_chunk_flushes_headers_before_waiting_and_does_not_read_upload(self) -> None:
        server = self.server()
        waiting, release = Event(), Event()
        def wait(timeout: float) -> bool:
            self.assertEqual(timeout, 1)
            waiting.set()
            release.wait(3)
            return False
        with patch.object(server._stopping, "wait", side_effect=wait), socket.create_connection(
            ("127.0.0.1", server.port), timeout=3,
        ) as connection:
            try:
                connection.sendall((
                    f"POST {RESOURCES}invalid-chunked-encoding-with-flush.py HTTP/1.1\r\n"
                    f"Host: localhost:{server.port}\r\nContent-Length: 1000000\r\n\r\n"
                ).encode())
                self.assertTrue(waiting.wait(2))
                with connection.makefile("rb") as response:
                    headers = []
                    while (line := response.readline()) != b"\r\n":
                        self.assertTrue(line)
                        headers.append(line)
                    self.assertIn(b"Transfer-Encoding: chunked\r\n", headers)
                    release.set()
                    self.assertEqual(response.read(), b"XX\r\n\r\n")
            finally:
                release.set()


    def test_discovery_only_accepts_supported_handler_locations(self) -> None:
        cases = {
            "mime.html": ("resources/mime-type-worker.py", True),
            "import-mime.html": ("resources/import-mime-type-worker.py", True),
            "malformed.html": ("resources/malformed-worker.py", True),
            "chunked.html": ("resources/invalid-chunked-encoding.py", True),
            "chunked-flush.html": ("resources/invalid-chunked-encoding-with-flush.py", True),
            "sub/malformed.html": ("../resources/malformed-worker.py", True),
            "sub/malformed-wrong.html": ("resources/malformed-worker.py", False),
            "malformed-suffix.html": ("resources/malformed-worker.py.extra", False),
            "unknown.html": ("resources/unknown.py", False),
        }
        for name, (reference, _) in cases.items():
            path = self.root / DIRECTORY / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                '<!doctype html><script src="/resources/testharness.js"></script>'
                '<script src="/resources/testharnessreport.js"></script>'
                f'<script src="{reference}?Key=ignored"></script>'
            )
        discovered = enumerate_cases(self.root, dir_prefixes=(DIRECTORY,))
        self.assertEqual(
            sorted(case.case_path for case in discovered),
            sorted(DIRECTORY + "/" + name for name, (_, allowed) in cases.items() if allowed),
        )
