from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE = "/xhr/resources/corsenabled.py"
PUT_RESOURCE = "/xhr/resources/access-control-basic-put-allow.py"
STAR_RESOURCE = "/xhr/resources/access-control-preflight-request-allow-headers-returns-star.py"


class XhrCorsFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def request(self, server, method="GET", query="", body=None, headers=None, resource=RESOURCE):
        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
        try:
            connection.request(method, resource + ("?" + query if query else ""), body, headers or {})
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_cors_headers_and_method_reflection(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for method in ("GET", "HEAD", "OPTIONS", "POST", "PUT", "FOO"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    server, method, headers={"Origin": "https://caller.test"}
                )
                self.assertEqual(status, 200)
                self.assertEqual(body, b"" if method == "HEAD" else b"Test")
                self.assertEqual(headers["Content-Length"], "4")
                self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
                self.assertEqual(headers["Access-Control-Allow-Credentials"], "true")
                self.assertEqual(headers["Access-Control-Allow-Methods"], "GET, POST, PUT, FOO")
                self.assertEqual(headers["Access-Control-Allow-Headers"], "x-test, x-foo")
                self.assertEqual(headers["X-Request-Method"], method)
                self.assertEqual(headers["X-Request-Query"], "NO")
                self.assertEqual(headers["X-Request-Content-Type"], "NO")
                self.assertEqual(headers["X-Request-Data"], "")
                self.assertEqual(headers["Access-Control-Expose-Headers"],
                                 "x-request-method, x-request-content-type, x-request-query, "
                                 "x-request-content-length, x-request-data")

    def test_reflects_raw_query_body_and_empty_content_type(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        status, headers, body = self.request(
            server, "POST", "value=a%2Bb&value=second", b"caf\xe9", {"Content-Type": ""}
        )
        self.assertEqual((status, body), (200, b"Test"))
        self.assertEqual(headers["X-Request-Query"], "value=a%2Bb&value=second")
        self.assertEqual(headers["X-Request-Content-Length"], "4")
        self.assertEqual(headers["X-Request-Content-Type"], "")
        self.assertEqual(headers["X-Request-Data"], "café")
        status, headers, _ = self.request(server)
        self.assertEqual(status, 200)
        self.assertEqual(headers["X-Request-Content-Length"], "NO")

    def test_safelist_query_appends_an_allow_headers_field(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for query in ("safelist_content_type", "safelist_content_type=", "safelist_content_type=no"):
            with self.subTest(query=query):
                status, headers, _ = self.request(server, "OPTIONS", query)
                self.assertEqual(status, 200)
                self.assertEqual(headers.get_all("Access-Control-Allow-Headers"),
                                 ["x-test, x-foo", "content-type"])

    def test_delay_uses_first_integer_parameter_and_rejects_invalid_values(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        with patch("moli_benchmark.wpt_cross.server.time.sleep") as sleep:
            status, _, _ = self.request(server, query="delay=1&delay=99")
            self.assertEqual(status, 200)
            sleep.assert_called_once_with(1)
        for query in ("delay=bad", "delay=", "delay=-1"):
            with self.subTest(query=query):
                status, _, _ = self.request(server, query=query)
                self.assertEqual(status, 500)

    def test_handler_requires_the_exact_resource_path(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for resource in (RESOURCE + "2", RESOURCE + ".js", "/wrong" + RESOURCE):
            with self.subTest(resource=resource):
                status, _, _ = self.request(server, resource=resource)
                self.assertEqual(status, 404)

    def test_put_preflight_and_upload_preserve_origin_and_bytes(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for origin in ("https://caller.test", "null", ""):
            with self.subTest(origin=origin):
                status, headers, body = self.request(
                    server, "OPTIONS", headers={"Origin": origin}, resource=PUT_RESOURCE
                )
                self.assertEqual((status, body), (200, b""))
                self.assertEqual(headers["Content-Type"], "text/plain")
                self.assertEqual(headers["Access-Control-Allow-Origin"], origin)
                self.assertEqual(headers["Access-Control-Allow-Credentials"], "true")
                self.assertEqual(headers["Access-Control-Allow-Methods"], "PUT")
                self.assertIsNone(headers["Access-Control-Allow-Headers"])
                payload = b"PUT \x00\xff body"
                status, headers, body = self.request(
                    server, "PUT", body=payload, headers={"Origin": origin},
                    resource=PUT_RESOURCE,
                )
                self.assertEqual((status, body),
                                 (200, b"PASS: Cross-domain access allowed.\n" + payload))
                self.assertEqual(headers["Content-Type"], "text/plain")
                self.assertEqual(headers["Access-Control-Allow-Origin"], origin)
                self.assertEqual(headers["Access-Control-Allow-Credentials"], "true")
                self.assertIsNone(headers["Access-Control-Allow-Methods"])

    def test_put_fixture_missing_origin_matches_upstream_failure(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for method in ("OPTIONS", "PUT"):
            with self.subTest(method=method):
                status, headers, _ = self.request(server, method, resource=PUT_RESOURCE)
                self.assertEqual(status, 500)
                self.assertIsNone(headers["Access-Control-Allow-Origin"])

    def test_put_fixture_reports_other_methods_without_cors_headers(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for method in ("GET", "HEAD", "POST", "DELETE", "FOO"):
            with self.subTest(method=method):
                status, headers, body = self.request(server, method, resource=PUT_RESOURCE)
                expected = b"Wrong method: " + method.encode("ascii")
                self.assertEqual((status, body),
                                 (200, b"" if method == "HEAD" else expected))
                self.assertEqual(headers["Content-Length"], str(len(expected)))
                self.assertIsNone(headers["Access-Control-Allow-Origin"])

    def test_wildcard_preflight_and_actual_request_have_distinct_headers(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        status, headers, body = self.request(server, "OPTIONS", resource=STAR_RESOURCE)
        self.assertEqual((status, body), (200, b""))
        self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
        self.assertEqual(headers["Access-Control-Allow-Headers"], "*")
        self.assertIsNone(headers["Access-Control-Allow-Methods"])
        self.assertIsNone(headers["Access-Control-Allow-Credentials"])
        self.assertIsNone(headers["Content-Type"])
        for value in ("foobar", "0"):
            with self.subTest(value=value):
                status, headers, body = self.request(
                    server, headers={"x-test": value}, resource=STAR_RESOURCE
                )
                self.assertEqual((status, body), (200, b"PASS"))
                self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
                self.assertEqual(headers["Content-Type"], "text/plain")
                self.assertIsNone(headers["Access-Control-Allow-Headers"])

    def test_wildcard_fixture_rejects_missing_or_empty_test_header(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for request_headers in ({}, {"X-Test": ""}):
            with self.subTest(headers=request_headers):
                status, headers, body = self.request(
                    server, headers=request_headers, resource=STAR_RESOURCE
                )
                self.assertEqual((status, body), (400, b""))
                self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
                self.assertIsNone(headers["Content-Type"])

    def test_wildcard_fixture_other_methods_have_no_cors_grants(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for method in ("HEAD", "POST", "PUT", "DELETE", "FOO"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    server, method, headers={"X-Test": "foobar"}, resource=STAR_RESOURCE
                )
                self.assertEqual((status, body), (200, b""))
                self.assertIsNone(headers["Access-Control-Allow-Origin"])
                self.assertIsNone(headers["Content-Type"])

    def test_preflight_fixtures_require_exact_resource_paths(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        for resource in (PUT_RESOURCE, STAR_RESOURCE):
            for wrong in (resource + "2", resource + ".js", "/wrong" + resource):
                with self.subTest(path=wrong):
                    status, _, _ = self.request(server, resource=wrong)
                    self.assertEqual(status, 404)

    def test_preflight_case_selection_rejects_unsupported_dependencies(self) -> None:
        for resource in (PUT_RESOURCE, STAR_RESOURCE):
            name = Path(resource).name
            sources = {
                f"xhr/absolute-{name}.html": f"fetch('{resource}');",
                f"xhr/relative-{name}.html": f"fetch('resources/{name}');",
                f"xhr/nested/relative-{name}.html": f"fetch('../resources/{name}?x=1');",
                f"xhr/unknown-{name}.html": f"fetch('{resource}'); fetch('unknown.py');",
                f"xhr/wrong-{name}.html": f"fetch('{name}');",
            }
            for path, script in sources.items():
                target = self.root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text('<script src="/resources/testharness.js"></script><script>'
                                  + script + '</script>')
        selected = enumerate_cases(self.root, dir_prefixes=("xhr",))
        expected = sorted(
            f"xhr/{prefix}{Path(resource).name}.html"
            for resource in (PUT_RESOURCE, STAR_RESOURCE)
            for prefix in ("absolute-", "relative-", "nested/relative-")
        )
        self.assertEqual([case.case_path for case in selected], expected)

    def test_case_selection_recognizes_relative_and_absolute_references(self) -> None:
        sources = {
            "xhr/absolute.html": f"fetch('{RESOURCE}');",
            "xhr/relative.html": "fetch('resources/corsenabled.py');",
            "xhr/nested/relative.html": "fetch('../resources/corsenabled.py?delay=0');",
            "xhr/unknown.html": "fetch('resources/corsenabled.py'); fetch('unknown.py');",
            "xhr/wrong-relative.html": "fetch('corsenabled.py');",
        }
        for name, script in sources.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('<script src="/resources/testharness.js"></script><script>'
                            + script + '</script>')
        selected = enumerate_cases(self.root, dir_prefixes=("xhr",))
        self.assertEqual([case.case_path for case in selected], [
            "xhr/absolute.html", "xhr/nested/relative.html", "xhr/relative.html",
        ])
