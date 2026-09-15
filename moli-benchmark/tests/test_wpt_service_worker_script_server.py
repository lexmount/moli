from __future__ import annotations

import re
import tempfile
import unittest
import uuid
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from datetime import datetime
from http.client import HTTPConnection
from pathlib import Path
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
        for name in ("redirect.py", "update-worker.py", "import-scripts-version.py"):
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
            connection.request(method, RESOURCES + resource + "?" + query)
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
        ):
            with self.subTest(resource=resource, query=query):
                self.assertEqual(self.request(server.port, resource, query)[0], 400)

    def test_discovery_only_accepts_supported_handler_locations(self) -> None:
        cases = {
            "redirect-relative.html": ("resources/redirect.py", True),
            "update-absolute.html": (RESOURCES + "update-worker.py", True),
            "sub/version-relative.html": ("../resources/import-scripts-version.py", True),
            "sub/redirect-dot-relative.html": ("./../resources/redirect.py", True),
            "sub/redirect-wrong-relative.html": ("resources/redirect.py", False),
            "redirect-suffix.html": ("resources/redirect.py.extra", False),
            "redirect-other.html": ("/unrelated/resources/redirect.py", False),
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
