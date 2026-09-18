from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer

EMPTY_LOCATION = "/fetch/api/resources/redirect-empty-location.py"


class EmptyLocationFixtureTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))


    def request(self, query="", *, method="GET", headers=None, body=None, port=None, path=EMPTY_LOCATION):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=2)
        try:
            connection.request(method, path + "?" + query, body, headers or {})
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()


    def test_empty_location_is_present_for_every_method_without_cors_grants(self):
        for method in ("GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "YO", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    "redirect_status=307&location=/ignored", path=EMPTY_LOCATION,
                    method=method, headers={"Origin": "https://caller.test"}, body=b"upload",
                )
                self.assertEqual((status, body), (302, b""))
                self.assertEqual(headers.get_all("Location"), [""])
                self.assertEqual(headers["Content-Length"], "0")
                for name in ("Content-Type", "Cache-Control", "Access-Control-Allow-Origin"):
                    self.assertIsNone(headers[name], name)
        for path in (EMPTY_LOCATION + "2", "/wrong" + EMPTY_LOCATION):
            with self.subTest(path=path):
                self.assertEqual(self.request(path=path)[0], 404)


    def test_empty_location_responds_before_unused_uploads_finish(self):
        for method in ("POST", "PUT", "OPTIONS"):
            for framing in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
                with self.subTest(method=method, framing=framing):
                    connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
                    try:
                        connection.putrequest(method, EMPTY_LOCATION)
                        connection.putheader(*framing)
                        connection.endheaders()
                        response = connection.getresponse()
                        self.assertEqual((response.status, response.read()), (302, b""))
                        self.assertEqual(response.headers.get_all("Location"), [""])
                        self.assertEqual(response.headers["Connection"], "close")
                    finally:
                        connection.close()


    def test_case_selection_recognizes_empty_location_references(self):
        sources = {
            "absolute": f"fetch('{EMPTY_LOCATION}');",
            "concat": 'fetch(RESOURCES_DIR + "redirect-empty-location.py");',
            "relative": "fetch('../resources/redirect-empty-location.py');",
            "template": "fetch(`${RESOURCES_DIR}redirect-empty-location.py?ignored=1`);",
            "bare": "fetch('redirect-empty-location.py');",
            "prefix": f"fetch('/wrong{EMPTY_LOCATION}');",
            "suffix": "fetch('../resources/redirect-empty-location.py2');",
            "unknown": f"fetch('{EMPTY_LOCATION}'); fetch('../resources/unknown.py');",
        }
        for name, source in sources.items():
            path = self.root / f"fetch/api/redirect/{name}.any.js"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// META: global=window,worker\n" + source)
        selected = enumerate_cases(self.root, dir_prefixes=("fetch",), any_js_global="both")
        self.assertEqual([case.case_path for case in selected], [
            f"fetch/api/redirect/{name}.any.js?moli-wpt-any={realm}"
            for name in ("absolute", "concat", "relative", "template")
            for realm in ("dedicatedworker", "window")
        ])
