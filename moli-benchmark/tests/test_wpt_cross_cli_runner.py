from __future__ import annotations

from wpt_cross_test_support import *


class WptCrossCliRunnerTests(WptCrossTestCase):
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
    def test_cli_runner_defaults_harness_timeout_independent_of_process_deadline(self) -> None:
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
                        "normal.html",
                        "http://[2001:db8::1]:9000/normal.html",
                        120.0,
                    ),
                    (
                        "long.html",
                        "http://[2001:db8::1]:9000/long.html",
                        120.0,
                        LONG_TIMEOUT_MULTIPLIER,
                    ),
                    (
                        "mixed.https.html",
                        "https://web-platform.test:8443/mixed.https.html",
                        120.0,
                    ),
                ],
                parallelism=1,
                progress_every=0,
            )

        pool = FakeProcessPoolExecutor.instances[0]
        self.assertEqual(pool.max_workers, 1)
        self.assertEqual(pool.mp_context.get_start_method(), "spawn")
        self.assertEqual(pool.submitted[0][1].case_path, "normal.html")
        self.assertTrue(pool.submitted[0][1].external)
        self.assertEqual(pool.submitted[0][1].timeout_seconds, 120.0)
        self.assertEqual(pool.submitted[0][1].harness_timeout_multiplier, 1.0)
        self.assertEqual(
            pool.submitted[1][1].harness_timeout_multiplier,
            LONG_TIMEOUT_MULTIPLIER,
        )
        self.assertFalse(pool.submitted[0][1].mixed_content)
        self.assertTrue(pool.submitted[2][1].mixed_content)
        self.assertFalse(pool.submitted[2][1].external)
        self.assertTrue(all(case.status == "pass" for case in result.cases))
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
            def __init__(self, wpt_root: Path, *, primary_hostname: str) -> None:
                assert primary_hostname == "web-platform.localhost"
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
