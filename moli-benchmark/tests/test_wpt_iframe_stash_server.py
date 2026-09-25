from __future__ import annotations

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


STASH_PATH = "/html/semantics/embedded-content/the-iframe-element/stash.py"


class IframeStashFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        handler = self.root / STASH_PATH.lstrip("/")
        handler.parent.mkdir(parents=True)
        handler.write_text("# Python source must not be served")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))
        self.key = str(uuid.uuid4())
        self.query = "id=" + self.key

    def request(self, query=None, *, method="GET", path=STASH_PATH, port=None, body=None):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=2)
        try:
            connection.request(
                method, path + "?" + (self.query if query is None else query), body,
                {"Origin": "http://caller.test"},
            )
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_polling_and_take_preserve_raw_body_bytes(self) -> None:
        for value in (b"OK", bytes(range(256)), b'{"result":false}', b"", b"{{host}}"):
            with self.subTest(value=value):
                status, headers, body = self.request()
                self.assertEqual((status, body), (200, b""))
                self.assertIsNone(headers["Content-Length"])
                status, headers, body = self.request(method="POST", body=value)
                self.assertEqual((status, body), (200, b""))
                self.assertEqual(headers["Content-Length"], "0")
                status, headers, body = self.request()
                self.assertEqual((status, body), (200, value))
                self.assertEqual(headers["Content-Length"], str(len(value)))
                for name in ("Content-Type", "Cache-Control", "Access-Control-Allow-Origin"):
                    self.assertIsNone(headers[name])
                self.assertEqual(self.request()[2], b"")

    def test_only_exact_post_writes_and_all_other_methods_take(self) -> None:
        for method in ("GET", "HEAD", "PUT", "PATCH", "DELETE", "OPTIONS", "YO", "chicken", "post"):
            with self.subTest(method=method):
                self.assertEqual(self.request(method="POST", body=b"stored")[0], 200)
                status, headers, body = self.request(method=method, body=b"unused")
                self.assertEqual(status, 200)
                self.assertEqual(headers["Content-Length"], "6")
                self.assertEqual(body, b"" if method == "HEAD" else b"stored")
                self.assertEqual(self.request()[2], b"")

    def test_uuid_normalization_and_first_decoded_query_value(self) -> None:
        queries = (
            "id=" + self.key.upper(),
            urlencode({"id": "{" + self.key + "}"}),
            "id=" + self.key.replace("-", ""),
            "id=urn:uuid:" + self.key,
            "i%64=" + self.key + "&id=invalid",
        )
        for query in queries:
            with self.subTest(query=query):
                self.assertEqual(self.request(query, method="POST", body=b"result")[0], 200)
                self.assertEqual(self.request()[2], b"result")
        for query in ("", "ID=" + self.key, "id=", "id=invalid", "id=%FF", "id=&" + self.query):
            for method in ("GET", "HEAD", "POST"):
                with self.subTest(query=query, method=method):
                    self.assertEqual(self.request(query, method=method, body=b"unused")[0], 500)

    def test_duplicate_posts_fail_without_overwriting_and_consumers_take_once(self) -> None:
        self.assertEqual(self.request(method="POST", body=b"first")[0], 200)
        self.assertEqual(self.request(method="POST", body=b"second")[0], 500)
        with ThreadPoolExecutor(max_workers=32) as pool:
            responses = list(pool.map(lambda _: self.request(), range(16)))
        self.assertTrue(all(status == 200 for status, _, _ in responses))
        self.assertEqual([body for _, _, body in responses if body], [b"first"])
        self.assertEqual(self.request(method="POST", body=b"reused")[0], 200)
        self.assertEqual(self.request()[2], b"reused")

    def test_origins_share_results_but_paths_and_servers_do_not(self) -> None:
        other = self.stack.enter_context(WptFixtureServer(self.root))
        sibling = STASH_PATH.rsplit("/", 1)[0] + "/other.py"
        self.server.fetch_stash.put(self.key, b"fetch")
        self.server.fetch_stash.put(self.key, b"sibling", path=sibling)
        self.assertEqual(self.request(method="POST", body=b"shared")[0], 200)
        self.assertEqual(self.request(port=other.port)[2], b"")
        self.assertEqual(self.request(port=self.server.alternate_port)[2], b"shared")
        self.assertEqual(self.request()[2], b"")
        self.assertEqual(self.server.fetch_stash.take(self.key), b"fetch")
        self.assertEqual(self.server.fetch_stash.take(self.key, path=sibling), b"sibling")
        encoded_path = STASH_PATH.replace("stash.py", "%73tash.py")
        self.assertEqual(self.request(method="POST", path=encoded_path, body=b"encoded")[0], 200)
        self.assertEqual(self.request()[2], b"")
        self.assertEqual(self.request(path=encoded_path)[2], b"encoded")

    def test_unused_uploads_do_not_delay_taking_the_result(self) -> None:
        for method in ("GET", "HEAD", "PUT", "OPTIONS", "chicken"):
            with self.subTest(method=method):
                self.request(method="POST", body=b"ready")
                connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
                try:
                    connection.putrequest(method, STASH_PATH + "?" + self.query)
                    connection.putheader("Content-Length", "100")
                    connection.endheaders()
                    response = connection.getresponse()
                    self.assertEqual(response.status, 200)
                    self.assertEqual(response.read(), b"" if method == "HEAD" else b"ready")
                finally:
                    connection.close()
                self.assertEqual(self.request()[2], b"")

    def test_post_uses_content_length_without_chunk_decoding(self) -> None:
        for length, value in ((None, b""), ("3", b"abc")):
            with self.subTest(length=length):
                connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
                try:
                    connection.putrequest("POST", STASH_PATH + "?" + self.query)
                    connection.putheader("Transfer-Encoding", "chunked")
                    if length is not None:
                        connection.putheader("Content-Length", length)
                    connection.endheaders(value)
                    response = connection.getresponse()
                    self.assertEqual(response.status, 200)
                    self.assertEqual(response.read(), b"")
                finally:
                    connection.close()
                status, headers, body = self.request()
                self.assertEqual((status, body), (200, value))
                self.assertEqual(headers["Content-Length"], str(len(value)))

    def test_pipes_distinguish_empty_content_from_a_missing_result(self) -> None:
        query = urlencode({"id": self.key, "pipe": "status(201)|header(X-Result,present)"})
        status, headers, body = self.request(query)
        self.assertEqual((status, body), (200, b""))
        self.assertIsNone(headers["X-Result"])
        self.assertIsNone(headers["Content-Length"])
        for method in ("POST", "GET"):
            status, headers, body = self.request(query, method=method, body=b"")
            self.assertEqual((status, body), (201, b""))
            self.assertEqual(headers["X-Result"], "present")
            self.assertEqual(headers["Content-Length"], "0")
        invalid = urlencode({"id": self.key, "pipe": "unknown()"})
        self.assertEqual(self.request(invalid)[0], 200)
        # A pipe failure happens after the handler has already written/taken.
        self.assertEqual(self.request(invalid, method="POST", body=b"saved")[0], 500)
        self.assertEqual(self.request(invalid)[0], 500)
        self.assertEqual(self.request(invalid)[0], 200)

    def test_response_pipes_transform_taken_content_in_order(self) -> None:
        self.request(method="POST", body=b"{{host}}")
        query = urlencode({
            "id": self.key,
            "pipe": "sub(none)|status(202)|header(Content-Type,text/plain)|header(X-Result,first)"
                    "|header(X-Result,second,true)|trickle(d0.01)",
        })
        with patch("moli_benchmark.wpt_cross.server.time.sleep") as sleep:
            status, headers, body = self.request(query)
        self.assertEqual((status, body), (202, b"localhost"))
        self.assertEqual(headers["Content-Type"], "text/plain")
        self.assertEqual(headers.get_all("X-Result"), ["first", "second"])
        self.assertIsNone(headers["Content-Length"])
        self.assertEqual(headers["Cache-Control"], "no-cache, no-store, must-revalidate")
        sleep.assert_called_once_with(0.01)
        self.assertEqual(self.request()[2], b"")

    def test_unrelated_paths_are_not_stash_endpoints(self) -> None:
        for path in ("/stash.py", STASH_PATH + "2", "/other" + STASH_PATH, STASH_PATH.upper()):
            with self.subTest(path=path):
                self.assertEqual(self.request(path=path)[0], 404)

    def test_discovery_only_accepts_references_to_the_supported_endpoint(self) -> None:
        directory = STASH_PATH.lstrip("/").rsplit("/", 1)[0]
        cases = {
            "absolute.html": (STASH_PATH, True),
            "relative.html": ("stash.py", True),
            "dot-relative.html": ("./stash.py", True),
            "nested/relative.html": ("../stash.py", True),
            "nested/wrong-relative.html": ("stash.py", False),
            "wrong-root.html": ("/stash.py", False),
            "wrong-case.html": ("Stash.py", False),
            "suffix.html": ("stash.py.extra", False),
            "wrong-sibling.html": ("support/download_stash.py", False),
        }
        for name, (reference, _) in cases.items():
            path = self.root / directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                '<script src="/resources/testharness.js"></script>'
                f'<script>fetch("{reference}?id={self.key}")</script>'
            )
        (self.root / directory / "unknown.html").write_text(
            '<script src="/resources/testharness.js"></script>'
            '<script>fetch("stash.py"); fetch("/unsupported.py")</script>'
        )
        (self.root / directory / "script.window.js").write_text(
            'fetch(`stash.py?id=${key}`);'
        )
        discovered = enumerate_cases(self.root, dir_prefixes=(directory,))
        self.assertEqual(
            sorted(case.case_path.split("?")[0] for case in discovered),
            sorted([directory + "/" + name for name, (_, allowed) in cases.items() if allowed]
                   + [directory + "/script.window.js"]),
        )


if __name__ == "__main__":
    unittest.main()
