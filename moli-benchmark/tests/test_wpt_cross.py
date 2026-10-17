from __future__ import annotations

from wpt_cross_test_support import *


class WptCrossTests(WptCrossTestCase):
    def test_cdp_command_failure_marks_page_session_unusable(self) -> None:
        navigation_response = {
            "result": {"frameId": "FRAME-1", "loaderId": "LOADER-1"},
            "sessionId": "SESSION-1",
        }
        navigation_event = {
            "method": "Page.frameNavigated",
            "sessionId": "SESSION-1",
            "params": {
                "frame": {"id": "FRAME-1", "loaderId": "LOADER-1"}
            },
        }
        for receive_effect, error_prefix in (
            (asyncio.TimeoutError(), "navigate failed:"),
            (
                [(navigation_response, [navigation_event]), asyncio.TimeoutError()],
                "evaluate failed:",
            ),
        ):
            with self.subTest(error_prefix=error_prefix):
                client = SimpleNamespace(
                    send=AsyncMock(return_value=1),
                    recv_until_id=AsyncMock(side_effect=receive_effect),
                )
                with self.assertRaises(_PageSessionUnusable) as raised:
                    asyncio.run(
                        _run_one_case(
                            client=client,  # type: ignore[arg-type]
                            session_id="SESSION-1",
                            case_path="example.html",
                            url="http://localhost/example.html",
                            timeout_seconds=0.01,
                        )
                    )

                case_result = raised.exception.case_result
                self.assertEqual(case_result.status, "error")
                self.assertTrue((case_result.error or "").startswith(error_prefix))
                self.assertIn("TimeoutError", case_result.error or "")
    def test_navigation_identity_rejects_failed_or_unbound_navigation(self) -> None:
        with self.assertRaisesRegex(RawCdpError, "ERR_NAME_NOT_RESOLVED"):
            _navigation_identity(
                {
                    "sessionId": "SESSION-1",
                    "result": {
                        "frameId": "FRAME-1",
                        "errorText": "net::ERR_NAME_NOT_RESOLVED",
                    },
                },
                session_id="SESSION-1",
                expected_url="http://localhost/case.html",
            )
        with self.assertRaisesRegex(RawCdpError, "produced a download"):
            _navigation_identity(
                {
                    "sessionId": "SESSION-1",
                    "result": {
                        "frameId": "FRAME-1",
                        "loaderId": "LOADER-1",
                        "isDownload": True,
                    },
                },
                session_id="SESSION-1",
                expected_url="http://localhost/case.html",
            )
        with self.assertRaisesRegex(RawCdpError, "no loaderId"):
            _navigation_identity(
                {
                    "sessionId": "SESSION-1",
                    "result": {"frameId": "FRAME-1"},
                },
                session_id="SESSION-1",
                expected_url="http://localhost/case.html",
            )
    def test_navigation_evidence_requires_exact_session_frame_and_loader(self) -> None:
        identity = _navigation_identity(
            {
                "sessionId": "SESSION-1",
                "result": {"frameId": "FRAME-1", "loaderId": "LOADER-1"},
            },
            session_id="SESSION-1",
            expected_url="http://localhost/case.html",
        )
        stale = {
            "method": "Page.lifecycleEvent",
            "sessionId": "SESSION-1",
            "params": {"frameId": "FRAME-1", "loaderId": "LOADER-OLD"},
        }
        wrong_session = {
            "method": "Page.frameNavigated",
            "sessionId": "SESSION-OLD",
            "params": {
                "frame": {"id": "FRAME-1", "loaderId": "LOADER-1"}
            },
        }
        current = {
            "method": "Page.frameNavigated",
            "sessionId": "SESSION-1",
            "params": {
                "frame": {
                    "id": "FRAME-1",
                    "loaderId": "LOADER-1",
                    "url": "http://localhost/case.html",
                }
            },
        }
        wrong_url = {
            "method": "Page.frameNavigated",
            "sessionId": "SESSION-1",
            "params": {
                "frame": {
                    "id": "FRAME-1",
                    "loaderId": "LOADER-1",
                    "url": "http://localhost/other.html",
                }
            },
        }

        self.assertFalse(_has_navigation_evidence([stale, wrong_session], identity))
        self.assertFalse(_has_navigation_evidence([wrong_url], identity))
        self.assertTrue(
            _has_navigation_evidence([stale, wrong_session, current], identity)
        )
    def test_case_path_and_url_normalization_ignore_leading_slash_and_fragment(self) -> None:
        self.assertEqual(
            _normalized_case_path("/dom/case.html?variant=1#ignored"),
            "dom/case.html?variant=1",
        )
        self.assertEqual(
            _normalized_navigation_url(
                "HTTP://LOCALHOST:8000/dom/case.html?variant=1#ignored"
            ),
            "http://localhost:8000/dom/case.html?variant=1",
        )
        self.assertEqual(
            _normalized_navigation_url(
                "http://localhost:8000/xhr/case.html?load fires normally"
            ),
            "http://localhost:8000/xhr/case.html?load%20fires%20normally",
        )
    def test_run_one_case_rejects_wrong_initial_commit_url(self) -> None:
        client = SimpleNamespace(
            send=AsyncMock(return_value=1),
            recv_until_id=AsyncMock(
                return_value=(
                    {
                        "sessionId": "SESSION-1",
                        "result": {"frameId": "FRAME-1", "loaderId": "LOADER-1"},
                    },
                    [
                        {
                            "method": "Page.frameNavigated",
                            "sessionId": "SESSION-1",
                            "params": {
                                "frame": {
                                    "id": "FRAME-1",
                                    "loaderId": "LOADER-1",
                                    "url": "http://localhost/wrong.html",
                                }
                            },
                        }
                    ],
                )
            ),
        )

        result = asyncio.run(
            _run_one_case(
                client=client,  # type: ignore[arg-type]
                session_id="SESSION-1",
                case_path="expected.html",
                url="http://localhost/expected.html",
                timeout_seconds=1.0,
            )
        )

        self.assertEqual(result.status, "error")
        self.assertIn("unexpected URL", result.error or "")
        self.assertEqual(client.send.await_count, 1)
    def test_run_one_case_ignores_stale_payload_until_navigation_is_bound(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.commands: list[tuple[str, dict | None, str | None]] = []

            async def send(
                self,
                method: str,
                params: dict | None = None,
                *,
                session_id: str | None = None,
            ) -> int:
                self.commands.append((method, params, session_id))
                return len(self.commands)

            async def recv_until_id(
                self, command_id: int, *, timeout: float
            ) -> tuple[dict, list[dict]]:
                method = self.commands[command_id - 1][0]
                if method == "Page.navigate":
                    return (
                        {
                            "sessionId": "SESSION-1",
                            "result": {
                                "frameId": "FRAME-1",
                                "loaderId": "LOADER-1",
                            },
                        },
                        [
                            {
                                "method": "Page.lifecycleEvent",
                                "sessionId": "SESSION-1",
                                "params": {
                                    "frameId": "FRAME-1",
                                    "loaderId": "LOADER-OLD",
                                },
                            }
                        ],
                    )
                evaluation_count = sum(
                    command[0] == "Runtime.evaluate"
                    for command in self.commands[:command_id]
                )
                payload_case = (
                    "/old-case.html" if evaluation_count == 1 else "/new-case.html"
                )
                payload = {
                    "case_path": payload_case,
                    "source": "completion-callback",
                    "harness": {"status": 0},
                    "tests": [{"name": "current result", "status": 0}],
                }
                events = []
                if evaluation_count == 2:
                    events.append(
                        {
                            "method": "Page.frameNavigated",
                            "sessionId": "SESSION-1",
                            "params": {
                                "frame": {
                                    "id": "FRAME-1",
                                    "loaderId": "LOADER-1",
                                    "url": "http://localhost/new-case.html",
                                }
                            },
                        }
                    )
                return (
                    {
                        "result": {
                            "result": {
                                "value": {
                                    # The test may legitimately change its live
                                    # URL after the initial loader committed.
                                    "href": "http://localhost/after-push-state",
                                    "casePath": "/after-push-state",
                                    "bridgeInstalled": True,
                                    "payload": payload,
                                }
                            }
                        }
                    },
                    events,
                )

        client = FakeClient()
        result = asyncio.run(
            _run_one_case(
                client=client,  # type: ignore[arg-type]
                session_id="SESSION-1",
                case_path="new-case.html",
                url="http://localhost/new-case.html",
                timeout_seconds=1.0,
            )
        )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.subtests_pass, 1)
        self.assertEqual(
            sum(command[0] == "Runtime.evaluate" for command in client.commands),
            2,
        )
    def test_run_one_case_gives_in_flight_probe_the_remaining_case_budget(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.commands: list[tuple[str, dict | None, str | None]] = []
                self.probe_timeout: float | None = None

            async def send(
                self,
                method: str,
                params: dict | None = None,
                *,
                session_id: str | None = None,
            ) -> int:
                self.commands.append((method, params, session_id))
                return len(self.commands)

            async def recv_until_id(
                self, command_id: int, *, timeout: float
            ) -> tuple[dict, list[dict]]:
                method = self.commands[command_id - 1][0]
                if method == "Page.navigate":
                    return (
                        {
                            "sessionId": "SESSION-1",
                            "result": {
                                "frameId": "FRAME-1",
                                "loaderId": "LOADER-1",
                            },
                        },
                        [
                            {
                                "method": "Page.frameNavigated",
                                "sessionId": "SESSION-1",
                                "params": {
                                    "frame": {
                                        "id": "FRAME-1",
                                        "loaderId": "LOADER-1",
                                        "url": "http://localhost/slow.html",
                                    }
                                },
                            }
                        ],
                    )

                self.probe_timeout = timeout
                # Model a renderer-bound command that needs more than the old
                # five-second receive cap without making this test actually wait.
                if timeout <= 5.0:
                    raise asyncio.TimeoutError()
                return (
                    {
                        "result": {
                            "result": {
                                "value": {
                                    "href": "http://localhost/slow.html",
                                    "casePath": "/slow.html",
                                    "bridgeInstalled": True,
                                    "payload": {
                                        "case_path": "/slow.html",
                                        "source": "completion-callback",
                                        "harness": {"status": 0},
                                        "tests": [
                                            {"name": "slow result", "status": 0}
                                        ],
                                    },
                                }
                            }
                        }
                    },
                    [],
                )

        client = FakeClient()
        result = asyncio.run(
            _run_one_case(
                client=client,  # type: ignore[arg-type]
                session_id="SESSION-1",
                case_path="slow.html",
                url="http://localhost/slow.html",
                timeout_seconds=30.0,
            )
        )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.subtests_pass, 1)
        self.assertIsNotNone(client.probe_timeout)
        self.assertGreater(client.probe_timeout or 0.0, 25.0)

    def test_run_one_case_gives_in_flight_probe_the_remaining_case_budget(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.commands: list[tuple[str, dict | None, str | None]] = []
                self.probe_timeout: float | None = None

            async def send(
                self,
                method: str,
                params: dict | None = None,
                *,
                session_id: str | None = None,
            ) -> int:
                self.commands.append((method, params, session_id))
                return len(self.commands)

            async def recv_until_id(
                self, command_id: int, *, timeout: float
            ) -> tuple[dict, list[dict]]:
                method = self.commands[command_id - 1][0]
                if method == "Page.navigate":
                    return (
                        {
                            "sessionId": "SESSION-1",
                            "result": {
                                "frameId": "FRAME-1",
                                "loaderId": "LOADER-1",
                            },
                        },
                        [
                            {
                                "method": "Page.frameNavigated",
                                "sessionId": "SESSION-1",
                                "params": {
                                    "frame": {
                                        "id": "FRAME-1",
                                        "loaderId": "LOADER-1",
                                        "url": "http://localhost/slow.html",
                                    }
                                },
                            }
                        ],
                    )

                self.probe_timeout = timeout
                # Model a renderer-bound command that needs more than the old
                # five-second receive cap without making this test actually wait.
                if timeout <= 5.0:
                    raise asyncio.TimeoutError()
                return (
                    {
                        "result": {
                            "result": {
                                "value": {
                                    "href": "http://localhost/slow.html",
                                    "casePath": "/slow.html",
                                    "bridgeInstalled": True,
                                    "payload": {
                                        "case_path": "/slow.html",
                                        "source": "completion-callback",
                                        "harness": {"status": 0},
                                        "tests": [
                                            {"name": "slow result", "status": 0}
                                        ],
                                    },
                                }
                            }
                        }
                    },
                    [],
                )

        client = FakeClient()
        result = asyncio.run(
            _run_one_case(
                client=client,  # type: ignore[arg-type]
                session_id="SESSION-1",
                case_path="slow.html",
                url="http://localhost/slow.html",
                timeout_seconds=30.0,
            )
        )

        self.assertEqual(result.status, "pass")
        self.assertEqual(result.subtests_pass, 1)
        self.assertIsNotNone(client.probe_timeout)
        self.assertGreater(client.probe_timeout or 0.0, 25.0)

    def test_close_page_disposes_context_with_all_auxiliary_targets(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.commands: list[tuple[str, dict | None]] = []
                self.get_targets_count = 0

            async def send(
                self,
                method: str,
                params: dict | None = None,
                *,
                session_id: str | None = None,
            ) -> int:
                self.commands.append((method, params))
                return len(self.commands)

            async def recv_until_id(
                self, command_id: int, *, timeout: float
            ) -> tuple[dict, list[dict]]:
                method = self.commands[command_id - 1][0]
                if method == "Target.getTargets":
                    self.get_targets_count += 1
                    infos = (
                        [
                            {
                                "targetId": "TARGET-1",
                                "browserContextId": "CONTEXT-1",
                            },
                            {
                                "targetId": "POPUP-1",
                                "browserContextId": "CONTEXT-1",
                            },
                        ]
                        if self.get_targets_count == 1
                        else []
                    )
                    return ({"result": {"targetInfos": infos}}, [])
                return ({"result": {}}, [])

        client = FakeClient()
        asyncio.run(
            _close_page(
                client,  # type: ignore[arg-type]
                _AttachedPage(
                    "CONTEXT-1",
                    "TARGET-1",
                    "SESSION-1",
                    frozenset({"PREWARM"}),
                ),
            )
        )

        self.assertIn(
            (
                "Target.disposeBrowserContext",
                {"browserContextId": "CONTEXT-1"},
            ),
            client.commands,
        )
        self.assertNotIn("Target.closeTarget", [method for method, _ in client.commands])
    def test_unusable_page_session_preserves_case_result_and_relaunches(self) -> None:
        def handle(name: str) -> SimpleNamespace:
            return SimpleNamespace(
                process=SimpleNamespace(poll=lambda: None),
                binary=None,
                binary_sha256=None,
                binary_version=None,
                endpoint=f"http://{name}",
                ready_ms=1.0,
            )

        first_handle = handle("first")
        second_handle = handle("second")
        first_client = SimpleNamespace(websocket=SimpleNamespace(close=AsyncMock()))
        second_client = SimpleNamespace(websocket=SimpleNamespace(close=AsyncMock()))
        driver = SimpleNamespace(
            name="moli",
            launch=Mock(return_value=first_handle),
            shutdown=Mock(return_value={"stopped": True}),
        )
        failed_case = CaseResult(
            case_path="first.html",
            url="http://localhost/first.html",
            status="error",
            duration_ms=5.0,
            error="evaluate failed: timed out",
        )
        passed_case = CaseResult(
            case_path="second.html",
            url="http://localhost/second.html",
            status="pass",
            duration_ms=1.0,
        )

        first_page = _AttachedPage("CONTEXT-1", "TARGET-1", "SESSION-1", frozenset())
        second_page = _AttachedPage("CONTEXT-2", "TARGET-2", "SESSION-2", frozenset())

        with (
            patch(
                "moli_benchmark.wpt_cross.runner.connect_raw_cdp",
                new=AsyncMock(return_value=first_client),
            ),
            patch(
                "moli_benchmark.wpt_cross.runner._attach_page",
                new=AsyncMock(return_value=first_page),
            ),
            patch(
                "moli_benchmark.wpt_cross.runner._run_one_case",
                new=AsyncMock(
                    side_effect=[_PageSessionUnusable(failed_case), passed_case]
                ),
            ),
            patch(
                "moli_benchmark.wpt_cross.runner._try_relaunch",
                new=AsyncMock(
                    return_value=(second_handle, second_client, second_page)
                ),
            ),
            patch(
                "moli_benchmark.wpt_cross.runner._close_page",
                new=AsyncMock(),
            ),
        ):
            result = asyncio.run(
                _run_async(
                    driver=driver,  # type: ignore[arg-type]
                    binary_override=None,
                    cases=[
                        ("first.html", "http://localhost/first.html"),
                        ("second.html", "http://localhost/second.html"),
                    ],
                    case_timeout_seconds=1.0,
                    launch_timeout_seconds=1.0,
                    viewport=None,
                    artifact_output_dir=None,
                )
            )

        self.assertIs(result.cases[0], failed_case)
        self.assertIs(result.cases[1], passed_case)
        self.assertEqual(driver.shutdown.call_count, 2)
    def test_successful_cases_rotate_isolated_page_contexts(self) -> None:
        handle = SimpleNamespace(
            process=SimpleNamespace(poll=lambda: None),
            binary=None,
            binary_sha256=None,
            binary_version=None,
            endpoint="http://engine",
            ready_ms=1.0,
        )
        client = SimpleNamespace(websocket=SimpleNamespace(close=AsyncMock()))
        driver = SimpleNamespace(
            name="moli",
            launch=Mock(return_value=handle),
            shutdown=Mock(return_value={"stopped": True}),
        )
        first_page = _AttachedPage(
            "CONTEXT-1", "TARGET-1", "SESSION-1", frozenset()
        )
        second_page = _AttachedPage(
            "CONTEXT-2", "TARGET-2", "SESSION-2", frozenset()
        )
        first_result = CaseResult(
            "first.html", "http://localhost/first.html", "pass", 1.0
        )
        second_result = CaseResult(
            "second.html", "http://localhost/second.html", "pass", 1.0
        )
        attach = AsyncMock(side_effect=[first_page, second_page])
        close = AsyncMock()
        run_case = AsyncMock(side_effect=[first_result, second_result])

        with (
            patch(
                "moli_benchmark.wpt_cross.runner.connect_raw_cdp",
                new=AsyncMock(return_value=client),
            ),
            patch(
                "moli_benchmark.wpt_cross.runner._attach_page",
                new=attach,
            ),
            patch(
                "moli_benchmark.wpt_cross.runner._close_page",
                new=close,
            ),
            patch(
                "moli_benchmark.wpt_cross.runner._run_one_case",
                new=run_case,
            ),
        ):
            result = asyncio.run(
                _run_async(
                    driver=driver,  # type: ignore[arg-type]
                    binary_override=None,
                    cases=[
                        ("first.html", "http://localhost/first.html"),
                        ("second.html", "http://localhost/second.html"),
                    ],
                    case_timeout_seconds=1.0,
                    launch_timeout_seconds=1.0,
                    viewport=None,
                    artifact_output_dir=None,
                )
            )

        self.assertEqual(result.cases, [first_result, second_result])
        self.assertEqual(
            [call.kwargs["session_id"] for call in run_case.await_args_list],
            ["SESSION-1", "SESSION-2"],
        )
        self.assertEqual(
            [call.args[1] for call in close.await_args_list],
            [first_page, second_page],
        )
        self.assertEqual(attach.await_count, 2)
        self.assertEqual(driver.launch.call_count, 1)
