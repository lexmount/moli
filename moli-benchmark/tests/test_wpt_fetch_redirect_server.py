from __future__ import annotations

import tempfile
import unittest
import uuid
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack
from http.client import HTTPConnection
from pathlib import Path
from threading import Event
from unittest.mock import patch
from urllib.parse import urlencode

from moli_benchmark.wpt_cross.case_set import enumerate_cases
from moli_benchmark.wpt_cross.server import WptFixtureServer


REDIRECT = "/fetch/api/resources/redirect.py"
EMPTY_LOCATION = "/fetch/api/resources/redirect-empty-location.py"


class FetchRedirectFixtureTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        self.root = Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        (self.root / "resources").mkdir()
        (self.root / "resources/testharness.js").write_text("// testharness")
        self.stack.enter_context(patch(
            "moli_benchmark.wpt_cross.server._global_ipv6_address", return_value=None
        ))
        self.server = self.stack.enter_context(WptFixtureServer(self.root))

    def request(self, query="", *, method="GET", headers=None, body=None, port=None, path=REDIRECT):
        connection = HTTPConnection("127.0.0.1", port or self.server.port, timeout=2)
        try:
            connection.request(method, path + "?" + query, body, headers or {})
            response = connection.getresponse()
            return response.status, response.headers, response.read()
        finally:
            connection.close()

    def test_standard_and_extension_methods_echo_the_request_origin(self):
        query = "redirect_status=303&location=%2Ftarget"
        for method in ("GET", "HEAD", "POST", "PUT", "PATCH", "patcH", "DELETE", "YO", "chicken", "*"):
            with self.subTest(method=method):
                status, headers, body = self.request(query, method=method, body=b"payload", headers={
                    "Origin": "https://caller.test",
                })
                self.assertEqual((status, body), (303, b""))
                self.assertEqual(headers["Content-Type"], "text/plain")
                self.assertEqual(headers.get_all("Cache-Control"), ["no-cache"])
                self.assertEqual(headers["Pragma"], "no-cache")
                self.assertEqual(headers["Access-Control-Allow-Origin"], "https://caller.test")
                self.assertEqual(headers["Access-Control-Allow-Credentials"], "true")
                self.assertEqual(headers["Location"], "/target?redirect_status=303&location=%2Ftarget&count=1")
        for origin in (None, "", "null", "\xff"):
            with self.subTest(origin=origin):
                _, headers, _ = self.request(query, headers={} if origin is None else {"Origin": origin})
                self.assertEqual(headers["Access-Control-Allow-Origin"], "*" if origin is None else origin)
                self.assertEqual(headers.get("Access-Control-Allow-Credentials"), None if origin is None else "true")

    def test_location_inherits_first_query_values_and_preserves_existing_syntax(self):
        query = "location=%2Ftarget%3Fexisting%3D1&location=%2Fignored&a=x+y&a=ignored&count=20"
        self.assertEqual(self.request(query)[1]["Location"],
                         "/target?existing=1&location=%2Ftarget%3Fexisting%3D1&a=x+y&count=20&count=1")
        for simple in ("", "false", "1"):
            with self.subTest(simple=simple):
                self.assertEqual(self.request(query + "&simple=" + simple)[1]["Location"], "/target?existing=1")
        self.assertIsNone(self.request()[1].get("Location"))
        self.assertEqual(self.request("location=")[1]["Location"], "?location=&count=1")
        self.assertEqual(self.request("location=&simple")[1]["Location"], "")
        self.assertEqual(self.request("location=%2Ftarget%23fragment")[1]["Location"],
                         "/target#fragment?location=%2Ftarget%23fragment&count=1")
        self.assertEqual(self.request("location=%2Ftarget&raw=%FF")[1]["Location"],
                         "/target?location=%2Ftarget&raw=%C3%BF&count=1")

    def test_non_http_locations_skip_parameter_inheritance(self):
        for location in ("data:text/plain,hello", "blob:https://a.test/id", "about:blank", "file:///tmp/example"):
            with self.subTest(location=location):
                self.assertEqual(self.request(urlencode({"location": location}))[1]["Location"], location)
        location = "http://user:password@a.test/path"
        self.assertEqual(self.request(urlencode({"location": location, "simple": ""}))[1]["Location"], location)

    def test_options_only_redirect_when_the_flag_is_present(self):
        query = "redirect_status=307&location=%2Ftarget&allow_headers=X-One%2C+x-two&allow_headers=ignored"
        status, headers, body = self.request(query, method="OPTIONS", headers={"Origin": "https://caller.test"})
        self.assertEqual((status, body), (200, b""))
        self.assertIsNone(headers.get("Location"))
        self.assertEqual(headers["Access-Control-Allow-Headers"], "X-One, x-two")
        self.assertIsNone(headers.get("Access-Control-Allow-Methods"))
        self.assertEqual(headers["Access-Control-Allow-Origin"], "https://caller.test")
        self.assertEqual(headers["Access-Control-Allow-Credentials"], "true")
        status, headers, body = self.request(query + "&redirect_preflight=false", method="OPTIONS")
        self.assertEqual((status, body), (307, b""))
        self.assertIn("redirect_preflight=false&count=1", headers["Location"])

    def test_counter_is_shared_between_origins_and_stops_without_redirect_headers(self):
        key = str(uuid.uuid4())
        query = "token=" + key + "&location=%2Ftarget&max_count=2"
        for count in range(1, 5):
            with self.subTest(count=count):
                status, headers, body = self.request(query, port=self.server.port if count % 2 else self.server.alternate_port)
                if count <= 2:
                    self.assertEqual((status, body), (302, b""))
                    self.assertTrue(headers["Location"].endswith("&count=" + str(count)))
                else:
                    self.assertEqual((status, body), (200, str(count - 1).encode()))
                    for name in ("Location", "Content-Type", "Cache-Control", "Pragma", "Access-Control-Allow-Origin"):
                        self.assertIsNone(headers.get(name), name)
        key = str(uuid.uuid4())
        query = "token=" + key + "&location=%2Ftarget&max_count=0"
        self.assertEqual(self.request(query, method="OPTIONS")[0], 200)
        self.assertEqual(self.request(query)[2], b"0")

    def test_counter_uses_the_complete_raw_request_path(self):
        key = str(uuid.uuid4())
        query = "token=" + key + "&location=%2Ftarget"
        encoded = REDIRECT.replace("redirect.py", "%72edirect.py")
        for count in (1, 2):
            for path in (REDIRECT, encoded):
                with self.subTest(path=path, count=count):
                    self.assertTrue(self.request(query, path=path)[1]["Location"].endswith("&count=" + str(count)))
        self.request("token=" + key, path="/fetch/api/resources/clean-stash.py")
        self.assertTrue(self.request(query)[1]["Location"].endswith("&count=3"))

    def test_status_uses_query_before_urlencoded_and_multipart_form_fields(self):
        form = b"redirect_status=307&redirect_status=301"
        for method in ("POST", "PUT"):
            with self.subTest(method=method):
                status, _, _ = self.request("location=%2Ftarget", method=method, body=form,
                                            headers={"Content-Type": "application/x-www-form-urlencoded"})
                self.assertEqual(status, 307)
        self.assertEqual(self.request("location=%2Ftarget", method="POST", body=form)[0], 307)
        self.assertEqual(self.request("location=%2Ftarget&redirect_status=301&redirect_status=308",
                                      method="POST", body=form)[0], 301)
        self.assertEqual(self.request("location=%2Ftarget", body=form,
                                      headers={"Content-Type": "application/x-www-form-urlencoded"})[0], 302)
        body = b'--boundary\r\nContent-Disposition: form-data; name="redirect_status"\r\n\r\n308\r\n--boundary--\r\n'
        self.assertEqual(self.request("location=%2Ftarget", method="POST", body=body,
                                      headers={"Content-Type": "multipart/form-data; boundary=boundary"})[0], 308)
        self.assertEqual(self.request("location=%2Ftarget", method="POST", body=form,
                                      headers={"Content-Type": "text/plain"})[0], 500)
        for content_type in ("", "APPLICATION/X-WWW-FORM-URLENCODED"):
            with self.subTest(content_type=content_type):
                self.assertEqual(self.request("location=%2Ftarget", method="POST", body=form,
                                              headers={"Content-Type": content_type})[0], 500)

    def test_non_redirect_statuses_and_referrer_policy_are_preserved(self):
        for code in (200, 201, 204, 299, 301, 302, 303, 304, 307, 308, 399, 400, 418, 500, 599):
            with self.subTest(code=code):
                status, headers, _ = self.request("redirect_status=" + str(code) + "&location=%2Ftarget")
                self.assertEqual(status, code)
                self.assertIsNotNone(headers.get("Location"))
        _, headers, _ = self.request("location=%2Ftarget&redirect_referrerpolicy=no-referrer&redirect_referrerpolicy=unsafe-url")
        self.assertEqual(headers["Referrer-Policy"], "no-referrer")
        for query in ("redirect_status=bad", "redirect_status=", "token=", "token=bad", "delay=bad", "delay=-1",
                      "redirect_status=%A0302%A0", "delay=%A01%A0",
                      "token=" + str(uuid.uuid4()) + "&max_count=%A01%A0"):
            with self.subTest(query=query):
                self.assertEqual(self.request(query)[0], 500)

    def test_multipart_status_uses_raw_first_field_bytes(self):
        for parts, expected in [
            ([(b'', b'307'), (b'', b'308')], 307),
            ([(b'Content-Transfer-Encoding: base64\r\n', b'MzA3')], 500),
            ([(b'Content-Type: text/plain; charset=utf-8\r\n', b'\xa0307\xa0')], 500),
        ]:
            with self.subTest(parts=parts):
                body = b''.join(
                    b'--boundary\r\nContent-Disposition: form-data; name="redirect_status"\r\n'
                    + headers + b'\r\n' + value + b'\r\n' for headers, value in parts
                ) + b'--boundary--\r\n'
                self.assertEqual(self.request("location=%2Ftarget", method="POST", body=body,
                                              headers={"Content-Type": "multipart/form-data; boundary=boundary"})[0], expected)
        body = b'--boundary\r\nContent-Disposition: form-data; name="redirect_status"; filename="status.txt"\r\n\r\n307\r\n--boundary--\r\n'
        self.assertEqual(self.request("location=%2Ftarget", method="POST", body=body,
                                      headers={"Content-Type": "multipart/form-data; boundary=boundary"})[0], 500)

    def test_delay_precedes_response_headers(self):
        entered, release = Event(), Event()

        def delay(seconds):
            self.assertEqual(seconds, 0.025)
            entered.set()
            release.wait(2)

        with patch("moli_benchmark.wpt_cross.server.time.sleep", side_effect=delay), ThreadPoolExecutor() as executor:
            future = executor.submit(self.request, "location=%2Ftarget&delay=25")
            try:
                self.assertTrue(entered.wait(2), "the redirect handler must delay its response")
                self.assertFalse(future.done(), "headers must not arrive before the delay completes")
            finally:
                release.set()
            self.assertEqual(future.result(timeout=2)[0], 302)

    def test_handlers_respond_before_unused_uploads_finish(self):
        for method in ("GET", "POST", "OPTIONS"):
            for framing in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
                with self.subTest(method=method, framing=framing):
                    connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
                    try:
                        connection.putrequest(method, REDIRECT + "?redirect_status=302&location=%2Ftarget")
                        connection.putheader(*framing)
                        connection.endheaders()
                        response = connection.getresponse()
                        self.assertEqual((response.status, response.read()), (200 if method == "OPTIONS" else 302, b""))
                        self.assertEqual(response.headers["Connection"], "close")
                    finally:
                        connection.close()

    def test_empty_location_is_present_for_every_method_without_cors_grants(self):
        for method in ("GET", "HEAD", "OPTIONS", "POST", "PUT", "PATCH", "DELETE", "YO", "chicken"):
            with self.subTest(method=method):
                status, headers, body = self.request(
                    "redirect_status=307&location=/ignored", path=EMPTY_LOCATION,
                    method=method, headers={"Origin": "https://caller.test"}, body=b"upload",
                )
                self.assertEqual((status, body), (302, b""))
                self.assertEqual(headers.get_all("Location"), [""])
                self.assertEqual(headers["Content-Length"], "0")
                for name in ("Content-Type", "Cache-Control", "Access-Control-Allow-Origin"):
                    self.assertIsNone(headers[name], name)
        for path in (EMPTY_LOCATION + "2", "/wrong" + EMPTY_LOCATION):
            with self.subTest(path=path):
                self.assertEqual(self.request(path=path)[0], 404)

    def test_empty_location_responds_before_unused_uploads_finish(self):
        for method in ("POST", "PUT", "OPTIONS"):
            for framing in (("Content-Length", "1000000"), ("Transfer-Encoding", "chunked")):
                with self.subTest(method=method, framing=framing):
                    connection = HTTPConnection("127.0.0.1", self.server.port, timeout=2)
                    try:
                        connection.putrequest(method, EMPTY_LOCATION)
                        connection.putheader(*framing)
                        connection.endheaders()
                        response = connection.getresponse()
                        self.assertEqual((response.status, response.read()), (302, b""))
                        self.assertEqual(response.headers.get_all("Location"), [""])
                        self.assertEqual(response.headers["Connection"], "close")
                    finally:
                        connection.close()

    def test_case_selection_recognizes_empty_location_references(self):
        sources = {
            "absolute": f"fetch('{EMPTY_LOCATION}');",
            "concat": 'fetch(RESOURCES_DIR + "redirect-empty-location.py");',
            "relative": "fetch('../resources/redirect-empty-location.py');",
            "template": "fetch(`${RESOURCES_DIR}redirect-empty-location.py?ignored=1`);",
            "bare": "fetch('redirect-empty-location.py');",
            "prefix": f"fetch('/wrong{EMPTY_LOCATION}');",
            "suffix": "fetch('../resources/redirect-empty-location.py2');",
            "unknown": f"fetch('{EMPTY_LOCATION}'); fetch('../resources/unknown.py');",
        }
        for name, source in sources.items():
            path = self.root / f"fetch/api/redirect/{name}.any.js"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// META: global=window,worker\n" + source)
        selected = enumerate_cases(self.root, dir_prefixes=("fetch",), any_js_global="both")
        self.assertEqual([case.case_path for case in selected], [
            f"fetch/api/redirect/{name}.any.js?moli-wpt-any={realm}"
            for name in ("absolute", "concat", "relative", "template")
            for realm in ("dedicatedworker", "window")
        ])

    def test_case_selection_recognizes_fetch_redirect_references(self):
        sources = {
            "absolute": "fetch('/fetch/api/resources/redirect.py');",
            "concat": 'fetch(RESOURCES_DIR + "redirect.py");',
            "relative": "fetch('../resources/redirect.py');",
            "template": "fetch(`${RESOURCES_DIR}redirect.py?token=${token}`); fetch(`${RESOURCES_DIR}clean-stash.py`);",
            "bare": "fetch('redirect.py');",
            "prefix": "fetch('/wrong/fetch/api/resources/redirect.py');",
            "suffix": "fetch('../resources/redirect.py2');",
            "unknown": "fetch('../resources/redirect.py'); fetch('../resources/unknown.py');",
        }
        for name, source in sources.items():
            path = self.root / f"fetch/api/redirect/{name}.any.js"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// META: global=window,worker\n" + source)
        selected = enumerate_cases(self.root, dir_prefixes=("fetch",), any_js_global="both")
        self.assertEqual([case.case_path for case in selected], [
            f"fetch/api/redirect/{name}.any.js?moli-wpt-any={realm}"
            for name in ("absolute", "concat", "relative", "template")
            for realm in ("dedicatedworker", "window")
        ])


if __name__ == "__main__":
    unittest.main()
