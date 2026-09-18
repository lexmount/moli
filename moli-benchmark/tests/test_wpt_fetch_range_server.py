from __future__ import annotations

import json
import tempfile
import unittest
import uuid
from contextlib import ExitStack
from http.client import HTTPConnection, IncompleteRead
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE_DIR = "/fetch/range/resources/"


class FetchRangeFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, resource="long-wav.py", *, method="GET", headers=(), query="", port=None):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=3)
        self.stack.callback(connection.close)
        connection.putrequest(method, RESOURCE_DIR + resource + "?" + query,
                              skip_accept_encoding=True)
        for name, value in headers:
            connection.putheader(name, value)
        connection.endheaders()
        response = connection.getresponse()
        self.stack.callback(response.close)
        return response

    def take(self, key, *, port=None):
        response = self.request("stash-take.py", query=urlencode({"key": key}), port=port)
        self.assertEqual(response.status, 200)
        self.assertEqual(response.getheader("Content-Type"), "application/json")
        self.assertIsNone(response.getheader("Access-Control-Allow-Origin"))
        self.assertIsNone(response.getheader("Cache-Control"))
        return json.loads(response.read())

    def test_closed_ranges_preserve_wav_bytes_and_response_metadata(self):
        response = self.request(headers=[("Range", "bytes=0-10"), ("Origin", "http://other.test")])
        self.assertEqual(response.status, 206)
        self.assertEqual(response.getheader("Content-Type"), "audio/wav")
        self.assertEqual(response.getheader("Content-Length"), "11")
        self.assertEqual(response.getheader("Content-Range"), "bytes 0-10/2400000")
        self.assertEqual(response.getheader("Accept-Ranges"), "bytes")
        self.assertEqual(response.getheader("Cache-Control"), "no-cache")
        self.assertEqual(response.getheader("Access-Control-Allow-Origin"), "http://other.test")
        self.assertEqual(response.read(), b"RIFF\x24\x9f\x24\0WAV")
        response = self.request(headers=[("Range", "bytes=100-103")])
        self.assertEqual(response.status, 206)
        self.assertEqual(response.read(), b"\0" * 4)

    def test_open_ranges_and_zero_end_match_upstream(self):
        for value, length, content_range in [
            ("bytes=11-", "2399989", "bytes 11-2399999/2400000"),
            ("bytes=0-0", "2400000", "bytes 0-2399999/2400000"),
            ("bytes=0000000000011-0000000000111", "101", "bytes 11-111/2400000"),
        ]:
            with self.subTest(value=value):
                response = self.request(method="HEAD", headers=[("Range", value)])
                self.assertEqual(response.status, 206)
                self.assertEqual(response.getheader("Content-Length"), length)
                self.assertEqual(response.getheader("Content-Range"), content_range)
                self.assertEqual(response.read(), b"")

    def test_invalid_units_empty_and_multiple_ranges_produce_full_response(self):
        for value in ["", "foo", "foo=0-10", "bytes=0-1, 4-5", "BYTES=0-10"]:
            with self.subTest(value=value):
                response = self.request(method="HEAD", headers=[("Range", value)])
                self.assertEqual(response.status, 200)
                self.assertEqual(response.getheader("Content-Length"), "2400000")
                self.assertIsNone(response.getheader("Content-Range"))
        response = self.request(method="HEAD", headers=[("Range", "bytes=0-1"), ("range", "4-5")])
        self.assertEqual(response.status, 200)

    def test_stash_preserves_header_bytes_duplicates_and_first_query_value(self):
        key, ignored = str(uuid.uuid4()), str(uuid.uuid4())
        self.server.fetch_stash.put(key, "separate abort namespace")
        query = urlencode({"accept-encoding-key": key, "range-received-key": ignored})
        response = self.request(method="HEAD", query=query + "&accept-encoding-key=" + ignored,
                                headers=[("Range", "bytes=0-10"), ("Accept-Encoding", "gzip"),
                                         ("accept-encoding", "\xff")])
        self.assertEqual(response.status, 206)
        self.assertEqual(self.take(key.upper().replace("-", ""), port=self.server.alternate_port), "gzip, \xff")
        self.assertIsNone(self.take(key))
        self.assertEqual(self.take(ignored), "range-header-received")
        self.assertEqual(self.server.fetch_stash.take(key), "separate abort namespace")

    def test_stash_overwrites_encoding_and_only_records_nonempty_range(self):
        key, range_key = str(uuid.uuid4()), str(uuid.uuid4())
        query = urlencode({"accept-encoding-key": key, "range-received-key": range_key})
        self.request(method="HEAD", query=query, headers=[("Range", "bytes=1-2"), ("Accept-Encoding", "gzip")])
        self.request(method="HEAD", query=query, headers=[("Range", ""), ("Accept-Encoding", "identity")])
        self.assertEqual(self.take(key), "identity")
        self.assertEqual(self.take(range_key), "range-header-received")
        self.request(method="HEAD", query=query)
        self.assertEqual(self.take(key), "")
        self.assertIsNone(self.take(range_key))

    def test_preflight_rejection_does_not_record_headers(self):
        key = str(uuid.uuid4())
        response = self.request(method="OPTIONS", query=urlencode({"accept-encoding-key": key}),
                                headers=[("Origin", "http://other.test"), ("Accept-Encoding", "gzip")])
        self.assertEqual(response.status, 404)
        self.assertEqual(response.read(), b"Preflight not accepted")
        self.assertIsNone(response.getheader("Access-Control-Allow-Origin"))
        self.assertIsNone(self.take(key))

    def test_invalid_keys_and_suffix_range_keep_upstream_errors(self):
        for resource, query, headers in [
            ("stash-take.py", "", []),
            ("stash-take.py", "key=not-a-uuid", []),
            ("long-wav.py", "accept-encoding-key=not-a-uuid", []),
            ("long-wav.py", "", [("Range", "bytes=-10")]),
        ]:
            with self.subTest(resource=resource, query=query, headers=headers):
                self.assertEqual(self.request(resource, query=query, headers=headers).status, 500)

    def test_streaming_and_shutdown_do_not_buffer_full_wav(self):
        with WptFixtureServer(self.root) as server:
            response = self.request(port=server.port)
            self.assertEqual(response.status, 200)
            self.assertEqual(response.getheader("Content-Length"), "2400000")
            prefix = response.read(8044)
            self.assertEqual(prefix[:4], b"RIFF")
            self.assertEqual(prefix[8:16], b"WAVEfmt ")
            self.assertEqual(prefix[36:40], b"data")
            self.assertEqual(prefix[44:], b"\0" * 8000)
        # Keep the client open: server shutdown must interrupt the slow stream.
        with self.assertRaises(IncompleteRead):
            response.read()

    def test_non_get_methods_use_the_same_resource_handlers(self):
        for method in ("POST", "PUT", "PATCH", "DELETE", "CUSTOM"):
            with self.subTest(method=method):
                key = str(uuid.uuid4())
                response = self.request(method=method, headers=[("Range", "bytes=0-10")],
                                        query=urlencode({"range-received-key": key}))
                self.assertEqual((response.status, response.read()), (206, b"RIFF\x24\x9f\x24\0WAV"))
                response = self.request("stash-take.py", method=method, query=urlencode({"key": key}))
                self.assertEqual(json.loads(response.read()), "range-header-received")
                self.assertIsNone(self.take(key))

    def test_case_selection_only_allows_supported_range_resource_paths(self):
        bodies = {
            "fetch/range/general.any.js": "fetch('resources/long-wav.py');fetch('resources/stash-take.py')",
            "fetch/range/absolute.any.js": "fetch('/fetch/range/resources/stash-take.py')",
            "fetch/range/dotted.any.js": "fetch('./resources/long-wav.py')",
            "fetch/range/unknown.any.js": "fetch('resources/other.py')",
            "fetch/range/suffix.any.js": "fetch('resources/long-wav.py2')",
            "fetch/range/parent.any.js": "fetch('../resources/long-wav.py')",
            "fetch/other/general.any.js": "fetch('resources/long-wav.py')",
        }
        for path, body in bodies.items():
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("// META: global=window,worker\n" + body)
        cases = enumerate_cases(self.root, dir_prefixes=("fetch",), any_js_global="both")
        self.assertEqual([case.case_path for case in cases], [
            f"fetch/range/{name}.any.js?moli-wpt-any={realm}"
            for name in ["absolute", "dotted", "general"]
            for realm in ["dedicatedworker", "window"]
        ])
