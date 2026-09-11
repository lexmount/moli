from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE = "/xhr/resources/inspect-headers.py"
ECHO_RESOURCE = "/xhr/resources/echo-headers.py"


class XhrHeaderFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))

    def server(self) -> WptFixtureServer:
        return self.stack.enter_context(WptFixtureServer(self.root))

    def request(
        self, port: int, query: str, headers: list[tuple[str, str]],
        *, method: str = "GET", resource: str = RESOURCE,
    ) -> tuple[int, dict[str, str], bytes]:
        connection = HTTPConnection("127.0.0.1", port, timeout=2)
        try:
            connection.putrequest(method, resource + "?" + query)
            for name, value in headers:
                connection.putheader(name, value)
            connection.endheaders()
            response = connection.getresponse()
            return (
                response.status,
                {name.lower(): value for name, value in response.getheaders()},
                response.read(),
            )
        finally:
            connection.close()

    def test_name_filter_preserves_case_duplicates_whitespace_and_bytes(self) -> None:
        server = self.server()
        status, headers, body = self.request(server.port, "filter_name=X-CaSe", [
            ("x-Case", "first"),
            ("X-Unrelated", "ignored"),
            ("X-CASE", ""),
            ("x-cASE", "caf\xe9\xff \t"),
            ("X-case", "folded\r\n\tvalue"),
        ])
        self.assertEqual(status, 200)
        self.assertEqual(body, b"x-Case: first\nX-CASE: \nx-cASE: caf\xe9\xff \t\n"
                              b"X-case: folded\r\n\tvalue\n")
        self.assertEqual(headers["content-type"], "text/plain")
        self.assertEqual(headers["content-length"], str(len(body)))
        self.assertFalse(any(name.startswith("access-control-") for name in headers))

    def test_value_filter_is_exact_and_takes_priority_over_name(self) -> None:
        server = self.server()
        status, _, body = self.request(
            server.port, "filter_value=%E9%FF&filter_name=x-other", [
                ("X-First", "\xe9\xff"),
                ("x-second", "\xe9\xff"),
                ("X-First", "\xe9\xff"),
                ("X-Different-Case", "\xc9\xff"),
                ("X-Longer", "\xe9\xff "),
                ("X-Other", "not-matched"),
            ]
        )
        self.assertEqual((status, body), (200, b"X-First,x-second,X-First,"))

    def test_query_uses_first_values_and_empty_value_falls_back_to_name(self) -> None:
        server = self.server()
        request_headers = [("X-First", "match"), ("X-Second", "other")]
        for query, expected in (
            ("", b""),
            ("filter_name=missing", b""),
            ("filter_name=x-first&filter_name=x-second", b"X-First: match\n"),
            ("filter_value=match&filter_value=other", b"X-First,"),
            ("filter_value=&filter_value=match&filter_name=x-second", b"X-Second: other\n"),
            ("filter_name=&filter_name=x-first", b""),
            ("filter_value=missing&filter_name=x-first", b""),
        ):
            with self.subTest(query=query):
                status, _, body = self.request(server.port, query, request_headers)
                self.assertEqual((status, body), (200, expected))

    def test_cors_flag_and_methods_match_upstream(self) -> None:
        server = self.server()
        cors_headers = {
            "access-control-allow-origin": "*",
            "access-control-allow-credentials": "true",
            "access-control-allow-methods": "GET, POST, PUT, FOO",
            "access-control-allow-headers": "x-test, x-foo",
            "access-control-expose-headers": (
                "x-request-method, x-request-content-type, x-request-query, "
                "x-request-content-length"
            ),
        }
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "YO", "FOO"):
            for flag in ("", "&cors", "&cors=", "&cors=false"):
                with self.subTest(method=method, flag=flag):
                    status, headers, body = self.request(
                        server.port, "filter_name=x-test" + flag, [
                            ("X-Test", "value"),
                            ("Origin", "http://example.test"),
                            ("Access-Control-Request-Method", "DELETE"),
                            ("Access-Control-Request-Headers", "x-unexpected"),
                        ], method=method,
                    )
                    expected = b"X-Test: value\n"
                    self.assertEqual((status, body), (200, b"" if method == "HEAD" else expected))
                    self.assertEqual(headers["content-type"], "text/plain")
                    self.assertEqual(headers["content-length"], str(len(expected)))
                    self.assertEqual(
                        {name: value for name, value in headers.items()
                         if name.startswith("access-control-")},
                        cors_headers if flag else {},
                    )
                    self.assertFalse(any(name.startswith("x-request-") for name in headers))

    def test_header_response_does_not_wait_for_upload(self) -> None:
        server = self.server()
        for name, value in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
            with self.subTest(framing=name):
                # request() sends only headers, leaving the promised body unread.
                status, headers, body = self.request(
                    server.port, "filter_name=" + name, [(name, value)], method="POST"
                )
                self.assertEqual((status, body), (200, f"{name}: {value}\n".encode()))
                self.assertEqual(headers["connection"], "close")

    def test_echo_preserves_header_order_case_duplicates_and_serialization(self) -> None:
        server = self.server()
        status, headers, body = self.request(server.port, "", [
            ("x-Case", "first"),
            ("X-Unrelated", "middle"),
            ("X-CASE", "second"),
            ("X-Empty", ""),
            ("X-Whitespace", "value \t"),
            ("X-Fold", "first\r\n\tsecond"),
            ("X-Bytes", "caf\xe9\xff"),
        ], resource=ECHO_RESOURCE)
        self.assertEqual(status, 200)
        self.assertEqual(body, (
            f"Host: 127.0.0.1:{server.port}\nAccept-Encoding: identity\n".encode()
            + b"x-Case: first\nX-Unrelated: middle\nX-CASE: second\n"
            b"X-Empty: \nX-Whitespace: value \t\nX-Fold: first\n\tsecond\n"
            b"X-Bytes: =?utf-8?b?Y2Fmw6nDvw==?=\n\n"
        ))
        self.assertEqual(headers["content-type"], "text/plain")
        self.assertEqual(headers["content-length"], str(len(body)))
        self.assertEqual(headers["connection"], "close")

    def test_echo_supports_methods_head_and_ignores_query_filters(self) -> None:
        server = self.server()
        expected = (
            f"Host: 127.0.0.1:{server.port}\nAccept-Encoding: identity\n"
            "X-Test: value\n\n"
        ).encode()
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "YO", "FOO"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    server.port, "filter_name=missing&cors", [("X-Test", "value")],
                    method=method, resource=ECHO_RESOURCE,
                )
                self.assertEqual((status, body), (200, b"" if method == "HEAD" else expected))
                self.assertEqual(headers["content-length"], str(len(expected)))
                self.assertEqual(headers["connection"], "close")
                self.assertFalse(any(name.startswith("access-control-") for name in headers))

    def test_echo_does_not_wait_for_or_validate_the_upload(self) -> None:
        server = self.server()
        for name, value in (("Content-Length", "100000000"), ("Transfer-Encoding", "chunked")):
            with self.subTest(framing=name):
                status, headers, body = self.request(
                    server.port, "", [(name, value)], method="POST", resource=ECHO_RESOURCE,
                )
                self.assertEqual(status, 200)
                self.assertIn(f"{name}: {value}\n".encode(), body)
                self.assertEqual(headers["connection"], "close")

    def test_echo_only_handles_the_exact_resource_path(self) -> None:
        server = self.server()
        for path in (
            ECHO_RESOURCE + "2", ECHO_RESOURCE + ".js", "/wrong" + ECHO_RESOURCE,
            "/resources/echo-headers.py",
        ):
            with self.subTest(path=path):
                status, _, _ = self.request(server.port, "", [], resource=path)
                self.assertEqual(status, 404)

    def test_case_selection_allows_only_supported_header_fixture_references(self) -> None:
        sources = {
            "xhr/absolute.window.js": "fetch('/xhr/resources/inspect-headers.py');",
            "xhr/relative.window.js": "fetch('./resources/inspect-headers.py?filter_name=test');",
            "xhr/nested/parent.window.js": "fetch('../resources/inspect-headers.py');",
            "xhr/unknown.window.js": "fetch('resources/inspect-headers.py'); fetch('resources/unknown.py');",
            "xhr/suffix.window.js": "fetch('resources/inspect-headers.py2');",
            "xhr/script-suffix.window.js": "fetch('resources/inspect-headers.py.js');",
            "xhr/prefix.window.js": "fetch('/wrong/xhr/resources/inspect-headers.py');",
            "xhr/wrong-parent.window.js": "fetch('../resources/inspect-headers.py');",
            "xhr/wrong-root.window.js": "fetch('inspect-headers.py');",
            "xhr/nested/wrong-relative.window.js": "fetch('resources/inspect-headers.py');",
        }
        sources.update({
            path.replace(".window.js", "-echo.window.js"): source.replace(
                "inspect-headers.py", "echo-headers.py"
            )
            for path, source in list(sources.items())
        })
        for path, source in sources.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(source)
        selected = enumerate_cases(self.root, dir_prefixes=("xhr",))
        self.assertEqual([case.case_path for case in selected], [
            "xhr/absolute-echo.window.js?moli-wpt-script=window",
            "xhr/absolute.window.js?moli-wpt-script=window",
            "xhr/nested/parent-echo.window.js?moli-wpt-script=window",
            "xhr/nested/parent.window.js?moli-wpt-script=window",
            "xhr/relative-echo.window.js?moli-wpt-script=window",
            "xhr/relative.window.js?moli-wpt-script=window",
        ])
