from __future__ import annotations

import tempfile
import unittest
import uuid
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


PREFLIGHT = "/fetch/api/resources/preflight.py"
CLEAN = "/fetch/api/resources/clean-stash.py"


class FetchPreflightFixtureTests(unittest.TestCase):
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

    def request(self, port, query="", *, method="GET", headers=None, path=PREFLIGHT, body=None):
        connection = HTTPConnection("127.0.0.1", port, timeout=3)
        try:
            connection.request(method, path + "?" + query, body, headers or {})
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def preflight(self, port, query="", *, headers=None):
        return self.request(port, query, method="OPTIONS", headers={
            "Accept": "*/*", "Access-Control-Request-Method": "PUT", **(headers or {}),
        })

    def test_actual_requests_reflect_headers_for_standard_and_extension_methods(self):
        server = self.server()
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "patcH", "DELETE", "YO", "chicken", "*"):
            with self.subTest(method=method):
                status, headers, body = self.request(server.port, method=method, body=b"payload", headers={
                    "Origin": "https://caller.test", "Referer": "https://caller.test/page",
                })
                self.assertEqual((status, body), (200, b""))
                self.assertEqual(headers["Content-Type"], "text/plain")
                self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
                self.assertEqual(headers["x-origin"], "https://caller.test")
                self.assertEqual(headers["x-referrer"], "https://caller.test/page")
                self.assertEqual(headers["x-did-preflight"], "0")
                self.assertEqual(headers["x-control-request-headers"], "")
                self.assertEqual(headers["x-preflight-referrer"], "")
                self.assertIn("x-origin", headers["Access-Control-Expose-Headers"])

    def test_preflight_state_is_shared_across_origins_and_survives_actual_requests(self):
        server = self.server()
        key = str(uuid.uuid4())
        query = urlencode({"token": key, "control_request_headers": "", "max_age": "12000",
                           "allow_methods": "PUT", "allow_headers": "X-One, X-Two", "credentials": ""})
        status, headers, body = self.preflight(server.port, query, headers={
            "Access-Control-Request-Headers": "x-one,x-two", "User-Agent": "preflight-agent",
            "Referer": "https://page.test/preflight",
        })
        self.assertEqual((status, body), (200, b""))
        for name, value in [("Methods", "PUT"), ("Headers", "X-One, X-Two"), ("Credentials", "true")]:
            self.assertEqual(headers["Access-Control-Allow-" + name], value)
        self.assertEqual(headers["Access-Control-Max-Age"], "12000")
        self.assertIsNone(headers.get("Access-Control-Expose-Headers"))
        for method, port in [("GET", server.alternate_port), ("HEAD", server.port), ("PUT", server.port)]:
            status, headers, body = self.request(port, urlencode({
                "token": key.upper().replace("-", ""), "checkUserAgentHeaderInPreflight": "",
                "allow_headers": "not-an-actual-response-permission",
            }), method=method, headers={"User-Agent": "preflight-agent", "Referer": "https://page.test/actual"})
            self.assertEqual((status, body), (200, b""))
            self.assertEqual(headers["x-did-preflight"], "1")
            self.assertEqual(headers["x-control-request-headers"], "x-one,x-two")
            self.assertEqual(headers["x-preflight-referrer"], "https://page.test/preflight")
            self.assertEqual(headers["x-referrer"], "https://page.test/actual")
            self.assertIsNone(headers.get("Access-Control-Allow-Headers"))

    def test_control_header_distinguishes_missing_empty_and_unrequested(self):
        server = self.server()
        for control, request_headers, expected in [
            (True, {}, None), (True, {"Access-Control-Request-Headers": ""}, ""),
            (False, {"Access-Control-Request-Headers": "x-hidden"}, ""),
        ]:
            key = str(uuid.uuid4())
            query = "token=" + key + ("&control_request_headers" if control else "")
            self.assertEqual(self.preflight(server.port, query, headers=request_headers)[0], 200)
            _, headers, _ = self.request(server.port, "token=" + key)
            self.assertEqual(headers.get("x-control-request-headers"), expected)

    def test_duplicate_origins_and_first_query_values_are_preserved(self):
        server = self.server()
        for origin, expected in [("https://a.test, https://b.test", ["https://a.test", "https://b.test"]),
                                 ("https://a.test,https://b.test", ["https://a.test,https://b.test"]),
                                 ("", [""]), ("\xff", ["\xff"])]:
            query = urlencode([("origin", origin), ("origin", "ignored"), ("credentials", "false"),
                               ("allow_methods", "PUT"), ("allow_methods", "DELETE"),
                               ("allow_headers", ""), ("allow_headers", "ignored")], encoding="latin-1")
            for preflight in (False, True):
                status, headers, _ = self.preflight(server.port, query) if preflight else self.request(server.port, query)
                self.assertEqual(status, 200)
                self.assertEqual(headers.get_all("Access-Control-Allow-Origin"), expected)
                self.assertEqual(headers["Access-Control-Allow-Credentials"], "true")
                if preflight:
                    self.assertEqual(headers["Access-Control-Allow-Methods"], "PUT")
                    self.assertEqual(headers["Access-Control-Allow-Headers"], "")

    def test_clear_stash_uses_resource_namespaces_and_consumes_falsey_values(self):
        server, other = self.server(), self.server()
        key = str(uuid.uuid4())
        query = "token=" + key
        self.assertEqual(self.preflight(server.port, query)[0], 200)
        self.assertEqual(self.request(server.alternate_port, query, path=CLEAN)[2], b"0")
        self.assertEqual(self.request(server.port, query)[1]["x-did-preflight"], "1")
        self.assertEqual(self.request(other.port, query + "&clear-stash")[2], b"0")
        status, headers, body = self.request(server.alternate_port, query + "&clear-stash&credentials")
        self.assertEqual((status, body), (200, b"1"))
        self.assertIsNone(headers.get("Access-Control-Allow-Credentials"))
        self.assertIsNone(headers.get("x-did-preflight"))
        self.assertEqual(self.request(server.port, query + "&clear-stash")[2], b"0")
        server.fetch_stash.put(key, "", path=CLEAN)
        status, headers, body = self.request(server.alternate_port, query, path=CLEAN)
        self.assertEqual((status, body), (200, b"1"))
        self.assertIsNone(headers.get("Content-Type"))
        self.assertIsNone(headers.get("Access-Control-Allow-Origin"))
        self.assertEqual(self.request(server.port, query, path=CLEAN)[2], b"0")
        # The pre-existing abort fixtures keep their directory-scoped namespace.
        server.fetch_stash.put(key, "abort-state")
        self.assertEqual(self.request(server.port, query, path=CLEAN)[2], b"0")
        self.assertEqual(server.fetch_stash.take(key), "abort-state")

    def test_preflight_rejects_overwrites_without_losing_previous_state(self):
        server = self.server()
        query = "token=" + str(uuid.uuid4())
        self.assertEqual(self.preflight(server.port, query)[0], 200)
        self.assertEqual(self.preflight(server.port, query)[0], 500)
        self.assertEqual(self.request(server.port, query)[1]["x-did-preflight"], "1")

    def test_stash_namespace_preserves_encoded_request_paths(self):
        server = self.server()
        query = "token=" + str(uuid.uuid4())
        encoded = PREFLIGHT.replace("preflight.py", "%70reflight.py")
        status, _, _ = self.request(server.port, query, path=encoded, method="OPTIONS", headers={
            "Access-Control-Request-Method": "PUT", "Accept": "*/*",
        })
        self.assertEqual(status, 200)
        self.assertEqual(self.request(server.port, query)[1]["x-did-preflight"], "0")
        self.assertEqual(self.request(server.port, query, path=encoded)[1]["x-did-preflight"], "1")

    def test_preflight_requires_method_header_and_exact_accept_value(self):
        server = self.server()
        for headers in ({}, {"Accept": "*/*"}, {"Access-Control-Request-Method": "PUT"},
                        {"Access-Control-Request-Method": "PUT", "Accept": "text/plain"}):
            status, response_headers, body = self.request(server.port, method="OPTIONS", headers=headers)
            self.assertEqual(status, 400)
            self.assertIsNone(response_headers.get("Access-Control-Allow-Origin"))
            self.assertTrue(body.startswith(b"ERROR:"))
        # The upstream handler checks presence, not the method header's grammar.
        self.assertEqual(self.preflight(server.port, headers={"Access-Control-Request-Method": ""})[0], 200)

    def test_preflight_status_and_invalid_tokens_follow_upstream_error_order(self):
        server = self.server()
        for status in (200, 201, 204, 205, 299, 301, 304, 307, 400, 403, 500, 505):
            self.assertEqual(self.preflight(server.port, f"preflight_status={status}&preflight_status=999")[0], status)
        for query in ("preflight_status=", "preflight_status=bad", "token=not-a-uuid", "token=%FF"):
            self.assertEqual(self.preflight(server.port, query)[0], 500)
        self.assertEqual(self.request(server.port, "token=bad", method="OPTIONS")[0], 400)
        self.assertEqual(self.preflight(server.port, "token=")[0], 200)
        self.assertEqual(self.request(server.port, "token=")[0], 200)
        for path, query in [(CLEAN, ""), (CLEAN, "token="), (CLEAN, "token=bad"), (PREFLIGHT, "clear-stash")]:
            self.assertEqual(self.request(server.port, query, path=path)[0], 500)

    def test_user_agent_mismatch_consumes_preflight_record(self):
        server = self.server()
        query = "token=" + str(uuid.uuid4())
        self.assertEqual(self.preflight(server.port, query, headers={"User-Agent": "before"})[0], 200)
        status, headers, body = self.request(server.port, query + "&checkUserAgentHeaderInPreflight", headers={"User-Agent": "after"})
        self.assertEqual((status, body), (400, b"ERROR: No user-agent header in preflight"))
        self.assertEqual(headers["Access-Control-Allow-Origin"], "*")
        self.assertEqual(self.request(server.port, query)[1]["x-did-preflight"], "0")
        self.assertEqual(self.request(server.port, query + "&checkUserAgentHeaderInPreflight")[0], 500)

    def test_resource_routes_do_not_accept_prefixes_or_suffixes(self):
        server = self.server()
        for path in (PREFLIGHT + "2", CLEAN + ".js", "/wrong" + PREFLIGHT, "/fetch/api/cors/preflight.py"):
            for method in ("GET", "OPTIONS", "chicken"):
                self.assertGreaterEqual(self.request(server.port, path=path, method=method)[0], 400)

    def test_handlers_respond_before_unused_uploads_finish(self):
        server = self.server()
        for path, method, expected in ((PREFLIGHT, "POST", b""),
                                       (PREFLIGHT, "OPTIONS", b""),
                                       (CLEAN + "?token=" + str(uuid.uuid4()), "POST", b"0")):
            for framing in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
                with self.subTest(path=path, method=method, framing=framing):
                    connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
                    try:
                        connection.putrequest(method, path)
                        connection.putheader(*framing)
                        connection.putheader("Accept", "*/*")
                        connection.putheader("Access-Control-Request-Method", "PUT")
                        connection.endheaders()
                        response = connection.getresponse()
                        self.assertEqual((response.status, response.read()), (200, expected))
                        self.assertEqual(response.headers["Connection"], "close")
                    finally:
                        connection.close()

    def test_case_selection_recognizes_supported_literal_and_resource_dir_references(self):
        cases = {
            "absolute": "fetch('/fetch/api/resources/preflight.py');",
            "relative": "fetch('../resources/clean-stash.py?token=1');",
            "concat": 'fetch(RESOURCES_DIR + "preflight.py");',
            "unknown": "fetch('../resources/preflight.py'); fetch('../resources/unknown.py');",
            "suffix": "fetch('../resources/preflight.py2');",
            "prefix": "fetch('/wrong/fetch/api/resources/preflight.py');",
            "wrong-relative": "fetch('resources/preflight.py');",
            "bare": "fetch('preflight.py');",
        }
        utils = self.root / "fetch/api/resources/utils.js"
        utils.parent.mkdir(parents=True)
        utils.write_text('var RESOURCES_DIR = "../resources/"; fetch(RESOURCES_DIR + "inspect-headers.py");')
        for name, source in cases.items():
            path = self.root / f"fetch/api/cors/{name}.any.js"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// META: global=window,worker\n// META: script=../resources/utils.js\n" + source)
        selected = enumerate_cases(self.root, dir_prefixes=("fetch",), any_js_global="both")
        self.assertEqual([case.case_path for case in selected], [
            f"fetch/api/cors/{name}.any.js?moli-wpt-any={realm}"
            for name in ("absolute", "concat", "relative")
            for realm in ("dedicatedworker", "window")
        ])
