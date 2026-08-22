from __future__ import annotations

import asyncio
import json
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

from moli_benchmark.chrome_dcl import (
    CdpDclDumpResult,
    CdpDclDumpTimeoutError,
    DEFAULT_CHROME_DCL_USER_AGENT,
    POST_DCL_SETTLE_MILLISECONDS,
    _POST_DCL_OUTER_HTML_EXPRESSION,
    _POST_DCL_SETTLE_EXPRESSION,
    _binary_main_resource_mime_type_from_message,
    _chrome_command,
    _navigation_frame_and_loader,
    _recv_command_response,
    _recv_until_dcl_or_binary_main_resource,
    run_chrome_dcl_dump,
)
from moli_benchmark.public_web import TOP_SITES_CLASSIFIER, WILD_WEB_CLASSIFIER
from moli_benchmark.raw_cdp import RawCdpClient, RawCdpError, RawCdpTimeoutError


class _NonMatchingWebSocket:
    async def send(self, _payload: str) -> None:
        return None

    async def recv(self) -> str:
        return json.dumps({"method": "Runtime.consoleAPICalled"})


class _FakeTemporaryFile:
    def __init__(self) -> None:
        self.closed = False

    def __enter__(self) -> "_FakeTemporaryFile":
        return self

    def __exit__(self, *_args: object) -> None:
        self.closed = True


class _FakeProcess:
    pid = 12345

    def __init__(self) -> None:
        self.returncode: int | None = None

    def poll(self) -> int | None:
        return self.returncode


class _RecordingCdpClient:
    def __init__(self) -> None:
        self.timeout: float | None = None

    async def recv_until_id(self, _message_id: int, *, timeout: float) -> tuple[dict[str, object], list[dict[str, object]]]:
        self.timeout = timeout
        return {"id": 1, "result": {}}, []


class _LateCommandResponseClient:
    def __init__(self, late_message: dict[str, object]) -> None:
        self.late_message = late_message

    async def recv_until_id(self, _message_id: int, *, timeout: float) -> tuple[dict[str, object], list[dict[str, object]]]:
        del timeout
        raise RawCdpTimeoutError("primary command deadline expired")

    async def recv(self) -> dict[str, object]:
        return self.late_message


class _QueuedMessageClient:
    def __init__(self, messages: list[dict[str, object]]) -> None:
        self.messages = messages

    async def recv(self) -> dict[str, object]:
        if not self.messages:
            raise AssertionError("CDP receiver exhausted before matching navigation evidence")
        return self.messages.pop(0)


def _document_response_message(
    mime_type: str,
    *,
    resource_type: str | None = "Document",
    request_id: str = "REQUEST-1",
    status: int = 200,
    loader_id: str = "LOADER-1",
) -> dict[str, object]:
    return {
        "sessionId": "SID-1",
        "method": "Network.responseReceived",
        "params": {
            "type": resource_type,
            "frameId": "FRAME-1",
            "loaderId": loader_id,
            "requestId": request_id,
            "response": {
                "mimeType": mime_type,
                "status": status,
                "url": "https://example.test/final",
            },
        },
    }


def _document_request_message(
    *,
    request_id: str = "REQUEST-1",
    loader_id: str = "LOADER-1",
) -> dict[str, object]:
    return {
        "sessionId": "SID-1",
        "method": "Network.requestWillBeSent",
        "params": {
            "type": "Document",
            "frameId": "FRAME-1",
            "loaderId": loader_id,
            "requestId": request_id,
            "request": {"url": "https://example.test/"},
        },
    }


def _dcl_lifecycle_message(*, loader_id: str = "LOADER-1") -> dict[str, object]:
    return {
        "sessionId": "SID-1",
        "method": "Page.lifecycleEvent",
        "params": {
            "frameId": "FRAME-1",
            "loaderId": loader_id,
            "name": "DOMContentLoaded",
        },
    }


class ChromeDclTests(unittest.TestCase):
    def test_post_dcl_snapshot_uses_page_event_loop_settle(self) -> None:
        self.assertEqual(POST_DCL_SETTLE_MILLISECONDS, 50)
        self.assertIn("new Promise", _POST_DCL_SETTLE_EXPRESSION)
        self.assertIn("setTimeout", _POST_DCL_SETTLE_EXPRESSION)
        self.assertIn("50", _POST_DCL_SETTLE_EXPRESSION)
        self.assertIn("document.documentElement.outerHTML", _POST_DCL_OUTER_HTML_EXPRESSION)

    def test_chrome_command_uses_non_headless_desktop_user_agent(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            command = _chrome_command(Path("/bin/chromium"), 12345, Path(temp_dir))

        user_agent_args = [arg for arg in command if arg.startswith("--user-agent=")]
        self.assertEqual(user_agent_args, [f"--user-agent={DEFAULT_CHROME_DCL_USER_AGENT}"])
        self.assertIn("Chrome/", user_agent_args[0])
        self.assertNotIn("HeadlessChrome", user_agent_args[0])

    def test_recv_until_id_deadline_raises_timeout_error(self) -> None:
        async def run() -> None:
            client = RawCdpClient(websocket=_NonMatchingWebSocket())  # type: ignore[arg-type]
            with self.assertRaises(RawCdpTimeoutError) as raised:
                await client.recv_until_id(1, timeout=0.001)
            self.assertIsInstance(raised.exception, TimeoutError)

        asyncio.run(run())

    def test_recv_command_response_uses_remaining_deadline_without_stage_cap(self) -> None:
        async def run() -> None:
            client = _RecordingCdpClient()
            deadline = time.perf_counter() + 12.0
            await _recv_command_response(  # type: ignore[arg-type]
                client,
                1,
                deadline=deadline,
                stage="outerHTML",
            )
            self.assertIsNotNone(client.timeout)
            self.assertGreater(client.timeout or 0.0, 10.0)

        asyncio.run(run())

    def test_recv_command_response_surfaces_late_command_error_for_classification(self) -> None:
        async def run() -> None:
            client = _LateCommandResponseClient(
                {
                    "id": 7,
                    "error": {
                        "code": -32000,
                        "message": "failed to fetch page `https://example.invalid/`: curl request failed",
                    },
                }
            )

            with self.assertRaises(RawCdpError) as raised:
                await _recv_command_response(  # type: ignore[arg-type]
                    client,
                    7,
                    deadline=time.perf_counter(),
                    stage="Page.navigate",
                    late_error_grace_seconds=0.1,
                )

            self.assertIn("failed to fetch page", str(raised.exception))

        asyncio.run(run())

    def test_recv_command_response_keeps_late_success_as_timeout(self) -> None:
        async def run() -> None:
            client = _LateCommandResponseClient({"id": 7, "result": {}})

            with self.assertRaises(CdpDclDumpTimeoutError) as raised:
                await _recv_command_response(  # type: ignore[arg-type]
                    client,
                    7,
                    deadline=time.perf_counter(),
                    stage="Page.navigate",
                    late_error_grace_seconds=0.1,
                )

            self.assertEqual(raised.exception.stage, "Page.navigate")

        asyncio.run(run())

    def test_navigation_result_surfaces_error_text_before_dcl_wait(self) -> None:
        with self.assertRaises(RawCdpError) as raised:
            _navigation_frame_and_loader(
                {
                    "result": {
                        "frameId": "FRAME-1",
                        "errorText": "net::ERR_NAME_NOT_RESOLVED",
                    }
                },
                "https://missing.example.test/",
            )

        self.assertEqual(
            str(raised.exception),
            "Page.navigate failed for `https://missing.example.test/`: "
            "net::ERR_NAME_NOT_RESOLVED",
        )

    def test_navigation_result_exposes_frame_and_loader_identity(self) -> None:
        self.assertEqual(
            _navigation_frame_and_loader(
                {"result": {"frameId": "FRAME-1", "loaderId": "LOADER-1"}},
                "https://example.test/",
            ),
            ("FRAME-1", "LOADER-1"),
        )

    def test_binary_main_document_response_returns_mime_evidence(self) -> None:
        mime_type = _binary_main_resource_mime_type_from_message(
            _document_response_message("application/pdf"),
            session_id="SID-1",
            frame_id="FRAME-1",
        )

        self.assertEqual(mime_type, "application/pdf")

    def test_binary_main_document_detection_ignores_html_and_subresources(self) -> None:
        html_mime_type = _binary_main_resource_mime_type_from_message(
            _document_response_message("text/html; charset=utf-8"),
            session_id="SID-1",
            frame_id="FRAME-1",
        )
        script_pdf_mime_type = _binary_main_resource_mime_type_from_message(
            _document_response_message("application/pdf", resource_type="Script"),
            session_id="SID-1",
            frame_id="FRAME-1",
        )

        self.assertIsNone(html_mime_type)
        self.assertIsNone(script_pdf_mime_type)

    def test_binary_main_document_detection_ignores_error_status(self) -> None:
        mime_type = _binary_main_resource_mime_type_from_message(
            _document_response_message("application/pdf", status=404),
            session_id="SID-1",
            frame_id="FRAME-1",
        )

        self.assertIsNone(mime_type)

    def test_recv_until_dcl_short_circuits_binary_document_seen_before_dcl(self) -> None:
        async def run() -> None:
            observation = await _recv_until_dcl_or_binary_main_resource(
                mock.Mock(),
                session_id="SID-1",
                frame_id="FRAME-1",
                expected_loader_id="LOADER-1",
                deadline=time.perf_counter() + 1.0,
                seen=[
                    _document_response_message("application/pdf"),
                    _dcl_lifecycle_message(),
                ],
            )
            binary_mime_type, response_status, response_mime_type, final_url = observation
            self.assertEqual(binary_mime_type, "application/pdf")
            self.assertEqual(response_status, 200)
            self.assertEqual(response_mime_type, "application/pdf")
            self.assertEqual(final_url, "https://example.test/final")

        asyncio.run(run())

    def test_recv_until_dcl_accepts_main_frame_event_seen_before_command_response(self) -> None:
        async def run() -> None:
            observation = await _recv_until_dcl_or_binary_main_resource(
                mock.Mock(),
                session_id="SID-1",
                frame_id="FRAME-1",
                expected_loader_id="LOADER-1",
                deadline=time.perf_counter() + 1.0,
                seen=[_dcl_lifecycle_message()],
            )
            binary_mime_type, response_status, response_mime_type, final_url = observation
            self.assertIsNone(binary_mime_type)
            self.assertIsNone(response_status)
            self.assertIsNone(response_mime_type)
            self.assertIsNone(final_url)

        asyncio.run(run())

    def test_recv_until_dcl_ignores_events_from_previous_loader(self) -> None:
        async def run() -> None:
            client = _QueuedMessageClient(
                [
                    _document_response_message(
                        "text/html",
                        status=200,
                        loader_id="LOADER-NEW",
                    ),
                    {
                        "sessionId": "SID-1",
                        "method": "Page.domContentEventFired",
                        "params": {},
                    },
                    _dcl_lifecycle_message(loader_id="LOADER-NEW"),
                ]
            )
            observation = await _recv_until_dcl_or_binary_main_resource(  # type: ignore[arg-type]
                client,
                session_id="SID-1",
                frame_id="FRAME-1",
                expected_loader_id="LOADER-NEW",
                deadline=time.perf_counter() + 1.0,
                seen=[
                    _document_response_message(
                        "text/html",
                        status=502,
                        loader_id="LOADER-OLD",
                    ),
                    _dcl_lifecycle_message(loader_id="LOADER-OLD"),
                ],
            )

            self.assertEqual(
                observation,
                (None, 200, "text/html", "https://example.test/final"),
            )
            self.assertEqual(client.messages, [])

        asyncio.run(run())

    def test_recv_until_dcl_retains_html_main_document_status(self) -> None:
        async def run() -> None:
            observation = await _recv_until_dcl_or_binary_main_resource(
                mock.Mock(),
                session_id="SID-1",
                frame_id="FRAME-1",
                expected_loader_id="LOADER-1",
                deadline=time.perf_counter() + 1.0,
                seen=[
                    _document_response_message("text/html", status=502),
                    _dcl_lifecycle_message(),
                ],
            )
            binary_mime_type, response_status, response_mime_type, final_url = observation
            self.assertIsNone(binary_mime_type)
            self.assertEqual(response_status, 502)
            self.assertEqual(response_mime_type, "text/html")
            self.assertEqual(final_url, "https://example.test/final")

        asyncio.run(run())

    def test_recv_until_dcl_correlates_response_without_resource_type(self) -> None:
        async def run() -> None:
            observation = await _recv_until_dcl_or_binary_main_resource(
                mock.Mock(),
                session_id="SID-1",
                frame_id="FRAME-1",
                expected_loader_id="LOADER-1",
                deadline=time.perf_counter() + 1.0,
                seen=[
                    _document_request_message(),
                    _document_response_message(
                        "text/html",
                        resource_type=None,
                        status=400,
                    ),
                    _dcl_lifecycle_message(),
                ],
            )
            binary_mime_type, response_status, response_mime_type, final_url = observation
            self.assertIsNone(binary_mime_type)
            self.assertEqual(response_status, 400)
            self.assertEqual(response_mime_type, "text/html")
            self.assertEqual(final_url, "https://example.test/final")

        asyncio.run(run())

    def test_recv_until_dcl_ignores_untracked_response_without_resource_type(self) -> None:
        async def run() -> None:
            observation = await _recv_until_dcl_or_binary_main_resource(
                mock.Mock(),
                session_id="SID-1",
                frame_id="FRAME-1",
                expected_loader_id="LOADER-1",
                deadline=time.perf_counter() + 1.0,
                seen=[
                    _document_request_message(),
                    _document_response_message(
                        "application/json",
                        resource_type=None,
                        request_id="SUBRESOURCE-1",
                        status=503,
                    ),
                    _dcl_lifecycle_message(),
                ],
            )
            binary_mime_type, response_status, response_mime_type, final_url = observation
            self.assertIsNone(binary_mime_type)
            self.assertIsNone(response_status)
            self.assertIsNone(response_mime_type)
            self.assertIsNone(final_url)

        asyncio.run(run())

    def test_recv_until_dcl_keeps_status_for_latest_document_request(self) -> None:
        async def run() -> None:
            observation = await _recv_until_dcl_or_binary_main_resource(
                mock.Mock(),
                session_id="SID-1",
                frame_id="FRAME-1",
                expected_loader_id="LOADER-1",
                deadline=time.perf_counter() + 1.0,
                seen=[
                    _document_request_message(request_id="OLD"),
                    _document_request_message(request_id="NEW"),
                    _document_response_message(
                        "text/html",
                        request_id="OLD",
                        status=500,
                    ),
                    _document_response_message(
                        "text/html",
                        resource_type=None,
                        request_id="NEW",
                        status=200,
                    ),
                    _dcl_lifecycle_message(),
                ],
            )
            binary_mime_type, response_status, response_mime_type, final_url = observation
            self.assertIsNone(binary_mime_type)
            self.assertEqual(response_status, 200)
            self.assertEqual(response_mime_type, "text/html")
            self.assertEqual(final_url, "https://example.test/final")

        asyncio.run(run())

    def test_chrome_runner_exposes_main_document_status(self) -> None:
        process = _FakeProcess()

        def terminate(fake_process: _FakeProcess) -> None:
            fake_process.returncode = -15

        with (
            mock.patch("moli_benchmark.chrome_dcl.subprocess.Popen", return_value=process),
            mock.patch(
                "moli_benchmark.chrome_dcl._dump_dcl_html",
                return_value=CdpDclDumpResult(
                    body="<html><body>gateway error</body></html>",
                    response_status=502,
                    response_mime_type="text/html",
                    main_document_body_capture="dom-snapshot",
                    final_url="https://example.test/final",
                ),
            ),
            mock.patch(
                "moli_benchmark.chrome_dcl._terminate_process_group",
                side_effect=terminate,
            ),
        ):
            result = run_chrome_dcl_dump(
                Path("/bin/chromium"),
                "https://example.test/",
                timeout_seconds=1.0,
                sample_resources=False,
            )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.response_status, 502)
        self.assertEqual(result.response_mime_type, "text/html")
        self.assertEqual(result.main_document_body_capture, "dom-snapshot")
        self.assertEqual(result.final_url, "https://example.test/final")
        self.assertIn(b"gateway error", result.stdout)

    def test_chrome_runner_classifies_navigation_error_without_timeout(self) -> None:
        process = _FakeProcess()

        def terminate(fake_process: _FakeProcess) -> None:
            fake_process.returncode = -15

        with (
            mock.patch("moli_benchmark.chrome_dcl.subprocess.Popen", return_value=process),
            mock.patch(
                "moli_benchmark.chrome_dcl._dump_dcl_html",
                side_effect=RawCdpError(
                    "Page.navigate failed for `https://missing.example.test/`: "
                    "net::ERR_NAME_NOT_RESOLVED"
                ),
            ),
            mock.patch(
                "moli_benchmark.chrome_dcl._terminate_process_group",
                side_effect=terminate,
            ),
        ):
            result = run_chrome_dcl_dump(
                Path("/bin/chromium"),
                "https://missing.example.test/",
                timeout_seconds=1.0,
                sample_resources=False,
            )

        self.assertFalse(result.timed_out)
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"net::ERR_NAME_NOT_RESOLVED", result.stderr)
        for classifier in (TOP_SITES_CLASSIFIER, WILD_WEB_CLASSIFIER):
            with self.subTest(policy=classifier.policy):
                self.assertEqual(
                    classifier.classify_output(
                        stdout=result.stdout,
                        stderr=result.stderr,
                        returncode=result.returncode,
                        timed_out=result.timed_out,
                    ),
                    "network-error",
                )

    def test_chrome_runner_does_not_fabricate_binary_response_body(self) -> None:
        process = _FakeProcess()

        def terminate(fake_process: _FakeProcess) -> None:
            fake_process.returncode = -15

        with (
            mock.patch("moli_benchmark.chrome_dcl.subprocess.Popen", return_value=process),
            mock.patch(
                "moli_benchmark.chrome_dcl._dump_dcl_html",
                return_value=CdpDclDumpResult(
                    body="",
                    response_status=200,
                    response_mime_type="application/pdf",
                    main_document_body_capture="response-headers-only",
                    final_url="https://example.test/document.pdf",
                ),
            ),
            mock.patch(
                "moli_benchmark.chrome_dcl._terminate_process_group",
                side_effect=terminate,
            ),
        ):
            result = run_chrome_dcl_dump(
                Path("/bin/chromium"),
                "https://example.test/document.pdf",
                timeout_seconds=1.0,
                sample_resources=False,
            )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.response_mime_type, "application/pdf")
        self.assertEqual(
            result.main_document_body_capture,
            "response-headers-only",
        )

    def test_chrome_runner_records_raw_cdp_deadline_as_timeout(self) -> None:
        process = _FakeProcess()

        def terminate(fake_process: _FakeProcess) -> None:
            fake_process.returncode = -15

        with (
            mock.patch("moli_benchmark.chrome_dcl.subprocess.Popen", return_value=process),
            mock.patch(
                "moli_benchmark.chrome_dcl._dump_dcl_html",
                side_effect=CdpDclDumpTimeoutError(
                    "DCL",
                    RawCdpTimeoutError("timed out waiting for CDP response id=1"),
                ),
            ),
            mock.patch(
                "moli_benchmark.chrome_dcl._terminate_process_group",
                side_effect=terminate,
            ),
        ):
            result = run_chrome_dcl_dump(
                Path("/bin/chromium"),
                "https://example.test/",
                timeout_seconds=1.0,
                sample_resources=False,
            )

        self.assertTrue(result.timed_out)
        self.assertEqual(result.returncode, 124)
        self.assertIn(b"chrome CDP DCL timeout", result.stderr)

    def test_chrome_runner_distinguishes_outer_html_timeout(self) -> None:
        process = _FakeProcess()

        def terminate(fake_process: _FakeProcess) -> None:
            fake_process.returncode = -15

        with (
            mock.patch("moli_benchmark.chrome_dcl.subprocess.Popen", return_value=process),
            mock.patch(
                "moli_benchmark.chrome_dcl._dump_dcl_html",
                side_effect=CdpDclDumpTimeoutError(
                    "outerHTML",
                    RawCdpTimeoutError("timed out waiting for CDP response id=9"),
                ),
            ),
            mock.patch(
                "moli_benchmark.chrome_dcl._terminate_process_group",
                side_effect=terminate,
            ),
        ):
            result = run_chrome_dcl_dump(
                Path("/bin/chromium"),
                "https://example.test/",
                timeout_seconds=1.0,
                sample_resources=False,
            )

        self.assertTrue(result.timed_out)
        self.assertEqual(result.returncode, 124)
        self.assertIn(b"chrome CDP outerHTML timeout", result.stderr)
        self.assertNotIn(b"chrome CDP DCL timeout", result.stderr)

    def test_chrome_runner_closes_tempfiles_when_popen_fails(self) -> None:
        files: list[_FakeTemporaryFile] = []

        def fake_temporary_file() -> _FakeTemporaryFile:
            file = _FakeTemporaryFile()
            files.append(file)
            return file

        with (
            mock.patch(
                "moli_benchmark.chrome_dcl.tempfile.TemporaryFile",
                side_effect=fake_temporary_file,
            ),
            mock.patch(
                "moli_benchmark.chrome_dcl.subprocess.Popen",
                side_effect=OSError("boom"),
            ),
        ):
            with self.assertRaises(OSError):
                run_chrome_dcl_dump(
                    Path("/bin/chromium"),
                    "https://example.test/",
                    timeout_seconds=1.0,
                )

        self.assertEqual(len(files), 2)
        self.assertTrue(all(file.closed for file in files))


if __name__ == "__main__":
    unittest.main()
