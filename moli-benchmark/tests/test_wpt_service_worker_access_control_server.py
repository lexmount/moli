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


RESOURCE = "/service-workers/service-worker/resources/fetch-access-control.py"


class ServiceWorkerAccessControlFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        stack = ExitStack()
        self.addCleanup(stack.close)
        root = Path(stack.enter_context(tempfile.TemporaryDirectory()))
        (root / "resources").mkdir()
        (root / "resources/testharness.js").write_text("// testharness")
        (root / "media").mkdir()
        (root / "media/movie_5.webm").write_bytes(b"01234567")
        source = root / RESOURCE.lstrip("/")
        source.parent.mkdir(parents=True)
        source.write_text("# The fixture must be handled dynamically")
        stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = stack.enter_context(WptFixtureServer(root))

    def request(
        self, method: str = "GET", query: str = "", *,
        headers: dict[str, str] | None = None, body: bytes | None = None,
    ):
        with ExitStack() as stack:
            connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
            stack.callback(connection.close)
            connection.request(method, f"{RESOURCE}?{query}", body, headers or {})
            response = connection.getresponse()
            return response.status, response.headers, response.read()

    def test_cors_headers_follow_the_query_including_auth_challenges(self) -> None:
        status, headers, _ = self.request(headers={"Origin": "https://caller.test"})
        self.assertEqual(status, 200)
        self.assertIsNone(headers["Access-Control-Allow-Origin"])
        self.assertEqual(headers["X-ServiceWorker-ServerHeader"], "SetInTheServer")

        query = urlencode({
            "ACAOrigin": "https://caller.test,https://second.test",
            "ACACredentials": "true",
            "ACAMethods": "GET, PUT",
            "ACAHeaders": "x-test",
            "ACEHeaders": "X-ServiceWorker-ServerHeader",
        })
        status, headers, _ = self.request("OPTIONS", query)
        self.assertEqual(status, 200)
        self.assertEqual(headers.get_all("Access-Control-Allow-Origin"), [
            "https://caller.test", "https://second.test",
        ])
        self.assertEqual(headers["Access-Control-Allow-Credentials"], "true")
        self.assertEqual(headers["Access-Control-Allow-Methods"], "GET, PUT")
        self.assertEqual(headers["Access-Control-Allow-Headers"], "x-test")
        self.assertEqual(headers["Access-Control-Expose-Headers"], "X-ServiceWorker-ServerHeader")

        status, headers, body = self.request(query="Auth&ACAOrigin=https://caller.test")
        self.assertEqual(status, 401)
        self.assertEqual(headers["Access-Control-Allow-Origin"], "https://caller.test")
        self.assertEqual(headers["WWW-Authenticate"], 'Basic realm="Restricted"')
        self.assertEqual(body, b"Authentication canceled")

    def test_authorized_upload_reports_the_request(self) -> None:
        raw_body = (
            b'--boundary\r\nContent-Disposition: form-data; name="a"\r\n\r\n\xff\r\n'
            b'--boundary\r\nContent-Disposition: form-data; name="b"; filename="test.bin"\r\n'
            b'Content-Type: application/octet-stream\r\n\r\n\x00\xff\r\n--boundary--\r\n'
        )
        status, _, body = self.request("POST", "Auth&key=%FF&key=second", headers={
            "Authorization": "Basic dXNlcjpwYXNz",
            "Cookie": "cookie=fixture",
            "Content-Type": "multipart/form-data; boundary=boundary",
        }, body=raw_body)
        self.assertEqual(status, 200)
        self.assertTrue(body.startswith(b"report( ") and body.endswith(b" )"))
        data = json.loads(body.removeprefix(b"report( ").removesuffix(b" )"))
        self.assertEqual(data["method"], "POST")
        self.assertEqual(data["GET"], {"Auth": "", "key": "ÿ"})
        self.assertEqual(data["POST"], {"a": "ÿ"})
        self.assertEqual(data["body"], raw_body.decode("latin-1"))
        self.assertEqual(data["files"], [{
            "key": "b", "name": "test.bin", "type": "application/octet-stream",
            "error": 0, "size": 2, "content": "\x00ÿ",
        }])
        self.assertEqual((data["username"], data["password"], data["cookie"]), ("user", "pass", "fixture"))

    def test_partial_video_returns_the_fixture_range(self) -> None:
        query = "VIDEO&PartialContent"
        for value, expected_range, expected_body in (
            ("0-", "0-0", b"0"),
            ("1-3", "1-3", b"123"),
            ("4-", "4-7", b"4567"),
        ):
            with self.subTest(range=value):
                status, headers, body = self.request(query=query, headers={"Range": f"bytes={value}"})
                self.assertEqual(status, 206)
                self.assertEqual(headers["Content-Type"], "video/webm")
                self.assertEqual(headers["Content-Range"], f"bytes {expected_range}/8")
                self.assertEqual(body, expected_body)
