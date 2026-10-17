from __future__ import annotations

from wpt_cross_test_support import *


class WptCrossCaseConfigurationTests(WptCrossTestCase):
    def test_moli_wpt_commands_enable_layout_scrollbars_and_resources(self) -> None:
        self.assertEqual(
            _moli_command(Path("/bin/moli"), 9222, None),
            [
                "/bin/moli",
                "serve",
                "--layout",
                "--scrollbars",
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
            fetch[:6],
            [
                "/bin/moli",
                "fetch",
                "--layout",
                "--scrollbars",
                "--resource",
                "http://127.0.0.1:8000/case.html",
            ],
        )
    def test_harness_timeout_multiplier_uses_wpt_case_metadata(self) -> None:
        self.assertEqual(_harness_timeout_multiplier(WptCase("normal.html")), 1.0)
        self.assertEqual(
            _harness_timeout_multiplier(
                WptCase("long.html", timeout_multiplier=LONG_TIMEOUT_MULTIPLIER)
            ),
            LONG_TIMEOUT_MULTIPLIER,
        )
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
    def test_parser_exposes_configurable_parallelism_with_stable_timeout(self) -> None:
        parser = _build_parser()
        help_text = parser.format_help()

        self.assertEqual(WPT_CROSS_CASE_TIMEOUT_SECONDS, 120.0)
        self.assertEqual(WPT_CROSS_PARALLELISM, 50)
        self.assertNotIn("--case-timeout", help_text)
        self.assertNotIn("--case-timeout-engine", help_text)
        self.assertIn("--parallelism", help_text)
        self.assertNotIn("--cdp-parallelism", help_text)
        self.assertNotIn("--run-order", parser.format_help())
        self.assertNotIn("--shuffle-seed", parser.format_help())
        required = [
            "--wpt-root",
            "/tmp/wpt",
            "--engine",
            "moli",
            "--output-dir",
            "/tmp/out",
        ]
        self.assertEqual(parser.parse_args(required).parallelism, 50)
        self.assertEqual(
            parser.parse_args([*required, "--parallelism", "7"]).parallelism,
            7,
        )
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
        self.assertEqual(WPT_CROSS_PARALLELISM, 50)
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
