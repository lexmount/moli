from __future__ import annotations

import re
import socket
import tempfile
import time
import unittest
import uuid
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from datetime import datetime
from http.client import HTTPConnection, IncompleteRead
from pathlib import Path
from threading import Event
from urllib.parse import urlencode
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


DIRECTORY = "service-workers/service-worker"
RESOURCES = "/" + DIRECTORY + "/resources/"


class ServiceWorkerScriptFixtureTests(unittest.TestCase):
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
            "redirect.py", "update-worker.py", "update-worker-from-file.py", "import-scripts-version.py",
            "update-during-installation-worker.py",
            "import-scripts-get.py", "import-scripts-echo.py",
            "subdir/import-scripts-echo.py", "scope2/import-scripts-echo.py",
        ):
            (resources / name).parent.mkdir(exist_ok=True)
            (resources / name).write_text("# Python source must not be sent as a script")
        global_resources = self.root / DIRECTORY / "ServiceWorkerGlobalScope/resources"
        global_resources.mkdir(parents=True)
        (global_resources / "update-worker.py").write_text("# Python source must not be sent as a script")
        (global_resources / "update-worker.js").write_bytes(b"// caf\xc3\xa9\r\nself.ready = true;\r\n")
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

    def test_redirect_preserves_status_location_and_repeated_cors_headers(self) -> None:
        server = self.server()
        query = urlencode([
            ("Redirect", "../target.js?one=1&two=%20"),
            ("Redirect", "ignored.js"),
            ("Status", "307"),
            ("ACAOrigin", "https://one.test,https://two.test"),
            ("ACAHeaders", "X-One, X-Two"),
            ("ACAMethods", "GET, OPTIONS"),
            ("ACACredentials", "true"),
            ("ACEHeaders", "X-Result"),
        ])
        for method in ("GET", "HEAD", "POST", "OPTIONS", "PUT", "YO", "CUSTOM"):
            with self.subTest(method=method):
                status, headers, body = self.request(server.port, "redirect.py", query, method=method)
                self.assertEqual(status, 307)
                self.assertEqual(body, b"")
                self.assertEqual(dict(headers)["location"], "../target.js?one=1&two=%20")
                self.assertEqual(
                    [value for name, value in headers if name == "access-control-allow-origin"],
                    ["https://one.test", "https://two.test"],
                )
                for name, value in (
                    ("access-control-allow-headers", "X-One, X-Two"),
                    ("access-control-allow-methods", "GET, OPTIONS"),
                    ("access-control-allow-credentials", "true"),
                    ("access-control-expose-headers", "X-Result"),
                ):
                    self.assertEqual(dict(headers)[name], value)
                self.assertNotIn("content-type", dict(headers))
                self.assertNotIn("cache-control", dict(headers))
        status, headers, _ = self.request(server.port, "%72edirect.py", "Redirect=%FF")
        self.assertEqual(status, 302)
        self.assertEqual(dict(headers)["location"], "\xff")

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

    def test_update_modes_only_change_the_second_response(self) -> None:
        server = self.server()
        modes = (
            ("normal", 200, "application/javascript", b"/* 2 */ "),
            ("bad_mime_type", 200, "text/html", b"/* 2 */ "),
            ("not_found", 404, "text/plain", b"Page not found"),
            ("redirect", 301, "application/javascript", b"/* 2 */"),
            ("syntax_error", 200, "application/javascript", b"/* 2 */ badsyntax(isbad;"),
            ("throw_install", 200, "application/javascript", b"/* 2 */ addEventListener('install', function(e) { throw new Error('boom'); });"),
            ("unknown", 200, "application/javascript", b"/* 2 */ "),
        )
        for mode, expected_status, expected_type, expected_body in modes:
            query = urlencode({"Key": str(uuid.uuid4()), "Mode": mode})
            with self.subTest(mode=mode):
                self.assertEqual(self.request(server.port, "update-worker.py", query)[2], b"/* 1 */ ")
                status, headers, body = self.request(server.port, "update-worker.py", query)
                self.assertEqual((status, dict(headers)["content-type"], body),
                                 (expected_status, expected_type, expected_body))
                if mode == "redirect":
                    self.assertEqual(dict(headers)["location"], "empty.js")
                if mode == "not_found":
                    self.assertNotIn("cache-control", dict(headers))
                else:
                    self.assertEqual(dict(headers)["cache-control"], "no-cache, must-revalidate")
                    self.assertEqual(dict(headers)["pragma"], "no-cache")
                self.assertEqual(self.request(server.port, "update-worker.py", query)[2], b"/* 3 */ ")

    def test_update_state_is_shared_across_origins_and_modes_but_not_servers(self) -> None:
        first, second = self.server(), self.server()
        key = str(uuid.uuid4())
        first.fetch_stash.put(key, 50)
        first_query = urlencode({"Key": key, "Mode": "normal"})
        self.assertEqual(self.request(first.port, "update-worker.py", first_query)[2], b"/* 1 */ ")
        # Equivalent UUID spelling, first repeated parameter, and a second
        # percent decode of Redirect all follow the upstream handler.
        redirect_query = urlencode([
            ("Key", "{" + key.upper() + "}"), ("Mode", "redirect"),
            ("Mode", "not_found"), ("Redirect", "target.js?one=1%26two=2"),
        ])
        status, headers, body = self.request(first.alternate_port, "update-worker.py", redirect_query)
        self.assertEqual((status, body), (301, b"/* 2 */"))
        self.assertEqual(dict(headers)["location"], "target.js?one=1&two=2")
        self.assertEqual(self.request(second.port, "update-worker.py", first_query)[2], b"/* 1 */ ")
        self.assertEqual(self.request(first.port, "update-worker.py", first_query)[2], b"/* 3 */ ")
        self.assertEqual(first.fetch_stash.take(key), 50)

    def test_concurrent_updates_keep_every_visit(self) -> None:
        server = self.server()
        query = urlencode({"Key": str(uuid.uuid4()), "Mode": "normal"})
        def visit(index: int) -> int:
            port = server.port if index % 2 else server.alternate_port
            status, _, body = self.request(port, "update-worker.py", query)
            self.assertEqual(status, 200)
            match = re.fullmatch(rb"/\* (\d+) \*/ ", body)
            self.assertIsNotNone(match)
            return int(match[1])
        with ThreadPoolExecutor(max_workers=6) as pool:
            self.assertEqual(sorted(pool.map(visit, range(18))), list(range(1, 19)))

    def test_head_runs_the_update_handler_without_emitting_its_body(self) -> None:
        server = self.server()
        query = urlencode({"Key": str(uuid.uuid4()), "Mode": "not_found"})
        for expected_status, expected_length in ((200, 8), (404, 14), (200, 8)):
            status, headers, body = self.request(server.port, "update-worker.py", query, method="HEAD")
            self.assertEqual(status, expected_status)
            self.assertEqual(dict(headers)["content-length"], str(expected_length))
            self.assertEqual(body, b"")

    def test_imported_version_changes_with_revalidation_headers(self) -> None:
        server = self.server()
        versions = []
        for port in (server.port, server.alternate_port, server.port):
            before = (datetime.now() - datetime(1970, 1, 1)).total_seconds()
            status, headers, body = self.request(port, "import-scripts-version.py")
            after = (datetime.now() - datetime(1970, 1, 1)).total_seconds()
            self.assertEqual(status, 200)
            self.assertEqual(dict(headers)["content-type"], "application/javascript")
            self.assertEqual(dict(headers)["cache-control"], "no-cache, must-revalidate")
            self.assertEqual(dict(headers)["pragma"], "no-cache")
            match = re.fullmatch(rb'version = "([0-9.]+)";\n', body)
            self.assertIsNotNone(match)
            versions.append(float(match[1]))
            self.assertLessEqual(before, versions[-1])
            self.assertLessEqual(versions[-1], after)
        self.assertLess(versions[0], versions[1])
        self.assertLess(versions[1], versions[2])

    def test_update_from_file_preserves_bytes_and_shares_only_its_own_stash(self) -> None:
        first, second = self.server(), self.server()
        resources = self.root / DIRECTORY / "resources"
        before, after = b"// before\r\n\xff\n", b"// after\x00\n"
        (resources / "café.js").write_bytes(before)
        (resources / "after.js").write_bytes(after)
        key = str(uuid.uuid4())
        query = urlencode([
            ("Key", key), ("First", "café.js"), ("First", "ignored.js"),
            ("Second", "after.js"),
        ])
        for port, expected in (
            (first.port, before),
            (second.port, before),
            (first.alternate_port, after),
        ):
            status, headers, body = self.request(port, "update-worker-from-file.py", query)
            self.assertEqual((status, body), (200, expected))
            self.assertEqual(dict(headers)["content-type"], "application/javascript")
            self.assertEqual(dict(headers)["cache-control"], "no-cache, must-revalidate")
            self.assertEqual(dict(headers)["pragma"], "no-cache")
        self.assertEqual(self.request(first.port, "update-worker-from-file.py", query)[0], 500)
        self.assertEqual(
            self.request(first.port, "update-worker.py", urlencode({"Key": key, "Mode": "normal"}))[2],
            b"/* 1 */ ",
        )
        head_query = urlencode({"Key": str(uuid.uuid4()), "First": "café.js", "Second": "after.js"})
        status, headers, body = self.request(first.port, "update-worker-from-file.py", head_query, method="HEAD")
        self.assertEqual((status, dict(headers)["content-length"], body), (200, str(len(before)), b""))
        self.assertEqual(self.request(first.port, "update-worker-from-file.py", head_query, method="POST")[2], after)
        failure_query = urlencode({"Key": str(uuid.uuid4()), "First": "missing.js", "Second": "after.js"})
        self.assertEqual(self.request(first.port, "update-worker-from-file.py", failure_query)[0], 500)
        self.assertEqual(self.request(first.port, "update-worker-from-file.py", failure_query)[2], after)

    def test_global_scope_update_script_uses_timestamp_and_original_text(self) -> None:
        server = self.server()
        resource = "/" + DIRECTORY + "/ServiceWorkerGlobalScope/resources/update-worker.py"
        script = "// café\nself.ready = true;\n".encode("utf-8")
        versions = []
        for method in ("GET", "POST", "OPTIONS", "HEAD"):
            before = time.time()
            status, headers, body = self.request(server.port, resource, method=method)
            after = time.time()
            self.assertEqual(status, 200)
            self.assertEqual(dict(headers)["cache-control"], "max-age: 0")
            self.assertEqual(dict(headers)["content-type"], "application/javascript")
            self.assertNotIn("pragma", dict(headers))
            if method == "HEAD":
                self.assertEqual(body, b"")
                self.assertGreater(int(dict(headers)["content-length"]), len(script))
                continue
            stamp, source = body.split(b"\n", 1)
            self.assertEqual(source, script)
            self.assertTrue(stamp.startswith(b"// "))
            versions.append(float(stamp[3:]))
            self.assertLessEqual(before, versions[-1])
            self.assertLessEqual(versions[-1], after)
        self.assertEqual(versions, sorted(set(versions)))

    def test_installation_update_script_changes_without_rewriting_its_import(self) -> None:
        server = self.server()
        versions = []
        for method in ("GET", "POST", "OPTIONS", "HEAD"):
            status, headers, body = self.request(server.port, "update-during-installation-worker.py", method=method)
            self.assertEqual(status, 200)
            self.assertEqual(dict(headers)["content-type"], "application/javascript")
            self.assertEqual(dict(headers)["cache-control"], "max-age=0")
            self.assertNotIn("pragma", dict(headers))
            if method == "HEAD":
                self.assertEqual(body, b"")
                continue
            version, script = body.split(b"\n", 1)
            self.assertEqual(script, b"importScripts('update-during-installation-worker.js');")
            self.assertTrue(version.startswith(b"// "))
            versions.append(float(version[3:]))
            self.assertGreaterEqual(versions[-1], 0)
            self.assertLess(versions[-1], 1)
        self.assertEqual(len(set(versions)), 3)

    def test_imported_assignments_preserve_raw_parameters_and_directory(self) -> None:
        server = self.server()
        for resource, query, expected in (
            ("import-scripts-get.py", "output=echo1&msg=test1", b'echo1 = "test1";\n'),
            ("import-scripts-get.py", "output=x&output=y&msg=%FF%22&msg=ignored", b'x = "\xff\"";\n'),
            ("import-scripts-echo.py", "msg=top+level", b'echo_output = "top level";\n'),
            ("import-scripts-echo.py", "msg=", b'echo_output = "";\n'),
            ("subdir/import-scripts-echo.py", "msg=install", b'echo_output = "install (subdir/)";\n'),
            ("scope2/import-scripts-echo.py", "msg=message", b'echo_output = "message (scope2/)";\n'),
        ):
            for method in ("GET", "HEAD", "POST", "OPTIONS"):
                with self.subTest(resource=resource, query=query, method=method):
                    status, headers, body = self.request(server.port, resource, query, method=method)
                    self.assertEqual(status, 200)
                    self.assertEqual(body, b"" if method == "HEAD" else expected)
                    self.assertEqual(dict(headers)["content-length"], str(len(expected)))
                    self.assertEqual(dict(headers)["content-type"], "application/javascript")
                    self.assertEqual(dict(headers)["cache-control"], "no-cache, must-revalidate")
                    self.assertEqual(dict(headers)["pragma"], "no-cache")

    def test_malformed_parameters_are_not_served_as_javascript(self) -> None:
        server = self.server()
        for resource, query in (
            ("redirect.py", ""),
            ("redirect.py", "Redirect=x&Status=invalid"),
            ("redirect.py", "Redirect=x&Status=999"),
            ("redirect.py", "Redirect=%0D%0AX-Injected:yes"),
            ("redirect.py", "Redirect=x&ACAOrigin=%0Ainjected"),
            ("update-worker.py", "Mode=normal"),
            ("update-worker.py", "Key=invalid&Mode=normal"),
            ("update-worker.py", "Key=" + str(uuid.uuid4())),
            ("update-worker-from-file.py", "First=x&Second=y"),
            ("update-worker-from-file.py", urlencode({"Key": str(uuid.uuid4()), "First": "../../../../outside.js"})),
            ("import-scripts-get.py", "output=x"),
            ("import-scripts-get.py", "msg=x"),
            ("import-scripts-echo.py", ""),
        ):
            with self.subTest(resource=resource, query=query):
                self.assertEqual(self.request(server.port, resource, query)[0], 400)

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
            "redirect-relative.html": ("resources/redirect.py", True),
            "update-absolute.html": (RESOURCES + "update-worker.py", True),
            "update-from-file.html": ("resources/update-worker-from-file.py", True),
            "update-installing.html": ("resources/update-during-installation-worker.py", True),
            "ServiceWorkerGlobalScope/update.html": ("resources/update-worker.py", True),
            "ServiceWorkerGlobalScope/update-wrong.html": ("resources/update-worker-from-file.py", False),
            "sub/version-relative.html": ("../resources/import-scripts-version.py", True),
            "sub/redirect-dot-relative.html": ("./../resources/redirect.py", True),
            "sub/redirect-wrong-relative.html": ("resources/redirect.py", False),
            "redirect-suffix.html": ("resources/redirect.py.extra", False),
            "redirect-other.html": ("/unrelated/resources/redirect.py", False),
            "get.html": ("resources/import-scripts-get.py", True),
            "echo.html": ("resources/import-scripts-echo.py", True),
            "echo-subdir.html": ("resources/subdir/import-scripts-echo.py", True),
            "echo-scope2.html": (RESOURCES + "scope2/import-scripts-echo.py", True),
            "echo-wrong.html": ("resources/wrong/import-scripts-echo.py", False),
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


if __name__ == "__main__":
    unittest.main()
