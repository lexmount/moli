from __future__ import annotations

import tempfile
import unittest
import uuid
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import (
    JSON_LOAD_ERROR_PATH, JSON_THEN_JS_PATH, WptFixtureServer,
)


SCRIPT_DIRECTORY = "html/semantics/scripting-1/the-script-element"


class JsonModuleFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        for path in (JSON_LOAD_ERROR_PATH, JSON_THEN_JS_PATH):
            resource = self.root / path.lstrip("/")
            resource.parent.mkdir(parents=True, exist_ok=True)
            resource.write_text("# Python source must not be served")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, path: str, query: str, *, method: str = "GET", port: int | None = None):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=2)
        try:
            connection.request(method, path + "?" + query)
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_json_load_error_fixture_fails_at_dependency_fetch(self) -> None:
        cases = (
            ("test1_load", b'import "./module.json" with { type: "json"}; test1_load.executed = true;'),
            ("test1_error", b'import "./not_found.json" with { type: "json"}; test1_error.test.step(function() { assert_unreached("404 script should not be executed"); });'),
        )
        for name, expected in cases:
            for method in ("GET", "HEAD"):
                with self.subTest(name=name, method=method):
                    status, headers, body = self.request(
                        JSON_LOAD_ERROR_PATH, f"test={name}&test=ignored", method=method,
                    )
                    self.assertEqual(status, 200)
                    self.assertEqual(headers["Content-Type"], "text/javascript")
                    self.assertEqual(headers["Content-Length"], str(len(expected)))
                    self.assertEqual(body, b"" if method == "HEAD" else expected)

    def test_json_then_js_counts_requests_per_uuid_and_across_origins(self) -> None:
        key = uuid.uuid4()
        first, headers, body = self.request(JSON_THEN_JS_PATH, f"key={key}&key=ignored")
        self.assertEqual((first, headers["Content-Type"], body),
                         (200, "text/json", b'{"hello": "world"}'))
        self.assertIsNone(headers["Cache-Control"])
        second, headers, body = self.request(
            JSON_THEN_JS_PATH, f"key={str(key).upper()}",
            method="HEAD", port=self.server.alternate_port,
        )
        self.assertEqual((second, headers["Content-Type"], body),
                         (200, "application/javascript", b""))
        self.assertEqual(headers["Content-Length"], str(len(b"export default 'hello';")))
        third, headers, body = self.request(JSON_THEN_JS_PATH, f"key={key.hex}")
        self.assertEqual((third, headers["Content-Type"], body),
                         (200, "application/javascript", b"export default 'hello';"))
        _, headers, body = self.request(JSON_THEN_JS_PATH, f"key={uuid.uuid4()}")
        self.assertEqual((headers["Content-Type"], body), ("text/json", b'{"hello": "world"}'))

    def test_head_consumes_first_response_and_servers_do_not_share_state(self) -> None:
        key = uuid.uuid4()
        _, headers, body = self.request(JSON_THEN_JS_PATH, f"key={key}", method="HEAD")
        self.assertEqual((headers["Content-Type"], body), ("text/json", b""))
        _, headers, body = self.request(JSON_THEN_JS_PATH, f"key={key}")
        self.assertEqual((headers["Content-Type"], body),
                         ("application/javascript", b"export default 'hello';"))
        other = self.stack.enter_context(WptFixtureServer(self.root))
        _, headers, body = self.request(JSON_THEN_JS_PATH, f"key={key}", port=other.port)
        self.assertEqual((headers["Content-Type"], body), ("text/json", b'{"hello": "world"}'))

    def test_invalid_parameters_return_errors(self) -> None:
        for path, queries, expected_status in (
            (JSON_THEN_JS_PATH, ("", "key=", "key=not-a-uuid", "key=%FF"), 400),
            (JSON_LOAD_ERROR_PATH, ("", "test=", "test=a.b_load", "test=a%0A_load"), 500),
        ):
            for query in queries:
                with self.subTest(path=path, query=query):
                    self.assertEqual(self.request(path, query)[0], expected_status)

    def test_discovery_accepts_only_supported_resource_locations(self) -> None:
        cases = {
            "json-module/switch.html": ("../serve-json-then-js.py", True),
            "json-module/switch-dot.html": ("./../serve-json-then-js.py", True),
            "json-module/switch-absolute.html": (JSON_THEN_JS_PATH, True),
            "json-module/load.html": ("./load-error-events.py", True),
            "json-module/load-absolute.html": (JSON_LOAD_ERROR_PATH, True),
            "json-module/wrong-switch.html": ("serve-json-then-js.py", False),
            "json-module/wrong-load.html": ("../load-error-events.py", False),
            "json-module/suffix.html": ("./load-error-events.py.extra", False),
        }
        for name, (reference, _) in cases.items():
            path = self.root / SCRIPT_DIRECTORY / name
            path.write_text(
                '<script src="/resources/testharness.js"></script>'
                '<script src="/resources/testharnessreport.js"></script>'
                f'<script src="{reference}?test=load"></script>'
            )
        discovered = enumerate_cases(self.root, dir_prefixes=(SCRIPT_DIRECTORY,))
        self.assertEqual(
            sorted(case.case_path for case in discovered),
            sorted(SCRIPT_DIRECTORY + "/" + name for name, (_, allowed) in cases.items() if allowed),
        )


if __name__ == "__main__":
    unittest.main()
