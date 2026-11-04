from __future__ import annotations

from wpt_cross_test_support import *


class WptCrossRunnerOrchestrationTests(WptCrossTestCase):
    def test_main_writes_schedule_and_passes_execution_order_to_cli_runner(self) -> None:
        captured: dict[str, object] = {}

        class FakeServer:
            def __init__(self, wpt_root: Path, *, primary_hostname: str) -> None:
                assert primary_hostname == "web-platform.localhost"
                self.wpt_root = Path(wpt_root)
                self.base_url = "http://127.0.0.1:8000"
                self.alternate_base_url = "http://127.0.0.1:8001"
                self.external_base_url = None
                self.external_alternate_base_url = None
                self.external_remote_base_url = None
                self.external_host = None

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
                return f"{self.base_url}/{case_path}"

        cases = [
            WptCase("content-security-policy/navigation/a.html"),
            WptCase("content-security-policy/navigation/b.html"),
            WptCase("html/browsers/a.html"),
            WptCase("html/browsers/b.html"),
            WptCase("trusted-types/reporting/a.html"),
            WptCase("trusted-types/reporting/b.html"),
        ]
        scheduled_cases, expected_metadata = build_run_schedule(
            cases,
            case_path=lambda case: case.case_path,
        )
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

        def fake_run_engine_on_cases_cli(**kwargs: object) -> SimpleNamespace:
            captured["cases"] = kwargs["cases"]
            captured["execution_cases"] = kwargs["execution_cases"]
            captured["case_timeout_seconds"] = kwargs["case_timeout_seconds"]
            captured["parallelism"] = kwargs["parallelism"]
            return SimpleNamespace(setup_error=None)

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir) / "out"
            with (
                patch(
                    "moli_benchmark.wpt_cross.__main__.enumerate_cases",
                    return_value=cases,
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
                (output_dir / "cases.txt").read_text(encoding="utf-8").splitlines(),
                [case.case_path for case in cases],
            )
            self.assertEqual(
                (output_dir / "schedule.txt").read_text(encoding="utf-8").splitlines(),
                [case.case_path for case in scheduled_cases],
            )
            schedule_json = json.loads(
                (output_dir / "schedule.json").read_text(encoding="utf-8")
            )
            self.assertEqual(schedule_json, expected_metadata)
            self.assertEqual(
                schedule_json["mode"],
                "fixed-prefix-balanced-shuffle",
            )
        self.assertEqual(
            [case[0] for case in captured["cases"]],
            [case.case_path for case in cases],
        )
        self.assertEqual(
            [case[0] for case in captured["execution_cases"]],
            [case.case_path for case in scheduled_cases],
        )
        self.assertEqual(captured["case_timeout_seconds"], WPT_CROSS_CASE_TIMEOUT_SECONDS)
        self.assertEqual(captured["parallelism"], WPT_CROSS_PARALLELISM)
    def test_main_uses_requested_parallelism_for_cdp_runner(self) -> None:
        calls: list[dict[str, object]] = []

        class FakeServer:
            def __init__(self, wpt_root: Path, *, primary_hostname: str) -> None:
                assert primary_hostname == "web-platform.localhost"
                self.wpt_root = Path(wpt_root)
                self.base_url = "http://127.0.0.1:8000"
                self.alternate_base_url = "http://127.0.0.1:8001"
                self.external_base_url = None
                self.external_alternate_base_url = None
                self.external_remote_base_url = None
                self.external_host = None

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
                return f"{self.base_url}/{case_path}"

        def fake_run_engine_on_cases(**kwargs: object) -> EngineRunResult:
            cases_arg = kwargs["cases"]
            assert isinstance(cases_arg, list)
            calls.append(
                {
                    "cases": cases_arg,
                    "case_timeout_seconds": kwargs["case_timeout_seconds"],
                }
            )
            case_path, url, _timeout = cases_arg[0]
            return EngineRunResult(
                engine="chrome",
                binary="/tmp/chrome",
                binary_sha256="sha",
                binary_version="version",
                endpoint="cdp://127.0.0.1:1",
                ready_ms=1.0,
                cases=[CaseResult(case_path, url, "pass", 1.0)],
            )

        result_dict = {
            "engine": "chrome",
            "binary": "/tmp/chrome",
            "binary_sha256": "sha",
            "binary_version": "version",
            "endpoint": "cdp://127.0.0.1:1",
            "ready_ms": 1.0,
            "setup_error": None,
            "cases": [],
        }

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir) / "out"
            cases = [WptCase(f"case-{index}.html") for index in range(3)]
            with (
                patch(
                    "moli_benchmark.wpt_cross.__main__.enumerate_cases",
                    return_value=cases,
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.build_driver",
                    return_value=SimpleNamespace(cli_fetch_command=None),
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.run_engine_on_cases",
                    side_effect=fake_run_engine_on_cases,
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
                        "chrome",
                        "--output-dir",
                        str(output_dir),
                        "--mode",
                        "cdp",
                        "--parallelism",
                        "2",
                    ]
                )

        self.assertEqual(code, 0)
        self.assertEqual(len(calls), 2)
        self.assertTrue(all(call["case_timeout_seconds"] == WPT_CROSS_CASE_TIMEOUT_SECONDS for call in calls))
        self.assertEqual(
            sorted(case[0] for call in calls for case in call["cases"]),
            [case.case_path for case in cases],
        )
    def test_layout_testharness_profile_forces_cdp_and_fixed_viewport(self) -> None:
        captured: dict[str, object] = {}

        class FakeServer:
            def __init__(self, wpt_root: Path, *, primary_hostname: str) -> None:
                assert primary_hostname == "web-platform.localhost"
                self.wpt_root = Path(wpt_root)
                self.base_url = "http://127.0.0.1:8000"
                self.alternate_base_url = "http://127.0.0.1:8001"
                self.external_base_url = None
                self.external_alternate_base_url = None
                self.external_remote_base_url = None
                self.external_host = None

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
                return f"{self.base_url}/{case_path}"

        def fake_cdp_run(**kwargs: object) -> EngineRunResult:
            captured.update(kwargs)
            cases_arg = kwargs["cases"]
            assert isinstance(cases_arg, list)
            case_path, url, _timeout = cases_arg[0]
            return EngineRunResult(
                engine="moli",
                binary="/tmp/moli",
                binary_sha256="sha",
                binary_version="version",
                endpoint="cdp://127.0.0.1:1",
                ready_ms=1.0,
                cases=[CaseResult(case_path, url, "pass", 1.0)],
            )

        case = WptCase("css/css-flexbox/layout.html")
        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir) / "out"
            with (
                patch(
                    "moli_benchmark.wpt_cross.__main__.enumerate_cases",
                    return_value=[case],
                ) as enumerate_mock,
                patch(
                    "moli_benchmark.wpt_cross.__main__.build_driver",
                    return_value=SimpleNamespace(cli_fetch_command=lambda *_: []),
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.run_engine_on_cases",
                    side_effect=fake_cdp_run,
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.run_engine_on_cases_cli",
                    side_effect=AssertionError("layout profile must not use CLI mode"),
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
                        "--profile",
                        "layout-testharness",
                    ]
                )

            summary = json.loads((output_dir / "summary.json").read_text(encoding="utf-8"))

        self.assertEqual(code, 0)
        self.assertEqual(captured["viewport"], LAYOUT_VIEWPORT)
        self.assertIsNone(captured["artifact_output_dir"])
        self.assertTrue(enumerate_mock.call_args.kwargs["layout_static_only"])
        self.assertEqual(summary["profile"], "layout-testharness")
        self.assertEqual(summary["viewport"]["width"], 800)
    def test_main_uses_wpt_harness_multipliers_independent_of_outer_deadline(self) -> None:
        calls = []

        class FakeServer:
            def __init__(self, wpt_root: Path, *, primary_hostname: str) -> None:
                assert primary_hostname == "web-platform.localhost"
                self.wpt_root = Path(wpt_root)
                self.base_url = "http://127.0.0.1:8000"
                self.alternate_base_url = "http://127.0.0.1:8001"
                self.external_base_url = None
                self.external_alternate_base_url = None
                self.external_remote_base_url = None
                self.external_host = None

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
                calls.append((multipliers, default_multiplier))

            def url_for_case(self, case_path: str, *, external: bool = False) -> str:
                return f"{self.base_url}/{case_path}"

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
        cases = [
            WptCase("normal.html", timeout_multiplier=1.0),
            WptCase("long.html", timeout_multiplier=LONG_TIMEOUT_MULTIPLIER),
            WptCase(
                "html/semantics/scripting-1/the-script-element/module/dynamic-import/delay-load-event.html",
                timeout_multiplier=1.0,
            ),
        ]

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir) / "out"
            with (
                patch(
                    "moli_benchmark.wpt_cross.__main__.enumerate_cases",
                    return_value=cases,
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.build_driver",
                    return_value=SimpleNamespace(cli_fetch_command=["moli"]),
                ),
                patch(
                    "moli_benchmark.wpt_cross.__main__.run_engine_on_cases_cli",
                    return_value=SimpleNamespace(setup_error=None),
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
                (
                    {
                        "normal.html": 1.0,
                        "long.html": 1.0,
                        "html/semantics/scripting-1/the-script-element/module/dynamic-import/delay-load-event.html": 1.0,
                    },
                    1.0,
                )
            ],
        )
    def test_normal_dynamic_import_uses_normal_wpt_timeout(self) -> None:
        case = WptCase(
            "html/semantics/scripting-1/the-script-element/module/dynamic-import/delay-load-event.html"
        )

        self.assertEqual(_harness_timeout_multiplier(case), 1.0)
    def test_parser_rejects_removed_timeout_flags_and_legacy_parallelism(self) -> None:
        parser = _build_parser()
        base = [
            "--wpt-root",
            "/tmp/wpt",
            "--engine",
            "chrome",
            "--output-dir",
            "/tmp/out",
        ]
        for removed_flag in (
            "--case-timeout",
            "--case-timeout-engine",
            "--cdp-parallelism",
        ):
            with self.subTest(removed_flag=removed_flag), self.assertRaises(SystemExit):
                parser.parse_args([*base, removed_flag, "1"])

        for invalid_parallelism in ("0", "-1", "not-an-integer"):
            with self.subTest(invalid_parallelism=invalid_parallelism), self.assertRaises(
                SystemExit
            ):
                parser.parse_args(
                    [*base, "--parallelism", invalid_parallelism]
                )
    def test_main_matrix_preserves_harness_message(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            manifest = root / "known.json"
            output_dir = root / "out"
            manifest.write_text(
                json.dumps(
                    {
                        "engine": "moli",
                        "rules": [
                            {
                                "case_path": "known.html",
                                "expected_status": "fail",
                                "message_contains": "tracked message",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            code = self._run_wpt_cross_with_fake_moli(
                output_dir=output_dir,
                known_failures=manifest,
                case_status="fail",
                failure_message="expected 555 but got 100",
            )

            self.assertEqual(code, 0)
            matrix = json.loads((output_dir / "matrix.json").read_text(encoding="utf-8"))
            result = matrix[0]["results"]["moli"]
            self.assertEqual(
                result["harness_message"],
                "Harness completed with a tracked message",
            )
            audit = json.loads(
                (output_dir / "known-failure-audit-moli.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(
                audit["known_failures"][0]["harness_message"],
                "Harness completed with a tracked message",
            )
    def test_build_partial_preserves_diagnostic_fields(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            (output_dir / "engine-moli.json").write_text(
                json.dumps(
                    {
                        "cases": [
                            {
                                "case_path": "known.html",
                                "status": "fail",
                                "duration_ms": 1.0,
                                "subtests": {
                                    "total": 0,
                                    "pass": 0,
                                    "fail": 0,
                                    "timeout": 0,
                                    "notrun": 0,
                                },
                                "harness_status_name": "ERROR",
                                "harness_message": "Unhandled rejection: cycle",
                                "error": "testharness completed without reporting any subtests",
                                "test_type": "reftest",
                                "failures": [{"name": "== known-ref.html"}],
                                "failure_names": ["== known-ref.html"],
                                "reftest_comparisons": [
                                    {
                                        "reference_path": "known-ref.html",
                                        "relation": "==",
                                        "passed": False,
                                        "max_difference": 255,
                                        "different_pixels": 10,
                                    }
                                ],
                                "artifacts": {
                                    "test": "artifacts/moli/known/test.png",
                                    "references": [
                                        {
                                            "reference": "artifacts/moli/known/reference-01.png",
                                            "diff": "artifacts/moli/known/diff-01.png",
                                        }
                                    ],
                                },
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )

            with redirect_stdout(StringIO()):
                code = build_partial_main([str(output_dir), "--engine", "moli"])

            self.assertEqual(code, 0)
            matrix = json.loads(
                (output_dir / "matrix.partial.moli.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(
                matrix[0]["results"]["moli"]["harness_message"],
                "Unhandled rejection: cycle",
            )
            self.assertEqual(matrix[0]["test_type"], "reftest")
            result = matrix[0]["results"]["moli"]
            self.assertEqual(result["failure_names"], ["== known-ref.html"])
            self.assertEqual(result["reftest_comparisons"][0]["relation"], "==")
            self.assertEqual(
                result["artifacts"]["test"],
                "artifacts/moli/known/test.png",
            )
    def test_parser_accepts_known_failure_audit_options(self) -> None:
        parser = _build_parser()
        args = parser.parse_args(
            [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                "/tmp/out",
                "--known-failures",
                "/tmp/known.json",
                "--known-failures-engine",
                "moli",
                "--allow-missing-known-failures",
            ]
        )

        self.assertEqual(args.known_failures, Path("/tmp/known.json"))
        self.assertEqual(args.known_failures_engine, "moli")
        self.assertTrue(args.allow_missing_known_failures)
    def test_main_writes_known_failure_audit_when_manifest_matches(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            docs = root / "docs"
            wpt = root / "wpt"
            docs.mkdir()
            wpt.mkdir()
            (docs / "wasm-current.md").write_text("# wasm\n", encoding="utf-8")
            (wpt / "known.html").write_text("<!doctype html>", encoding="utf-8")
            manifest = root / "known.json"
            output_dir = root / "out"
            manifest.write_text(
                json.dumps(
                    {
                        "engine": "moli",
                        "categories": {
                            "wasm": {
                                "tracking_doc": "docs/wasm-current.md",
                                "scope": "tracked wasm failure",
                                "evidence": [
                                    {
                                        "kind": "doc",
                                        "path": "docs/wasm-current.md",
                                        "note": "local fixture evidence",
                                    },
                                    {
                                        "kind": "wpt",
                                        "path": "wpt/known.html",
                                        "note": "known failure source fixture",
                                    }
                                ],
                            }
                        },
                        "rules": [
                            {
                                "case_path": "known.html",
                                "category": "wasm",
                                "expected_status": "fail",
                                "message_contains": "expected 555",
                                "reason": "tracked wasm failure",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            code = self._run_wpt_cross_with_fake_moli(
                output_dir=output_dir,
                known_failures=manifest,
                case_status="fail",
            )

            self.assertEqual(code, 0)
            audit = json.loads(
                (output_dir / "known-failure-audit-moli.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertTrue(audit["ok"])
            self.assertEqual(audit["counts"]["known_failures"], 1)
            self.assertEqual(audit["categories"]["wasm"]["scope"], "tracked wasm failure")
            summary = json.loads((output_dir / "summary.json").read_text(encoding="utf-8"))
            self.assertTrue(summary["known_failure_audits"]["moli"]["ok"])
            self.assertEqual(
                summary["known_failure_audits"]["moli"]["categories"]["wasm"][
                    "tracking_doc"
                ],
                "docs/wasm-current.md",
            )
            self.assertEqual(
                summary["known_failure_audits"]["moli"]["category_counts"][
                    "known_failures"
                ],
                {"wasm": 1},
            )
    def test_main_can_skip_missing_known_failures_for_focused_runs(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            manifest = root / "known.json"
            output_dir = root / "out"
            manifest.write_text(
                json.dumps(
                    {
                        "engine": "moli",
                        "rules": [
                            {
                                "case_path": "known.html",
                                "expected_status": "fail",
                                "message_contains": "expected 555",
                            },
                            {
                                "case_path": "not-run.html",
                                "expected_status": "fail",
                            },
                        ],
                    }
                ),
                encoding="utf-8",
            )

            code = self._run_wpt_cross_with_fake_moli(
                output_dir=output_dir,
                known_failures=manifest,
                case_status="fail",
                allow_missing_known_failures=True,
            )

            self.assertEqual(code, 0)
            audit = json.loads(
                (output_dir / "known-failure-audit-moli.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertTrue(audit["ok"])
            self.assertEqual(audit["counts"]["known_failures"], 1)
            self.assertEqual(audit["counts"]["missing_expected_failures"], 0)
            self.assertEqual(audit["counts"]["skipped_known_failures"], 1)
            summary = json.loads((output_dir / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(
                summary["known_failure_audits"]["moli"]["counts"][
                    "skipped_known_failures"
                ],
                1,
            )
    def test_main_returns_nonzero_when_known_failure_audit_finds_unexpected_failure(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            manifest = root / "known.json"
            output_dir = root / "out"
            manifest.write_text(
                json.dumps({"engine": "moli", "rules": []}), encoding="utf-8"
            )

            code = self._run_wpt_cross_with_fake_moli(
                output_dir=output_dir,
                known_failures=manifest,
                case_status="fail",
            )

            self.assertEqual(code, 5)
            audit = json.loads(
                (output_dir / "known-failure-audit-moli.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertFalse(audit["ok"])
            self.assertEqual(audit["counts"]["unexpected_failures"], 1)
    def test_main_returns_nonzero_when_known_failure_is_resolved(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            manifest = root / "known.json"
            output_dir = root / "out"
            manifest.write_text(
                json.dumps(
                    {
                        "engine": "moli",
                        "rules": [
                            {
                                "case_path": "known.html",
                                "expected_status": "fail",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )

            code = self._run_wpt_cross_with_fake_moli(
                output_dir=output_dir,
                known_failures=manifest,
                case_status="pass",
            )

            self.assertEqual(code, 5)
            audit = json.loads(
                (output_dir / "known-failure-audit-moli.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertFalse(audit["ok"])
            self.assertEqual(audit["counts"]["resolved_known_failures"], 1)
