import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from threading import Barrier, Event
from unittest.mock import patch
from uuid import uuid4

from moli_benchmark.wpt_cross.case_set import IMAGE_AND_STASH_PATH, enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


class ImageStashFixtureTests(unittest.TestCase):
    def setUp(self):
        stack = ExitStack()
        self.addCleanup(stack.close)
        self.root = Path(stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        fixture = self.root / IMAGE_AND_STASH_PATH.lstrip("/")
        fixture.parent.mkdir(parents=True)
        fixture.write_text("raise AssertionError('do not execute the Python handler')")
        self.image = b"\x89PNG\r\n\x1a\nimage fixture"
        fixture.with_name("image.png").write_bytes(self.image)
        stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = stack.enter_context(WptFixtureServer(self.root))
        self.key = str(uuid4())

    def request(self, query, *, path=IMAGE_AND_STASH_PATH, method="GET", alternate=False):
        port = self.server.alternate_port if alternate else self.server.port
        connection = HTTPConnection("127.0.0.1", port, timeout=4)
        try:
            connection.request(method, f"{path}?{query}")
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def assert_count(self, expected, *, key=None, alternate=False):
        status, headers, body = self.request(
            f"read={key or self.key}", alternate=alternate,
        )
        self.assertEqual((status, body), (200, str(expected).encode("ascii")))
        self.assertIsNone(headers["Content-Type"])
        self.assertIsNone(headers["Cache-Control"])

    def test_real_image_requests_and_non_destructive_reads(self):
        other = str(uuid4())
        self.assert_count(0)
        status, headers, body = self.request(f"increment={self.key}&increment={other}&read={other}")
        self.assertEqual((status, body), (200, self.image))
        self.assertEqual(headers["Content-Type"], "image/png")
        self.assertIsNone(headers["Cache-Control"])
        self.assert_count(1, alternate=True)
        self.assert_count(1)
        self.assert_count(0, key=other)

    def test_query_parameters_apply_to_all_methods(self):
        methods = ("HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "YO", "chicken")
        for method in methods:
            with self.subTest(method=method):
                status, headers, body = self.request(f"increment={self.key}", method=method)
                self.assertEqual((status, body), (200, b"" if method == "HEAD" else self.image))
                self.assertEqual(headers["Content-Length"], str(len(self.image)))
        self.assertEqual(self.request(f"read={self.key}", method="HEAD")[2], b"")
        self.assert_count(len(methods))
        status, _, body = self.request("unsupported=1")
        self.assertEqual((status, body), (404, b"Not found"))
        self.assertEqual(self.request("increment=&read=")[0], 500)
        self.assert_count(len(methods))

    def test_trickle_keeps_multiple_requests_in_flight(self):
        entered, released = Barrier(3), Event()
        delays = []

        def hold_body(seconds):
            delays.append(seconds)
            entered.wait(timeout=3)
            released.wait(timeout=3)

        query = f"increment={self.key}&pipe=trickle(d1)"
        with patch("moli_benchmark.wpt_cross.server.time.sleep", side_effect=hold_body):
            with ThreadPoolExecutor(max_workers=2) as pool:
                pending = [
                    pool.submit(self.request, query, alternate=alternate)
                    for alternate in (False, True)
                ]
                try:
                    entered.wait(timeout=3)
                    self.assert_count(2)
                finally:
                    released.set()
                for request in pending:
                    status, _, body = request.result()
                    self.assertEqual((status, body), (200, self.image))
        self.assertEqual(delays, [1.0, 1.0])

    def test_discovery_only_accepts_references_to_the_fixture(self):
        directory = Path(IMAGE_AND_STASH_PATH.lstrip("/")).parent.parent
        references = {
            "absolute": IMAGE_AND_STASH_PATH,
            "relative": "resources/image-and-stash.py",
            "dot-relative": "./resources/image-and-stash.py",
            "unrecognized": "/other" + IMAGE_AND_STASH_PATH,
            "suffix": IMAGE_AND_STASH_PATH + ".other",
        }
        for name, reference in references.items():
            (self.root / directory / f"{name}.html").write_text(
                '<script src="/resources/testharness.js"></script>'
                f'<script>fetch("{reference}?read=key")</script>'
            )
        (self.root / directory / "mixed.html").write_text(
            '<script src="/resources/testharness.js"></script>'
            f'<script>fetch("{IMAGE_AND_STASH_PATH}?read=key"); fetch("/unsupported.py")</script>'
        )
        cases = enumerate_cases(self.root, dir_prefixes=(directory.as_posix(),))
        self.assertEqual(
            sorted(case.case_path for case in cases),
            [f"{directory}/{name}.html" for name in ("absolute", "dot-relative", "relative")],
        )
        self.assertEqual(self.request(f"read={self.key}", path="/other" + IMAGE_AND_STASH_PATH)[0], 404)


if __name__ == "__main__":
    unittest.main()
