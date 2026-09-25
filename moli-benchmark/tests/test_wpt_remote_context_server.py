from __future__ import annotations

import html
import json
import tempfile
import unittest
import uuid
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


DISPATCHER = "/common/dispatcher/dispatcher.py"
EXECUTOR = "/html/browsers/browsing-the-web/remote-context-helper/resources/executor-window.py"


class RemoteContextFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        for name in (DISPATCHER, EXECUTOR):
            path = self.root / name.lstrip("/")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("# Python source must not be served")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))
        self.key = str(uuid.uuid4())
        self.query = "uuid=" + self.key

    def request(self, query=None, *, path=DISPATCHER, method="GET", body=None, headers=(), port=None):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=3)
        try:
            connection.putrequest(
                method, path + "?" + (self.query if query is None else query),
                skip_host=True, skip_accept_encoding=True,
            )
            connection.putheader("Host", "fixture.test:8000")
            for name, value in headers:
                connection.putheader(name, value)
            if body is not None:
                connection.putheader("Content-Length", str(len(body)))
            connection.endheaders(body)
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_dispatcher_keeps_raw_messages_in_fifo_order(self) -> None:
        self.assertEqual(self.request()[2], b"not ready")
        values = (b"first", b"", bytes(range(256)), b'{"result":false}', b"last")
        for value in values:
            status, headers, body = self.request(method="POST", body=value)
            self.assertEqual((status, body), (200, b"done"))
            self.assertEqual(headers["Content-Length"], "4")
        for value in values:
            status, headers, body = self.request()
            self.assertEqual((status, body), (200, value))
            self.assertEqual(headers["Content-Length"], str(len(value)))
            self.assertIsNone(headers["Content-Type"])
        self.assertEqual(self.request()[2], b"not ready")

    def test_dispatcher_cors_and_cache_presence_flags(self) -> None:
        for origin in (None, "", "null", "https://caller.test"):
            for flag in ("", "&cacheable", "&cacheable=0", "&Cacheable=1"):
                with self.subTest(origin=origin, flag=flag):
                    headers = [] if origin is None else [("Origin", origin)]
                    status, response, body = self.request(self.query + flag, headers=headers)
                    self.assertEqual((status, body), (200, b"not ready"))
                    self.assertEqual(response["Access-Control-Allow-Origin"], origin or "*")
                    self.assertEqual(response["Access-Control-Allow-Credentials"], "true")
                    self.assertEqual(response["Access-Control-Allow-Methods"], "OPTIONS, GET, POST")
                    self.assertEqual(response["Access-Control-Allow-Headers"], "Content-Type")
                    expected = "max-age=31536000" if flag.startswith("&cacheable") else "no-cache, no-store, must-revalidate"
                    self.assertEqual(response["Cache-Control"], expected)

    def test_options_does_not_require_uuid_or_touch_the_queue(self) -> None:
        self.request(method="POST", body=b"saved")
        for query in ("", "uuid=invalid", self.query + "&show-headers"):
            status, headers, body = self.request(query, method="OPTIONS")
            self.assertEqual((status, body), (200, b""))
            self.assertEqual(headers["Content-Length"], "0")
        self.assertEqual(self.request()[2], b"saved")
        self.assertEqual(self.request()[2], b"not ready")

    def test_show_headers_precedes_post_and_preserves_header_values(self) -> None:
        for method in ("GET", "HEAD", "POST", "PUT", "chicken"):
            with self.subTest(method=method):
                self.request(method="POST", body=b"first")
                status, _, body = self.request(
                    self.query + "&show-headers=0", method=method, body=b"ignored",
                    headers=[("Cookie", "one=1; two=2"), ("X-Value", "first"),
                             ("x-value", "second"), ("X-Bytes", "\xff")],
                )
                self.assertEqual((status, body), (200, b""))
                self.assertEqual(self.request()[2], b"first")
                saved = json.loads(self.request()[2])
                self.assertEqual(saved, {
                    "host": "fixture.test:8000", "cookie": "one=1; two=2",
                    "x-value": "first, second", "x-bytes": "\xff", "content-length": "7",
                })
                self.assertEqual(self.request()[2], b"not ready")

    def test_other_methods_take_and_head_consumes_without_a_body(self) -> None:
        for method in ("HEAD", "PUT", "PATCH", "DELETE", "YO", "chicken", "post"):
            with self.subTest(method=method):
                self.request(method="POST", body=b"message")
                status, headers, body = self.request(method=method, body=b"unused")
                self.assertEqual(status, 200)
                self.assertEqual(headers["Content-Length"], "7")
                self.assertEqual(body, b"" if method == "HEAD" else b"message")
                self.assertEqual(self.request()[2], b"not ready")

    def test_uuid_normalization_first_value_and_invalid_keys(self) -> None:
        for query in (
            "uuid=" + self.key.upper(), "uuid=" + self.key.replace("-", ""),
            urlencode({"uuid": "{" + self.key + "}"}), "u%75id=" + self.key + "&uuid=bad",
        ):
            self.assertEqual(self.request(query, method="POST", body=b"message")[0], 200)
            self.assertEqual(self.request()[2], b"message")
        for query in ("", "uuid=", "uuid=bad", "uuid=%FF", "uuid=&" + self.query, "UUID=" + self.key):
            for method in ("GET", "POST"):
                self.assertEqual(self.request(query, method=method, body=b"unused")[0], 500)

    def test_queues_share_origins_and_explicit_namespace_but_not_servers(self) -> None:
        other = self.stack.enter_context(WptFixtureServer(self.root))
        self.server.fetch_stash.put(self.key, b"fetch")
        self.server.fetch_stash.put(self.key, b"other-path", path=DISPATCHER)
        self.request(method="POST", path=DISPATCHER.replace("dispatcher.py", "%64ispatcher.py"), body=b"shared")
        self.assertEqual(self.request(port=other.port)[2], b"not ready")
        self.assertEqual(self.request(port=self.server.alternate_port)[2], b"shared")
        self.assertEqual(self.server.fetch_stash.take(self.key), b"fetch")
        self.assertEqual(self.server.fetch_stash.take(self.key, path=DISPATCHER), b"other-path")
        self.assertEqual(self.server.fetch_stash.take(self.key, path="/common/dispatcher"), [])

    def test_concurrent_producers_and_consumers_do_not_lose_or_duplicate_messages(self) -> None:
        values = [str(index).encode("ascii") for index in range(16)]
        with ThreadPoolExecutor(max_workers=32) as pool:
            posted = list(pool.map(lambda value: self.request(method="POST", body=value), values))
            received = list(pool.map(
                lambda index: self.request(port=self.server.port if index % 2 else self.server.alternate_port),
                range(len(values)),
            ))
        self.assertTrue(all(status == 200 and body == b"done" for status, _, body in posted))
        self.assertTrue(all(status == 200 for status, _, _ in received))
        self.assertEqual(sorted(body for _, _, body in received), sorted(values))
        self.assertEqual(self.request()[2], b"not ready")

    def test_unused_uploads_do_not_block_either_handler(self) -> None:
        for path, method, query in (
            (DISPATCHER, "OPTIONS", ""), (DISPATCHER, "POST", self.query + "&show-headers"),
            (DISPATCHER, "PUT", self.query), (EXECUTOR, "POST", self.query),
            (DISPATCHER, "POST", "uuid=invalid"),
        ):
            connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
            try:
                connection.putrequest(method, path + "?" + query)
                connection.putheader("Content-Length", "100")
                connection.endheaders()
                response = connection.getresponse()
                self.assertEqual(response.status, 500 if query == "uuid=invalid" else 200)
                response.read()
            finally:
                connection.close()

    def test_executor_generates_scripts_base_url_and_request_header_initialization(self) -> None:
        query = urlencode([
            ("uuid", "executor-id"), ("script", "/one.js?x='a'&y=<b>"),
            ("script", "/two.js"), ("startOn", "pageshow"),
        ])
        status, headers, body = self.request(
            query, path=EXECUTOR,
            headers=[("X-Value", 'first"'), ("x-value", "second"), ("X-Utf8", "caf\xc3\xa9")],
        )
        text = body.decode("utf-8")
        self.assertEqual(status, 200)
        self.assertEqual(headers["Content-Type"], "text/html")
        self.assertEqual(headers["Content-Length"], str(len(body)))
        self.assertIsNone(headers["Cache-Control"])
        self.assertIsNone(headers["Access-Control-Allow-Origin"])
        self.assertIn('<base href="' + html.escape("http://fixture.test:8000" + EXECUTOR + "?" + query) + '">', text)
        self.assertIn('<script src="/common/dispatcher/dispatcher.js"></script>', text)
        self.assertIn('<script src="./executor-common.js"></script>', text)
        self.assertIn('<script src="./executor-window.js"></script>', text)
        self.assertLess(text.index("/one.js"), text.index("/two.js"))
        self.assertIn("<script src='" + html.escape("/one.js?x='a'&y=<b>") + "'></script>", text)
        self.assertIn('window.__requestHeaders.append("x-value", "first\\\"");', text)
        self.assertIn('window.__requestHeaders.append("x-value", "second");', text)
        self.assertIn('window.__requestHeaders.append("x-utf8", "caf\\u00e9");', text)
        self.assertIn('requestExecutor("executor-id", \'pageshow\');', text)

    def test_executor_query_defaults_first_nonempty_values_and_errors(self) -> None:
        status, _, body = self.request(
            "uuid=&uuid=chosen&uuid=ignored&script=&startOn=&startOn=load", path=EXECUTOR,
        )
        self.assertEqual(status, 200)
        self.assertIn(b'requestExecutor("chosen", \'load\');', body)
        self.assertIn(b'requestExecutor("not-a-uuid", null);', self.request("uuid=not-a-uuid", path=EXECUTOR)[2])
        for query in ("", "uuid=", "UUID=wrong", self.query + "&status=", self.query + "&status=bad"):
            self.assertEqual(self.request(query, path=EXECUTOR)[0], 500)
        self.assertEqual(self.request(path=EXECUTOR, headers=[("X-Bytes", "\xff")])[0], 500)
        self.assertEqual(self.request(self.query + "&status=201&status=404", path=EXECUTOR)[0], 201)

    def test_executor_supports_all_methods_and_head_omits_the_generated_body(self) -> None:
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "YO", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(path=EXECUTOR, method=method, body=b"unused")
                self.assertEqual(status, 200)
                self.assertEqual(headers["Content-Type"], "text/html")
                if method == "HEAD":
                    self.assertEqual(body, b"")
                    self.assertGreater(int(headers["Content-Length"]), 0)
                else:
                    self.assertIn(b"requestExecutor(", body)

    def test_pipes_apply_after_queue_side_effects_and_executor_status(self) -> None:
        query = urlencode({"uuid": self.key, "pipe": "status(202)|header(Cache-Control,private)|header(X-Result,yes)"})
        status, headers, body = self.request(query, method="POST", body=b"{{host}}")
        self.assertEqual((status, body), (202, b"done"))
        self.assertEqual(headers["Cache-Control"], "private")
        self.assertEqual(headers["X-Result"], "yes")
        status, _, body = self.request(urlencode({"uuid": self.key, "pipe": "sub(none)"}))
        self.assertEqual((status, body), (200, b"localhost"))
        self.assertEqual(self.request()[2], b"not ready")
        status, headers, body = self.request(query + "&status=404", path=EXECUTOR)
        self.assertEqual(status, 202)
        self.assertEqual(headers["Cache-Control"], "private")
        self.assertIn(b"requestExecutor(", body)
        invalid = urlencode({"uuid": self.key, "pipe": "unknown()"})
        self.assertEqual(self.request(invalid, method="POST", body=b"retained")[0], 500)
        self.assertEqual(self.request()[2], b"retained")

    def test_discovery_and_routing_only_allow_the_two_supported_paths(self) -> None:
        directory = "fixture-tests"
        cases = {
            "dispatcher.html": (DISPATCHER, True),
            "executor.html": (EXECUTOR, True),
            "relative.html": ("../common/dispatcher/dispatcher.py", True),
            "dot-relative.html": ("./../common/dispatcher/dispatcher.py", True),
            "wrong-relative.html": ("dispatcher.py", False),
            "wrong-root.html": ("/dispatcher.py", False),
            "wrong-case.html": (DISPATCHER.replace("common", "COMMON"), False),
            "suffix.html": (EXECUTOR + ".extra", False),
            "unsupported.html": ("/common/dispatcher/other.py", False),
        }
        for name, (reference, allowed) in cases.items():
            path = self.root / directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                '<script src="/resources/testharness.js"></script>'
                f'<script>fetch("{reference}?uuid={self.key}")</script>'
            )
            if not allowed and reference.startswith("/"):
                self.assertEqual(self.request(path=reference)[0], 404)
        (self.root / directory / "unknown.html").write_text(
            '<script src="/resources/testharness.js"></script>'
            f'<script>fetch("{DISPATCHER}"); fetch("/unsupported.py")</script>'
        )
        self.assertEqual(
            sorted(case.case_path for case in enumerate_cases(self.root, dir_prefixes=(directory,))),
            sorted(directory + "/" + name for name, (_, allowed) in cases.items() if allowed),
        )


if __name__ == "__main__":
    unittest.main()
