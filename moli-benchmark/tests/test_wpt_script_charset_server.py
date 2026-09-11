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


SCRIPT_DIRECTORY = "html/semantics/scripting-1/the-script-element"
RESOURCE = "/" + SCRIPT_DIRECTORY + "/serve-with-content-type.py"


class ScriptCharsetFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.directory = self.root / SCRIPT_DIRECTORY
        self.directory.mkdir(parents=True)
        self.body = b"\xef\xbb\xbfvar result = '\xe9';\n"
        (self.directory / "source.js").write_bytes(self.body)
        (self.directory / "source.js.headers").write_text("X-Unrelated: ignored\n")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def request(
        self, server: WptFixtureServer, query: str,
        *, method: str = "GET", resource: str = RESOURCE,
    ) -> tuple[int, dict[str, str], bytes]:
        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
        try:
            connection.request(method, resource + "?" + query)
            response = connection.getresponse()
            return (
                response.status,
                {name.lower(): value for name, value in response.getheaders()},
                response.read(),
            )
        finally:
            connection.close()

    def test_preserves_source_bytes_and_requested_content_type(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for content_type in ("text/javascript", "text/javascript;charset=windows-1250", ""):
            with self.subTest(content_type=content_type):
                status, headers, body = self.request(
                    server, urlencode({"fn": "source.js", "ct": content_type})
                )
                self.assertEqual((status, body), (200, self.body))
                self.assertEqual(headers["content-type"], content_type)
                self.assertEqual(headers["content-length"], str(len(self.body)))
                self.assertNotIn("x-unrelated", headers)

    def test_head_and_duplicate_query_parameters_use_first_values(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        query = urlencode([
            ("fn", "source.js"), ("fn", "missing.js"),
            ("ct", "application/json"), ("ct", "text/plain"),
        ])
        for method in ("GET", "HEAD"):
            with self.subTest(method=method):
                status, headers, body = self.request(server, query, method=method)
                self.assertEqual((status, body), (200, b"" if method == "HEAD" else self.body))
                self.assertEqual(headers["content-type"], "application/json")
                self.assertEqual(headers["content-length"], str(len(self.body)))

    def test_missing_parameters_and_files_return_bad_request(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for query in ("", "fn=source.js", "ct=text/plain", "fn=&ct=text/plain",
                      "fn=missing.js&ct=text/plain", "fn=.&ct=text/plain"):
            with self.subTest(query=query):
                status, _, _ = self.request(server, query)
                self.assertEqual(status, 400)

    def test_file_resolution_stays_inside_fixture_root(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        (self.directory.parent / "neighbor.js").write_bytes(b"neighbor")
        status, _, body = self.request(
            server, urlencode({"fn": "../neighbor.js", "ct": "text/javascript"})
        )
        self.assertEqual((status, body), (200, b"neighbor"))
        outside = Path(self.stack.enter_context(tempfile.TemporaryDirectory())) / "secret.js"
        outside.write_bytes(b"outside fixture")
        (self.directory / "escape.js").symlink_to(outside)
        for filename in (str(outside), "escape.js"):
            with self.subTest(filename=filename):
                status, _, body = self.request(
                    server, urlencode({"fn": filename, "ct": "text/javascript"})
                )
                self.assertEqual(status, 400)
                self.assertNotIn(b"outside fixture", body)

    def test_handler_requires_the_exact_resource_path(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for resource in (RESOURCE + "2", RESOURCE + ".js", "/wrong" + RESOURCE):
            with self.subTest(resource=resource):
                status, _, _ = self.request(
                    server, "fn=source.js&ct=text/javascript", resource=resource
                )
                self.assertEqual(status, 404)

    def test_case_selection_resolves_only_exact_supported_references(self) -> None:
        sources = {
            SCRIPT_DIRECTORY + "/module/relative.window.js": "fetch('../serve-with-content-type.py?fn=x&ct=text/javascript');",
            SCRIPT_DIRECTORY + "/json-module/relative.window.js": "fetch('../serve-with-content-type.py?fn=x&ct=application/json');",
            SCRIPT_DIRECTORY + "/css-module/nested/parent.window.js": "fetch('../../serve-with-content-type.py?fn=x&ct=text/css');",
            "other/absolute.window.js": f"fetch('{RESOURCE}?fn=x&ct=text/plain');",
            "other/suffix.window.js": f"fetch('{RESOURCE}2');",
            "other/script-suffix.window.js": f"fetch('{RESOURCE}.js');",
            "other/prefix.window.js": f"fetch('/wrong{RESOURCE}');",
            "other/unknown.window.js": f"fetch('{RESOURCE}'); fetch('unknown.py');",
            SCRIPT_DIRECTORY + "/module/wrong-relative.window.js": "fetch('serve-with-content-type.py');",
        }
        for path, source in sources.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(source)
        selected = enumerate_cases(self.root, dir_prefixes=(SCRIPT_DIRECTORY, "other"))
        self.assertEqual([case.case_path for case in selected], [
            SCRIPT_DIRECTORY + "/css-module/nested/parent.window.js?moli-wpt-script=window",
            SCRIPT_DIRECTORY + "/json-module/relative.window.js?moli-wpt-script=window",
            SCRIPT_DIRECTORY + "/module/relative.window.js?moli-wpt-script=window",
            "other/absolute.window.js?moli-wpt-script=window",
        ])
