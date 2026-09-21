from __future__ import annotations

import tempfile
import unittest
import uuid
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import NAVIGATION_SECOND_VISIT_PATH, WptFixtureServer


CASE_DIRECTORY = "navigation-api/navigation-methods/return-value"


class NavigationResponseFixtureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        resource = self.root / NAVIGATION_SECOND_VISIT_PATH.lstrip("/")
        resource.parent.mkdir(parents=True)
        resource.write_text("# Python source must not be served")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None,
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, query: str, *, method: str = "GET", port: int | None = None,
                path: str = NAVIGATION_SECOND_VISIT_PATH):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=2)
        try:
            connection.request(method, path + "?" + query,
                               body=b"ignored request body" if method == "POST" else None)
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def assert_initial_page(self, query: str, *, port: int | None = None) -> None:
        status, headers, body = self.request(query, port=port)
        self.assertEqual((status, headers["Content-Type"], body),
                         (200, "text/html", b"initial page"))
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertIsNone(headers["Content-Disposition"])

    def test_post_programs_one_get_response(self) -> None:
        for action in ("204", "205", "download"):
            with self.subTest(action=action):
                key = uuid.uuid4()
                query = f"id={key}"
                self.assert_initial_page(query)
                status, headers, body = self.request(query + f"&action={action}", method="POST")
                self.assertEqual((status, body), (204, b""))
                self.assertIsNone(headers["Content-Type"])
                self.assertIsNone(headers["Cache-Control"])
                status, headers, body = self.request(query)
                if action == "download":
                    self.assertEqual((status, headers["Content-Type"], body),
                                     (200, "text/plain", b"some text to download"))
                    self.assertEqual(headers["Content-Disposition"], "attachment")
                else:
                    self.assertEqual((status, body), (int(action), b""))
                    self.assertIsNone(headers["Content-Type"])
                self.assertIsNone(headers["Cache-Control"])
                self.assert_initial_page(query)

    def test_state_is_shared_across_origins_but_isolated_by_key_server_and_path(self) -> None:
        key = uuid.uuid4()
        query = f"id={key}"
        self.assertEqual(self.request(query + "&action=205", method="POST")[0], 204)
        self.assert_initial_page(f"id={uuid.uuid4()}")
        other = self.stack.enter_context(WptFixtureServer(self.root))
        self.assert_initial_page(query, port=other.port)
        status, _, body = self.request(f"key={key}", path="/fetch/api/resources/stash-take.py")
        self.assertEqual((status, body), (200, b"null"))
        status, _, _ = self.request(f"id={str(key).upper()}", port=self.server.alternate_port)
        self.assertEqual(status, 205)
        self.assert_initial_page(f"id={key.hex}")

    def test_unsupported_methods_do_not_consume_the_programmed_response(self) -> None:
        query = f"id={uuid.uuid4()}"
        self.assertEqual(self.request(query + "&action=204", method="POST")[0], 204)
        for method in ("HEAD", "OPTIONS", "PUT", "PATCH", "DELETE", "YO", "CUSTOM"):
            with self.subTest(method=method):
                status, _, body = self.request(query, method=method)
                self.assertEqual((status, body), (400, b""))
        self.assertEqual(self.request(query)[0], 204)
        self.assert_initial_page(query)

    def test_unknown_actions_are_consumed_and_duplicate_posts_do_not_overwrite(self) -> None:
        query = f"id={uuid.uuid4()}"
        for action in ("unknown", ""):
            self.assertEqual(self.request(query + f"&action={action}", method="POST")[0], 204)
            self.assertEqual(self.request(query)[0], 400)
            self.assert_initial_page(query)
        self.assertEqual(self.request(query + "&action=205&action=204", method="POST")[0], 204)
        self.assertEqual(self.request(query + "&action=204", method="POST")[0], 500)
        self.assertEqual(self.request(query + "&id=ignored")[0], 205)

    def test_invalid_parameters_report_errors(self) -> None:
        for query, method in (("", "GET"), ("id=", "GET"), ("id=not-a-uuid", "GET"),
                              (f"id={uuid.uuid4()}", "POST"), ("id=%FF&action=204", "POST")):
            with self.subTest(query=query, method=method):
                self.assertEqual(self.request(query, method=method)[0], 500)

    def test_discovery_accepts_only_supported_resource_locations(self) -> None:
        resource = "resources/204-205-download-on-second-visit.py"
        cases = {
            "relative.html": (resource, True),
            "dot.html": ("./" + resource, True),
            "absolute.html": (NAVIGATION_SECOND_VISIT_PATH, True),
            "wrong-directory.html": ("../" + resource, False),
            "basename.html": (resource.rsplit("/", 1)[1], False),
            "suffix.html": (resource + ".extra", False),
        }
        for name, (reference, _) in cases.items():
            (self.root / CASE_DIRECTORY / name).write_text(
                '<script src="/resources/testharness.js"></script>'
                '<script src="/resources/testharnessreport.js"></script>'
                f'<script>fetch(`{reference}?id=${{id}}`);</script>'
            )
        discovered = enumerate_cases(self.root, dir_prefixes=(CASE_DIRECTORY,))
        self.assertEqual(
            sorted(case.case_path for case in discovered),
            sorted(CASE_DIRECTORY + "/" + name for name, (_, allowed) in cases.items() if allowed),
        )


if __name__ == "__main__":
    unittest.main()
