from __future__ import annotations

from wpt_cross_test_support import *


class WptCrossFixtureResourcesTests(WptCrossTestCase):
    def test_fixture_server_models_reporting_resource_stash(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            with WptFixtureServer(root_path) as server:
                report_url = f"{server.base_url}/reporting/resources/report.py?op=put&reportID=abc"
                payload = json.dumps(
                    {"csp-report": {"violated-directive": "frame-src 'none'"}}
                ).encode("utf-8")
                request = Request(
                    report_url,
                    data=payload,
                    headers={"Content-Type": "application/csp-report"},
                    method="POST",
                )
                with urlopen(request, timeout=2) as response:
                    self.assertEqual(response.status, 200)

                retrieve_url = (
                    f"{server.base_url}/reporting/resources/report.py"
                    "?op=retrieve_report&timeout=0&reportID=abc"
                )
                with urlopen(retrieve_url, timeout=2) as response:
                    reports = json.loads(response.read().decode("utf-8"))

        self.assertEqual(
            reports[0]["csp-report"]["violated-directive"],
            "frame-src 'none'",
        )
        self.assertEqual(
            reports[0]["metadata"]["content_type"],
            "application/csp-report",
        )

    def test_fixture_server_accepts_standard_websocket_echo_endpoint(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            with WptFixtureServer(root_path) as server:
                with socket.create_connection(
                    ("127.0.0.1", server.port), timeout=2
                ) as connection:
                    connection.sendall(
                        b"GET /echo HTTP/1.1\r\n"
                        b"Host: localhost\r\n"
                        b"Connection: Upgrade\r\n"
                        b"Upgrade: websocket\r\n"
                        b"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
                        b"Sec-WebSocket-Version: 13\r\n\r\n"
                    )
                    response = b""
                    while b"\r\n\r\n" not in response:
                        response += connection.recv(4096)
                    self.assertTrue(response.startswith(b"HTTP/1.0 101"))
                    self.assertIn(b"Upgrade: websocket\r\n", response)
                    self.assertIn(
                        b"Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n",
                        response,
                    )

                    mask = b"\x01\x02\x03\x04"
                    payload = b"ping"
                    masked = bytes(
                        value ^ mask[index % len(mask)]
                        for index, value in enumerate(payload)
                    )
                    connection.sendall(b"\x81\x84" + mask + masked)
                    echoed = b""
                    while len(echoed) < 6:
                        echoed += connection.recv(6 - len(echoed))
                    self.assertEqual(echoed, b"\x81\x04ping")

    def test_fixture_server_serves_raw_asis_header_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            (root_path / "raw.asis").write_bytes(
                b"HTTP/1.1 200 OK\r\n"
                b"Content-Type: text/plain\r\n"
                b"X-Custom-Header-Bytes: \xe2\x80\xa6\r\n"
                b"\r\n"
                b"OK"
            )

            with WptFixtureServer(root_path) as server:
                with urlopen(f"{server.base_url}/raw.asis", timeout=2) as response:
                    self.assertEqual(response.read(), b"OK")
                    header = response.headers["X-Custom-Header-Bytes"]

        self.assertEqual(header.encode("latin-1"), b"\xe2\x80\xa6")

    def test_fixture_server_serves_raw_sidecar_header_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            (root_path / "manifest.webmanifest").write_bytes(b"{}")
            (root_path / "manifest.webmanifest.headers").write_bytes(
                b"Content-Type: \xc3\x97\xc2\xba invalid\r\n"
            )

            with WptFixtureServer(root_path) as server:
                with urlopen(
                    f"{server.base_url}/manifest.webmanifest",
                    timeout=2,
                ) as response:
                    self.assertEqual(response.read(), b"{}")
                    header = response.headers["Content-Type"]

        self.assertEqual(header.encode("latin-1"), b"\xc3\x97\xc2\xba invalid")

    def test_fixture_server_uses_explicit_content_type_header_as_override(self) -> None:
        self.assertEqual(
            _static_response_header_block(
                "application/octet-stream",
                [("Content-Type", "application/wasm"), ("X-Test", "ok")],
            ),
            [("Content-Type", "application/wasm"), ("X-Test", "ok")],
        )
        self.assertEqual(
            _static_response_header_block("application/javascript", [("X-Test", "ok")]),
            [("Content-Type", "application/javascript"), ("X-Test", "ok")],
        )

    def test_fixture_server_detects_explicit_content_length_header(self) -> None:
        header_block = _static_response_header_block(
            "text/html",
            [("Content-Length", "403"), ("X-Test", "ok")],
        )

        self.assertTrue(_headers_include(header_block, "content-length"))

    def test_fixture_server_preserves_explicit_content_length_body_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness",
                encoding="utf-8",
            )
            body = (
                b'<!doctype html><script src="/resources/testharnessreport.js"></script>'
                b"<body>PASS"
            )
            fixture = root_path / "content-length.html"
            fixture.write_bytes(body + b"FAIL")
            fixture.with_name("content-length.html.headers").write_text(
                f"Content-Length: {len(body)}\n",
                encoding="utf-8",
            )

            with WptFixtureServer(root_path) as server:
                server.set_harness_timeout_multipliers(
                    {"content-length.html": 12.0}
                )
                with urlopen(
                    f"{server.base_url}/content-length.html",
                    timeout=2,
                ) as response:
                    served = response.read()

        self.assertEqual(served, body)
        self.assertNotIn(BENCH_TIMEOUT_MULTIPLIER_QUERY.encode("ascii"), served)

    def test_fixture_server_maps_legacy_webidl_parser_resource(self) -> None:
        self.assertEqual(
            _legacy_wpt_resource_alias("/resources/WebIDLParser.js"),
            "resources/webidl2/lib/webidl2.js",
        )
        self.assertIsNone(_legacy_wpt_resource_alias("/resources/testharness.js"))

    def test_fixture_server_builds_any_js_window_wrapper(self) -> None:
        body = _any_js_window_wrapper(
            "/WebCryptoAPI/sign_verify/hmac.https.any.html",
            b"// META: title=WebCryptoAPI hmac\n"
            b"// META: script=../util/helpers.js\n"
            b"// META: script=hmac_vectors.js\n"
            b"// META: timeout=long\n"
            b"run_test();\n",
        )

        self.assertIn(b'<meta name="timeout" content="long">', body)
        self.assertIn(b"<title>WebCryptoAPI hmac</title>", body)
        self.assertIn(b"self.GLOBAL = {", body)
        self.assertIn(b'<script src="/resources/testharness.js"></script>', body)
        self.assertIn(b'<script src="/resources/testharnessreport.js"></script>', body)
        self.assertIn(b'<script src="../util/helpers.js"></script>', body)
        self.assertIn(b'<script src="hmac_vectors.js"></script>', body)
        self.assertIn(b'<div id="log"></div>', body)
        self.assertIn(b'<script src="hmac.https.any.js"></script>', body)

    def test_fixture_server_builds_window_js_wrapper_without_any_global(self) -> None:
        body = _window_js_window_wrapper(
            "/WebCryptoAPI/algorithm-discards-context.https.window.html",
            b"// META: title=Window case\n"
            b"// META: script=helper.js\n"
            b"test(() => {}, 'ok');\n",
        )

        self.assertIn(b"<title>Window case</title>", body)
        self.assertIn(b'<script src="/resources/testharness.js"></script>', body)
        self.assertIn(b'<script src="/resources/testharnessreport.js"></script>', body)
        self.assertIn(b'<script src="helper.js"></script>', body)
        self.assertIn(b'<script src="algorithm-discards-context.https.window.js"></script>', body)
        self.assertNotIn(b"self.GLOBAL", body)

    def test_fixture_server_builds_wpt_any_js_query_wrapper(self) -> None:
        html = _wpt_any_window_wrapper_html(
            "wasm/jsapi/feature.any.js",
            "// META: script=../support/helper.js\n// META: script=/common/gc.js?run=1\n",
            query="variant=1&moli-wpt-any=window",
        )

        self.assertIn('<script src="/resources/testharness.js"></script>', html)
        self.assertIn('<script src="/resources/testharnessreport.js"></script>', html)
        self.assertIn('<script src="/wasm/support/helper.js"></script>', html)
        self.assertIn('<script src="/common/gc.js?run=1"></script>', html)
        self.assertIn(
            '<script src="/wasm/jsapi/feature.any.js?variant=1"></script>',
            html,
        )
        self.assertNotIn("moli-wpt-any", html)

    def test_fixture_server_builds_any_js_dedicated_worker_wrapper(self) -> None:
        html = _wpt_any_dedicated_worker_wrapper_html(
            "wasm/jsapi/feature.any.js",
            query="variant=1&moli-wpt-any=dedicatedworker",
        )

        self.assertIn('<script src="/resources/testharness.js"></script>', html)
        self.assertIn('<script src="/resources/testharnessreport.js"></script>', html)
        self.assertIn(
            'fetch_tests_from_worker(new Worker("/wasm/jsapi/feature.any.worker.js?variant=1"))',
            html,
        )
        self.assertNotIn("moli-wpt-any", html)

    def test_fixture_server_builds_any_js_dedicated_worker_script(self) -> None:
        js = _wpt_any_dedicated_worker_wrapper_js(
            "wasm/jsapi/feature.any.js",
            "// META: script=../support/helper.js\n// META: script=/common/gc.js?run=1\n",
            query="variant=1&moli-wpt-any=dedicatedworker",
        )

        self.assertIn("isWindow:function(){return false;}", js)
        self.assertIn("isWorker:function(){return true;}", js)
        self.assertIn('importScripts("/resources/testharness.js");', js)
        self.assertIn('importScripts("/wasm/support/helper.js");', js)
        self.assertIn('importScripts("/common/gc.js?run=1");', js)
        self.assertIn('importScripts("/wasm/jsapi/feature.any.js?variant=1");', js)
        self.assertTrue(js.rstrip().endswith("done();"))
        self.assertNotIn("moli-wpt-any", js)

    def test_fixture_server_builds_window_js_wrapper(self) -> None:
        html = _wpt_window_js_wrapper_html(
            "wasm/serialization/transfer.window.js",
            "// META: script=../support/helper.js\n// META: script=/common/gc.js?run=1\n",
            query="variant=1&moli-wpt-script=window",
        )

        self.assertIn('<script src="/resources/testharness.js"></script>', html)
        self.assertIn('<script src="/resources/testharnessreport.js"></script>', html)
        self.assertIn('<script src="/wasm/support/helper.js"></script>', html)
        self.assertIn('<script src="/common/gc.js?run=1"></script>', html)
        self.assertIn(
            '<script src="/wasm/serialization/transfer.window.js?variant=1"></script>',
            html,
        )
        self.assertNotIn("moli-wpt-script", html)

    def test_fixture_server_builds_worker_js_wrapper(self) -> None:
        html = _wpt_dedicated_worker_js_wrapper_html(
            "wasm/create_multiple_memory.worker.js",
            query="variant=1&moli-wpt-script=dedicatedworker",
        )

        self.assertIn('<script src="/resources/testharness.js"></script>', html)
        self.assertIn('<script src="/resources/testharnessreport.js"></script>', html)
        self.assertIn(
            'fetch_tests_from_worker(new Worker("/wasm/create_multiple_memory.worker.js?variant=1"))',
            html,
        )
        self.assertNotIn("moli-wpt-script", html)

    def test_any_js_worker_script_path_round_trips_to_source_path(self) -> None:
        worker_path = any_js_worker_script_path(
            "wasm/jsapi/feature.any.js?variant=1"
        )

        self.assertEqual(
            worker_path,
            "wasm/jsapi/feature.any.worker.js?variant=1",
        )
        self.assertEqual(
            any_js_source_script_path(worker_path),
            "wasm/jsapi/feature.any.js?variant=1",
        )

    def test_any_js_case_path_for_global_replaces_existing_wrapper_query(self) -> None:
        self.assertEqual(
            any_js_case_path_for_global(
                "wasm/jsapi/feature.any.js?moli-wpt-any=window&variant=1",
                ANY_JS_DEDICATED_WORKER_GLOBAL,
            ),
            "wasm/jsapi/feature.any.js?variant=1&moli-wpt-any=dedicatedworker",
        )

    def test_fixture_server_resolves_any_js_meta_scripts_within_wpt_root(self) -> None:
        self.assertEqual(
            _resolve_wpt_static_script_url(
                "wasm/jsapi/feature.any.js",
                "../support/helper.js?mode=1",
            ),
            "/wasm/support/helper.js?mode=1",
        )
        self.assertEqual(
            _resolve_wpt_static_script_url(
                "wasm/jsapi/feature.any.js",
                "/resources/WebIDLParser.js",
            ),
            "/resources/WebIDLParser.js",
        )
        self.assertIsNone(
            _resolve_wpt_static_script_url(
                "wasm/jsapi/feature.any.js",
                "../../../escape.js",
            )
        )

    def test_fixture_server_substitutes_core_sub_template_variables(self) -> None:
        body = (
            b"http://{{domains[www2]}}:{{ports[http][0]}}/"
            b" location={{location[port]}}"
            b" scheme={{location[scheme]}}"
            b" hostname={{location[hostname]}}"
            b" server={{location[server]}}"
            b" path={{location[path]}}"
            b" alt={{ports[http][1]}}"
            b" same={{domains[www]}} host={{host}}"
            b" hosts={{hosts[][]}}/{{hosts[][www]}}/{{hosts[alt][]}}/{{hosts[alt][www]}}"
            b" https={{ports[https][0]}}/{{ports[https][1]}}"
            b" https-url=https://{{domains[www]}}:{{ports[https][0]}}/secure/"
            b" https-remote=https://{{domains[www1]}}:{{ports[https][0]}}/remote/"
            b" https-hosts-remote=https://{{hosts[][www]}}:{{ports[https][1]}}/cross/"
            b" https-hosts-bare=https://{{hosts[][]}}:{{ports[https][0]}}/bare/"
            b" https-hosts-alt=https://{{hosts[alt][]}}:{{ports[https][0]}}/alt/"
            b" https-hosts-alt-www=https://{{hosts[alt][www]}}:{{ports[https][1]}}/alt-www/"
            b" ws={{ports[ws][0]}}/{{ports[ws][1]}}"
            b" wss={{ports[wss][0]}}/{{ports[wss][1]}}"
            b" ws-url=ws://{{host}}:{{ports[ws][0]}}/socket"
            b" wss-url=wss://{{host}}:{{ports[wss][0]}}/socket"
        )

        self.assertEqual(
            _substitute_wpt_template_variables(
                body,
                port=12345,
                alternate_port=23456,
                request_path="/secure-contexts/server-locations.sub.js",
                request_hostname="example.test",
            ),
            b"http://www2.localhost:12345/ location=12345 scheme=http hostname=example.test"
            b" server=http://example.test:12345 path=/secure-contexts/server-locations.sub.js"
            b" alt=23456 same=www.localhost host=example.test"
            b" hosts=localhost/www.localhost/alt.localhost/www.alt.localhost https=12345/23456"
            b" https-url=http://example.test:12345/secure/"
            b" https-remote=http://example.test:23456/remote/"
            b" https-hosts-remote=http://www.localhost:23456/cross/"
            b" https-hosts-bare=http://localhost:12345/bare/"
            b" https-hosts-alt=http://alt.localhost:12345/alt/"
            b" https-hosts-alt-www=http://www.alt.localhost:23456/alt-www/"
            b" ws=12345/23456 wss=12345/23456"
            b" ws-url=ws://example.test:12345/socket"
            b" wss-url=ws://example.test:12345/socket",
        )

    def test_fixture_server_substitutes_idna_domain_aliases(self) -> None:
        cases = [
            ("{{domains[天気の良い日]}}", b"xn--n8j6ds53lwwkrqhv28a.localhost"),
            ("{{hosts[][天気の良い日]}}", b"xn--n8j6ds53lwwkrqhv28a.localhost"),
            ("{{domains[élève]}}", b"xn--lve-6lad.localhost"),
            ("{{hosts[alt][élève]}}", b"xn--lve-6lad.alt.localhost"),
            ("{{domains[www.élève]}}", b"www.xn--lve-6lad.localhost"),
            ("{{hosts[alt][www1.www2]}}", b"www1.www2.alt.localhost"),
            ("{{hosts[alt][www2]}}", b"www2.alt.localhost"),
            ("{{hosts[unknown][élève]}}", "{{hosts[unknown][élève]}}".encode()),
            ("{{domains[unconfigured]}}", b"{{domains[unconfigured]}}"),
        ]
        for marker, expected in cases:
            with self.subTest(marker=marker):
                self.assertEqual(
                    _substitute_wpt_template_variables(
                        b"\xff" + marker.encode("utf-8") + b"\xfe",
                        port=12345,
                        request_hostname="www1.localhost",
                        primary_hostname="localhost",
                    ),
                    b"\xff" + expected + b"\xfe",
                )

    def test_fixture_server_serves_idna_domain_aliases_in_body_and_headers(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text("", encoding="utf-8")
            body = (
                "{{domains[élève]}} {{hosts[alt][天気の良い日]}} {{GET[value]}}"
            ).encode("utf-8")
            for name in ("aliases.sub.js", "aliases.txt"):
                (root_path / name).write_bytes(body)
                (root_path / (name + ".sub.headers")).write_text(
                    "Access-Control-Allow-Origin: http://{{hosts[alt][élève]}}:{{ports[http][0]}}\n",
                    encoding="utf-8",
                )
            with WptFixtureServer(root_path) as server:
                for path in ("aliases.sub.js?", "aliases.txt?pipe=sub&"):
                    with self.subTest(path=path):
                        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
                        try:
                            connection.request(
                                "GET", "/" + path + "value=%7B%7Bdomains%5Bwww%5D%7D%7D",
                                headers={"Host": f"www1.localhost:{server.port}"},
                            )
                            response = connection.getresponse()
                            self.assertEqual(response.status, 200)
                            self.assertEqual(
                                response.read(),
                                b"xn--lve-6lad.localhost "
                                b"xn--n8j6ds53lwwkrqhv28a.alt.localhost {{domains[www]}}",
                            )
                            self.assertEqual(
                                response.getheader("Access-Control-Allow-Origin"),
                                f"http://xn--lve-6lad.alt.localhost:{server.port}",
                            )
                        finally:
                            connection.close()

    def test_fixture_server_uses_configured_primary_hostname_for_aliases(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text("", encoding="utf-8")
            (root_path / "hosts.sub.txt").write_text(
                "{{host}}|{{domains[]}}|{{hosts[][]}}|{{domains[www1]}}|"
                "{{hosts[][élève]}}|{{hosts[alt][www2]}}|{{location[hostname]}}",
                encoding="utf-8",
            )
            (root_path / "hosts.sub.txt.sub.headers").write_text(
                "Access-Control-Allow-Origin: http://{{hosts[][]}}:{{ports[http][0]}}\n",
                encoding="utf-8",
            )
            with WptFixtureServer(root_path, primary_hostname="web-platform.localhost") as server:
                self.assertEqual(server.base_url, f"http://web-platform.localhost:{server.port}")
                self.assertEqual(
                    server.alternate_base_url,
                    f"http://web-platform.localhost:{server.alternate_port}",
                )
                for hostname in ("web-platform.localhost", "www1.web-platform.localhost"):
                    with self.subTest(hostname=hostname):
                        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
                        try:
                            connection.request(
                                "GET", "/hosts.sub.txt",
                                headers={"Host": f"{hostname}:{server.port}"},
                            )
                            response = connection.getresponse()
                            self.assertEqual(response.status, 200)
                            self.assertEqual(
                                response.read().decode(),
                                "web-platform.localhost|web-platform.localhost|web-platform.localhost|"
                                "www1.web-platform.localhost|xn--lve-6lad.web-platform.localhost|"
                                f"www2.alt.localhost|{hostname}",
                            )
                            self.assertEqual(
                                response.getheader("Access-Control-Allow-Origin"), server.base_url,
                            )
                        finally:
                            connection.close()

    def test_fixture_server_pipe_sub_requests_template_substitution(self) -> None:
        self.assertTrue(
            _needs_wpt_template_substitution(
                "sandboxed-tests.html",
                b'importScripts("http://{{host}}:{{ports[http][0]}}/resources/testharness.js");',
                "pipe=sub",
            )
        )
        self.assertTrue(
            _needs_wpt_template_substitution(
                "resource.html",
                b"{{host}}",
                "pipe=header(X-Test,yes)|sub",
            )
        )
        self.assertFalse(
            _needs_wpt_template_substitution(
                "resource.html",
                b"{{host}}",
                "",
            )
        )

    def test_fixture_server_maps_external_ipv6_domain_location_port_to_remote_port(
        self,
    ) -> None:
        body = (
            b"http://{{domains[www]}}:{{location[port]}}/a"
            b" http://{{domains[www2]}}:{{location[port]}}/b"
            b" http://{{domains[www]}}:{{ports[http][0]}}/c"
            b" http://{{domains[www1]}}:{{ports[http][0]}}/d"
            b" http://{{domains[www1]}}:{{ports[http][1]}}/e"
        )

        self.assertEqual(
            _substitute_wpt_template_variables(
                body,
                port=12345,
                alternate_port=23456,
                remote_port=34567,
                request_hostname="[2001:db8::1]",
            ),
            b"http://[2001:db8::1]:34567/a"
            b" http://[2001:db8::1]:34567/b"
            b" http://[2001:db8::1]:34567/c"
            b" http://[2001:db8::1]:34567/d"
            b" http://[2001:db8::1]:23456/e",
        )

    def test_fixture_server_substitutes_get_query_template_variables(self) -> None:
        body = (
            b"var expected_logs = {{GET[logs]}};"
            b" var timeout = \"{{GET[timeout]}}\";"
            b" var missing = \"{{GET[missing]}}\";"
        )

        self.assertEqual(
            _substitute_wpt_template_variables(
                body,
                port=12345,
                query='logs=["xhr allowed","TEST COMPLETE"]&timeout=2',
            ),
            b'var expected_logs = ["xhr allowed","TEST COMPLETE"];'
            b' var timeout = "2";'
            b' var missing = "";',
        )

    def test_fixture_server_substitutes_template_variables_in_sidecar_headers(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            fixture = Path(root) / "frame-ancestors.sub.html"
            fixture.write_text("<!doctype html>", encoding="utf-8")
            fixture.with_name("frame-ancestors.sub.html.sub.headers").write_text(
                "Content-Security-Policy: frame-ancestors {{GET[policy]}} {{location[scheme]}}://{{location[host]}}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                _static_response_headers(
                    fixture,
                    "policy=%27none%27",
                    port=12345,
                    alternate_port=23456,
                    request_hostname="example.test",
                ),
                [
                    (
                        "Content-Security-Policy",
                        "frame-ancestors 'none' http://example.test:12345",
                    )
                ],
            )

    def test_fixture_server_substitution_preserves_non_utf8_bytes(self) -> None:
        body = b"\xff{{host}}\xfe{{ports[http][0]}}"

        self.assertEqual(
            _substitute_wpt_template_variables(body, port=12345),
            b"\xfflocalhost\xfe12345",
        )

    def test_fixture_server_distinguishes_primary_host_from_request_hostname(
        self,
    ) -> None:
        body = (
            b"host={{host}} domain={{domains[]}} "
            b"location={{location[hostname]}} "
            b"HTTP_ORIGIN: 'http://' + ORIGINAL_HOST + HTTP_PORT_ELIDED,"
        )

        self.assertEqual(
            _substitute_wpt_template_variables(
                body,
                port=12345,
                request_hostname="www1.localhost",
                primary_hostname="localhost",
            ),
            b"host=localhost domain=localhost location=www1.localhost "
            b"HTTP_ORIGIN: 'http://' + ORIGINAL_HOST + HTTP_PORT_ELIDED,",
        )

    def test_host_header_hostname_preserves_ipv6_brackets(self) -> None:
        self.assertEqual(_host_header_hostname("[2001:db8::1]:1234"), "[2001:db8::1]")
        self.assertEqual(_host_header_hostname("localhost:1234"), "localhost")
        self.assertEqual(_host_header_hostname("example.test"), "example.test")

    def test_fixture_server_substitutes_ipv6_websocket_template_urls(self) -> None:
        body = (
            b"ws=ws://{{host}}:{{ports[ws][0]}}/echo"
            b" wss=wss://{{host}}:{{ports[wss][0]}}/echo"
        )

        self.assertEqual(
            _substitute_wpt_template_variables(
                body,
                port=12345,
                request_hostname="[2001:db8::1]",
            ),
            b"ws=ws://[2001:db8::1]:12345/echo"
            b" wss=ws://[2001:db8::1]:12345/echo",
        )

    def test_fixture_server_wasm_status_handler_normalizes_status_codes(self) -> None:
        self.assertEqual(_wasm_webapi_status_code("status=404"), 404)
        self.assertEqual(_wasm_webapi_status_code("status=300"), 300)
        self.assertEqual(_wasm_webapi_status_code("status=0"), 599)
        self.assertEqual(_wasm_webapi_status_code("status=700"), 599)
        self.assertEqual(_wasm_webapi_status_code("status=not-a-number"), 400)
        self.assertEqual(_wasm_webapi_status_code(""), 200)

    def test_fixture_server_redirect_handler_models_wasm_origin_probe(self) -> None:
        self.assertEqual(
            _redirect_fixture_response(
                "redirect_status=301&location=/wasm/incrementer.wasm"
            ),
            (301, "/wasm/incrementer.wasm"),
        )
        self.assertIsNone(_redirect_fixture_response("redirect_status=200&location=/x"))
        self.assertIsNone(_redirect_fixture_response("redirect_status=301"))
        self.assertIsNone(
            _redirect_fixture_response("redirect_status=301&location=/x%0Dbad")
        )
        self.assertEqual(
            _redirect_fixture_response("status=307&location=/target"),
            (307, "/target"),
        )

    def test_fixture_server_models_csp_resource_py(self) -> None:
        body, headers = _content_security_policy_resource_response()

        self.assertIn(b"success", body)
        self.assertIn(("Access-Control-Allow-Origin", "*"), headers)

    def test_fixture_server_models_workers_modules_export_on_load_script_py(self) -> None:
        body, headers = _workers_modules_export_on_load_script_response()

        self.assertIn(b"export const importedModules", body)
        self.assertIn(("Content-Type", "text/javascript"), headers)
        self.assertIn(("Access-Control-Allow-Origin", "*"), headers)

    def test_fixture_server_preserves_raw_sidecar_header_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            resource = root / "resource.webmanifest"
            resource.write_bytes(b"{}")
            resource.with_name(resource.name + ".headers").write_bytes(
                b"Content-Type: \xc3\x97\xc2\xba invalid\n"
            )

            headers = _sidecar_response_headers(resource)

        self.assertEqual(headers, [("Content-Type", "\xc3\x97\xc2\xba invalid")])
        for _name, value in headers:
            value.encode("latin-1")

    def test_fixture_server_substitutes_get_host_info_remote_host_for_loopback(self) -> None:
        body = (
            b"var REMOTE_HOST = (ORIGINAL_HOST === 'localhost') ? "
            b"'127.0.0.1' : ('www1.' + ORIGINAL_HOST);"
            b"\nHTTP_REMOTE_ORIGIN: 'http://' + REMOTE_HOST + HTTP_PORT_ELIDED,"
            b"\nREMOTE_ORIGIN: PROTOCOL + \"//\" + REMOTE_HOST + PORT_ELIDED,"
            b"\nOTHER_ORIGIN: PROTOCOL + \"//\" + OTHER_HOST + PORT_ELIDED,"
            b"\nHTTP_NOTSAMESITE_ORIGIN: 'http://' + NOTSAMESITE_HOST + HTTP_PORT_ELIDED,"
            b"\nHTTPS_ORIGIN: 'https://' + ORIGINAL_HOST + HTTPS_PORT_ELIDED,"
            b"\nHTTPS_ORIGIN_WITH_CREDS: 'https://foo:bar@' + ORIGINAL_HOST + HTTPS_PORT_ELIDED,"
            b"\nHTTPS_REMOTE_ORIGIN: 'https://' + REMOTE_HOST + HTTPS_PORT_ELIDED,"
            b"\nHTTPS_REMOTE_ORIGIN_WITH_CREDS: 'https://foo:bar@' + REMOTE_HOST + HTTPS_PORT_ELIDED,"
        )

        self.assertEqual(
            _substitute_wpt_template_variables(
                body,
                port=12345,
                alternate_port=23456,
                remote_port=34567,
            ),
            b"var REMOTE_HOST = (ORIGINAL_HOST === 'localhost') ? "
            b"'www1.localhost' : ((ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"ORIGINAL_HOST : ('www1.' + ORIGINAL_HOST));"
            b"\nHTTP_REMOTE_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + REMOTE_HOST + ':23456') : ('http://' + REMOTE_HOST + HTTP_PORT_ELIDED),"
            b"\nREMOTE_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + REMOTE_HOST + ':23456') : (PROTOCOL + \"//\" + REMOTE_HOST + PORT_ELIDED),"
            b"\nOTHER_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + ORIGINAL_HOST + ':34567') : (PROTOCOL + \"//\" + OTHER_HOST + PORT_ELIDED),"
            b"\nHTTP_NOTSAMESITE_ORIGIN: 'http://' + NOTSAMESITE_HOST + HTTP_PORT_ELIDED,"
            b"\nHTTPS_ORIGIN: 'http://' + ORIGINAL_HOST + HTTP_PORT2_ELIDED,"
            b"\nHTTPS_ORIGIN_WITH_CREDS: 'http://foo:bar@' + ORIGINAL_HOST + HTTP_PORT2_ELIDED,"
            b"\nHTTPS_REMOTE_ORIGIN: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://' + REMOTE_HOST + ':34567') : ('http://' + REMOTE_HOST + HTTP_PORT2_ELIDED),"
            b"\nHTTPS_REMOTE_ORIGIN_WITH_CREDS: (ORIGINAL_HOST.indexOf(':') !== -1) ? "
            b"('http://foo:bar@' + REMOTE_HOST + ':34567') : "
            b"('http://foo:bar@' + REMOTE_HOST + HTTP_PORT2_ELIDED),",
        )

    def test_fixture_server_uses_configured_primary_hostname_for_aliases(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text("", encoding="utf-8")
            (root_path / "hosts.sub.txt").write_text(
                "{{host}}|{{domains[]}}|{{hosts[][]}}|{{domains[www1]}}|"
                "{{hosts[][élève]}}|{{hosts[alt][www2]}}|{{location[hostname]}}",
                encoding="utf-8",
            )
            (root_path / "hosts.sub.txt.sub.headers").write_text(
                "Access-Control-Allow-Origin: http://{{hosts[][]}}:{{ports[http][0]}}\n",
                encoding="utf-8",
            )
            with WptFixtureServer(root_path, primary_hostname="web-platform.localhost") as server:
                self.assertEqual(server.base_url, f"http://web-platform.localhost:{server.port}")
                self.assertEqual(
                    server.alternate_base_url,
                    f"http://web-platform.localhost:{server.alternate_port}",
                )
                for hostname in ("web-platform.localhost", "www1.web-platform.localhost"):
                    with self.subTest(hostname=hostname):
                        connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
                        try:
                            connection.request(
                                "GET", "/hosts.sub.txt",
                                headers={"Host": f"{hostname}:{server.port}"},
                            )
                            response = connection.getresponse()
                            self.assertEqual(response.status, 200)
                            self.assertEqual(
                                response.read().decode(),
                                "web-platform.localhost|web-platform.localhost|web-platform.localhost|"
                                "www1.web-platform.localhost|xn--lve-6lad.web-platform.localhost|"
                                f"www2.alt.localhost|{hostname}",
                            )
                            self.assertEqual(
                                response.getheader("Access-Control-Allow-Origin"), server.base_url,
                            )
                        finally:
                            connection.close()

    def test_fixture_server_substitutes_primary_domain_in_cors_sidecar(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            fixture = Path(root) / "style.css"
            fixture.write_text("body {}", encoding="utf-8")
            fixture.with_name("style.css.sub.headers").write_text(
                "Access-Control-Allow-Origin: {{location[scheme]}}://{{domains[]}}{{GET[acao_port]}}\n"
                "Access-Control-Allow-Credentials: true\n",
                encoding="utf-8",
            )

            for port in (80, 12345):
                with self.subTest(port=port):
                    suffix = "" if port == 80 else f":{port}"
                    query = "" if port == 80 else f"acao_port=%3A{port}"
                    self.assertEqual(
                        _static_response_headers(
                            fixture,
                            query,
                            port=port,
                            request_hostname="www.example.test",
                            primary_hostname="example.test",
                        ),
                        [
                            ("Access-Control-Allow-Origin", f"http://example.test{suffix}"),
                            ("Access-Control-Allow-Credentials", "true"),
                        ],
                    )

    def test_fixture_server_substitutes_request_header_or_default(self) -> None:
        body = (
            b"present={{header_or_default(Referer, missing)}}"
            b" absent={{header_or_default(X-Absent, <missing>)}}"
            b" empty={{header_or_default(X-Empty, )}}"
        )

        self.assertEqual(
            _substitute_wpt_template_variables(
                body,
                port=12345,
                request_headers={
                    "referer": "https://example.test/path?a=1&b=2",
                },
            ),
            b"present=https://example.test/path?a=1&amp;b=2"
            b" absent=&lt;missing&gt; empty=",
        )
