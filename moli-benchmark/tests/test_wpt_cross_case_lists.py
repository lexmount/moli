from __future__ import annotations

from wpt_cross_test_support import *


class WptCrossCaseListsTests(WptCrossTestCase):
    def test_repo_case_lists_are_overwritten_for_primary_engine(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            out_dir = Path(temp_dir)
            matrix = [
                {
                    "case_path": "c/timeout.html",
                    "results": {
                        "chrome": {"status": "pass"},
                        "moli": {"status": "timeout"},
                    },
                },
                {
                    "case_path": "b/fail.html",
                    "results": {
                        "chrome": {"status": "pass"},
                        "moli": {"status": "fail"},
                    },
                },
                {
                    "case_path": "a/pass.html",
                    "results": {
                        "chrome": {"status": "fail"},
                        "moli": {"status": "pass"},
                    },
                },
                {
                    "case_path": "d/crash.html",
                    "results": {
                        "chrome": {"status": "pass"},
                        "moli": {"status": "crash"},
                    },
                },
                {
                    "case_path": "f/error.html",
                    "results": {
                        "chrome": {"status": "pass"},
                        "moli": {"status": "error"},
                    },
                },
                {
                    "case_path": "e/stalled.html",
                    "results": {
                        "chrome": {"status": "pass"},
                        "moli": {"status": "harness-stalled"},
                    },
                },
                {
                    "case_path": "g/missing.html",
                    "results": {
                        "chrome": {"status": "pass"},
                    },
                },
                {
                    "case_path": "h/unknown.html",
                    "results": {
                        "chrome": {"status": "pass"},
                        "moli": {"status": "unknown"},
                    },
                },
            ]

            _write_repo_case_lists(
                matrix,
                ["chrome", "moli"],
                case_list_dir=out_dir,
            )

            expected = {
                "passed-cases.txt": "a/pass.html\n",
                "failed-cases.txt": "b/fail.html\n",
                "timeout-cases.txt": "c/timeout.html\n",
                "crash-cases.txt": "d/crash.html\n",
                "harness-stalled-cases.txt": "e/stalled.html\n",
                "error-cases.txt": "f/error.html\n",
                "missing-cases.txt": "g/missing.html\n",
                "other-cases.txt": "h/unknown.html\n",
            }
            for file_name, content in expected.items():
                self.assertEqual((out_dir / file_name).read_text(encoding="utf-8"), content)

            _write_repo_case_lists(
                [
                    {
                        "case_path": "only/pass.html",
                        "results": {"moli": {"status": "pass"}},
                    }
                ],
                ["moli"],
                case_list_dir=out_dir,
            )

            self.assertEqual(
                (out_dir / "passed-cases.txt").read_text(encoding="utf-8"),
                "only/pass.html\n",
            )
            for file_name in CASE_LIST_FILES.values():
                expected_content = "only/pass.html\n" if file_name == "passed-cases.txt" else ""
                self.assertEqual(
                    (out_dir / file_name).read_text(encoding="utf-8"),
                    expected_content,
                )
    def test_repo_case_lists_only_refresh_for_full_runs(self) -> None:
        parser = _build_parser()
        full = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
            ]
        )
        explicit_case = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
                "--case",
                "custom-elements/Document-createElement.html",
            ]
        )
        dir_prefix = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
                "--dir-prefix",
                "shadow-dom",
            ]
        )
        limited = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
                "--limit",
                "1",
            ]
        )
        tentative = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
                "--include-tentative",
            ]
        )
        any_js = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
                "--any-js-global",
                "window",
            ]
        )
        layout = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
                "--profile",
                "layout",
            ]
        )
        all_profiles = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
                "--profile",
                "all",
            ]
        )

        self.assertTrue(_is_full_case_list_run(full))
        self.assertTrue(_is_full_case_list_run(all_profiles))
        self.assertFalse(_is_full_case_list_run(layout))
        self.assertFalse(_is_full_case_list_run(explicit_case))
        self.assertFalse(_is_full_case_list_run(dir_prefix))
        self.assertFalse(_is_full_case_list_run(limited))
        self.assertFalse(_is_full_case_list_run(tentative))
        self.assertFalse(_is_full_case_list_run(any_js))
    def test_main_does_not_refresh_repo_case_lists_for_non_full_runs(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            manifest = root / "known.json"
            output_dir = root / "out"
            repo_case_lists = root / "repo-case-lists"
            repo_case_lists.mkdir()
            (repo_case_lists / "passed-cases.txt").write_text(
                "existing/full-case.html\n",
                encoding="utf-8",
            )
            manifest.write_text(
                json.dumps({"engine": "moli", "rules": []}),
                encoding="utf-8",
            )

            code = self._run_wpt_cross_with_fake_moli(
                output_dir=output_dir,
                known_failures=manifest,
                case_status="pass",
                extra_args=["--limit", "1"],
            )

            self.assertEqual(code, 0)
            self.assertEqual(
                (repo_case_lists / "passed-cases.txt").read_text(encoding="utf-8"),
                "existing/full-case.html\n",
            )
            self.assertFalse((repo_case_lists / "failed-cases.txt").exists())
    def test_case_origin_classification_preserves_secure_context_signal(self) -> None:
        worker_case = any_js_case_path_for_global(
            "WebCryptoAPI/idlharness.https.any.js",
            ANY_JS_DEDICATED_WORKER_GLOBAL,
        )
        secure_window_case = any_js_case_path_for_global(
            "WebCryptoAPI/digest/digest.https.any.js",
            ANY_JS_WINDOW_GLOBAL,
        )
        nonsecure_window_case = any_js_case_path_for_global(
            "WebCryptoAPI/historical.any.js",
            ANY_JS_WINDOW_GLOBAL,
        )

        self.assertTrue(_case_requires_trustworthy_origin(worker_case))
        self.assertTrue(_case_requires_trustworthy_origin(secure_window_case))
        self.assertFalse(_case_requires_trustworthy_origin(nonsecure_window_case))
    def test_url_for_case_origin_uses_non_loopback_only_for_secure_context_negative_cases(
        self,
    ) -> None:
        class FakeServer:
            external_base_url = "http://[2001:db8::1]:9000"

            def url_for_case(self, case_path: str, *, external: bool = False) -> str:
                base = self.external_base_url if external else "http://127.0.0.1:8000"
                return f"{base}/{case_path}"

        secure_case = any_js_case_path_for_global(
            "WebCryptoAPI/digest/digest.https.any.js",
            ANY_JS_WINDOW_GLOBAL,
        )
        nonsecure_case = any_js_case_path_for_global(
            "WebCryptoAPI/historical.any.js",
            ANY_JS_WINDOW_GLOBAL,
        )
        secure_context_negative_case = "secure-contexts/basic-shared-worker.html"
        audio_output_negative_case = "audio-output/secure-context.html"
        credential_management_negative_case = (
            "credential-management/require_securecontext.html"
        )
        explicit_http_case = "digital-credentials/non-secure-contexts.http.html"
        underscore_insecure_context_case = "web-nfc/nfc_insecure_context.html"
        pointer_event_negative_case = "pointerevents/pointerevent_constructor.html"
        navigator_negative_case = (
            "html/webappapis/system-state-and-capabilities/"
            "the-navigator-object/secure_context.html"
        )
        host_sensitive_case = "webmessaging/with-ports/020.html"

        self.assertTrue(_case_requires_trustworthy_origin(secure_case))
        self.assertFalse(_case_requires_trustworthy_origin(nonsecure_case))
        self.assertTrue(
            _case_requires_non_trustworthy_origin(secure_context_negative_case)
        )
        self.assertTrue(_case_requires_non_trustworthy_origin(audio_output_negative_case))
        self.assertTrue(
            _case_requires_non_trustworthy_origin(credential_management_negative_case)
        )
        self.assertTrue(_case_requires_non_trustworthy_origin(explicit_http_case))
        self.assertTrue(
            _case_requires_non_trustworthy_origin(underscore_insecure_context_case)
        )
        self.assertTrue(_case_requires_non_trustworthy_origin(pointer_event_negative_case))
        for suffix in ("", "?variant=test", "#probe", "?variant=test#probe"):
            with self.subTest(navigator_suffix=suffix):
                case = navigator_negative_case + suffix
                self.assertFalse(_case_requires_trustworthy_origin(case))
                self.assertTrue(_case_requires_non_trustworthy_origin(case))
                for external in (False, True):
                    self.assertEqual(
                        _url_for_case_origin(FakeServer(), case, external=external),
                        f"http://[2001:db8::1]:9000/{case}",
                    )
        self.assertFalse(_case_requires_non_trustworthy_origin(host_sensitive_case))
        self.assertEqual(
            _url_for_case_origin(FakeServer(), secure_case, external=False),
            f"http://127.0.0.1:8000/{secure_case}",
        )
        self.assertEqual(
            _url_for_case_origin(FakeServer(), nonsecure_case, external=False),
            f"http://127.0.0.1:8000/{nonsecure_case}",
        )
        self.assertEqual(
            _url_for_case_origin(
                FakeServer(), secure_context_negative_case, external=False
            ),
            f"http://[2001:db8::1]:9000/{secure_context_negative_case}",
        )
        self.assertEqual(
            _url_for_case_origin(FakeServer(), audio_output_negative_case, external=False),
            f"http://[2001:db8::1]:9000/{audio_output_negative_case}",
        )
        self.assertEqual(
            _url_for_case_origin(
                FakeServer(), credential_management_negative_case, external=False
            ),
            f"http://[2001:db8::1]:9000/{credential_management_negative_case}",
        )
        self.assertEqual(
            _url_for_case_origin(FakeServer(), explicit_http_case, external=False),
            f"http://[2001:db8::1]:9000/{explicit_http_case}",
        )
        self.assertEqual(
            _url_for_case_origin(
                FakeServer(), underscore_insecure_context_case, external=False
            ),
            f"http://[2001:db8::1]:9000/{underscore_insecure_context_case}",
        )
        self.assertEqual(
            _url_for_case_origin(
                FakeServer(), pointer_event_negative_case, external=False
            ),
            f"http://[2001:db8::1]:9000/{pointer_event_negative_case}",
        )
        self.assertEqual(
            _url_for_case_origin(FakeServer(), host_sensitive_case, external=False),
            f"http://127.0.0.1:8000/{host_sensitive_case}",
        )
        self.assertEqual(
            _url_for_case_origin(FakeServer(), secure_case, external=True),
            f"http://[2001:db8::1]:9000/{secure_case}",
        )
    def test_cli_mode_preserves_case_origin_classification(self) -> None:
        calls = []
        navigator_negative_case = (
            "html/webappapis/system-state-and-capabilities/"
            "the-navigator-object/secure_context.html"
        )

        class FakeServer:
            def __init__(self, wpt_root: Path, *, primary_hostname: str) -> None:
                assert primary_hostname == "web-platform.localhost"
                self.wpt_root = Path(wpt_root)
                self.base_url = "http://127.0.0.1:8000"
                self.alternate_base_url = "http://127.0.0.1:8001"
                self.external_base_url = "http://[2001:db8::1]:9000"
                self.external_alternate_base_url = "http://[2001:db8::1]:9001"
                self.external_remote_base_url = "http://[2001:db8::1]:9002"
                self.external_host = "2001:db8::1"

            def __enter__(self) -> "FakeServer":
                return self

            def __exit__(self, *args: object) -> None:
                return None

            def set_harness_timeout_multipliers(
                self,
                multipliers: dict[str, float],
                *,
                default_multiplier: float,
            ) -> None:
                return None

            def url_for_case(self, case_path: str, *, external: bool = False) -> str:
                base = self.external_base_url if external else self.base_url
                return f"{base}/{case_path}"

        result_dict = {
            "engine": "moli",
            "binary": "/tmp/moli",
            "binary_sha256": "sha",
            "binary_version": "0.1.0",
            "endpoint": "cli:/tmp/moli",
            "ready_ms": None,
            "setup_error": None,
            "cases": [],
        }

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir) / "out"

            def fake_run_engine_on_cases_cli(**kwargs: object) -> SimpleNamespace:
                calls.append(kwargs["cases"])
                return SimpleNamespace(setup_error=None)

            with (
                patch(
                    "moli_benchmark.wpt_cross.__main__.enumerate_cases",
                    return_value=[
                        WptCase("secure-contexts/basic-shared-worker.html"),
                        WptCase("digital-credentials/non-secure-contexts.http.html"),
                        WptCase(navigator_negative_case),
                        WptCase("WebCryptoAPI/digest/digest.https.html"),
                    ],
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.build_driver",
                    return_value=SimpleNamespace(cli_fetch_command=["moli"]),
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.run_engine_on_cases_cli",
                    side_effect=fake_run_engine_on_cases_cli,
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.engine_result_to_dict",
                    return_value=result_dict,
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.REPO_CASE_LIST_DIR",
                    Path(temp_dir) / "repo-case-lists",
                ),
                patch("moli_benchmark.wpt_cross.server.WptFixtureServer", FakeServer),
                redirect_stdout(StringIO()),
                redirect_stderr(StringIO()),
            ):
                code = main(
                    [
                        "--wpt-root",
                        "/tmp/wpt",
                        "--engine",
                        "moli",
                        "--output-dir",
                        str(output_dir),
                    ]
                )

        self.assertEqual(code, 0)
        self.assertEqual(
            calls,
            [
                [
                    (
                        "secure-contexts/basic-shared-worker.html",
                        "http://[2001:db8::1]:9000/secure-contexts/basic-shared-worker.html",
                        WPT_CROSS_CASE_TIMEOUT_SECONDS,
                        1.0,
                    ),
                    (
                        "digital-credentials/non-secure-contexts.http.html",
                        "http://[2001:db8::1]:9000/digital-credentials/non-secure-contexts.http.html",
                        WPT_CROSS_CASE_TIMEOUT_SECONDS,
                        1.0,
                    ),
                    (
                        navigator_negative_case,
                        f"http://[2001:db8::1]:9000/{navigator_negative_case}",
                        WPT_CROSS_CASE_TIMEOUT_SECONDS,
                        1.0,
                    ),
                    (
                        "WebCryptoAPI/digest/digest.https.html",
                        "http://127.0.0.1:8000/WebCryptoAPI/digest/digest.https.html",
                        WPT_CROSS_CASE_TIMEOUT_SECONDS,
                        1.0,
                    )
                ]
            ],
        )
