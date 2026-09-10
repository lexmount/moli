from __future__ import annotations

import json
import tempfile
import threading
import time
import unittest
import uuid
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from http.client import HTTPConnection, HTTPResponse
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE_DIR = "/fetch/api/resources/"


class FetchAbortFixtureTests(unittest.TestCase):
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
        self, port: int, resource: str, query: str = "", *, method: str = "GET"
    ) -> tuple[int, dict[str, str], bytes]:
        connection = HTTPConnection("127.0.0.1", port, timeout=3)
        try:
            connection.request(method, RESOURCE_DIR + resource + "?" + query)
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def take(self, port: int, key: str) -> str | None:
        status, headers, body = self.request(port, "stash-take.py", urlencode({"key": key}))
        self.assertEqual(status, 200)
        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
        return json.loads(body)

    def put(self, port: int, key: str, value: str) -> None:
        status, _, body = self.request(
            port, "stash-put.py", urlencode({"key": key, "value": value})
        )
        self.assertEqual((status, body), (200, b"done"))

    def stream(self, port: int, **params: str) -> HTTPResponse:
        connection = HTTPConnection("127.0.0.1", port, timeout=3)
        self.stack.callback(connection.close)
        connection.request("GET", RESOURCE_DIR + "infinite-slow-response.py?" + urlencode(params))
        response = connection.getresponse()
        self.stack.callback(response.close)
        self.assertEqual(response.status, 200)
        self.assertEqual(response.headers["Content-Type"], "text/plain")
        for header in ("Content-Length", "Transfer-Encoding", "Access-Control-Allow-Origin"):
            self.assertIsNone(response.headers.get(header))
        self.assertEqual(response.read(2048), b"." * 2048)
        return response

    def wait_closed(self, port: int, key: str) -> None:
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            value = self.take(port, key)
            if value == "closed":
                return
            self.assertIsNone(value)
            time.sleep(0.01)
        self.fail("stream did not record its connection closing")

    def test_stash_preserves_bytes_first_query_values_and_uuid_equivalence(self) -> None:
        server = self.server()
        key, unused = str(uuid.uuid4()), str(uuid.uuid4())
        self.assertIsNone(self.take(server.port, key))
        status, headers, body = self.request(
            server.port, "stash-put.py",
            f"key={key}&key={unused}&value=%FF%00+%2B&value=ignored",
        )
        self.assertEqual((status, body), (200, b"done"))
        self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
        self.assertNotIn("Content-Type", headers)
        self.assertEqual(self.take(server.alternate_port, key.upper().replace("-", "")), "\xff\0 +")
        self.assertIsNone(self.take(server.port, key))
        self.assertIsNone(self.take(server.port, unused))
        self.put(server.alternate_port, key, "")
        self.assertEqual(self.take(server.port, key), "")
        self.assertIsNone(self.take(server.port, key))

    def test_stash_rejects_invalid_keys_missing_parameters_and_overwrites(self) -> None:
        server = self.server()
        key = str(uuid.uuid4())
        self.put(server.port, key, "original")
        for resource, query in [
            ("stash-put.py", f"key={key}&value=overwrite"),
            ("stash-put.py", f"key={key}"),
            ("stash-put.py", "value=missing-key"),
            ("stash-take.py", ""),
            ("stash-take.py", "key=not-a-uuid"),
            ("stash-put.py", "key=%FF&value=x"),
            ("infinite-slow-response.py", f"stateKey={key}&abortKey=invalid"),
        ]:
            with self.subTest(resource=resource, query=query):
                self.assertEqual(self.request(server.port, resource, query)[0], 500)
        self.assertEqual(self.take(server.port, key), "original")

    def test_stash_take_is_atomic_across_origins_and_isolated_between_servers(self) -> None:
        server, other = self.server(), self.server()
        key = str(uuid.uuid4())
        self.put(server.port, key, "only once")
        self.assertIsNone(self.take(other.port, key))
        barrier = threading.Barrier(4)

        def take(port: int) -> str | None:
            barrier.wait(timeout=3)
            return self.take(port, key)

        with ThreadPoolExecutor(max_workers=4) as pool:
            values = list(pool.map(take, [server.port, server.alternate_port] * 2))
        self.assertEqual(values.count("only once"), 1)
        self.assertEqual(values.count(None), 3)

    def test_stash_methods_and_preflight(self) -> None:
        server = self.server()
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "YO"):
            with self.subTest(method=method):
                key = str(uuid.uuid4())
                status, _, body = self.request(
                    server.port, "stash-put.py", f"key={key}&value=stored", method=method
                )
                self.assertEqual(status, 200)
                self.assertEqual(body, b"" if method == "HEAD" else b"done")
                self.assertEqual(self.take(server.port, key), "stored")
                self.put(server.port, key, "taken")
                status, _, body = self.request(
                    server.port, "stash-take.py", f"key={key}", method=method
                )
                self.assertEqual((status, body), (200, b"" if method == "HEAD" else b'"taken"'))
                self.assertIsNone(self.take(server.port, key))
        key = str(uuid.uuid4())
        status, headers, body = self.request(
            server.port, "stash-put.py", f"key={key}&value=not-written", method="OPTIONS"
        )
        self.assertEqual((status, body), (200, b"done"))
        for name in ("Origin", "Methods", "Headers"):
            self.assertEqual(headers["Access-Control-Allow-" + name], "*")
        self.assertIsNone(self.take(server.port, key))

    def test_stash_put_respects_upstream_cross_origin_restrictions(self) -> None:
        server = self.server()
        key = str(uuid.uuid4())
        base = {"key": key, "value": "stored", "disallow_cross_origin": ""}
        for mode, frame_origin, expected in [
            ("no-cors", "http://other.test", "stored"),
            ("cors", "http://other.test", None),
            ("cors", f"http://127.0.0.1:{server.port}", "stored"),
        ]:
            with self.subTest(mode=mode, frame_origin=frame_origin):
                status, headers, body = self.request(server.port, "stash-put.py", urlencode({
                    **base, "mode": mode, "frame_origin ": "", "frame_origin": frame_origin,
                }))
                self.assertEqual(status, 200)
                self.assertNotIn("Access-Control-Allow-Origin", headers)
                self.assertEqual(body, b"done" if expected else b"not stashing for cors request")
                self.assertEqual(self.take(server.port, key), expected)
        # The checked-in WPT handler also rejects a missing trailing-space guard.
        self.assertEqual(self.request(server.port, "stash-put.py", urlencode({
            **base, "mode": "cors", "frame_origin": f"http://127.0.0.1:{server.port}",
        }))[0], 500)
        self.assertIsNone(self.take(server.port, key))

    def test_stream_stays_open_until_a_truthy_abort_key_is_taken(self) -> None:
        server = self.server()
        state_key, abort_key = str(uuid.uuid4()), str(uuid.uuid4())
        self.put(server.port, state_key, "stale")
        response = self.stream(server.port, stateKey=state_key, abortKey=abort_key)
        self.assertEqual(self.take(server.alternate_port, state_key), "open")
        self.assertIsNone(self.take(server.port, state_key))
        self.put(server.alternate_port, abort_key, "")
        # Receiving later bytes proves the connection remains live after an
        # empty cleanup value, without buffering the response until EOF.
        self.assertEqual(response.read(32), b"." * 32)
        self.assertIsNone(self.take(server.port, state_key))
        self.put(server.alternate_port, abort_key, "close")
        self.assertEqual(response.read().strip(b"."), b"")
        self.wait_closed(server.port, state_key)
        self.assertIsNone(self.take(server.port, abort_key))

    def test_client_disconnect_closes_only_its_stream(self) -> None:
        server = self.server()
        first_key, second_key, abort_key = (str(uuid.uuid4()) for _ in range(3))
        first = self.stream(server.port, stateKey=first_key)
        second = self.stream(server.alternate_port, stateKey=second_key, abortKey=abort_key)
        self.assertEqual(self.take(server.port, first_key), "open")
        first.close()
        self.wait_closed(server.alternate_port, first_key)
        self.assertEqual(self.take(server.port, second_key), "open")
        self.assertEqual(second.read(8), b"." * 8)
        self.put(server.port, abort_key, "close")
        second.read()
        self.wait_closed(server.port, second_key)

    def test_shutdown_ends_streams_without_client_disconnect_or_cleanup_key(self) -> None:
        key = str(uuid.uuid4())
        with WptFixtureServer(self.root) as server:
            response = self.stream(server.port, stateKey=key)
            self.assertEqual(self.take(server.port, key), "open")
        self.assertEqual(response.read().strip(b"."), b"")
        self.assertEqual(server.fetch_stash.take(key), "closed")

    def test_case_selection_allows_only_supported_abort_resource_paths(self) -> None:
        cases = {
            "fetch/api/abort/general.any.js": "\n".join(
                f"fetch('../resources/{name}.py?key=${{key}}');"
                for name in ("stash-put", "stash-take", "infinite-slow-response")
            ),
            "fetch/api/abort/absolute.any.js": "fetch('/fetch/api/resources/stash-take.py');",
            "fetch/api/abort/unsupported.any.js": "fetch('../resources/other.py');",
            "fetch/api/abort/suffix.any.js": "fetch('../resources/stash-take.py2');",
            "fetch/api/abort/prefix.any.js": "fetch('/other/fetch/api/resources/stash-take.py');",
            "fetch/api/abort/parent.any.js": "fetch('../../resources/stash-take.py');",
            "fetch/api/other/general.any.js": "fetch('../resources/stash-take.py');",
        }
        for path, body in cases.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("// META: global=window,worker\n" + body)
        selected = enumerate_cases(self.root, dir_prefixes=("fetch",), any_js_global="both")
        self.assertEqual([case.case_path for case in selected], [
            f"fetch/api/abort/{name}.any.js?moli-wpt-any={realm}"
            for name in ("absolute", "general")
            for realm in ("dedicatedworker", "window")
        ])
