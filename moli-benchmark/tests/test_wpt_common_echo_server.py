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


ECHO_PATH = "/common/echo.py"


class CommonEchoFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        (self.root / "common").mkdir()
        (self.root / "common/echo.py").write_text("# Python source must not be served")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, query: str, *, method: str = "GET", path: str = ECHO_PATH, body=None):
        connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
        try:
            connection.request(method, path + "?" + query, body, {"Origin": "https://caller.test"})
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_content_is_html_and_preserves_query_bytes_and_first_value(self) -> None:
        cases = (
            ("content=%3Cscript%3EglobalThis.answer%3D42%3C%2Fscript%3E", b"<script>globalThis.answer=42</script>"),
            ("content=first&content=second", b"first"),
            ("content=&content=second", b""),
            ("content", b""),
            ("content=A+B%2BC%26D%3DE", b"A B+C&D=E"),
            ("content=%FF%00%C3%A9%FE", b"\xff\x00\xc3\xa9\xfe"),
            ("co%6Etent=decoded-key", b"decoded-key"),
            ("content=%2520;%invalid", b"%20;%invalid"),
            ("content={{host}}", b"{{host}}"),
        )
        for query, expected in cases:
            for method in ("GET", "HEAD"):
                with self.subTest(query=query, method=method):
                    status, headers, body = self.request(query, method=method)
                    self.assertEqual(status, 200)
                    self.assertEqual(headers.get_all("Content-Type"), ["text/html"])
                    self.assertEqual(headers["X-XSS-Protection"], "0")
                    self.assertEqual(headers["Content-Length"], str(len(expected)))
                    self.assertEqual(body, b"" if method == "HEAD" else expected)
                    self.assertIsNone(headers["Access-Control-Allow-Origin"])
                    self.assertIsNone(headers["Cache-Control"])

    def test_all_methods_read_query_content_instead_of_request_body(self) -> None:
        for method in ("POST", "OPTIONS", "PUT", "PATCH", "DELETE", "YO", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    "content=query", method=method, body=b"content=upload",
                )
                self.assertEqual((status, body), (200, b"query"))
                self.assertEqual(headers["Content-Type"], "text/html")
                self.assertIsNone(headers["Access-Control-Allow-Origin"])

    def test_unused_uploads_do_not_delay_the_response(self) -> None:
        for method in ("POST", "PUT", "OPTIONS"):
            for framing in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
                with self.subTest(method=method, framing=framing):
                    connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
                    try:
                        connection.putrequest(method, ECHO_PATH + "?content=ready")
                        connection.putheader(*framing)
                        connection.endheaders()
                        response = connection.getresponse()
                        self.assertEqual((response.status, response.read()), (200, b"ready"))
                        self.assertEqual(response.headers["Connection"], "close")
                    finally:
                        connection.close()

    def test_missing_content_is_an_error_and_other_paths_are_not_echo_handlers(self) -> None:
        for method in ("GET", "HEAD", "POST"):
            for query in ("", "Content=wrong-case"):
                with self.subTest(method=method, query=query):
                    self.assertEqual(self.request(query, method=method)[0], 500)
        for path in (ECHO_PATH + "2", "/wrong" + ECHO_PATH, "/COMMON/echo.py"):
            with self.subTest(path=path):
                self.assertEqual(self.request("content=not-an-echo", path=path)[0], 404)

    def test_response_pipes_apply_after_the_handler(self) -> None:
        query = urlencode({
            "content": "{{host}}",
            "pipe": "sub(none)|status(201)|header(Content-Type,text/plain)|header(X-XSS-Protection,1)",
        })
        for method in ("GET", "HEAD", "POST", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(query, method=method)
                self.assertEqual(status, 201)
                self.assertEqual(headers.get_all("Content-Type"), ["text/plain"])
                self.assertEqual(headers.get_all("X-XSS-Protection"), ["1"])
                self.assertEqual(headers["Content-Length"], "9")
                self.assertEqual(body, b"" if method == "HEAD" else b"localhost")
        with patch("moli_benchmark.wpt_cross.server.time.sleep") as sleep:
            status, headers, body = self.request(urlencode({"content": "delayed", "pipe": "trickle(d0.01)"}))
        self.assertEqual((status, body), (200, b"delayed"))
        self.assertIsNone(headers["Content-Length"])
        self.assertEqual(headers["Cache-Control"], "no-cache, no-store, must-revalidate")
        self.assertEqual(headers["Pragma"], "no-cache")
        self.assertEqual(headers["Expires"], "0")
        sleep.assert_called_once_with(0.01)
        for command, expected, pragma, expires in (
            ("trickle(d0)|header(Cache-Control,private,true)",
             ["no-cache, no-store, must-revalidate", "private"], "no-cache", "0"),
            ("header(Cache-Control,private)|trickle(d0)", ["private"], None, None),
            ("header(Pragma,no-cache)|trickle(d0)", None, "no-cache", None),
            ("header(Expires,0)|trickle(d0)", None, None, "0"),
        ):
            with self.subTest(command=command):
                status, headers, body = self.request(urlencode({"content": "ok", "pipe": command}))
                self.assertEqual((status, body), (200, b"ok"))
                self.assertEqual(headers.get_all("Cache-Control"), expected)
                self.assertEqual(headers["Pragma"], pragma)
                self.assertEqual(headers["Expires"], expires)
        for method in ("GET", "POST", "chicken"):
            self.assertEqual(self.request("content=not-served&pipe=unknown", method=method)[0], 500)

    def test_discovery_recognizes_only_references_to_the_common_echo_handler(self) -> None:
        directory = "html/browsers/echo-tests"
        cases = {
            "absolute.html": (ECHO_PATH, True),
            "relative.html": ("../../../common/echo.py", True),
            "dot-relative.html": ("./../../../common/echo.py", True),
            "bare.html": ("echo.py", False),
            "wrong-relative.html": ("../../common/echo.py", False),
            "wrong-root.html": ("/other/common/echo.py", False),
            "suffix.html": (ECHO_PATH + ".extra", False),
        }
        for name, (reference, _) in cases.items():
            path = self.root / directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                '<script src="/resources/testharness.js"></script>'
                f'<script>fetch("{reference}?content=hello")</script>'
            )
        (self.root / directory / "unknown.html").write_text(
            '<script src="/resources/testharness.js"></script>'
            f'<script>fetch("{ECHO_PATH}?content=x"); fetch("/unsupported.py")</script>'
        )
        (self.root / directory / "script.window.js").write_text(
            f'const echoURL = content => `{ECHO_PATH}?content=${{encodeURIComponent(content)}}`;'
        )
        discovered = enumerate_cases(self.root, dir_prefixes=(directory,))
        self.assertEqual(
            sorted(case.case_path.split("?")[0] for case in discovered),
            sorted([directory + "/" + name for name, (_, allowed) in cases.items() if allowed]
                   + [directory + "/script.window.js"]),
        )


if __name__ == "__main__":
    unittest.main()
