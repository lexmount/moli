from __future__ import annotations

import json
import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE = "/service-workers/service-worker/resources/request-headers.py"


class ServiceWorkerRequestHeadersFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        resource = self.root / RESOURCE.lstrip("/")
        resource.parent.mkdir(parents=True)
        resource.write_text("import json\n# Python source must not be returned")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(
        self, method: str = "GET", query: str = "", *, path: str = RESOURCE,
        headers: list[tuple[str, str]] | None = None,
    ):
        connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
        try:
            connection.putrequest(method, path + "?" + query)
            for name, value in headers or []:
                connection.putheader(name, value)
            connection.endheaders()
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_returns_actual_request_fields_as_lowercase_json(self) -> None:
        for query in ("", "ignore=true", "url=request-headers.py", "irrelevant=%FF"):
            with self.subTest(query=query):
                status, headers, body = self.request(query=query, headers=[
                    ("Referer", "https://caller.test/private"),
                    ("Origin", "https://caller.test"), ("X-Empty", ""),
                    ("X-Value", "x\x01\t\x1f\x80\xffx"),
                ])
                value = json.loads(body)
                self.assertEqual(status, 200)
                self.assertEqual(headers.get_all("Content-Type"), ["application/json"])
                self.assertEqual(headers["Content-Length"], str(len(body)))
                self.assertEqual(value["referer"], "https://caller.test/private")
                self.assertEqual(value["origin"], "https://caller.test")
                self.assertEqual(value["x-empty"], "")
                self.assertEqual(value["x-value"], "x\x01\t\x1f\x80\xffx")
                self.assertTrue(all(name == name.lower() for name in value))
                self.assertFalse(body.startswith(b"import json"))
                self.assertIsNone(headers["Access-Control-Allow-Origin"])
                self.assertIsNone(headers["Cache-Control"])

    def test_repeated_fields_join_in_wire_order_and_absent_fields_stay_absent(self) -> None:
        status, _, body = self.request(headers=[
            ("X-List", "first "), ("x-list", "second"),
            ("X-LIST", ""), ("X-Single", "trailing "),
        ])
        value = json.loads(body)
        self.assertEqual(status, 200)
        self.assertEqual(value["x-list"], "first, second, ")
        self.assertEqual(value["x-single"], "trailing ")
        self.assertNotIn("referer", value)
        self.assertNotIn("cookie", value)

    def test_head_and_extension_methods_use_the_header_handler(self) -> None:
        for method in ("GET", "HEAD", "POST", "OPTIONS", "PUT", "PATCH", "DELETE", "YO", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(method, headers=[("X-Method", method)])
                self.assertEqual(status, 200)
                self.assertEqual(headers["Content-Type"], "application/json")
                self.assertGreater(int(headers["Content-Length"]), 0)
                if method == "HEAD":
                    self.assertEqual(body, b"")
                else:
                    self.assertEqual(json.loads(body)["x-method"], method)

    def test_unread_uploads_do_not_block_header_responses(self) -> None:
        for method in ("POST", "OPTIONS", "PUT", "chicken"):
            for framing in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
                with self.subTest(method=method, framing=framing):
                    status, headers, body = self.request(method, headers=[framing])
                    self.assertEqual(status, 200)
                    self.assertEqual(headers["Connection"], "close")
                    self.assertEqual(json.loads(body)[framing[0].lower()], framing[1])

    def test_pipes_apply_after_json_and_resource_paths_are_exact(self) -> None:
        query = urlencode({"pipe": "status(201)|header(Content-Type,text/plain)|header(X-Result,ready)"})
        for method in ("GET", "HEAD", "POST", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(method, query, headers=[("X-Value", "{{host}}")])
                self.assertEqual(status, 201)
                self.assertEqual(headers.get_all("Content-Type"), ["text/plain"])
                self.assertEqual(headers["X-Result"], "ready")
                if method != "HEAD":
                    self.assertEqual(json.loads(body)["x-value"], "{{host}}")
        self.assertEqual(self.request(path=RESOURCE.replace("request-", "%72equest-"))[0], 200)
        for path in (RESOURCE + "2", "/wrong" + RESOURCE, RESOURCE.replace("resources", "RESOURCES")):
            with self.subTest(path=path):
                self.assertEqual(self.request(path=path)[0], 404)


if __name__ == "__main__":
    unittest.main()
