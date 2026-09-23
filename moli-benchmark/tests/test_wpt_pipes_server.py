from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.server import WptFixtureServer


class WptPipeFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        (self.root / "module.json").write_bytes(b'{"value": 1}')
        (self.root / "template.txt").write_bytes(b"{{host}}")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, query: str, *, method: str = "GET", path: str = "/module.json"):
        connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
        try:
            connection.request(method, path + "?" + query)
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_header_values_follow_wptserve_argument_escaping(self) -> None:
        cases = (
            ("applic(ation/vnd.api+json", "applic(ation/vnd.api+json"),
            (r"application/vnd\,api+json", "application/vnd,api+json"),
            (r"application/vnd\)api+json", "application/vnd)api+json"),
            (r"a\(b\)c", "a(b)c"),
            ("text/plain|status(201", "text/plain|status(201"),
            (r"text/plain\;a=b", "text/plain;a=b"),
            (r"text/plain; x=a\\b", "text/plain; x=a\\b"),
            ("text/plain; x=a=b", "text/plain; x=a=b"),
            ("text/plain; x=%2C", "text/plain; x=%2C"),
            ("application/café+json", "application/café+json"),
            ("\u00a0application/json\u00a0", "\u00a0application/json\u00a0"),
            (r"text/plain; x=a\tb", "text/plain; x=a\tb"),
        )
        for value, expected in cases:
            for method in ("GET", "HEAD"):
                with self.subTest(value=value, method=method):
                    status, headers, body = self.request(
                        urlencode({"pipe": f"header(Content-Type,{value})"}),
                        method=method,
                    )
                    self.assertEqual(status, 200)
                    self.assertEqual(headers.get_all("Content-Type"), [expected])
                    self.assertEqual(headers["Content-Length"], "12")
                    self.assertEqual(body, b"" if method == "HEAD" else b'{"value": 1}')

    def test_malformed_pipes_return_http_errors_instead_of_default_json(self) -> None:
        commands = (
            "header(Content-Type,applic)ation/vnd.api+json)",
            "header(Content-Type,application/vnd)api+json)",
            "header(Content-Type,app(lic)ation/vnd(api)+json)",
            "header(Content-Type,applic,ation/vnd.api+json)",
            "header(Content-Type,application/vnd,api+json)",
            "header(Content-Type,application/json,true,extra)",
            "header(Content-Type,application/json, true)",
            "header(Content-Type)",
            "header",
            "header(Content-Type,application/json)|unknown",
            "header(Content-Type,application/json)| status(201)",
            "header(Content-Type,application/json)extra",
            "header(Content-Type,application/json\\",
            "status(invalid)",
            "status(201,202)",
        )
        for command in commands:
            for method in ("GET", "HEAD"):
                with self.subTest(command=command, method=method):
                    status, _, body = self.request(
                        urlencode({"pipe": command}), method=method
                    )
                    self.assertEqual(status, 500)
                    self.assertNotEqual(body, b'{"value": 1}')
                    if method == "HEAD":
                        self.assertEqual(body, b"")

    def test_non_latin1_headers_return_http_errors_without_aborting(self) -> None:
        for value in ("申请/vnd.api+json", "application/vnd.api🚀+json", "app™/json"):
            for method in ("GET", "HEAD"):
                with self.subTest(value=value, method=method):
                    status, _, body = self.request(
                        urlencode({"pipe": f"header(Content-Type,{value})"}),
                        method=method,
                    )
                    self.assertEqual(status, 500)
                    if method == "HEAD":
                        self.assertEqual(body, b"")

    def test_header_append_and_replacement_keep_command_order(self) -> None:
        (self.root / "module.json.headers").write_text("X-Test: sidecar\n")
        commands = (
            ("header(X-Test,one,TrUe)|header(x-test,two,1)", ["sidecar", "one", "two"]),
            ("header(X-Test,one,true)|header(x-test,two,0)", ["two"]),
            ("header(X-Test,one,false)|header(x-test,two,true)", ["one", "two"]),
            ("header(X-Test,one)header(x-test,two,true)", ["one", "two"]),
            ("header(X-Test,one", ["one"]),
            (r"header(X-Test,one\,true)", ["one,true"]),
            ("header(X-Test,)", [""]),
        )
        for command, expected in commands:
            with self.subTest(command=command):
                status, headers, _ = self.request(urlencode({"pipe": command}))
                self.assertEqual(status, 200)
                self.assertEqual(headers.get_all("X-Test"), expected)

    def test_explicit_cache_control_is_not_overridden_by_the_fixture_default(self) -> None:
        for method in ("GET", "HEAD"):
            with self.subTest(method=method, source="default"):
                status, headers, _ = self.request("", method=method)
                self.assertEqual(status, 200)
                self.assertEqual(headers.get_all("Cache-Control"), ["no-store"])
        (self.root / "module.json.headers").write_text("cAcHe-CoNtRoL: max-age=600\n")
        for pipe, expected in (
            ("", ["max-age=600"]),
            ("header(Cache-Control,max-age=30)", ["max-age=30"]),
            ("header(cache-control,public,true)", ["max-age=600", "public"]),
            ("header(Cache-Control,)", [""]),
        ):
            for method in ("GET", "HEAD"):
                with self.subTest(method=method, pipe=pipe):
                    status, headers, body = self.request(urlencode({"pipe": pipe}), method=method)
                    self.assertEqual(status, 200)
                    self.assertEqual(headers.get_all("Cache-Control"), expected)
                    self.assertEqual(body, b"" if method == "HEAD" else b'{"value": 1}')
        (self.root / "module.json.headers").unlink()
        _, headers, _ = self.request(urlencode({"pipe": "header(Cache-Control,max-age=60)"}))
        self.assertEqual(headers.get_all("Cache-Control"), ["max-age=60"])

    def test_only_last_nonempty_pipe_parameter_is_applied(self) -> None:
        cases = (
            (["unknown", "header(X-Test,last)"], 200, "last"),
            (["header(X-Test,first)", "status(201)"], 201, None),
            (["header(X-Test,first)", ""], 200, "first"),
            (["header(X-Test,first)", "unknown"], 500, None),
        )
        for commands, expected_status, expected_header in cases:
            with self.subTest(commands=commands):
                status, headers, _ = self.request(urlencode([
                    ("pipe", command) for command in commands
                ]))
                self.assertEqual(status, expected_status)
                self.assertEqual(headers.get("X-Test"), expected_header)

    def test_pipes_embedded_in_values_do_not_change_the_response(self) -> None:
        command = r"header(X-Test,first|sub|trickle(d3\)|last)|status(201)"
        with patch("moli_benchmark.wpt_cross.server.time.sleep") as sleep:
            status, headers, body = self.request(
                urlencode({"pipe": command}), path="/template.txt"
            )
        self.assertEqual(status, 201)
        self.assertEqual(headers["X-Test"], "first|sub|trickle(d3)|last")
        self.assertEqual(body, b"{{host}}")
        sleep.assert_not_called()
        status, _, body = self.request("pipe=sub(none)", path="/template.txt")
        self.assertEqual(status, 200)
        self.assertEqual(body, b"localhost")

    def test_header_newlines_cannot_inject_response_headers(self) -> None:
        for value in ("before\r\nInjected: value", r"before\r\nInjected: value"):
            with self.subTest(value=value):
                status, headers, _ = self.request(urlencode({"pipe": f"header(X-Test,{value})"}))
                self.assertEqual(status, 200)
                self.assertEqual(headers["X-Test"], "before  Injected: value")
                self.assertIsNone(headers["Injected"])

    def test_status_conversion_and_errors_also_work_for_inspection_methods(self) -> None:
        for method in ("GET", "HEAD", "POST", "OPTIONS", "CUSTOM"):
            for command, expected_status in (
                ("status(00201)", 201),
                ("status( +201 )", 201),
                ("status(invalid)", 500),
                ("status(201)|unknown", 500),
            ):
                with self.subTest(method=method, command=command):
                    status, _, body = self.request(
                        urlencode({"pipe": command}), method=method,
                        path="/fetch/api/resources/inspect-headers.py",
                    )
                    self.assertEqual(status, expected_status)
                    if method == "HEAD":
                        self.assertEqual(body, b"")


if __name__ == "__main__":
    unittest.main()
