from __future__ import annotations

import asyncio
import os
import signal
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, reserve_port
from .process import ProcessResult
from .public_web import POST_DCL_SETTLE_MILLISECONDS
from .raw_cdp import RawCdpClient, RawCdpError, connect_raw_cdp
from .sampling import ResourceSampler
from .target_serve import start_target_serve, stop_target_serve


DEFAULT_CHROME_DCL_USER_AGENT = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36"
)

# Only Page.navigate uses this grace. It preserves a late protocol error that
# races the benchmark deadline, but a late successful response is still a DCL
# timeout. Keep this small so failed navigations do not distort timing reports.
CDP_LATE_ERROR_GRACE_SECONDS = 2.0

_POST_DCL_SETTLE_EXPRESSION = (
    "new Promise(resolve => "
    f"setTimeout(resolve, {POST_DCL_SETTLE_MILLISECONDS}))"
)
_POST_DCL_OUTER_HTML_EXPRESSION = (
    "document.documentElement ? document.documentElement.outerHTML : ''"
)


class CdpDclDumpTimeoutError(TimeoutError):
    def __init__(self, stage: str, error: BaseException) -> None:
        self.stage = stage
        self.original_error = error
        detail = str(error)
        super().__init__(f"{stage}: {detail}" if detail else stage)

    @property
    def detail(self) -> str:
        detail = str(self.original_error)
        return detail if detail else self.stage


@dataclass(frozen=True)
class CdpDclDumpResult:
    body: str
    response_status: int | None
    response_mime_type: str | None
    main_document_body_capture: str
    final_url: str | None


@dataclass(frozen=True)
class _NavigationResult:
    frame_id: str | None
    loader_id: str | None
    is_download: bool


@dataclass(frozen=True)
class _MainDocumentObservation:
    headers_only: bool
    response_status: int | None
    response_mime_type: str | None
    final_url: str | None


def _chrome_command(binary: Path, port: int, profile_dir: Path) -> list[str]:
    return [
        str(binary),
        "--headless=new",
        "--disable-gpu",
        "--no-sandbox",
        "--disable-dev-shm-usage",
        "--no-first-run",
        "--no-default-browser-check",
        "--remote-debugging-address=127.0.0.1",
        f"--remote-debugging-port={port}",
        f"--user-data-dir={profile_dir}",
        f"--user-agent={DEFAULT_CHROME_DCL_USER_AGENT}",
        "about:blank",
    ]


async def _wait_for_cdp(endpoint: str, process: subprocess.Popen[bytes], timeout_seconds: float) -> RawCdpClient:
    deadline = time.perf_counter() + timeout_seconds
    last_error: Exception | None = None
    while time.perf_counter() < deadline:
        if process.poll() is not None:
            raise RawCdpError(f"chromium exited before CDP became available: rc={process.returncode}")
        try:
            return await connect_raw_cdp(endpoint)
        except Exception as error:  # noqa: BLE001 - surface the last startup failure in context.
            last_error = error
            await asyncio.sleep(0.05)
    raise TimeoutError(f"timed out waiting for Chrome CDP at {endpoint}; last_error={last_error!r}")


async def _recv_command_response(
    client: RawCdpClient,
    message_id: int,
    *,
    deadline: float,
    stage: str,
    late_error_grace_seconds: float = 0.0,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    try:
        return await client.recv_until_id(
            message_id,
            timeout=max(0.1, deadline - time.perf_counter()),
        )
    except TimeoutError as error:
        if late_error_grace_seconds > 0.0:
            await _raise_late_command_error_or_timeout(
                client,
                message_id,
                stage=stage,
                timeout_error=error,
                grace_seconds=late_error_grace_seconds,
            )
        raise CdpDclDumpTimeoutError(stage, error) from error


async def _raise_late_command_error_or_timeout(
    client: RawCdpClient,
    message_id: int,
    *,
    stage: str,
    timeout_error: BaseException,
    grace_seconds: float,
) -> None:
    """Surface a command error that arrives just after the benchmark deadline.

    The grace path is only for classification. A late successful response still
    remains a timeout, so the DCL benchmark does not accept pages that complete
    after its configured deadline.
    """
    grace_deadline = time.perf_counter() + grace_seconds
    while True:
        remaining = grace_deadline - time.perf_counter()
        if remaining <= 0.0:
            raise CdpDclDumpTimeoutError(stage, timeout_error) from timeout_error
        try:
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
        except TimeoutError:
            raise CdpDclDumpTimeoutError(stage, timeout_error) from timeout_error
        if message.get("id") != message_id:
            continue
        if "error" in message:
            raise RawCdpError(f"CDP command id={message_id} failed: {message['error']}")
        raise CdpDclDumpTimeoutError(stage, timeout_error) from timeout_error


def _is_dcl_event(
    message: dict[str, Any],
    session_id: str,
    frame_id: str | None,
    expected_loader_id: str | None,
) -> bool:
    if message.get("sessionId") != session_id:
        return False
    # Page.domContentEventFired has no loader identity. Lifecycle events are
    # enabled for this runner and can be matched to the loader returned by
    # Page.navigate below.
    if message.get("method") != "Page.lifecycleEvent":
        return False
    params = message.get("params")
    if not isinstance(params, dict):
        return False
    if frame_id is not None and params.get("frameId") != frame_id:
        return False
    if (
        expected_loader_id is not None
        and params.get("loaderId") != expected_loader_id
    ):
        return False
    return params.get("name") in {"DOMContentLoaded", "domContentLoaded"}


_BINARY_DOCUMENT_MIME_PREFIXES = (
    "audio/",
    "font/",
    "image/",
    "video/",
)

_BINARY_DOCUMENT_MIME_TYPES = {
    "application/gzip",
    "application/octet-stream",
    "application/pdf",
    "application/vnd.ms-fontobject",
    "application/x-7z-compressed",
    "application/x-bzip2",
    "application/x-gzip",
    "application/x-rar-compressed",
    "application/x-tar",
    "application/zip",
}


def _is_binary_document_mime_type(mime_type: str) -> bool:
    normalized = mime_type.split(";", 1)[0].strip().lower()
    return normalized in _BINARY_DOCUMENT_MIME_TYPES or normalized.startswith(
        _BINARY_DOCUMENT_MIME_PREFIXES
    )


def _main_document_response_from_message(
    message: dict[str, Any],
    *,
    session_id: str,
    frame_id: str | None,
    expected_loader_id: str | None = None,
    main_document_request_ids: set[str] | None = None,
) -> dict[str, Any] | None:
    if message.get("sessionId") != session_id:
        return None
    params = message.get("params")
    if not isinstance(params, dict):
        return None
    if (
        expected_loader_id is not None
        and params.get("loaderId") != expected_loader_id
    ):
        return None

    method = message.get("method")
    if method == "Network.requestWillBeSent":
        if params.get("type") != "Document":
            return None
        if frame_id is not None and params.get("frameId") != frame_id:
            return None
        request_id = params.get("requestId")
        if main_document_request_ids is not None and isinstance(request_id, str):
            main_document_request_ids.clear()
            main_document_request_ids.add(request_id)
        return None

    if method != "Network.responseReceived":
        return None
    if frame_id is not None and params.get("frameId") != frame_id:
        return None
    request_id = params.get("requestId")
    request_was_document = (
        isinstance(request_id, str)
        and main_document_request_ids is not None
        and request_id in main_document_request_ids
    )
    if (
        isinstance(request_id, str)
        and main_document_request_ids
        and not request_was_document
    ):
        return None
    if params.get("type") != "Document" and not request_was_document:
        return None
    if main_document_request_ids is not None and isinstance(request_id, str):
        main_document_request_ids.clear()
        main_document_request_ids.add(request_id)
    response = params.get("response")
    if not isinstance(response, dict):
        return None
    return response


def _response_status(response: dict[str, Any]) -> int | None:
    status = response.get("status")
    if isinstance(status, bool) or not isinstance(status, (int, float)):
        return None
    return int(status)


def _response_mime_type(response: dict[str, Any]) -> str | None:
    mime_type = response.get("mimeType")
    if not isinstance(mime_type, str):
        return None
    normalized = mime_type.strip().lower()
    return normalized or None


def _response_url(response: dict[str, Any]) -> str | None:
    url = response.get("url")
    return url if isinstance(url, str) and url else None


def _binary_main_resource_mime_type_from_message(
    message: dict[str, Any],
    *,
    session_id: str,
    frame_id: str | None,
    expected_loader_id: str | None = None,
) -> str | None:
    response = _main_document_response_from_message(
        message,
        session_id=session_id,
        frame_id=frame_id,
        expected_loader_id=expected_loader_id,
    )
    return _binary_main_resource_mime_type_from_response(response)


def _binary_main_resource_mime_type_from_response(
    response: dict[str, Any] | None,
) -> str | None:
    if response is None:
        return None
    status = _response_status(response)
    if status is not None and not 200 <= status < 400:
        return None
    mime_type = response.get("mimeType")
    if not isinstance(mime_type, str) or not _is_binary_document_mime_type(mime_type):
        return None
    return _response_mime_type(response)


async def _recv_until_dcl_or_binary_main_resource(
    client: RawCdpClient,
    *,
    session_id: str,
    frame_id: str | None,
    expected_loader_id: str | None,
    deadline: float,
    seen: list[dict[str, Any]],
    download_navigation: bool = False,
) -> _MainDocumentObservation:
    response_status: int | None = None
    response_mime_type: str | None = None
    response_url: str | None = None
    main_document_request_ids: set[str] = set()
    for message in seen:
        response = _main_document_response_from_message(
            message,
            session_id=session_id,
            frame_id=frame_id,
            expected_loader_id=expected_loader_id,
            main_document_request_ids=main_document_request_ids,
        )
        if response is not None:
            response_status = _response_status(response)
            response_mime_type = _response_mime_type(response)
            response_url = _response_url(response)
        if response is not None and (
            download_navigation
            or _binary_main_resource_mime_type_from_response(response) is not None
        ):
            return _MainDocumentObservation(
                headers_only=True,
                response_status=response_status,
                response_mime_type=response_mime_type,
                final_url=response_url,
            )
    if not download_navigation and any(
        _is_dcl_event(message, session_id, frame_id, expected_loader_id)
        for message in seen
    ):
        return _MainDocumentObservation(
            headers_only=False,
            response_status=response_status,
            response_mime_type=response_mime_type,
            final_url=response_url,
        )
    while True:
        remaining = deadline - time.perf_counter()
        if remaining <= 0:
            raise TimeoutError("timed out waiting for Page.domContentEventFired")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        response = _main_document_response_from_message(
            message,
            session_id=session_id,
            frame_id=frame_id,
            expected_loader_id=expected_loader_id,
            main_document_request_ids=main_document_request_ids,
        )
        if response is not None:
            response_status = _response_status(response)
            response_mime_type = _response_mime_type(response)
            response_url = _response_url(response)
        if response is not None and (
            download_navigation
            or _binary_main_resource_mime_type_from_response(response) is not None
        ):
            return _MainDocumentObservation(
                headers_only=True,
                response_status=response_status,
                response_mime_type=response_mime_type,
                final_url=response_url,
            )
        if not download_navigation and _is_dcl_event(
            message,
            session_id,
            frame_id,
            expected_loader_id,
        ):
            return _MainDocumentObservation(
                headers_only=False,
                response_status=response_status,
                response_mime_type=response_mime_type,
                final_url=response_url,
            )


def _parse_navigation_result(
    navigate_response: dict[str, Any],
    url: str,
) -> _NavigationResult:
    result = navigate_response.get("result")
    if not isinstance(result, dict):
        result = {}
    is_download = result.get("isDownload") is True
    error_text = result.get("errorText")
    if isinstance(error_text, str) and error_text and not is_download:
        raise RawCdpError(f"Page.navigate failed for `{url}`: {error_text}")
    frame_id = result.get("frameId")
    loader_id = result.get("loaderId")
    return _NavigationResult(
        frame_id=str(frame_id) if frame_id is not None else None,
        loader_id=str(loader_id) if loader_id is not None else None,
        is_download=is_download,
    )


async def _dump_dcl_html(
    endpoint: str,
    process: subprocess.Popen[bytes],
    url: str,
    timeout_seconds: float,
) -> CdpDclDumpResult:
    deadline = time.perf_counter() + timeout_seconds
    try:
        client = await _wait_for_cdp(endpoint, process, min(5.0, max(0.1, timeout_seconds)))
    except TimeoutError as error:
        raise CdpDclDumpTimeoutError("startup", error) from error
    target_id: str | None = None
    try:
        create_id = await client.send("Target.createTarget", {"url": "about:blank"})
        create_response, _ = await _recv_command_response(
            client,
            create_id,
            deadline=deadline,
            stage="Target.createTarget",
        )
        target_id = str(create_response["result"]["targetId"])

        attach_id = await client.send("Target.attachToTarget", {"targetId": target_id, "flatten": True})
        attach_response, _ = await _recv_command_response(
            client,
            attach_id,
            deadline=deadline,
            stage="Target.attachToTarget",
        )
        session_id = str(attach_response["result"]["sessionId"])

        for method in ("Page.enable", "Runtime.enable", "Network.enable"):
            message_id = await client.send(method, session_id=session_id)
            await _recv_command_response(
                client,
                message_id,
                deadline=deadline,
                stage=method,
            )
        lifecycle_id = await client.send("Page.setLifecycleEventsEnabled", {"enabled": True}, session_id=session_id)
        await _recv_command_response(
            client,
            lifecycle_id,
            deadline=deadline,
            stage="Page.setLifecycleEventsEnabled",
        )

        navigate_id = await client.send("Page.navigate", {"url": url}, session_id=session_id)
        navigate_response, seen = await _recv_command_response(
            client,
            navigate_id,
            deadline=deadline,
            stage="Page.navigate",
            late_error_grace_seconds=CDP_LATE_ERROR_GRACE_SECONDS,
        )
        navigation = _parse_navigation_result(
            navigate_response,
            url,
        )
        try:
            main_document = await _recv_until_dcl_or_binary_main_resource(
                client,
                session_id=session_id,
                frame_id=navigation.frame_id,
                expected_loader_id=navigation.loader_id,
                deadline=deadline,
                seen=seen,
                download_navigation=navigation.is_download,
            )
        except TimeoutError as error:
            raise CdpDclDumpTimeoutError("DCL", error) from error
        if main_document.headers_only:
            return CdpDclDumpResult(
                body="",
                response_status=main_document.response_status,
                response_mime_type=main_document.response_mime_type,
                main_document_body_capture="response-headers-only",
                final_url=main_document.final_url,
            )

        response_status = main_document.response_status
        response_mime_type = main_document.response_mime_type
        final_url = main_document.final_url

        settle_id = await client.send(
            "Runtime.evaluate",
            {
                "expression": _POST_DCL_SETTLE_EXPRESSION,
                "awaitPromise": True,
                "returnByValue": True,
            },
            session_id=session_id,
        )
        await _recv_command_response(
            client,
            settle_id,
            deadline=deadline,
            stage="post-DCL settle",
        )

        evaluate_id = await client.send(
            "Runtime.evaluate",
            {
                "expression": _POST_DCL_OUTER_HTML_EXPRESSION,
                "returnByValue": True,
            },
            session_id=session_id,
        )
        evaluate_response, _ = await _recv_command_response(
            client,
            evaluate_id,
            deadline=deadline,
            stage="outerHTML",
        )
        result = evaluate_response.get("result", {}).get("result", {})
        value = result.get("value", "")
        return CdpDclDumpResult(
            body=value if isinstance(value, str) else "",
            response_status=response_status,
            response_mime_type=response_mime_type,
            main_document_body_capture="dom-snapshot",
            final_url=final_url,
        )
    finally:
        if target_id is not None:
            try:
                close_id = await client.send("Target.closeTarget", {"targetId": target_id})
                await client.recv_until_id(close_id, timeout=1.0)
            except Exception:
                pass
        await client.websocket.close()


def _terminate_process_group(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except OSError:
        pass
    try:
        process.wait(timeout=2)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except OSError:
        pass
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        pass


def _read_tempfile(file_obj: Any) -> bytes:
    file_obj.flush()
    file_obj.seek(0)
    return file_obj.read()


def _process_returncode_or(process: subprocess.Popen[bytes], fallback: int) -> int:
    returncode = process.poll()
    return int(returncode) if returncode is not None else fallback


def run_chrome_dcl_dump(
    binary: Path,
    url: str,
    *,
    cwd: Path = REPO_ROOT,
    timeout_seconds: float,
    env: dict[str, str] | None = None,
    sample_resources: bool = True,
) -> ProcessResult:
    started = time.perf_counter()
    command: list[str] = []
    stdout = b""
    stderr = b""
    browser_stdout = b""
    error_suffix = b""
    timed_out = False
    returncode: int | None = None
    response_status: int | None = None
    response_mime_type: str | None = None
    main_document_body_capture: str | None = None
    final_url: str | None = None
    with tempfile.TemporaryDirectory(prefix="moli-benchmark-chrome-") as temp_dir:
        profile_dir = Path(temp_dir) / "profile"
        profile_dir.mkdir()
        with tempfile.TemporaryFile() as stdout_file, tempfile.TemporaryFile() as stderr_file:
            reserved_port = reserve_port()
            try:
                port = reserved_port.port
                command = _chrome_command(binary, port, profile_dir)
                endpoint = f"http://127.0.0.1:{port}"
                reserved_port.release_socket()
                process = subprocess.Popen(
                    command,
                    cwd=cwd,
                    env=env,
                    stdout=stdout_file,
                    stderr=stderr_file,
                    start_new_session=True,
                )
            except BaseException:
                reserved_port.close()
                raise
            sampler = ResourceSampler(process.pid) if sample_resources else None
            if sampler is not None:
                sampler.start()
            try:
                try:
                    dump = asyncio.run(
                        _dump_dcl_html(endpoint, process, url, timeout_seconds)
                    )
                    stdout = dump.body.encode("utf-8", errors="replace")
                    response_status = dump.response_status
                    response_mime_type = dump.response_mime_type
                    main_document_body_capture = dump.main_document_body_capture
                    final_url = dump.final_url
                    returncode = 0
                except CdpDclDumpTimeoutError as error:
                    timed_out = True
                    returncode = _process_returncode_or(process, 124)
                    error_suffix = (
                        f"\nchrome CDP {error.stage} timeout: {error.detail}\n"
                    ).encode("utf-8", errors="replace")
                except TimeoutError as error:
                    timed_out = True
                    returncode = _process_returncode_or(process, 124)
                    error_suffix = f"\nchrome CDP timeout: {error}\n".encode("utf-8", errors="replace")
                except Exception as error:  # noqa: BLE001 - convert CDP/browser failures into benchmark process output.
                    returncode = _process_returncode_or(process, 1)
                    error_suffix = f"\nchrome CDP DCL error: {error}\n".encode("utf-8", errors="replace")
                finally:
                    _terminate_process_group(process)
                    reserved_port.close()
                    if returncode is None:
                        returncode = process.returncode
                    browser_stdout = _read_tempfile(stdout_file)
                    stderr = _read_tempfile(stderr_file) + error_suffix
            finally:
                elapsed_ms = (time.perf_counter() - started) * 1000.0
                resources = sampler.stop() if sampler is not None else {}
    if (
        browser_stdout.strip()
        and not stdout
        and main_document_body_capture is None
    ):
        stdout = browser_stdout
    return ProcessResult(
        command=command,
        returncode=returncode,
        elapsed_ms=elapsed_ms,
        stdout=stdout,
        stderr=stderr,
        timed_out=timed_out,
        resources=resources,
        response_status=response_status,
        response_mime_type=response_mime_type,
        main_document_body_capture=main_document_body_capture,
        final_url=final_url,
    )


def run_served_cdp_dcl_dump(
    target: str,
    binary: Path,
    url: str,
    *,
    cwd: Path = REPO_ROOT,
    timeout_seconds: float,
    env: dict[str, str] | None = None,
) -> ProcessResult:
    del cwd, env
    started = time.perf_counter()
    command: list[str] = [str(binary), "serve"]
    stdout = b""
    stderr = b""
    error_suffix = b""
    timed_out = False
    returncode: int | None = None
    resources: dict[str, Any] = {}
    response_status: int | None = None
    response_mime_type: str | None = None
    main_document_body_capture: str | None = None
    final_url: str | None = None
    serve = None
    try:
        serve = start_target_serve(target, binary, timeout_seconds)
        command = serve.command
        try:
            dump = asyncio.run(
                _dump_dcl_html(serve.endpoint, serve.process, url, timeout_seconds)
            )
            stdout = dump.body.encode("utf-8", errors="replace")
            response_status = dump.response_status
            response_mime_type = dump.response_mime_type
            main_document_body_capture = dump.main_document_body_capture
            final_url = dump.final_url
            returncode = 0
        except CdpDclDumpTimeoutError as error:
            timed_out = True
            returncode = _process_returncode_or(serve.process, 124)
            error_suffix = (
                f"\n{target} CDP {error.stage} timeout: {error.detail}\n"
            ).encode("utf-8", errors="replace")
        except TimeoutError as error:
            timed_out = True
            returncode = _process_returncode_or(serve.process, 124)
            error_suffix = f"\n{target} CDP timeout: {error}\n".encode("utf-8", errors="replace")
        except Exception as error:  # noqa: BLE001 - convert CDP/browser failures into benchmark process output.
            returncode = _process_returncode_or(serve.process, 1)
            error_suffix = f"\n{target} CDP DCL error: {error}\n".encode("utf-8", errors="replace")
        finally:
            stopped = stop_target_serve(serve)
            serve = None
            if returncode is None:
                stopped_returncode = stopped.get("returncode")
                returncode = int(stopped_returncode) if isinstance(stopped_returncode, int) else None
            stopped_resources = stopped.get("resources")
            resources = stopped_resources if isinstance(stopped_resources, dict) else {}
            log_tail = stopped.get("log_tail")
            if isinstance(log_tail, list) and log_tail:
                stderr = "\n".join(str(line) for line in log_tail).encode("utf-8", errors="replace")
            stderr += error_suffix
    except TimeoutError as error:
        timed_out = True
        error_suffix = f"\n{target} CDP timeout: {error}\n".encode("utf-8", errors="replace")
        stderr += error_suffix
    except Exception as error:  # noqa: BLE001 - startup failures are benchmark process errors.
        error_suffix = f"\n{target} CDP DCL error: {error}\n".encode("utf-8", errors="replace")
        stderr += error_suffix
    finally:
        if serve is not None:
            stopped = stop_target_serve(serve)
            stopped_resources = stopped.get("resources")
            if isinstance(stopped_resources, dict) and not resources:
                resources = stopped_resources
        elapsed_ms = (time.perf_counter() - started) * 1000.0
    return ProcessResult(
        command=command,
        returncode=returncode,
        elapsed_ms=elapsed_ms,
        stdout=stdout,
        stderr=stderr,
        timed_out=timed_out,
        resources=resources,
        response_status=response_status,
        response_mime_type=response_mime_type,
        main_document_body_capture=main_document_body_capture,
        final_url=final_url,
    )
