from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


SCRIPT_DIRECTORY = "html/semantics/scripting-1/the-script-element"
RESOURCE = "/" + SCRIPT_DIRECTORY + "/resources/load-error-events.py"


class ScriptLoadErrorFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        for name in ("testharness.js", "testharnessreport.js"):
            (self.root / "resources" / name).write_text("// test harness")
        self.directory = self.root / SCRIPT_DIRECTORY
        (self.directory / "resources").mkdir(parents=True)
        (self.directory / "resources/load-error-events.py").write_text(
            "# Python source must not be served as JavaScript"
        )
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def request(
        self, server: WptFixtureServer, query: str, *, method: str = "GET",
    ) -> tuple[int, dict[str, str], bytes]:
        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
        try:
            connection.request(method, RESOURCE + "?" + query)
            response = connection.getresponse()
            return (
                response.status,
                {name.lower(): value for name, value in response.getheaders()},
                response.read(),
            )
        finally:
            connection.close()

    def test_load_and_error_responses_match_script_fixture(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        cases = (
            ("test=test1_load", 200, b'"use strict"; test1_load.executed = true;'),
            (
                "test=test1_error&test=ignored_load", 404,
                b'"use strict"; test1_error.test.step(function() { '
                b'assert_unreached("404 script should not be executed"); });',
            ),
            ("test=test%32_load", 200, b'"use strict"; test2_load.executed = true;'),
        )
        for query, expected_status, expected_body in cases:
            for method in ("GET", "HEAD"):
                with self.subTest(query=query, method=method):
                    status, headers, body = self.request(server, query, method=method)
                    self.assertEqual(status, expected_status)
                    self.assertEqual(headers["content-type"], "text/javascript")
                    self.assertEqual(headers["content-length"], str(len(expected_body)))
                    self.assertEqual(body, b"" if method == "HEAD" else expected_body)

    def test_missing_or_invalid_test_names_return_bad_request(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for query in ("", "test=", "test=a.b_load", "test=a%0A_load", "test=%C3%A9_load"):
            with self.subTest(query=query):
                status, _, _ = self.request(server, query)
                self.assertEqual(status, 400)

    def test_discovery_only_accepts_references_to_the_supported_handler(self) -> None:
        cases = {
            "load-relative.html": ("resources/load-error-events.py", True),
            "load-absolute.html": (RESOURCE, True),
            "module/load-relative.html": ("../resources/load-error-events.py", True),
            "module/load-dot-relative.html": ("./../resources/load-error-events.py", True),
            "module/load-own-resource.html": ("resources/load-error-events.py", False),
            "module/load-suffix.html": ("../resources/load-error-events.py.extra", False),
            "load-other.html": ("/unrelated/resources/load-error-events.py", False),
        }
        for name, (reference, _) in cases.items():
            path = self.directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                '<!doctype html><script src="/resources/testharness.js"></script>'
                '<script src="/resources/testharnessreport.js"></script>'
                f'<script src="{reference}?test=test1_load"></script>'
            )
        discovered = enumerate_cases(self.root, dir_prefixes=(SCRIPT_DIRECTORY,))
        self.assertEqual(
            sorted(case.case_path for case in discovered),
            sorted(SCRIPT_DIRECTORY + "/" + name for name, (_, allowed) in cases.items() if allowed),
        )


if __name__ == "__main__":
    unittest.main()
