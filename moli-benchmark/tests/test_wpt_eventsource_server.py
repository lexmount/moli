from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


RESOURCE = "/eventsource/resources/message.py"


class EventSourceFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        for name in ("testharness.js", "testharnessreport.js"):
            (self.root / "resources" / name).write_text("// harness")
        self.directory = self.root / "eventsource"
        (self.directory / "resources").mkdir(parents=True)
        (self.directory / "resources/message.py").write_text("# never serve Python source as SSE")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))

    def request(self, server: WptFixtureServer, query: str, method: str = "GET"):
        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
        try:
            connection.request(method, RESOURCE + "?" + query)
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def test_message_body_mime_and_byte_queries_match_upstream(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        cases = (
            ("", "text/event-stream", b"data: data\n\n\n"),
            ("message=data%3A+first&message=ignored", "text/event-stream", b"data: first\n\n\n"),
            ("message=&newline=none", "text/event-stream", b"\n"),
            ("message=%FF%00%0D%0A&newline=none", "text/event-stream", b"\xff\x00\r\n\n"),
            ("mime=text%2Fplain&message=custom", "text/plain", b"custom\n\n\n"),
        )
        for query, mime, body in cases:
            for method in ("GET", "HEAD"):
                with self.subTest(query=query, method=method):
                    status, headers, actual = self.request(server, query, method)
                    self.assertEqual(status, 200)
                    self.assertEqual(headers["Content-Type"], mime)
                    self.assertEqual(headers["Content-Length"], str(len(body)))
                    self.assertEqual(actual, b"" if method == "HEAD" else body)

    def test_sleep_parameter_delays_response_and_invalid_values_fail(self) -> None:
        server = self.stack.enter_context(WptFixtureServer(self.root))
        with patch.object(server._stopping, "wait", return_value=False) as wait:
            status, _, body = self.request(server, "sleep=25&sleep=999")
            self.assertEqual((status, body), (200, b"data: data\n\n\n"))
            wait.assert_called_once_with(0.025)
        for query in ("sleep=invalid", "sleep=-1", "sleep="):
            with self.subTest(query=query):
                self.assertEqual(self.request(server, query)[0], 500)

    def test_discovery_only_allows_the_implemented_resource(self) -> None:
        cases = {
            "relative.html": ("resources/message.py", True),
            "absolute.html": (RESOURCE, True),
            "nested/relative.html": ("../resources/message.py", True),
            "nested/dot-relative.html": ("./../resources/message.py", True),
            "nested/wrong.html": ("resources/message.py", False),
            "other.html": ("/other/resources/message.py", False),
            "suffix.html": (RESOURCE + ".extra", False),
        }
        for name, (reference, _) in cases.items():
            path = self.directory / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                '<script src="/resources/testharness.js"></script>'
                '<script src="/resources/testharnessreport.js"></script>'
                f'<script>new EventSource("{reference}?message=data%3A+data")</script>'
            )
        discovered = enumerate_cases(self.root, dir_prefixes=("eventsource",))
        self.assertEqual(
            sorted(case.case_path for case in discovered),
            sorted("eventsource/" + name for name, (_, allowed) in cases.items() if allowed),
        )


if __name__ == "__main__":
    unittest.main()
