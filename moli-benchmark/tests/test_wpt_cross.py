from __future__ import annotations

import hashlib
import io
import json
import socket
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from http.client import HTTPConnection
from io import StringIO
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
from urllib.request import Request, urlopen

from PIL import Image

from moli_benchmark.config import clear_current_proxy_env
from moli_benchmark.wpt_cross.__main__ import (
    CASE_LIST_FILES,
    WPT_CROSS_CASE_TIMEOUT_SECONDS,
    WPT_CROSS_PARALLELISM,
    _build_parser,
    _case_requires_non_trustworthy_origin,
    _case_requires_trustworthy_origin,
    _deduplicate_cases,
    _harness_timeout_multiplier,
    _is_full_case_list_run,
    _recorded_failure_drift,
    _url_for_case_origin,
    _write_repo_case_lists,
    main,
)
from moli_benchmark.wpt_cross.build_partial import main as build_partial_main
from moli_benchmark.wpt_cross.case_set import (
    ANY_JS_WINDOW_QUERY,
    WINDOW_JS_WINDOW_QUERY,
    any_js_window_case_path,
    DEFAULT_EXCLUDE_DIR_PREFIXES,
    FuzzyTolerance,
    LAYOUT_PROFILE_DIR_PREFIXES,
    LONG_TIMEOUT_MULTIPLIER,
    ReftestReference,
    WptCase,
    enumerate_cases,
    enumerate_reftest_cases,
    explicit_reftest_case,
    explicit_case,
    parse_any_js_meta,
    window_js_window_case_path,
)
from moli_benchmark.wpt_cross.engine import (
    _moli_command,
    _moli_fetch,
    _lightpanda_fetch,
)
from moli_benchmark.wpt_cross.cli_runner import (
    MOLI_WPT_USER_AGENT,
    _CliCaseWorkerInput,
    _CliSubprocessResult,
    _classify_cli_case_result,
    _moli_fixture_host_resolve_args,
    _nonzero_exit_status,
    _payload_from_stdout_html,
    _payload_grace_for_process_result,
    _run_cli_case_worker,
    _stderr_tail,
    run_engine_on_cases_cli,
)
from moli_benchmark.wpt_cross.any_js import (
    ANY_JS_DEDICATED_WORKER_GLOBAL,
    ANY_JS_WINDOW_GLOBAL,
    any_js_case_path_for_global,
    any_js_source_script_path,
    any_js_worker_script_path,
)
from moli_benchmark.wpt_cross.render_html import render_html
from moli_benchmark.wpt_cross.runner import (
    _ReftestEvidence,
    _write_reftest_failure_artifacts,
    CapturedScreenshot,
    CaseResult,
    EngineRunResult,
    LAYOUT_VIEWPORT,
    ReftestReferenceRun,
    ReftestRun,
    case_result_to_dict,
    classify_payload,
    compare_reftest_screenshots,
    reftest_comparisons_pass,
    reftest_relation_passes,
)
from moli_benchmark.wpt_cross.scheduler import (
    FIXED_RUN_SHUFFLE_SEED,
    build_run_schedule,
)
from moli_benchmark.wpt_cross.server import (
    BENCH_REPORT_BRIDGE,
    BENCH_TESTDRIVER_VENDOR_BRIDGE,
    BENCH_TIMEOUT_MULTIPLIER_QUERY,
    WptFixtureServer,
    ResultsStore,
    _asis_response_parts,
    _any_js_window_wrapper,
    _bench_report_bridge,
    _content_security_policy_resource_response,
    _workers_modules_export_on_load_script_response,
    _inject_bench_report_bridge_config,
    _host_header_hostname,
    _headers_include,
    _normalize_harness_case_key,
    _needs_wpt_template_substitution,
    _legacy_wpt_resource_alias,
    _pipe_response_headers,
    _pipe_response_status,
    _pipe_trickle_delay_seconds,
    _redirect_fixture_response,
    _response_content_type_and_extra_headers,
    _resolve_wpt_static_script_url,
    _sidecar_response_headers,
    _static_response_header_block,
    _static_response_headers,
    _substitute_wpt_template_variables,
    _window_js_window_wrapper,
    _wasm_webapi_status_code,
    _wpt_delay_seconds,
    _wpt_dedicated_worker_js_wrapper_html,
    _wpt_any_dedicated_worker_wrapper_html,
    _wpt_any_dedicated_worker_wrapper_js,
    _wpt_any_window_wrapper_html,
    _wpt_window_js_wrapper_html,
)


# WPT fixture tests use loopback servers; inherited shell proxies can intercept
# urllib requests to those servers, so normalize the script process once.
clear_current_proxy_env()


class WptCrossTests(unittest.TestCase):
    def test_moli_wpt_commands_enable_layout_and_resources(self) -> None:
        self.assertEqual(
            _moli_command(Path("/bin/moli"), 9222, None),
            [
                "/bin/moli",
                "serve",
                "--layout",
                "--resource",
                "--host",
                "127.0.0.1",
                "--port",
                "9222",
            ],
        )
        fetch = _moli_fetch(
            Path("/bin/moli"),
            "http://127.0.0.1:8000/case.html",
            30.0,
        )
        self.assertEqual(
            fetch[:5],
            [
                "/bin/moli",
                "fetch",
                "--layout",
                "--resource",
                "http://127.0.0.1:8000/case.html",
            ],
        )

    def test_harness_timeout_multiplier_never_shortens_wpt_delays(self) -> None:
        self.assertEqual(_harness_timeout_multiplier(8.0, 10.0), 1.0)
        self.assertEqual(_harness_timeout_multiplier(10.0, 10.0), 1.0)
        self.assertEqual(_harness_timeout_multiplier(50.0, 10.0), 5.0)

    def test_lightpanda_cli_fetch_keeps_configured_timeout(self) -> None:
        command = _lightpanda_fetch(Path("/bin/lightpanda"), "http://127.0.0.1:8000/case.html", 30.0)

        self.assertEqual(command[:4], ["/bin/lightpanda", "fetch", "http://127.0.0.1:8000/case.html", "--dump"])
        self.assertEqual(command[command.index("--wait-until") + 1], "done")
        self.assertEqual(command[command.index("--wait-ms") + 1], "30000")
        self.assertEqual(command[command.index("--http-timeout") + 1], "30000")
        self.assertEqual(command[command.index("--terminate-ms") + 1], "30000")
        self.assertNotIn("--wait_until", command)
        self.assertNotIn("--wait_ms", command)
        self.assertNotIn("--http_timeout", command)

    def test_parser_hides_fixed_timeout_and_parallelism_knobs(self) -> None:
        parser = _build_parser()
        help_text = parser.format_help()

        self.assertEqual(WPT_CROSS_CASE_TIMEOUT_SECONDS, 120.0)
        self.assertEqual(WPT_CROSS_PARALLELISM, 100)
        self.assertNotIn("--case-timeout", help_text)
        self.assertNotIn("--case-timeout-engine", help_text)
        self.assertNotIn("--parallelism", help_text)
        self.assertNotIn("--cdp-parallelism", help_text)
        self.assertNotIn("--run-order", parser.format_help())
        self.assertNotIn("--shuffle-seed", parser.format_help())

    def test_layout_profiles_are_explicit_and_keep_fixed_parallelism(self) -> None:
        parser = _build_parser()
        args = parser.parse_args(
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

        self.assertEqual(args.profile, "layout")
        self.assertEqual(
            LAYOUT_PROFILE_DIR_PREFIXES,
            (
                "css/css-flexbox",
                "css/css-grid",
                "css/css-sizing",
                "css/cssom-view",
            ),
        )
        self.assertEqual(
            (LAYOUT_VIEWPORT.width, LAYOUT_VIEWPORT.height),
            (800, 600),
        )
        self.assertEqual(LAYOUT_VIEWPORT.device_scale_factor, 1.0)
        self.assertEqual(WPT_CROSS_PARALLELISM, 100)

    def test_all_profile_matrix_deduplicates_default_and_layout_cases(self) -> None:
        semantic = WptCase("css/cssom-view/shared.html")
        duplicate_layout = WptCase("css/cssom-view/shared.html")
        reftest = WptCase(
            "css/css-grid/reference.html",
            test_type="reftest",
            references=(
                ReftestReference("css/css-grid/reference-ref.html", "=="),
            ),
        )

        merged = _deduplicate_cases([semantic, duplicate_layout, reftest])

        self.assertEqual(
            [case.case_path for case in merged],
            ["css/css-grid/reference.html", "css/cssom-view/shared.html"],
        )
        self.assertEqual(merged[0].test_type, "reftest")

    def test_manifest_reftest_enumeration_supports_relations_fuzzy_and_filters(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)

            def write_document(rel: str, body: str = "<!doctype html><p>static</p>") -> None:
                path = root / rel
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(body, encoding="utf-8")

            documents = {
                "css/css-flexbox/static.html": "<!doctype html><meta name=timeout content=long><p>test</p>",
                "css/css-flexbox/ref.html": "<!doctype html><p>reference</p>",
                "css/css-flexbox/notref.html": "<!doctype html><p>not reference</p>",
                "css/css-flexbox/animation/dynamic.html": "<!doctype html><style>p { animation: pulse 1s }</style>",
                "css/css-flexbox/media.html": "<!doctype html><video></video>",
                "css/css-flexbox/server.html": "<!doctype html><script src='/handler.py'></script>",
                "css/css-flexbox/protocol.h2.html": "<!doctype html><p>h2</p>",
                "css/css-flexbox/driver.html": "<!doctype html><p>driver</p>",
            }
            for rel, body in documents.items():
                write_document(rel, body)

            reftests: dict[str, object] = {}

            def add_manifest_item(rel: str, item: list[object]) -> None:
                node = reftests
                parts = rel.split("/")
                for part in parts[:-1]:
                    child = node.setdefault(part, {})
                    assert isinstance(child, dict)
                    node = child
                node[parts[-1]] = ["sha", item]

            add_manifest_item(
                "css/css-flexbox/static.html",
                [
                    None,
                    [
                        ["/css/css-flexbox/ref.html", "=="],
                        ["/css/css-flexbox/notref.html", "!="],
                    ],
                    {
                        "timeout": "long",
                        "fuzzy": [
                            [None, [[0, 1], [0, 2]]],
                            [
                                [
                                    "/css/css-flexbox/static.html",
                                    "/css/css-flexbox/notref.html",
                                    "!=",
                                ],
                                [[0, 3], [0, 4]],
                            ],
                        ],
                    },
                ],
            )
            for rel in (
                "css/css-flexbox/animation/dynamic.html",
                "css/css-flexbox/media.html",
                "css/css-flexbox/server.html",
                "css/css-flexbox/protocol.h2.html",
            ):
                add_manifest_item(
                    rel,
                    [None, [["/css/css-flexbox/ref.html", "=="]], {}],
                )
            add_manifest_item(
                "css/css-flexbox/driver.html",
                [
                    None,
                    [["/css/css-flexbox/ref.html", "=="]],
                    {"testdriver": True},
                ],
            )
            (root / "MANIFEST.json").write_text(
                json.dumps(
                    {
                        "version": 9,
                        "url_base": "/",
                        "items": {"reftest": reftests},
                    }
                ),
                encoding="utf-8",
            )

            cases = enumerate_reftest_cases(
                root,
                dir_prefixes=("css/css-flexbox",),
            )

            self.assertEqual([case.case_path for case in cases], ["css/css-flexbox/static.html"])
            case = cases[0]
            self.assertEqual(case.test_type, "reftest")
            self.assertEqual(case.timeout_multiplier, LONG_TIMEOUT_MULTIPLIER)
            self.assertEqual(
                case.references,
                (
                    ReftestReference(
                        "css/css-flexbox/ref.html",
                        "==",
                        FuzzyTolerance((0, 1), (0, 2)),
                    ),
                    ReftestReference(
                        "css/css-flexbox/notref.html",
                        "!=",
                        FuzzyTolerance((0, 3), (0, 4)),
                    ),
                ),
            )
            self.assertEqual(
                explicit_reftest_case(root, "css/css-flexbox/static.html"),
                case,
            )

    def test_reftest_pixel_comparison_supports_exact_and_fuzzy_bounds(self) -> None:
        def captured(image: Image.Image) -> CapturedScreenshot:
            stream = io.BytesIO()
            image.save(stream, format="PNG")
            png = stream.getvalue()
            return CapturedScreenshot(
                png=png,
                sha256=hashlib.sha256(png).hexdigest(),
                width=image.width,
                height=image.height,
            )

        test_image = Image.new("RGB", (4, 4), (0, 0, 0))
        reference_image = test_image.copy()
        reference_image.putpixel((2, 1), (2, 0, 0))
        test_png = captured(test_image)
        reference_png = captured(reference_image)

        exact_equal, exact_metrics, exact_diff = compare_reftest_screenshots(
            test_png,
            reference_png,
            None,
        )
        fuzzy_equal, fuzzy_metrics, fuzzy_diff = compare_reftest_screenshots(
            test_png,
            reference_png,
            FuzzyTolerance((0, 2), (0, 1)),
        )
        too_strict, _, strict_diff = compare_reftest_screenshots(
            test_png,
            reference_png,
            FuzzyTolerance((0, 1), (0, 1)),
        )
        self.addCleanup(exact_diff.close)
        self.addCleanup(fuzzy_diff.close)
        self.addCleanup(strict_diff.close)

        self.assertFalse(exact_equal)
        self.assertEqual(exact_metrics["max_difference"], 2)
        self.assertEqual(exact_metrics["different_pixels"], 1)
        self.assertTrue(fuzzy_equal)
        self.assertEqual(
            fuzzy_metrics["fuzzy"],
            {"max_difference": [0, 2], "total_pixels": [0, 1]},
        )
        self.assertFalse(too_strict)

    def test_reftest_failure_artifacts_write_test_reference_and_diff_pngs(self) -> None:
        def captured(color: tuple[int, int, int]) -> CapturedScreenshot:
            image = Image.new("RGB", (3, 2), color)
            stream = io.BytesIO()
            image.save(stream, format="PNG")
            image.close()
            png = stream.getvalue()
            return CapturedScreenshot(
                png=png,
                sha256=hashlib.sha256(png).hexdigest(),
                width=3,
                height=2,
            )

        test_png = captured((255, 255, 255))
        reference_png = captured((0, 0, 0))
        diff_image = Image.new("RGB", (3, 2), (255, 255, 255))
        self.addCleanup(diff_image.close)
        evidence = _ReftestEvidence(
            reference=ReftestReferenceRun(
                reference_path="css/example-ref.html",
                url="http://example.test/css/example-ref.html",
                relation="==",
            ),
            screenshot=reference_png,
            diff_image=diff_image,
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            artifacts = _write_reftest_failure_artifacts(
                output_dir=output_dir,
                engine="moli",
                case_path="css/example.html",
                test_screenshot=test_png,
                evidence=[evidence],
            )
            written = sorted(
                path.name
                for path in (output_dir / artifacts["directory"]).glob("*.png")
            )
            artifact_paths = [
                artifacts["test"],
                artifacts["references"][0]["reference"],
                artifacts["references"][0]["diff"],
            ]

            self.assertEqual(
                written,
                ["diff-01.png", "reference-01.png", "test.png"],
            )
            self.assertTrue(
                all((output_dir / artifact_path).stat().st_size > 0 for artifact_path in artifact_paths)
            )

    def test_reftest_match_and_mismatch_relationship_semantics(self) -> None:
        self.assertTrue(reftest_relation_passes("==", equal=True))
        self.assertFalse(reftest_relation_passes("==", equal=False))
        self.assertTrue(reftest_relation_passes("!=", equal=False))
        self.assertFalse(reftest_relation_passes("!=", equal=True))
        self.assertTrue(
            reftest_comparisons_pass(
                [
                    {"relation": "==", "passed": False},
                    {"relation": "==", "passed": True},
                    {"relation": "!=", "passed": True},
                ]
            )
        )
        self.assertFalse(
            reftest_comparisons_pass(
                [
                    {"relation": "==", "passed": True},
                    {"relation": "!=", "passed": False},
                ]
            )
        )

    def test_fixed_run_schedule_is_deterministic(self) -> None:
        cases = [
            "html/browsers/a.html",
            "html/browsers/b.html",
            "content-security-policy/navigation/a.html",
            "content-security-policy/navigation/b.html",
            "trusted-types/reporting/a.html",
            "trusted-types/reporting/b.html",
        ]

        scheduled_a, metadata_a = build_run_schedule(
            cases,
            case_path=lambda case: case,
        )
        scheduled_b, metadata_b = build_run_schedule(
            cases,
            case_path=lambda case: case,
        )

        self.assertEqual(scheduled_a, scheduled_b)
        self.assertEqual(metadata_a, metadata_b)
        self.assertCountEqual(scheduled_a, cases)
        self.assertNotEqual(scheduled_a, cases)
        self.assertEqual(metadata_a["mode"], "fixed-prefix-balanced-shuffle")
        self.assertEqual(metadata_a["seed"], FIXED_RUN_SHUFFLE_SEED)
        buckets = [case.split("/", 1)[0] for case in scheduled_a]
        self.assertTrue(all(left != right for left, right in zip(buckets, buckets[1:])))

    def _run_wpt_cross_with_fake_moli(
        self,
        *,
        output_dir: Path,
        known_failures: Path,
        case_status: str,
        failure_message: str = "expected 555 but got 100",
        allow_missing_known_failures: bool = False,
        extra_args: list[str] | None = None,
    ) -> int:
        class FakeServer:
            def __init__(self, wpt_root: Path) -> None:
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

        failures = []
        if case_status != "pass":
            failures = [{"name": "subtest", "message": failure_message}]
        result_dict = {
            "engine": "moli",
            "binary": "/tmp/moli",
            "binary_sha256": "sha",
            "binary_version": "0.1.0",
            "endpoint": "cli:/tmp/moli",
            "ready_ms": None,
            "setup_error": None,
            "cases": [
                {
                    "case_path": "known.html",
                    "status": case_status,
                    "duration_ms": 1.0,
                    "subtests": {
                        "total": 1,
                        "pass": int(case_status == "pass"),
                        "fail": int(case_status != "pass"),
                        "timeout": 0,
                        "notrun": 0,
                    },
                    "failures": failures,
                    "harness_status_name": "OK",
                    "harness_message": (
                        "Harness completed with a tracked message"
                        if case_status != "pass"
                        else None
                    ),
                    "error": None,
                }
            ],
        }
        with (
            patch(
                "moli_benchmark.wpt_cross.__main__.enumerate_cases",
                return_value=[WptCase("known.html")],
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
            patch("moli_benchmark.wpt_cross.server.WptFixtureServer", FakeServer),
            patch(
                "moli_benchmark.wpt_cross.__main__.REPO_CASE_LIST_DIR",
                output_dir.parent / "repo-case-lists",
            ),
            redirect_stdout(StringIO()),
            redirect_stderr(StringIO()),
        ):
            args = [
                "--wpt-root",
                "/tmp/wpt",
                "--engine",
                "moli",
                "--output-dir",
                str(output_dir),
                "--known-failures",
                str(known_failures),
            ]
            if allow_missing_known_failures:
                args.append("--allow-missing-known-failures")
            if extra_args is not None:
                args.extend(extra_args)
            return main(args)

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

        class FakeServer:
            def __init__(self, wpt_root: Path) -> None:
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
                        WPT_CROSS_CASE_TIMEOUT_SECONDS / 10.0,
                    ),
                    (
                        "digital-credentials/non-secure-contexts.http.html",
                        "http://[2001:db8::1]:9000/digital-credentials/non-secure-contexts.http.html",
                        WPT_CROSS_CASE_TIMEOUT_SECONDS,
                        WPT_CROSS_CASE_TIMEOUT_SECONDS / 10.0,
                    ),
                    (
                        "WebCryptoAPI/digest/digest.https.html",
                        "http://127.0.0.1:8000/WebCryptoAPI/digest/digest.https.html",
                        WPT_CROSS_CASE_TIMEOUT_SECONDS,
                        WPT_CROSS_CASE_TIMEOUT_SECONDS / 10.0,
                    )
                ]
            ],
        )

    def test_main_writes_schedule_and_passes_execution_order_to_cli_runner(self) -> None:
        captured: dict[str, object] = {}

        class FakeServer:
            def __init__(self, wpt_root: Path) -> None:
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

    def test_main_uses_fixed_parallelism_for_cdp_runner(self) -> None:
        calls: list[dict[str, object]] = []

        class FakeServer:
            def __init__(self, wpt_root: Path) -> None:
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
                    ]
                )

        self.assertEqual(code, 0)
        self.assertEqual(len(calls), len(cases))
        self.assertTrue(all(call["case_timeout_seconds"] == WPT_CROSS_CASE_TIMEOUT_SECONDS for call in calls))
        self.assertEqual(
            sorted(call["cases"][0][0] for call in calls),
            [case.case_path for case in cases],
        )

    def test_layout_testharness_profile_forces_cdp_and_fixed_viewport(self) -> None:
        captured: dict[str, object] = {}

        class FakeServer:
            def __init__(self, wpt_root: Path) -> None:
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

    def test_main_preserves_harness_multiplier_except_step_timeout_sensitive_cases(self) -> None:
        calls = []

        class FakeServer:
            def __init__(self, wpt_root: Path) -> None:
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
                        "normal.html": 12.0,
                        "long.html": 12.0,
                        "html/semantics/scripting-1/the-script-element/module/dynamic-import/delay-load-event.html": 1.0,
                    },
                    12.0,
                )
            ],
        )

    def test_harness_timeout_multiplier_keeps_dynamic_import_step_timeout_unscaled(self) -> None:
        case = WptCase(
            "html/semantics/scripting-1/the-script-element/module/dynamic-import/delay-load-event.html"
        )

        self.assertEqual(_harness_timeout_multiplier(30.0, 10.0, case.case_path), 1.0)

    def test_parser_rejects_removed_timeout_and_parallelism_flags(self) -> None:
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
            "--parallelism",
            "--cdp-parallelism",
        ):
            with self.subTest(removed_flag=removed_flag), self.assertRaises(SystemExit):
                parser.parse_args([*base, removed_flag, "1"])

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

    def test_enumerate_cases_expands_wpt_meta_variants(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "dom" / "ranges"
            case_dir.mkdir(parents=True)
            (case_dir / "variant.html").write_text(
                """<!doctype html>
<meta name="variant" content="?mode=open">
<meta name="variant" content="?mode=closed#frag">
<meta name="variant" content="?command=heading&param=h1">
<meta name="variant" content="?mode=encoded&amp;flag=1">
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )
            (case_dir / "plain.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )
            (case_dir / "long.html").write_text(
                """<!doctype html>
<meta name="timeout" content="long">
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )

            cases = enumerate_cases(wpt_root, dir_prefixes=("dom/ranges",))

        self.assertEqual(
            [case.case_path for case in cases],
            [
                "dom/ranges/long.html",
                "dom/ranges/plain.html",
                "dom/ranges/variant.html?command=heading&param=h1",
                "dom/ranges/variant.html?mode=closed#frag",
                "dom/ranges/variant.html?mode=encoded&flag=1",
                "dom/ranges/variant.html?mode=open",
            ],
        )
        timeouts = {case.case_path: case.timeout_multiplier for case in cases}
        self.assertEqual(timeouts["dom/ranges/long.html"], LONG_TIMEOUT_MULTIPLIER)
        self.assertEqual(timeouts["dom/ranges/plain.html"], 1.0)
        self.assertEqual(timeouts["dom/ranges/variant.html?mode=open"], 1.0)

    def test_enumerate_cases_skips_wptserve_python_resources(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "html" / "semantics"
            common_dir = wpt_root / "common" / "security-features" / "resources"
            case_dir.mkdir(parents=True)
            common_dir.mkdir(parents=True)
            (case_dir / "direct.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script>fetch("/resources/inspect.py?cmd=get");</script>
""",
                encoding="utf-8",
            )
            (case_dir / "external.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="helper.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )
            (case_dir / "helper.js").write_text(
                'fetch("/resources/inspect-header.py?cmd=get");',
                encoding="utf-8",
            )
            (case_dir / "kept.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )
            (case_dir / "generated-comment.html").write_text(
                """<!doctype html>
<!-- DO NOT EDIT! This test has been generated by /html/canvas/tools/gentest.py. -->
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )
            (case_dir / "absolute-helper.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/common/security-features/resources/common.sub.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )
            (common_dir / "common.sub.js").write_text(
                'fetch("/common/security-features/subresource/xhr.py");',
                encoding="utf-8",
            )

            cases = enumerate_cases(wpt_root, dir_prefixes=("html/semantics",))

        self.assertEqual(
            [case.case_path for case in cases],
            [
                "html/semantics/generated-comment.html",
                "html/semantics/kept.html",
            ],
        )

    def test_default_enumeration_uses_non_layout_blacklist_without_encoding(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            dom_dir = wpt_root / "dom" / "nodes"
            encoding_dir = wpt_root / "encoding" / "legacy-mb-japanese"
            canvas_dir = wpt_root / "html" / "canvas" / "element"
            layout_dir = wpt_root / "css" / "css-grid"
            container_query_dir = (
                wpt_root / "css" / "css-conditional" / "container-queries"
            )
            css_conditional_dir = wpt_root / "css" / "css-conditional"
            css_animation_dir = wpt_root / "css" / "css-animations" / "parsing"
            css_animation_timeline_dir = wpt_root / "css" / "css-animations"
            css_transition_dir = wpt_root / "css" / "css-transitions"
            css_transition_parsing_dir = (
                wpt_root / "css" / "css-transitions" / "parsing"
            )
            css_paint_dir = wpt_root / "css" / "css-paint-api"
            css_scrollbar_dir = wpt_root / "css" / "css-scrollbars"
            css_viewport_dir = wpt_root / "css" / "css-viewport" / "zoom"
            css_viewport_parsing_dir = (
                wpt_root / "css" / "css-viewport" / "zoom" / "parsing"
            )
            typed_om_dir = wpt_root / "css" / "css-typed-om"
            render_blocking_dir = wpt_root / "html" / "dom" / "render-blocking"
            service_worker_dir = wpt_root / "service-workers" / "service-worker"
            service_worker_named_dir = wpt_root / "webusb"
            non_service_worker_https_dir = wpt_root / "WebCryptoAPI"
            media_dir = (
                wpt_root / "html" / "semantics" / "embedded-content" / "media-elements"
            )
            iframe_dir = (
                wpt_root
                / "html"
                / "semantics"
                / "embedded-content"
                / "the-iframe-element"
            )
            dom_dir.mkdir(parents=True)
            encoding_dir.mkdir(parents=True)
            canvas_dir.mkdir(parents=True)
            layout_dir.mkdir(parents=True)
            container_query_dir.mkdir(parents=True)
            css_conditional_dir.mkdir(parents=True, exist_ok=True)
            css_animation_dir.mkdir(parents=True)
            css_transition_dir.mkdir(parents=True)
            css_transition_parsing_dir.mkdir(parents=True)
            css_paint_dir.mkdir(parents=True)
            css_scrollbar_dir.mkdir(parents=True)
            css_viewport_dir.mkdir(parents=True)
            css_viewport_parsing_dir.mkdir(parents=True)
            typed_om_dir.mkdir(parents=True)
            render_blocking_dir.mkdir(parents=True)
            service_worker_dir.mkdir(parents=True)
            service_worker_named_dir.mkdir(parents=True)
            non_service_worker_https_dir.mkdir(parents=True)
            media_dir.mkdir(parents=True)
            iframe_dir.mkdir(parents=True)
            case_html = """<!doctype html>
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
"""
            (dom_dir / "kept.html").write_text(case_html, encoding="utf-8")
            (encoding_dir / "kept.html").write_text(case_html, encoding="utf-8")
            (canvas_dir / "filtered.html").write_text(case_html, encoding="utf-8")
            (layout_dir / "filtered.html").write_text(case_html, encoding="utf-8")
            (container_query_dir / "at-container-parsing.html").write_text(
                case_html, encoding="utf-8"
            )
            (container_query_dir / "size-feature-evaluation.html").write_text(
                case_html, encoding="utf-8"
            )
            (css_conditional_dir / "at-supports-named-feature-001.html").write_text(
                case_html, encoding="utf-8"
            )
            (css_animation_dir / "kept.html").write_text(case_html, encoding="utf-8")
            (css_animation_timeline_dir / "animate-with-color-mix.html").write_text(
                case_html, encoding="utf-8"
            )
            (css_transition_dir / "events-001.html").write_text(
                case_html, encoding="utf-8"
            )
            (css_transition_parsing_dir / "transition-computed.html").write_text(
                case_html, encoding="utf-8"
            )
            (css_paint_dir / "filtered.html").write_text(case_html, encoding="utf-8")
            (css_scrollbar_dir / "scrollbar-width-001.html").write_text(
                case_html, encoding="utf-8"
            )
            (css_scrollbar_dir / "scrollbar-width-parsing.html").write_text(
                case_html, encoding="utf-8"
            )
            (css_viewport_dir / "widget.html").write_text(case_html, encoding="utf-8")
            (css_viewport_parsing_dir / "zoom-computed.html").write_text(
                case_html, encoding="utf-8"
            )
            (typed_om_dir / "kept.html").write_text(case_html, encoding="utf-8")
            (render_blocking_dir / "filtered.html").write_text(
                case_html, encoding="utf-8"
            )
            (service_worker_dir / "registration.https.html").write_text(
                case_html, encoding="utf-8"
            )
            (service_worker_named_dir / "usb.serviceworker.https.html").write_text(
                case_html, encoding="utf-8"
            )
            (
                non_service_worker_https_dir
                / "crypto-subtle-secure-context-available.https.sub.html"
            ).write_text(case_html, encoding="utf-8")
            (media_dir / "filtered.html").write_text(case_html, encoding="utf-8")
            (iframe_dir / "kept.html").write_text(case_html, encoding="utf-8")

            cases = enumerate_cases(wpt_root)
            focused_transition_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("css/css-transitions",),
            )

        self.assertNotIn("encoding", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertIn("html/canvas", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertIn("css/css-grid", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertNotIn(
            "css/css-conditional/container-queries", DEFAULT_EXCLUDE_DIR_PREFIXES
        )
        self.assertIn("css/css-paint-api", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertNotIn("css/css-animations", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertNotIn("css/css-transitions", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertIn("html/dom/render-blocking", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertNotIn("service-workers", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertIn(
            "html/semantics/embedded-content/media-elements",
            DEFAULT_EXCLUDE_DIR_PREFIXES,
        )
        self.assertNotIn("css/css-typed-om", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertNotIn(
            "html/semantics/embedded-content/the-iframe-element",
            DEFAULT_EXCLUDE_DIR_PREFIXES,
        )
        self.assertEqual(
            [case.case_path for case in cases],
            [
                "css/css-animations/parsing/kept.html",
                "css/css-conditional/container-queries/at-container-parsing.html",
                "css/css-scrollbars/scrollbar-width-parsing.html",
                "css/css-transitions/parsing/transition-computed.html",
                "css/css-typed-om/kept.html",
                "css/css-viewport/zoom/parsing/zoom-computed.html",
                "dom/nodes/kept.html",
                "encoding/legacy-mb-japanese/kept.html",
                "html/semantics/embedded-content/the-iframe-element/kept.html",
                "service-workers/service-worker/registration.https.html",
                "webusb/usb.serviceworker.https.html",
            ],
        )
        self.assertEqual(
            [case.case_path for case in focused_transition_cases],
            [
                "css/css-transitions/events-001.html",
                "css/css-transitions/parsing/transition-computed.html",
            ],
        )

    def test_default_enumeration_excludes_jpegxl_codec_fidelity(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            jpegxl_dir = wpt_root / "jpegxl"
            jpegxl_dir.mkdir(parents=True)
            case_html = """<!doctype html>
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
"""
            (jpegxl_dir / "decode.html").write_text(case_html, encoding="utf-8")

            default_cases = enumerate_cases(wpt_root)
            focused_cases = enumerate_cases(wpt_root, dir_prefixes=("jpegxl",))

        self.assertIn("jpegxl", DEFAULT_EXCLUDE_DIR_PREFIXES)
        self.assertEqual(default_cases, [])
        self.assertEqual(
            [case.case_path for case in focused_cases],
            ["jpegxl/decode.html"],
        )

    def test_enumerate_cases_skips_manual_and_support_resource_pages(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "FileAPI"
            resource_dir = wpt_root / "appmanifest" / "resources"
            support_dir = wpt_root / "workers" / "support"
            case_dir.mkdir(parents=True)
            resource_dir.mkdir(parents=True)
            support_dir.mkdir(parents=True)
            case_html = """<!doctype html>
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
"""
            (case_dir / "kept.html").write_text(case_html, encoding="utf-8")
            (case_dir / "upload-manual.html").write_text(case_html, encoding="utf-8")
            (resource_dir / "helper.html").write_text(case_html, encoding="utf-8")
            (support_dir / "helper.html").write_text(case_html, encoding="utf-8")

            cases = enumerate_cases(wpt_root)

        self.assertEqual([case.case_path for case in cases], ["FileAPI/kept.html"])

    def test_explicit_dir_prefix_bypasses_default_rendering_blacklist(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            canvas_dir = wpt_root / "html" / "canvas"
            canvas_dir.mkdir(parents=True)
            (canvas_dir / "case.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )

            cases = enumerate_cases(wpt_root, dir_prefixes=("html/canvas",))

        self.assertEqual([case.case_path for case in cases], ["html/canvas/case.html"])

    def test_preload_dir_prefix_allows_modulepreload_helper_without_stash_usage(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            preload_dir = wpt_root / "preload"
            resources_dir = preload_dir / "resources"
            resources_dir.mkdir(parents=True)
            harness_dir = wpt_root / "resources"
            harness_dir.mkdir(parents=True)
            (harness_dir / "testharness.js").write_text("", encoding="utf-8")
            (harness_dir / "testharnessreport.js").write_text("", encoding="utf-8")
            (resources_dir / "preload_helper.js").write_text(
                "function unused() { return '/preload/resources/stash-put.py'; }",
                encoding="utf-8",
            )
            modulepreload_case = """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script src="/preload/resources/preload_helper.js"></script>
<link rel=modulepreload href="resources/dummy.js?pipe=trickle(d5)">
<script>promise_test(async () => {}, "ok");</script>
"""
            (preload_dir / "avoid-delaying-onload-link-modulepreload.html").write_text(
                modulepreload_case,
                encoding="utf-8",
            )
            (
                preload_dir / "avoid-delaying-onload-link-modulepreload-exec.html"
            ).write_text(
                modulepreload_case,
                encoding="utf-8",
            )
            (preload_dir / "avoid-delaying-onload-link-preload.html").write_text(
                modulepreload_case,
                encoding="utf-8",
            )

            cases = enumerate_cases(wpt_root, dir_prefixes=("preload",))

        self.assertEqual(
            [case.case_path for case in cases],
            [
                "preload/avoid-delaying-onload-link-modulepreload-exec.html",
                "preload/avoid-delaying-onload-link-modulepreload.html",
            ],
        )

    def test_preload_dir_prefix_excludes_sri_non_goal_cases(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            preload_dir = wpt_root / "preload"
            preload_dir.mkdir(parents=True)
            harness_dir = wpt_root / "resources"
            harness_dir.mkdir(parents=True)
            (harness_dir / "testharness.js").write_text("", encoding="utf-8")
            (harness_dir / "testharnessreport.js").write_text("", encoding="utf-8")
            case_html = """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>promise_test(async () => {}, "ok");</script>
"""
            (preload_dir / "modulepreload-json.html").write_text(
                case_html,
                encoding="utf-8",
            )
            (preload_dir / "modulepreload-sri.html").write_text(
                case_html,
                encoding="utf-8",
            )
            (preload_dir / "modulepreload-sri-importmap.html").write_text(
                case_html,
                encoding="utf-8",
            )

            cases = enumerate_cases(wpt_root, dir_prefixes=("preload",))

        self.assertEqual(
            [case.case_path for case in cases],
            ["preload/modulepreload-json.html"],
        )

    def test_enumerate_cases_excludes_malformed_select_fragment_non_goal(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            parsing_dir = wpt_root / "html" / "syntax" / "parsing"
            parsing_dir.mkdir(parents=True)
            harness_dir = wpt_root / "resources"
            harness_dir.mkdir(parents=True)
            (harness_dir / "testharness.js").write_text("", encoding="utf-8")
            (harness_dir / "testharnessreport.js").write_text("", encoding="utf-8")
            case_html = """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>test(() => {}, "ok");</script>
"""
            (parsing_dir / "html5lib_innerHTML_webkit02.html").write_text(
                case_html,
                encoding="utf-8",
            )
            (parsing_dir / "html5lib_scripted_webkit01.html").write_text(
                case_html,
                encoding="utf-8",
            )

            default_cases = enumerate_cases(wpt_root)
            focused_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("html/syntax/parsing",),
            )

        expected = ["html/syntax/parsing/html5lib_scripted_webkit01.html"]
        self.assertEqual([case.case_path for case in default_cases], expected)
        self.assertEqual([case.case_path for case in focused_cases], expected)

    def test_enumerate_cases_excludes_formdata_non_goal_cases(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            formdata_dir = wpt_root / "xhr" / "formdata"
            formdata_dir.mkdir(parents=True)
            harness_dir = wpt_root / "resources"
            harness_dir.mkdir(parents=True)
            (harness_dir / "testharness.js").write_text("", encoding="utf-8")
            (harness_dir / "testharnessreport.js").write_text("", encoding="utf-8")
            case_html = """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>test(() => {}, "ok");</script>
"""
            (formdata_dir / "constructor-formelement.html").write_text(
                case_html,
                encoding="utf-8",
            )
            (formdata_dir / "constructor-submitter-coordinate.html").write_text(
                case_html,
                encoding="utf-8",
            )
            (formdata_dir / "constructor-submitter.html").write_text(
                case_html,
                encoding="utf-8",
            )

            default_cases = enumerate_cases(wpt_root)
            focused_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("xhr/formdata",),
            )
            diagnostic_case = explicit_case(
                wpt_root,
                "xhr/formdata/constructor-formelement.html",
            )
            coordinate_diagnostic_case = explicit_case(
                wpt_root,
                "xhr/formdata/constructor-submitter-coordinate.html",
            )

        expected = ["xhr/formdata/constructor-submitter.html"]
        self.assertEqual([case.case_path for case in default_cases], expected)
        self.assertEqual([case.case_path for case in focused_cases], expected)
        self.assertEqual(
            diagnostic_case.case_path,
            "xhr/formdata/constructor-formelement.html",
        )
        self.assertEqual(
            coordinate_diagnostic_case.case_path,
            "xhr/formdata/constructor-submitter-coordinate.html",
        )

    def test_enumerate_cases_allows_supported_xhr_delay_handler(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            xhr_dir = wpt_root / "xhr"
            xhr_dir.mkdir(parents=True)
            harness_dir = wpt_root / "resources"
            harness_dir.mkdir(parents=True)
            (harness_dir / "testharness.js").write_text("", encoding="utf-8")
            (harness_dir / "testharnessreport.js").write_text("", encoding="utf-8")
            (xhr_dir / "delay-supported.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>fetch("resources/delay.py?ms=1"); test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )
            (xhr_dir / "delay-and-unsupported.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>fetch("resources/delay.py"); fetch("resources/other.py");</script>
""",
                encoding="utf-8",
            )

            cases = enumerate_cases(wpt_root, dir_prefixes=("xhr",))

        self.assertEqual(
            [case.case_path for case in cases],
            ["xhr/delay-supported.html"],
        )

    def test_enumerate_cases_allows_supported_delayed_module_handler(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            harness_dir = wpt_root / "resources"
            harness_dir.mkdir(parents=True)
            (harness_dir / "testharness.js").write_text("", encoding="utf-8")
            (harness_dir / "testharnessreport.js").write_text("", encoding="utf-8")
            module_dir = (
                wpt_root
                / "html"
                / "semantics"
                / "scripting-1"
                / "the-script-element"
                / "module"
            )
            (module_dir / "resources").mkdir(parents=True)
            (module_dir / "delay-supported.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script type="module" src="./delay-supported.js"></script>
""",
                encoding="utf-8",
            )
            (module_dir / "delay-supported.js").write_text(
                'import "./resources/delayed-modulescript.py?ms=1";',
                encoding="utf-8",
            )
            (module_dir / "resources" / "delayed-modulescript.py").write_text(
                "# modeled by wpt-cross",
                encoding="utf-8",
            )

            cases = enumerate_cases(
                wpt_root,
                dir_prefixes=(
                    "html/semantics/scripting-1/the-script-element/module",
                ),
            )

        self.assertEqual(
            [case.case_path for case in cases],
            [
                "html/semantics/scripting-1/the-script-element/module/"
                "delay-supported.html"
            ],
        )

    def test_enumerate_cases_wraps_window_js_for_explicit_dir_prefix(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            api_dir = wpt_root / "WebCryptoAPI"
            api_dir.mkdir(parents=True)
            (api_dir / "algorithm-discards-context.https.window.js").write_text(
                """// META: title=window case
// META: timeout=long
promise_test(async () => {}, "ok");
""",
                encoding="utf-8",
            )

            cases = enumerate_cases(wpt_root, dir_prefixes=("WebCryptoAPI",))

        self.assertEqual(
            [case.case_path for case in cases],
            [
                "WebCryptoAPI/algorithm-discards-context.https.window.js?moli-wpt-script=window",
            ],
        )
        self.assertEqual(cases[0].timeout_multiplier, LONG_TIMEOUT_MULTIPLIER)

    def test_explicit_dir_prefix_includes_simple_https_sub_html(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            api_dir = wpt_root / "WebCryptoAPI" / "secure_context"
            api_dir.mkdir(parents=True)
            (api_dir / "crypto-subtle-secure-context-available.https.sub.html").write_text(
                """<!doctype html>
<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>test(() => assert_equals("{{host}}", "{{host}}"), "ok");</script>
""",
                encoding="utf-8",
            )

            default_cases = enumerate_cases(wpt_root)
            selected_cases = enumerate_cases(wpt_root, dir_prefixes=("WebCryptoAPI",))

        self.assertEqual(default_cases, [])
        self.assertEqual(
            [case.case_path for case in selected_cases],
            ["WebCryptoAPI/secure_context/crypto-subtle-secure-context-available.https.sub.html"],
        )

    def test_dir_prefix_can_opt_into_tentative_cases(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "wasm" / "webapi" / "esm-integration"
            case_dir.mkdir(parents=True)
            case_html = """<!doctype html>
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
"""
            (case_dir / "stable.html").write_text(case_html, encoding="utf-8")
            (case_dir / "feature.tentative.html").write_text(
                case_html,
                encoding="utf-8",
            )

            default_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("wasm/webapi/esm-integration",),
            )
            tentative_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("wasm/webapi/esm-integration",),
                include_tentative=True,
            )

        self.assertEqual(
            [case.case_path for case in default_cases],
            ["wasm/webapi/esm-integration/stable.html"],
        )
        self.assertEqual(
            [case.case_path for case in tentative_cases],
            [
                "wasm/webapi/esm-integration/feature.tentative.html",
                "wasm/webapi/esm-integration/stable.html",
            ],
        )

    def test_dir_prefix_can_opt_into_any_js_globals(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "wasm" / "jsapi" / "js-string"
            resource_dir = case_dir / "resources"
            case_dir.mkdir(parents=True)
            resource_dir.mkdir(parents=True)
            (case_dir / "basic.any.js").write_text(
                """// META: timeout=long
promise_test(async () => {}, "ok");
""",
                encoding="utf-8",
            )
            (case_dir / "feature.tentative.any.js").write_text(
                """promise_test(async () => {}, "ok");""",
                encoding="utf-8",
            )
            (case_dir / "testdriver.any.js").write_text(
                """// META: script=/resources/testdriver.js
promise_test(async () => {}, "requires testdriver");
""",
                encoding="utf-8",
            )
            (resource_dir / "helper.any.js").write_text(
                """promise_test(async () => {}, "not a top-level case");""",
                encoding="utf-8",
            )

            default_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("wasm/jsapi/js-string",),
            )
            window_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("wasm/jsapi/js-string",),
                any_js_global=ANY_JS_WINDOW_GLOBAL,
            )
            both_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("wasm/jsapi/js-string",),
                include_tentative=True,
                any_js_global="both",
            )

        self.assertEqual(default_cases, [])
        self.assertEqual(
            [(case.case_path, case.timeout_multiplier) for case in window_cases],
            [
                (
                    "wasm/jsapi/js-string/basic.any.js?moli-wpt-any=window",
                    LONG_TIMEOUT_MULTIPLIER,
                )
            ],
        )
        self.assertEqual(
            [case.case_path for case in both_cases],
            [
                "wasm/jsapi/js-string/basic.any.js?moli-wpt-any=dedicatedworker",
                "wasm/jsapi/js-string/basic.any.js?moli-wpt-any=window",
                "wasm/jsapi/js-string/feature.tentative.any.js?moli-wpt-any=dedicatedworker",
                "wasm/jsapi/js-string/feature.tentative.any.js?moli-wpt-any=window",
            ],
        )

    def test_default_enumeration_includes_streams_script_cases(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            streams_dir = wpt_root / "streams" / "readable-streams"
            other_dir = wpt_root / "wasm" / "jsapi"
            streams_dir.mkdir(parents=True)
            other_dir.mkdir(parents=True)
            (streams_dir / "general.any.js").write_text(
                """// META: global=window,worker
promise_test(async () => {}, "general");
""",
                encoding="utf-8",
            )
            (streams_dir / "window-only.any.js").write_text(
                """// META: global=window
test(() => {}, "window only");
""",
                encoding="utf-8",
            )
            (streams_dir / "task.window.js").write_text(
                """// META: timeout=long
test(() => {}, "window script");
""",
                encoding="utf-8",
            )
            (streams_dir / "task.worker.js").write_text(
                """test(() => {}, "worker script");
done();
""",
                encoding="utf-8",
            )
            (streams_dir / "feature.tentative.any.js").write_text(
                """test(() => {}, "tentative");""",
                encoding="utf-8",
            )
            (other_dir / "excluded.any.js").write_text(
                """test(() => {}, "not in the broad baseline");""",
                encoding="utf-8",
            )

            cases = enumerate_cases(wpt_root)

        self.assertEqual(
            [(case.case_path, case.timeout_multiplier) for case in cases],
            [
                (
                    "streams/readable-streams/general.any.js?moli-wpt-any=dedicatedworker",
                    1.0,
                ),
                (
                    "streams/readable-streams/general.any.js?moli-wpt-any=window",
                    1.0,
                ),
                (
                    "streams/readable-streams/task.window.js?moli-wpt-script=window",
                    LONG_TIMEOUT_MULTIPLIER,
                ),
                (
                    "streams/readable-streams/task.worker.js?moli-wpt-script=dedicatedworker",
                    1.0,
                ),
                (
                    "streams/readable-streams/window-only.any.js?moli-wpt-any=window",
                    1.0,
                ),
            ],
        )

    def test_dir_prefix_expands_any_js_variants_and_respects_globals(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "WebCryptoAPI"
            case_dir.mkdir(parents=True)
            (case_dir / "multi.any.js").write_text(
                """// META: global=window,worker
// META: timeout=long
// META: variant=?mode=1
// META: variant=?mode=2
promise_test(async () => {}, "multi");
""",
                encoding="utf-8",
            )
            (case_dir / "window-only.any.js").write_text(
                """// META: global=window
promise_test(async () => {}, "window");
""",
                encoding="utf-8",
            )
            (case_dir / "worker-only.any.js").write_text(
                """// META: global=dedicatedworker
promise_test(async () => {}, "worker");
""",
                encoding="utf-8",
            )

            cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("WebCryptoAPI",),
                any_js_global="both",
            )

        self.assertEqual(
            [(case.case_path, case.timeout_multiplier) for case in cases],
            [
                (
                    "WebCryptoAPI/multi.any.js?mode=1&moli-wpt-any=dedicatedworker",
                    LONG_TIMEOUT_MULTIPLIER,
                ),
                (
                    "WebCryptoAPI/multi.any.js?mode=1&moli-wpt-any=window",
                    LONG_TIMEOUT_MULTIPLIER,
                ),
                (
                    "WebCryptoAPI/multi.any.js?mode=2&moli-wpt-any=dedicatedworker",
                    LONG_TIMEOUT_MULTIPLIER,
                ),
                (
                    "WebCryptoAPI/multi.any.js?mode=2&moli-wpt-any=window",
                    LONG_TIMEOUT_MULTIPLIER,
                ),
                (
                    "WebCryptoAPI/window-only.any.js?moli-wpt-any=window",
                    1.0,
                ),
                (
                    "WebCryptoAPI/worker-only.any.js?moli-wpt-any=dedicatedworker",
                    1.0,
                ),
            ],
        )

    def test_dir_prefix_includes_window_and_worker_js_wrappers(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "wasm" / "serialization"
            case_dir.mkdir(parents=True)
            (case_dir / "transfer.window.js").write_text(
                """// META: timeout=long
test(() => {}, "window");
""",
                encoding="utf-8",
            )
            (case_dir / "memory.worker.js").write_text(
                """importScripts("/resources/testharness.js");
test(() => {}, "worker");
done();
""",
                encoding="utf-8",
            )
            (case_dir / "feature.tentative.window.js").write_text(
                """test(() => {}, "tentative");""",
                encoding="utf-8",
            )

            default_cases = enumerate_cases(wpt_root)
            focused_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("wasm/serialization",),
            )
            tentative_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("wasm/serialization",),
                include_tentative=True,
            )

        self.assertEqual(default_cases, [])
        self.assertEqual(
            [(case.case_path, case.timeout_multiplier) for case in focused_cases],
            [
                (
                    "wasm/serialization/memory.worker.js?moli-wpt-script=dedicatedworker",
                    1.0,
                ),
                (
                    "wasm/serialization/transfer.window.js?moli-wpt-script=window",
                    LONG_TIMEOUT_MULTIPLIER,
                ),
            ],
        )
        self.assertEqual(
            [case.case_path for case in tentative_cases],
            [
                "wasm/serialization/feature.tentative.window.js?moli-wpt-script=window",
                "wasm/serialization/memory.worker.js?moli-wpt-script=dedicatedworker",
                "wasm/serialization/transfer.window.js?moli-wpt-script=window",
            ],
        )

    def test_dir_prefix_includes_sub_https_and_supported_wasm_status_handler(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            serialization_dir = wpt_root / "wasm" / "serialization" / "module"
            serialization_dir.mkdir(parents=True)
            webapi_dir = wpt_root / "wasm" / "webapi"
            webapi_dir.mkdir(parents=True)
            esm_dir = webapi_dir / "esm-integration"
            esm_dir.mkdir()
            (serialization_dir / "window-domain-success.sub.html").write_text(
                """<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>test(() => {}, "sub");</script>
""",
                encoding="utf-8",
            )
            (esm_dir / "worklet-import-source-phase.tentative.https.html").write_text(
                """<script src="/resources/testharness.js"></script>
<script src="/resources/testharnessreport.js"></script>
<script>test(() => {}, "https tentative");</script>
""",
                encoding="utf-8",
            )
            (webapi_dir / "status.any.js").write_text(
                """// META: global=window,worker
promise_test(async t => {
  await promise_rejects_js(t, TypeError, WebAssembly.compileStreaming(fetch("status.py?status=404")));
}, "status");
""",
                encoding="utf-8",
            )
            (webapi_dir / "badstatus.any.js").write_text(
                """// META: global=window,worker
promise_test(async t => {
  await promise_rejects_js(t, TypeError, WebAssembly.compileStreaming(fetch("badstatus.py?status=404")));
}, "badstatus");
""",
                encoding="utf-8",
            )
            (webapi_dir / "origin.sub.any.js").write_text(
                """// META: global=window,worker
promise_test(async t => {
  await promise_rejects_js(t, TypeError, WebAssembly.compileStreaming(fetch("/fetch/api/resources/redirect.py?redirect_status=301&location=/wasm/incrementer.wasm")));
}, "origin");
""",
                encoding="utf-8",
            )

            broad_cases = enumerate_cases(wpt_root)
            focused_cases = enumerate_cases(
                wpt_root,
                dir_prefixes=("wasm",),
                include_tentative=True,
                any_js_global="both",
            )

        self.assertEqual(broad_cases, [])
        self.assertEqual(
            [case.case_path for case in focused_cases],
            [
                "wasm/serialization/module/window-domain-success.sub.html",
                "wasm/webapi/esm-integration/worklet-import-source-phase.tentative.https.html",
                "wasm/webapi/origin.sub.any.js?moli-wpt-any=dedicatedworker",
                "wasm/webapi/origin.sub.any.js?moli-wpt-any=window",
                "wasm/webapi/status.any.js?moli-wpt-any=dedicatedworker",
                "wasm/webapi/status.any.js?moli-wpt-any=window",
            ],
        )

    def test_explicit_case_bypasses_curated_filename_filters(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "shadow-dom"
            case_dir.mkdir(parents=True)
            (case_dir / "feature.tentative.html").write_text(
                """<!doctype html>
<meta name="timeout" content="long">
<script src="/resources/testharness.js"></script>
<script>test(() => {}, "ok");</script>
""",
                encoding="utf-8",
            )

            case = explicit_case(wpt_root, "shadow-dom/feature.tentative.html?variant=1")

        self.assertEqual(case.case_path, "shadow-dom/feature.tentative.html?variant=1")
        self.assertEqual(case.timeout_multiplier, LONG_TIMEOUT_MULTIPLIER)

    def test_explicit_case_wraps_any_js(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "WebCryptoAPI"
            case_dir.mkdir(parents=True)
            (case_dir / "historical.any.js").write_text(
                """// META: global=window,dedicatedworker
// META: timeout=long
test(() => {}, "ok");
""",
                encoding="utf-8",
            )

            case = explicit_case(wpt_root, "WebCryptoAPI/historical.any.js?mode=1")

        self.assertEqual(
            case.case_path,
            "WebCryptoAPI/historical.any.js?mode=1&moli-wpt-any=window",
        )
        self.assertEqual(case.timeout_multiplier, LONG_TIMEOUT_MULTIPLIER)

    def test_explicit_window_and_worker_js_cases_use_wrappers(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "wasm"
            case_dir.mkdir(parents=True)
            (case_dir / "transfer.window.js").write_text(
                """// META: timeout=long
test(() => {}, "ok");
""",
                encoding="utf-8",
            )
            (case_dir / "memory.worker.js").write_text(
                """test(() => {}, "ok");""",
                encoding="utf-8",
            )

            window_case = explicit_case(wpt_root, "wasm/transfer.window.js?variant=1")
            worker_case = explicit_case(wpt_root, "wasm/memory.worker.js")

        self.assertEqual(
            window_case.case_path,
            "wasm/transfer.window.js?variant=1&moli-wpt-script=window",
        )
        self.assertEqual(window_case.timeout_multiplier, LONG_TIMEOUT_MULTIPLIER)
        self.assertEqual(
            worker_case.case_path,
            "wasm/memory.worker.js?moli-wpt-script=dedicatedworker",
        )

    def test_explicit_case_accepts_generated_any_html_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "WebCryptoAPI"
            case_dir.mkdir(parents=True)
            (case_dir / "historical.any.js").write_text(
                """// META: timeout=long
test(() => {}, "ok");
""",
                encoding="utf-8",
            )

            case = explicit_case(
                wpt_root,
                f"WebCryptoAPI/historical.any.html?{ANY_JS_WINDOW_QUERY}",
            )

        self.assertEqual(
            case.case_path,
            f"WebCryptoAPI/historical.any.html?{ANY_JS_WINDOW_QUERY}",
        )
        self.assertEqual(case.timeout_multiplier, LONG_TIMEOUT_MULTIPLIER)

    def test_explicit_case_accepts_generated_window_html_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            wpt_root = Path(temp_dir)
            case_dir = wpt_root / "WebCryptoAPI"
            case_dir.mkdir(parents=True)
            (case_dir / "algorithm-discards-context.https.window.js").write_text(
                """// META: timeout=long
test(() => {}, "ok");
""",
                encoding="utf-8",
            )

            case = explicit_case(
                wpt_root,
                f"WebCryptoAPI/algorithm-discards-context.https.window.html?{WINDOW_JS_WINDOW_QUERY}",
            )

        self.assertEqual(
            case.case_path,
            f"WebCryptoAPI/algorithm-discards-context.https.window.html?{WINDOW_JS_WINDOW_QUERY}",
        )
        self.assertEqual(case.timeout_multiplier, LONG_TIMEOUT_MULTIPLIER)

    def test_any_js_window_case_path_does_not_duplicate_window_query(self) -> None:
        self.assertEqual(
            any_js_window_case_path(
                f"WebCryptoAPI/historical.any.js?{ANY_JS_WINDOW_QUERY}#frag"
            ),
            f"WebCryptoAPI/historical.any.html?{ANY_JS_WINDOW_QUERY}#frag",
        )

    def test_window_js_window_case_path_does_not_duplicate_window_query(self) -> None:
        self.assertEqual(
            window_js_window_case_path(
                f"WebCryptoAPI/example.window.js?{WINDOW_JS_WINDOW_QUERY}#frag"
            ),
            f"WebCryptoAPI/example.window.html?{WINDOW_JS_WINDOW_QUERY}#frag",
        )

    def test_any_js_metadata_matches_wpt_header_rules(self) -> None:
        meta = parse_any_js_meta(
            """//META: variant=?first
//  META: script=helper.js

// META: variant=?ignored
"""
        )

        self.assertEqual(meta.variants, ["?first"])
        self.assertEqual(meta.scripts, ["helper.js"])

    def test_report_bridge_keys_results_by_initial_search_without_hash(self) -> None:
        self.assertIn(b"var initialCasePath =", BENCH_REPORT_BRIDGE)
        self.assertIn(b"case_path: initialCasePath", BENCH_REPORT_BRIDGE)
        self.assertIn(b"location.pathname + location.search", BENCH_REPORT_BRIDGE)
        self.assertNotIn(b"location.pathname + location.search + location.hash", BENCH_REPORT_BRIDGE)

    def test_report_bridge_does_not_post_incremental_payloads(self) -> None:
        self.assertIn(b"if (source === 'incremental') {", BENCH_REPORT_BRIDGE)
        incremental_start = BENCH_REPORT_BRIDGE.index(b"if (source === 'incremental') {")
        final_payload_start = BENCH_REPORT_BRIDGE.index(b"var snapshot = {")
        incremental_block = BENCH_REPORT_BRIDGE[incremental_start:final_payload_start]

        self.assertIn(b"partial_count: accumulator.tests.length", incremental_block)
        self.assertIn(b"tests: []", incremental_block)
        self.assertIn(b"return;", incremental_block)
        self.assertNotIn(b"accumulator.tests.slice()", incremental_block)

    def test_report_bridge_posts_only_bounded_final_payloads_synchronously(self) -> None:
        self.assertIn(b"if (body.length <= 60000)", BENCH_REPORT_BRIDGE)
        self.assertIn(b"xhr.open('POST', '/__bench__/result', false)", BENCH_REPORT_BRIDGE)
        self.assertLess(
            BENCH_REPORT_BRIDGE.index(b"xhr.open('POST', '/__bench__/result', false)"),
            BENCH_REPORT_BRIDGE.rindex(b"window.__bench_wpt__ = snapshot;"),
        )

    def test_report_bridge_writes_dom_payload_before_posting(self) -> None:
        self.assertIn(b"__bench_wpt_payload", BENCH_REPORT_BRIDGE)
        self.assertLess(
            BENCH_REPORT_BRIDGE.index(b"node.textContent = body;"),
            BENCH_REPORT_BRIDGE.index(b"xhr.open('POST', '/__bench__/result', false)"),
        )

    def test_report_bridge_disables_in_page_testharness_output(self) -> None:
        self.assertIn(b"output: false", BENCH_REPORT_BRIDGE)

    def test_report_bridge_applies_configured_timeout_multiplier(self) -> None:
        bridge = _bench_report_bridge(7.5)

        self.assertIn(b"timeout_multiplier: 7.5", bridge)

    def test_report_bridge_config_is_injected_without_changing_case_url(self) -> None:
        body = (
            b'<!doctype html><script src="/resources/testharness.js"></script>'
            b'<script src="/resources/testharnessreport.js"></script>'
            b"<script>test(() => location.search, 'query visible to case')</script>"
        )

        injected = _inject_bench_report_bridge_config(body, 3.0)

        self.assertIn(
            (
                b'src="/resources/testharnessreport.js?'
                + BENCH_TIMEOUT_MULTIPLIER_QUERY.encode("ascii")
                + b'=3"'
            ),
            injected,
        )
        self.assertIn(b"location.search", injected)

    def test_report_bridge_config_keeps_default_multiplier_body_unchanged(self) -> None:
        body = (
            b'<!doctype html><script src="/resources/testharness.js"></script>'
            b'<script src="/resources/testharnessreport.js"></script>'
        )

        self.assertEqual(_inject_bench_report_bridge_config(body, 1.0), body)

    def test_harness_case_key_ignores_url_fragment(self) -> None:
        self.assertEqual(
            _normalize_harness_case_key("/dom/ranges/feature.html?mode=open#frag"),
            "dom/ranges/feature.html?mode=open",
        )

    def test_report_bridge_does_not_publish_empty_done_fallback(self) -> None:
        self.assertIn(b"function canPublishDoneFallback()", BENCH_REPORT_BRIDGE)
        self.assertIn(
            b"window.__bench_wpt__ === undefined && canPublishDoneFallback()",
            BENCH_REPORT_BRIDGE,
        )
        self.assertIn(
            b"canPublishDoneFallback()) {\n            publish('done-hook-late', null);",
            BENCH_REPORT_BRIDGE,
        )

    def test_case_result_dict_records_payload_source(self) -> None:
        row = case_result_to_dict(
            CaseResult(
                case_path="example.html",
                url="http://example.test/example.html",
                status="pass",
                duration_ms=1.0,
                payload_source="completion-callback",
            )
        )

        self.assertEqual(row["payload_source"], "completion-callback")

    def test_case_result_dict_records_reftest_comparisons_and_artifacts(self) -> None:
        row = case_result_to_dict(
            CaseResult(
                case_path="css/example.html",
                url="http://example.test/css/example.html",
                status="fail",
                duration_ms=1.0,
                test_type="reftest",
                reftest_comparisons=[
                    {
                        "reference_path": "css/example-ref.html",
                        "relation": "==",
                        "passed": False,
                        "max_difference": 255,
                        "different_pixels": 10,
                    }
                ],
                artifacts={
                    "test": "artifacts/moli/example/test.png",
                    "references": [
                        {
                            "reference": "artifacts/moli/example/reference-01.png",
                            "diff": "artifacts/moli/example/diff-01.png",
                        }
                    ],
                },
            )
        )

        self.assertEqual(row["test_type"], "reftest")
        self.assertEqual(row["reftest_comparisons"][0]["relation"], "==")
        self.assertEqual(row["artifacts"]["test"], "artifacts/moli/example/test.png")

    def test_cli_runner_extracts_payload_from_stdout_html(self) -> None:
        payload = {
            "case_path": "/case.html",
            "harness": {"status": 0, "message": None},
            "tests": [{"name": "ok", "status": 0}],
            "source": "completion-callback",
        }
        stdout = (
            "<!doctype html><html><pre id=\"__bench_wpt_payload\" hidden>"
            + json.dumps(payload)
            + "</pre></html>"
        ).encode()

        self.assertEqual(_payload_from_stdout_html(stdout), payload)

    def test_cli_runner_prefers_final_stdout_payload_without_callback_wait(self) -> None:
        class UnexpectedResultsAccess:
            def wait_for_final(self, key: str, timeout: float) -> None:
                raise AssertionError("final stdout payload should skip callback wait")

            def get(self, key: str) -> None:
                raise AssertionError("final stdout payload should skip callback lookup")

        payload = {
            "case_path": "/case.html",
            "harness": {"status": 0, "message": None},
            "tests": [{"name": "ok", "status": 0}],
            "source": "completion-callback",
        }
        stdout = (
            '<!doctype html><pre id="__bench_wpt_payload" hidden>'
            + json.dumps(payload)
            + "</pre>"
        ).encode()

        result = _classify_cli_case_result(
            case_path="case.html",
            url="http://example.test/case.html",
            bridge_key="/case.html",
            fixture_server=SimpleNamespace(results=UnexpectedResultsAccess()),
            subprocess_result=_CliSubprocessResult(
                duration_ms=10.0,
                proc_error=None,
                proc_returncode=0,
                proc_stderr="",
                proc_stdout=stdout,
                wait_script_timeout=False,
            ),
            payload_grace_seconds=2.0,
            successful_process_payload_grace_seconds=8.0,
        )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.payload_source, "completion-callback")

    def test_case_result_dict_records_failure_details(self) -> None:
        row = case_result_to_dict(
            CaseResult(
                case_path="example.html",
                url="http://example.test/example.html",
                status="fail",
                duration_ms=1.0,
                failures=[
                    {
                        "name": "subtest name",
                        "status": 1,
                        "status_name": "FAIL",
                        "message": "expected true got false",
                    }
                ],
            )
        )

        self.assertEqual(
            row["failures"],
            [
                {
                    "name": "subtest name",
                    "status": 1,
                    "status_name": "FAIL",
                    "message": "expected true got false",
                }
            ],
        )

    def test_classify_payload_records_limited_failure_details(self) -> None:
        result = classify_payload(
            payload={
                "source": "completion-callback",
                "harness": {"status": 0},
                "tests": [
                    {"name": "ok", "status": 0},
                    {
                        "name": "bad",
                        "status": 1,
                        "message": "x" * 700,
                    },
                    {"name": "notrun", "status": 3},
                ],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.subtests_total, 3)
        self.assertEqual([failure["name"] for failure in result.failures], ["bad", "notrun"])
        self.assertEqual(result.failure_names, ["bad", "notrun"])
        self.assertEqual(result.failures[0]["status_name"], "FAIL")
        self.assertEqual(len(result.failures[0]["message"]), 500)
        self.assertTrue(result.failures[0]["message_truncated"])

    def test_recorded_failure_drift_reports_hidden_subtest_differences(self) -> None:
        drift = _recorded_failure_drift(
            [
                {
                    "case_path": "webcrypto/shared-fail.html",
                    "results": {
                        "moli": {
                            "status": "fail",
                            "failures": [
                                {"name": "shared", "message": "lm detail"},
                                {"name": "lm-only", "message": "bad"},
                            ],
                        },
                        "chrome": {
                            "status": "fail",
                            "failures": [
                                {"name": "shared", "message": "chrome detail"},
                                {"name": "chrome-only", "message": "bad"},
                            ],
                        },
                    },
                }
            ],
            ["moli", "chrome"],
        )

        self.assertEqual(drift["comparison_count"], 1)
        self.assertEqual(drift["primary_only_comparison_count"], 1)
        self.assertEqual(drift["peer_only_comparison_count"], 1)
        self.assertEqual(drift["message_diff_comparison_count"], 1)
        row = drift["comparisons"][0]
        self.assertEqual(row["primary_only_examples"], ["lm-only"])
        self.assertEqual(row["peer_only_examples"], ["chrome-only"])
        self.assertEqual(row["message_diff_examples"], ["shared"])

    def test_recorded_failure_drift_uses_full_failure_names_when_available(self) -> None:
        drift = _recorded_failure_drift(
            [
                {
                    "case_path": "webcrypto/large-shared-fail.html",
                    "results": {
                        "moli": {
                            "status": "fail",
                            "failures": [{"name": "recorded", "message": "same"}],
                            "failure_names": ["recorded", "late-moli-only"],
                        },
                        "chrome": {
                            "status": "fail",
                            "failures": [{"name": "recorded", "message": "same"}],
                            "failure_names": ["recorded"],
                        },
                    },
                }
            ],
            ["moli", "chrome"],
        )

        self.assertEqual(drift["comparison_count"], 1)
        self.assertEqual(drift["primary_only_comparison_count"], 1)
        self.assertEqual(drift["comparisons"][0]["primary_only_examples"], ["late-moli-only"])

    def test_classify_payload_does_not_pass_incremental_only_result(self) -> None:
        result = classify_payload(
            payload={
                "source": "incremental",
                "harness": {"status": None},
                "tests": [{"name": "observed pass", "status": 0}],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "harness-stalled")
        self.assertEqual(result.subtests_total, 1)
        self.assertEqual(result.subtests_pass, 1)
        self.assertEqual(result.payload_source, "incremental")
        self.assertIn("incremental", result.error or "")

    def test_classify_payload_does_not_pass_empty_final_result(self) -> None:
        result = classify_payload(
            payload={
                "source": "completion-callback",
                "harness": {"status": 0},
                "tests": [],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.subtests_total, 0)
        self.assertIn("without reporting any subtests", result.error or "")

    def test_classify_payload_includes_empty_harness_error_message(self) -> None:
        result = classify_payload(
            payload={
                "source": "completion-callback",
                "harness": {
                    "status": 1,
                    "message": "Unhandled rejection: cyclic wasm dependency",
                },
                "tests": [],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.harness_status, 1)
        self.assertEqual(result.subtests_total, 0)
        self.assertIn("without reporting any subtests", result.error or "")
        self.assertIn("cyclic wasm dependency", result.error or "")

    def test_classify_payload_keeps_observed_incremental_failure(self) -> None:
        result = classify_payload(
            payload={
                "source": "incremental",
                "harness": {"status": None},
                "tests": [{"name": "observed fail", "status": 1}],
            },
            case_path="example.html",
            url="http://example.test/example.html",
            duration_ms=1.0,
            bridge_installed=True,
        )

        self.assertEqual(result.status, "fail")
        self.assertEqual(result.subtests_fail, 1)
        self.assertEqual(result.payload_source, "incremental")

    def test_results_store_wait_for_final_does_not_return_incremental(self) -> None:
        store = ResultsStore()
        store.put("example.html", {"source": "incremental"})

        self.assertIsNone(store.wait_for_final("example.html", timeout=0))
        self.assertEqual(store.get("example.html"), {"source": "incremental"})

    def test_testdriver_vendor_bridge_provides_action_sequence(self) -> None:
        self.assertIn(b"action_sequence", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"pointerMove", BENCH_TESTDRIVER_VENDOR_BRIDGE)

    def test_testdriver_vendor_bridge_preserves_key_modifiers(self) -> None:
        self.assertIn(b"altKey: !!modifiers.Alt", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"ctrlKey: !!modifiers.Control", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"shiftKey: !!modifiers.Shift", BENCH_TESTDRIVER_VENDOR_BRIDGE)

    def test_testdriver_vendor_bridge_focuses_user_activation_target(self) -> None:
        self.assertIn(b"focusForUserActivation", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"document.activeElement === before", BENCH_TESTDRIVER_VENDOR_BRIDGE)

    def test_testdriver_vendor_bridge_falls_back_to_recorded_hit_test_targets(self) -> None:
        self.assertIn(b"nativeGetClientRects.apply", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"nativeElementsFromPoint.apply", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"return target ? [target] : []", BENCH_TESTDRIVER_VENDOR_BRIDGE)

    def test_testdriver_vendor_bridge_accepts_storage_access_permission_setup(self) -> None:
        self.assertIn(
            b"params.descriptor.name === 'storage-access'",
            BENCH_TESTDRIVER_VENDOR_BRIDGE,
        )
        self.assertIn(
            b"set_permission() is not implemented by the Moli WPT bridge",
            BENCH_TESTDRIVER_VENDOR_BRIDGE,
        )

    def test_testdriver_vendor_bridge_provides_computed_label(self) -> None:
        self.assertIn(b"get_computed_label", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"resolveReferenceTarget", BENCH_TESTDRIVER_VENDOR_BRIDGE)
        self.assertIn(b"data-expectedlabel", BENCH_TESTDRIVER_VENDOR_BRIDGE)

    def test_fixture_server_parses_wpt_trickle_pipe_delay(self) -> None:
        self.assertEqual(_pipe_trickle_delay_seconds("pipe=trickle(d1)&cachebust=1"), 1.0)
        self.assertEqual(_pipe_trickle_delay_seconds("pipe=header(X,Y)|trickle(d2.5)"), 2.5)
        self.assertEqual(
            _pipe_trickle_delay_seconds("pipe=trickle(d3)&pipe=trickle(d1)"),
            3.0,
        )
        self.assertEqual(_pipe_trickle_delay_seconds("pipe=trickle(d999)"), 10.0)
        self.assertEqual(_pipe_trickle_delay_seconds("notpipe=trickle(d1)"), 0.0)

    def test_fixture_server_parses_delay_handler_duration(self) -> None:
        self.assertEqual(_wpt_delay_seconds("ms=3000"), 3.0)
        self.assertEqual(_wpt_delay_seconds("ms=2.5"), 0.0025)
        self.assertEqual(_wpt_delay_seconds("ms=250&ms=750"), 0.25)
        self.assertEqual(_wpt_delay_seconds(""), 0.5)
        self.assertIsNone(_wpt_delay_seconds("ms=invalid"))
        self.assertIsNone(_wpt_delay_seconds("ms=-1"))
        self.assertIsNone(_wpt_delay_seconds("ms=nan"))

    def test_fixture_server_models_xhr_delay_py_methods(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            with WptFixtureServer(root_path) as server:
                with patch(
                    "moli_benchmark.wpt_cross.server.time.sleep"
                ) as sleep_mock:
                    url = f"{server.base_url}/xhr/resources/delay.py?ms=250"
                    responses = []
                    for method in ("GET", "HEAD", "POST", "OPTIONS", "YO"):
                        request = Request(
                            url,
                            data=b"upload" if method in {"POST", "YO"} else None,
                            method=method,
                        )
                        with urlopen(request, timeout=2) as response:
                            responses.append(
                                (
                                    method,
                                    response.status,
                                    response.read(),
                                    response.headers["Content-Type"],
                                    response.headers["Access-Control-Allow-Origin"],
                                    response.headers["Access-Control-Allow-Methods"],
                                )
                            )

        self.assertEqual(
            [call.args for call in sleep_mock.call_args_list],
            [(0.25,)] * 5,
        )
        self.assertEqual(
            responses,
            [
                ("GET", 200, b"TEST_DELAY", "text/plain", "*", "YO"),
                ("HEAD", 200, b"", "text/plain", "*", "YO"),
                ("POST", 200, b"TEST_DELAY", "text/plain", "*", "YO"),
                ("OPTIONS", 200, b"TEST_DELAY", "text/plain", "*", "YO"),
                ("YO", 200, b"TEST_DELAY", "text/plain", "*", "YO"),
            ],
        )

    def test_fixture_server_models_delayed_module_script_handler(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            with WptFixtureServer(root_path) as server:
                with patch(
                    "moli_benchmark.wpt_cross.server.time.sleep"
                ) as sleep_mock:
                    url = (
                        f"{server.base_url}/html/semantics/scripting-1/"
                        "the-script-element/module/resources/"
                        "delayed-modulescript.py?ms=250"
                    )
                    responses = []
                    for method in ("GET", "HEAD"):
                        request = Request(url, method=method)
                        with urlopen(request, timeout=2) as response:
                            responses.append(
                                (
                                    method,
                                    response.status,
                                    response.read(),
                                    response.headers["Content-Type"],
                                )
                            )

        self.assertEqual(
            [call.args for call in sleep_mock.call_args_list],
            [(0.25,), (0.25,)],
        )
        self.assertEqual(
            responses,
            [
                (
                    "GET",
                    200,
                    b"export let delayedLoaded = true;",
                    "text/javascript",
                ),
                ("HEAD", 200, b"", "text/javascript"),
            ],
        )

    def test_fixture_server_models_common_redirect_opt_in_handler(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            with WptFixtureServer(root_path) as server:
                connection = HTTPConnection("127.0.0.1", server.port, timeout=2)
                connection.request(
                    "GET",
                    "/common/redirect-opt-in.py?status=307&location=%2Ftarget",
                )
                response = connection.getresponse()
                self.assertEqual(response.status, 307)
                self.assertEqual(response.headers.get("Location"), "/target")
                self.assertEqual(response.headers.get("Timing-Allow-Origin"), "*")
                connection.close()

    def test_fixture_server_drains_xhr_delay_yo_body_before_next_request(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            server = WptFixtureServer(root_path)
            server.httpd.RequestHandlerClass.protocol_version = "HTTP/1.1"
            with server, patch(
                "moli_benchmark.wpt_cross.server.time.sleep"
            ):
                connection = HTTPConnection("localhost", server.port, timeout=2)
                try:
                    path = "/xhr/resources/delay.py?ms=0"
                    connection.request("YO", path, body=b"upload")
                    first = connection.getresponse()
                    self.assertEqual(
                        (first.status, first.read()),
                        (200, b"TEST_DELAY"),
                    )
                    first_socket = connection.sock

                    connection.request("GET", path)
                    second = connection.getresponse()
                    self.assertEqual(
                        (second.status, second.read()),
                        (200, b"TEST_DELAY"),
                    )
                    self.assertIs(connection.sock, first_socket)
                finally:
                    connection.close()

    def test_fixture_server_drains_chunked_xhr_delay_body_before_next_request(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            (root_path / "resources").mkdir()
            (root_path / "resources" / "testharness.js").write_text(
                "// testharness", encoding="utf-8"
            )
            server = WptFixtureServer(root_path)
            server.httpd.RequestHandlerClass.protocol_version = "HTTP/1.1"
            with server, patch(
                "moli_benchmark.wpt_cross.server.time.sleep"
            ):
                connection = HTTPConnection("localhost", server.port, timeout=2)
                try:
                    path = "/xhr/resources/delay.py?ms=0"
                    connection.request(
                        "POST",
                        path,
                        body=[b"chunk-one", b"chunk-two"],
                        encode_chunked=True,
                    )
                    first = connection.getresponse()
                    self.assertEqual(
                        (first.status, first.read()),
                        (200, b"TEST_DELAY"),
                    )
                    first_socket = connection.sock

                    connection.request("GET", path)
                    second = connection.getresponse()
                    self.assertEqual(
                        (second.status, second.read()),
                        (200, b"TEST_DELAY"),
                    )
                    self.assertIs(connection.sock, first_socket)
                finally:
                    connection.close()

    def test_fixture_server_parses_wpt_header_pipe(self) -> None:
        self.assertEqual(
            _pipe_response_headers(
                "pipe=header(Access-Control-Allow-Origin,*)|header(X-Test,ok)"
            ),
            [("Access-Control-Allow-Origin", "*"), ("X-Test", "ok")],
        )
        self.assertEqual(_pipe_response_headers("pipe=header(Bad Header,ok)"), [])
        self.assertEqual(
            _pipe_response_headers("pipe=header(X-Test,bad%0D%0AInjected:%20x)"),
            [],
        )
        self.assertEqual(_pipe_response_headers("notpipe=header(X,Y)"), [])

    def test_fixture_server_parses_wpt_status_pipe(self) -> None:
        self.assertEqual(_pipe_response_status("pipe=status(204)&cachebust=1"), 204)
        self.assertEqual(
            _pipe_response_status("pipe=header(X,Y)|status(205)"),
            205,
        )
        self.assertEqual(
            _pipe_response_status("pipe=status(204)&pipe=status(205)"),
            205,
        )
        self.assertIsNone(_pipe_response_status("pipe=status(099)"))
        self.assertIsNone(_pipe_response_status("pipe=status(600)"))
        self.assertIsNone(_pipe_response_status("notpipe=status(204)"))

    def test_fixture_server_parses_wpt_headers_sidecar(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            fixture = Path(root) / "case.html"
            fixture.write_text("<!doctype html>", encoding="utf-8")
            fixture.with_name("case.html.headers").write_text(
                "Referrer-Policy: no-referrer\n"
                "Bad Header: skip\n"
                "X-Test: ok\n",
                encoding="utf-8",
            )

            self.assertEqual(
                _sidecar_response_headers(fixture),
                [("Referrer-Policy", "no-referrer"), ("X-Test", "ok")],
            )

    def test_fixture_server_parses_sub_headers_sidecar_for_sub_files(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            fixture = Path(root) / "worker.sub.js"
            fixture.write_text("// worker", encoding="utf-8")
            fixture.with_name("worker.sub.js.sub.headers").write_text(
                "Content-Security-Policy: connect-src 'none'\n",
                encoding="utf-8",
            )

            self.assertEqual(
                _sidecar_response_headers(fixture),
                [("Content-Security-Policy", "connect-src 'none'")],
            )

    def test_fixture_server_combines_immediate_directory_and_file_headers(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            fixture_dir = root_path / "support"
            fixture_dir.mkdir()
            fixture = fixture_dir / "ufoo"
            fixture.write_text("ufoo", encoding="utf-8")
            fixture_dir.joinpath("__dir__.headers").write_text(
                "Content-Type: text/html\nX-Directory: immediate\n",
                encoding="utf-8",
            )
            fixture.with_name("ufoo.headers").write_text(
                "X-File: exact\n",
                encoding="utf-8",
            )
            root_path.joinpath("__dir__.headers").write_text(
                "X-Directory: parent\n",
                encoding="utf-8",
            )

            self.assertEqual(
                _sidecar_response_headers(fixture),
                [
                    ("Content-Type", "text/html"),
                    ("X-Directory", "immediate"),
                    ("X-File", "exact"),
                ],
            )

    def test_fixture_server_prefers_file_sub_headers_when_both_exist(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            fixture = Path(root) / "case.sub.html"
            fixture.write_text("<!doctype html>", encoding="utf-8")
            fixture.with_name("case.sub.html.headers").write_text(
                "X-Source: plain\n",
                encoding="utf-8",
            )
            fixture.with_name("case.sub.html.sub.headers").write_text(
                "X-Source: substituted\n",
                encoding="utf-8",
            )

            self.assertEqual(
                _sidecar_response_headers(fixture),
                [("X-Source", "substituted")],
            )

    def test_fixture_server_prefers_directory_sub_headers_when_both_exist(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            fixture = root_path / "case.html"
            fixture.write_text("<!doctype html>", encoding="utf-8")
            root_path.joinpath("__dir__.headers").write_text(
                "X-Source: plain\n",
                encoding="utf-8",
            )
            root_path.joinpath("__dir__.sub.headers").write_text(
                "X-Source: substituted\n",
                encoding="utf-8",
            )

            self.assertEqual(
                _sidecar_response_headers(fixture),
                [("X-Source", "substituted")],
            )

    def test_fixture_server_content_type_sidecar_overrides_static_guess(self) -> None:
        content_type, headers = _response_content_type_and_extra_headers(
            "application/octet-stream",
            [
                ("Content-Type", "text/javascript; charset=utf-8"),
                ("X-Test", "ok"),
            ],
        )

        self.assertEqual(content_type, "text/javascript; charset=utf-8")
        self.assertEqual(headers, [("X-Test", "ok")])

    def test_fixture_server_combines_sidecar_and_pipe_headers_for_static_responses(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as root:
            fixture = Path(root) / "feature.any.js"
            fixture.write_text("// test", encoding="utf-8")
            fixture.with_name("feature.any.js.headers").write_text(
                "Content-Security-Policy: script-src 'self'\n",
                encoding="utf-8",
            )

            self.assertEqual(
                _static_response_headers(fixture, "pipe=header(X-Test,ok)"),
                [
                    ("Content-Security-Policy", "script-src 'self'"),
                    ("X-Test", "ok"),
                ],
            )

    def test_fixture_server_reads_sub_headers_for_plain_static_resource(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            fixture = Path(root) / "policy.html"
            fixture.write_text("<!doctype html>", encoding="utf-8")
            fixture.with_name("policy.html.sub.headers").write_text(
                "Set-Cookie: policy={{$id:uuid()}}\n"
                "Content-Security-Policy: frame-src 'none'; report-uri /report.py?reportID={{$id}}\n",
                encoding="utf-8",
            )

            headers = _static_response_headers(fixture, "", port=8000)

        self.assertEqual(len(headers), 2)
        cookie_value = headers[0][1].removeprefix("policy=")
        self.assertEqual(headers[0][0], "Set-Cookie")
        self.assertNotIn("{{", cookie_value)
        self.assertEqual(
            headers[1],
            (
                "Content-Security-Policy",
                f"frame-src 'none'; report-uri /report.py?reportID={cookie_value}",
            ),
        )

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
            b"host={{host}} "
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
            b"host=localhost location=www1.localhost "
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

    def test_fixture_server_parses_asis_response_parts(self) -> None:
        self.assertEqual(
            _asis_response_parts(
                b"HTTP/1.1 200 OK\n"
                b"Content-Type: text/plain\n"
                b"Access-Control-Allow-Origin: *\n"
                b"Content-Length: 999\n"
                b"\n"
                b"FAIL"
            ),
            (
                200,
                b"FAIL",
                [
                    ("Content-Type", "text/plain"),
                    ("Access-Control-Allow-Origin", "*"),
                ],
            ),
        )

    def test_fixture_server_preserves_raw_asis_header_bytes(self) -> None:
        parts = _asis_response_parts(
            b"HTTP/1.1 200 OK\n"
            b"X-Custom-Header-Bytes: \xe2\x80\xa6\n"
            b"\n"
            b"OK"
        )

        self.assertIsNotNone(parts)
        _status, _body, headers = parts
        self.assertIn(("X-Custom-Header-Bytes", "\xe2\x80\xa6"), headers)
        for _name, value in headers:
            value.encode("latin-1")

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

    def test_cli_runner_distinguishes_app_errors_from_process_crashes(self) -> None:
        self.assertEqual(_nonzero_exit_status(1, "Error: unsupported navigation"), "error")
        self.assertEqual(_nonzero_exit_status(-11, ""), "crash")
        self.assertEqual(_nonzero_exit_status(101, "thread 'main' panicked at x"), "crash")

    def test_cli_runner_stderr_tail_is_bounded(self) -> None:
        self.assertEqual(_stderr_tail("short"), "short")
        self.assertEqual(len(_stderr_tail("x" * 3000)), 2000)

    def test_cli_runner_uses_longer_payload_grace_only_after_successful_process(self) -> None:
        self.assertEqual(
            _payload_grace_for_process_result(
                proc_error=None,
                proc_returncode=0,
                payload_grace_seconds=2.0,
                successful_process_payload_grace_seconds=8.0,
            ),
            8.0,
        )
        self.assertEqual(
            _payload_grace_for_process_result(
                proc_error="engine subprocess wall timeout",
                proc_returncode=None,
                payload_grace_seconds=2.0,
                successful_process_payload_grace_seconds=5.0,
            ),
            2.0,
        )
        self.assertEqual(
            _payload_grace_for_process_result(
                proc_error=None,
                proc_returncode=1,
                payload_grace_seconds=2.0,
                successful_process_payload_grace_seconds=5.0,
            ),
            2.0,
        )

    def test_cli_runner_keeps_final_payload_status_after_process_timeout(self) -> None:
        class FakeResults:
            def wait_for_final(self, key: str, timeout: float) -> dict:
                self.wait_key = key
                return {
                    "source": "completion-callback",
                    "harness": {"status": 0, "message": None},
                    "tests": [{"name": "done", "status": 0, "message": None}],
                }

            def get(self, key: str) -> None:
                return None

        fixture_server = SimpleNamespace(
            results=FakeResults(),
        )

        result = _classify_cli_case_result(
            case_path="case.html",
            url="http://example.test/case.html",
            bridge_key="/case.html",
            fixture_server=fixture_server,
            subprocess_result=_CliSubprocessResult(
                duration_ms=10.0,
                proc_error="engine subprocess wall timeout after 0.0s",
                proc_returncode=None,
                proc_stderr="",
                proc_stdout=b"",
                wait_script_timeout=False,
            ),
            payload_grace_seconds=0,
            successful_process_payload_grace_seconds=8.0,
        )

        self.assertEqual(result.status, "pass")
        self.assertIsNone(result.error)
        self.assertEqual(result.payload_source, "completion-callback")

    def test_cli_runner_uses_process_pool_even_for_single_parallelism(self) -> None:
        class FakeFuture:
            def __init__(self, result: CaseResult) -> None:
                self._result = result

            def result(self) -> CaseResult:
                return self._result

        class FakeProcessPoolExecutor:
            instances: list["FakeProcessPoolExecutor"] = []

            def __init__(self, *, max_workers: int, mp_context: object) -> None:
                self.max_workers = max_workers
                self.mp_context = mp_context
                self.submitted = []
                self.instances.append(self)

            def __enter__(self) -> "FakeProcessPoolExecutor":
                return self

            def __exit__(self, *args: object) -> None:
                return None

            def submit(self, fn, job):
                self.submitted.append((fn, job))
                return FakeFuture(fn(job))

        def fake_as_completed(futures):
            return list(futures)

        def fake_worker(job):
            return CaseResult(
                case_path=job.case_path,
                url=f"http://worker.test/{job.case_path}",
                status="pass",
                duration_ms=1.0,
            )

        driver = SimpleNamespace(
            name="moli",
            version_args=["--version"],
            extra_env={},
            cli_fetch_command=lambda binary, url, timeout: [str(binary), "fetch", url],
            resolve_binary=lambda override: Path("/tmp/moli"),
        )
        fixture_server = SimpleNamespace(
            external_host="2001:db8::1",
            external_base_url="http://[2001:db8::1]:9000",
            wpt_root=Path("/tmp/wpt"),
        )

        def fake_subprocess_run(argv, **kwargs):
            self.assertEqual(argv, ["/tmp/moli", "--version"])
            return SimpleNamespace(stdout="moli 0\n", stderr="", returncode=0)

        with (
            patch("moli_benchmark.wpt_cross.cli_runner.sha256_file", return_value="sha"),
            patch("moli_benchmark.wpt_cross.cli_runner.subprocess.run", fake_subprocess_run),
            patch(
                "moli_benchmark.wpt_cross.cli_runner.ProcessPoolExecutor",
                FakeProcessPoolExecutor,
            ),
            patch("moli_benchmark.wpt_cross.cli_runner.as_completed", fake_as_completed),
            patch("moli_benchmark.wpt_cross.cli_runner._run_cli_case_worker", fake_worker),
        ):
            result = run_engine_on_cases_cli(
                driver=driver,
                fixture_server=fixture_server,
                cases=[
                    (
                        "case.html",
                        "http://[2001:db8::1]:9000/case.html",
                        12.0,
                        1.2,
                    )
                ],
                parallelism=1,
                progress_every=0,
            )

        pool = FakeProcessPoolExecutor.instances[0]
        self.assertEqual(pool.max_workers, 1)
        self.assertEqual(pool.mp_context.get_start_method(), "spawn")
        self.assertEqual(pool.submitted[0][1].case_path, "case.html")
        self.assertTrue(pool.submitted[0][1].external)
        self.assertEqual(pool.submitted[0][1].timeout_seconds, 12.0)
        self.assertEqual(pool.submitted[0][1].harness_timeout_multiplier, 1.2)
        self.assertEqual(result.cases[0].status, "pass")
        self.assertEqual(result.shutdown_info["scheduler"], "process-pool")

    def test_moli_cli_worker_appends_hardcoded_wpt_user_agent(self) -> None:
        captured_argv = []

        class FakeResults:
            def clear(self, key: str) -> None:
                return None

            def wait_for_final(self, key: str, timeout: float) -> dict:
                return {
                    "source": "completion-callback",
                    "harness": {"status": 0, "message": None},
                    "tests": [{"name": "done", "status": 0, "message": None}],
                }

            def get(self, key: str) -> None:
                return None

        class FakeServer:
            def __init__(self, wpt_root: Path) -> None:
                self.wpt_root = Path(wpt_root)
                self.results = FakeResults()
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
                return f"http://127.0.0.1:8000/{case_path}"

        driver = SimpleNamespace(
            name="moli",
            cli_fetch_command=lambda binary, url, timeout: [str(binary), "fetch", url],
        )

        def fake_run_cli_subprocess(argv, env, proc_timeout):
            captured_argv.append(argv)
            return _CliSubprocessResult(
                duration_ms=1.0,
                proc_error=None,
                proc_returncode=0,
                proc_stderr="",
                proc_stdout=b"",
                wait_script_timeout=False,
            )

        with (
            patch(
                "moli_benchmark.wpt_cross.cli_runner.build_driver",
                return_value=driver,
            ),
            patch(
                "moli_benchmark.wpt_cross.cli_runner.WptFixtureServer",
                FakeServer,
            ),
            patch(
                "moli_benchmark.wpt_cross.cli_runner._run_cli_subprocess",
                fake_run_cli_subprocess,
            ),
        ):
            result = _run_cli_case_worker(
                _CliCaseWorkerInput(
                    engine="moli",
                    binary="/tmp/moli",
                    wpt_root="/tmp/wpt",
                    case_path="case.html",
                    external=False,
                    timeout_seconds=8.0,
                    harness_timeout_multiplier=1.0,
                    env={},
                    process_timeout_margin_seconds=4.0,
                    payload_grace_seconds=0.0,
                    successful_process_payload_grace_seconds=0.0,
                )
            )

        self.assertEqual(result.status, "pass")
        self.assertEqual(
            captured_argv,
            [
                [
                    "/tmp/moli",
                    "fetch",
                    "http://127.0.0.1:8000/case.html",
                    "--user-agent",
                    MOLI_WPT_USER_AGENT,
                ]
            ],
        )

    def test_moli_cli_resolves_external_fixture_template_hosts(self) -> None:
        fixture_server = SimpleNamespace(
            external_host="2001:db8::42",
            external_port=12345,
            external_alternate_port=23456,
            external_remote_port=23456,
        )

        args = _moli_fixture_host_resolve_args(fixture_server)

        self.assertIn(
            "--http-host-resolve",
            args,
        )
        self.assertIn(
            "alt.localhost:12345:[2001:db8::42]",
            args,
        )
        self.assertIn(
            "www.localhost:23456:[2001:db8::42]",
            args,
        )
        self.assertEqual(
            args.count("localhost:23456:[2001:db8::42]"),
            1,
        )

    def test_render_html_escapes_embedded_json_script_data(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            (output_dir / "summary.json").write_text(
                json.dumps({"total": 1, "engines": {"moli": {"pass": 1}}}),
                encoding="utf-8",
            )
            (output_dir / "matrix.json").write_text(
                json.dumps(
                    [
                        {
                            "case_path": "xss.html",
                            "results": {
                                "moli": {
                                    "status": "</script><img src=x onerror=alert(1)>",
                                    "duration_ms": 1,
                                }
                            },
                        }
                    ]
                ),
                encoding="utf-8",
            )

            html = render_html(output_dir).read_text(encoding="utf-8")

        self.assertNotIn("</script><img", html)
        self.assertIn("\\u003c/script\\u003e", html)

    def test_render_html_includes_recorded_failure_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            (output_dir / "summary.json").write_text(
                json.dumps(
                    {
                        "total": 1,
                        "engines": {"moli": {"fail": 1}, "chrome": {"fail": 1}},
                        "recorded_failure_drift": {
                            "primary": "moli",
                            "recorded_failure_limit_per_engine": 40,
                            "comparison_count": 1,
                            "comparisons": [
                                {
                                    "case_path": "WebCryptoAPI/shared.html",
                                    "primary": "moli",
                                    "peer": "chrome",
                                    "primary_only_count": 1,
                                    "peer_only_count": 0,
                                    "message_diff_count": 0,
                                    "primary_only_examples": ["lm-only"],
                                    "peer_only_examples": [],
                                    "message_diff_examples": [],
                                }
                            ],
                        },
                    }
                ),
                encoding="utf-8",
            )
            (output_dir / "matrix.json").write_text(
                json.dumps(
                    [
                        {
                            "case_path": "WebCryptoAPI/shared.html",
                            "results": {
                                "moli": {"status": "fail", "duration_ms": 1},
                                "chrome": {"status": "fail", "duration_ms": 1},
                            },
                        }
                    ]
                ),
                encoding="utf-8",
            )

            html = render_html(output_dir).read_text(encoding="utf-8")

        self.assertIn("Recorded subtest drift", html)
        self.assertIn("lm-only", html)

    def test_render_html_includes_known_failure_audit_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = Path(temp_dir)
            (output_dir / "summary.json").write_text(
                json.dumps(
                    {
                        "total": 1,
                        "engines": {"moli": {"fail": 1}},
                        "known_failure_audits": {
                            "moli": {
                                "ok": False,
                                "manifest": "known.json",
                                "output": "known-failure-audit-moli.json",
                                "counts": {
                                    "known_failures": 1,
                                    "resolved_known_failures": 1,
                                    "mismatched_known_failures": 0,
                                    "missing_expected_failures": 0,
                                    "skipped_known_failures": 2,
                                    "unexpected_failures": 0,
                                },
                                "category_counts": {
                                    "known_failures": {
                                        "wasm-global-live-binding": 1,
                                    }
                                },
                                "categories": {
                                    "wasm-global-live-binding": {
                                        "tracking_doc": "docs/wasm-global-live-binding-design-current.md",
                                        "scope": "V8-backed live binding work",
                                        "evidence": [
                                            {
                                                "kind": "doc",
                                                "path": "docs/wasm-global-live-binding-design-current.md",
                                                "note": "local fixture evidence",
                                            }
                                        ],
                                    }
                                },
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            (output_dir / "matrix.json").write_text(
                json.dumps(
                    [
                        {
                            "case_path": "wasm/example.html",
                            "results": {
                                "moli": {
                                    "status": "fail",
                                    "duration_ms": 1,
                                }
                            },
                        }
                    ]
                ),
                encoding="utf-8",
            )

            html = render_html(output_dir).read_text(encoding="utf-8")

        self.assertIn("Known-failure audit", html)
        self.assertIn("known-failure-audit-moli.json", html)
        self.assertIn("wasm-global-live-binding", html)
        self.assertIn("docs/wasm-global-live-binding-design-current.md", html)
        self.assertIn("V8-backed live binding work", html)
        self.assertIn("doc <code>docs/wasm-global-live-binding-design-current.md</code>", html)
        self.assertIn("local fixture evidence", html)
        self.assertIn("<th>skipped</th>", html)
        self.assertIn("<td>2</td>", html)
        self.assertIn(">attention<", html)


if __name__ == "__main__":
    unittest.main()
