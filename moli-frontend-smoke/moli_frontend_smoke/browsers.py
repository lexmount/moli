from __future__ import annotations

import asyncio
import json
import os
import re
import shutil
import sys
import tempfile
import time
import urllib.error
import urllib.request
from collections import deque
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable
from urllib.parse import urlsplit

from .config import REPO_ROOT, clear_proxy_env


EndpointParser = Callable[[str], str | None]
_BROWSER_START_TIMEOUT_SECONDS = 15.0
_CHROMIUM_ENDPOINT_MARKER = "DevTools listening on "
_ANSI_CONTROL_SEQUENCE_PATTERN = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
_MOLI_ENDPOINT_PATTERN = re.compile(
    r"\baddr=(?P<host>127\.0\.0\.1|localhost):(?P<port>[0-9]+)\b"
)


def _loopback_http_endpoint(host: str | None, port: int | None) -> str | None:
    if host not in {"127.0.0.1", "::1", "localhost"}:
        return None
    if port is None or not 1 <= port <= 65535:
        return None
    authority = f"[{host}]" if host == "::1" else host
    return f"http://{authority}:{port}"


def _chromium_endpoint_from_log(text: str) -> str | None:
    text = _ANSI_CONTROL_SEQUENCE_PATTERN.sub("", text)
    marker_offset = text.find(_CHROMIUM_ENDPOINT_MARKER)
    if marker_offset < 0:
        return None
    websocket_url = text[marker_offset + len(_CHROMIUM_ENDPOINT_MARKER) :].strip()
    try:
        parsed = urlsplit(websocket_url)
        port = parsed.port
    except ValueError:
        return None
    if (
        parsed.scheme not in {"ws", "wss"}
        or parsed.username is not None
        or parsed.password is not None
        or not parsed.path.startswith("/devtools/browser/")
    ):
        return None
    return _loopback_http_endpoint(parsed.hostname, port)


def _moli_endpoint_from_log(text: str) -> str | None:
    text = _ANSI_CONTROL_SEQUENCE_PATTERN.sub("", text)
    if "protocol server listening" not in text:
        return None
    match = _MOLI_ENDPOINT_PATTERN.search(text)
    if match is None:
        return None
    return _loopback_http_endpoint(match.group("host"), int(match.group("port")))


def _read_json_url(url: str) -> dict[str, Any]:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(url, timeout=0.75) as response:
        value = json.loads(response.read().decode("utf-8"))
    if not isinstance(value, dict):
        raise RuntimeError(f"expected JSON object from {url}")
    return value


async def wait_for_cdp_endpoint(
    endpoint: str,
    process: asyncio.subprocess.Process | None,
    logs: deque[str],
    *,
    timeout: float = 15.0,
) -> dict[str, Any]:
    version_url = endpoint.rstrip("/") + "/json/version"
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process is not None and process.returncode is not None:
            tail = "\n".join(logs)
            raise RuntimeError(f"browser process exited early with {process.returncode}\n{tail}")
        try:
            return await asyncio.to_thread(_read_json_url, version_url)
        except (urllib.error.URLError, TimeoutError, OSError, json.JSONDecodeError):
            await asyncio.sleep(0.05)
    tail = "\n".join(logs)
    suffix = f"\n{tail}" if tail else ""
    raise RuntimeError(f"timed out waiting for CDP endpoint {endpoint}{suffix}")


async def _collect_output(
    stream: asyncio.StreamReader | None,
    logs: deque[str],
    label: str,
    endpoint_future: asyncio.Future[str],
    endpoint_parser: EndpointParser,
) -> None:
    if stream is None:
        return
    while True:
        line = await stream.readline()
        if not line:
            return
        text = line.decode("utf-8", errors="replace").rstrip()
        logs.append(f"{label}: {text}")
        if not endpoint_future.done():
            endpoint = endpoint_parser(text)
            if endpoint is not None:
                endpoint_future.set_result(endpoint)
        if os.environ.get("MOLI_FRONTEND_SMOKE_TRACE_BG") == "1":
            print(f"[{label}] {text}", file=sys.stderr, flush=True)


async def _wait_for_announced_endpoint(
    process: asyncio.subprocess.Process,
    output_tasks: list[asyncio.Task[Any]],
    endpoint_future: asyncio.Future[str],
    logs: deque[str],
    *,
    timeout: float,
) -> str:
    process_wait = asyncio.create_task(process.wait())
    try:
        done, _pending = await asyncio.wait(
            {endpoint_future, process_wait},
            timeout=timeout,
            return_when=asyncio.FIRST_COMPLETED,
        )
        if endpoint_future in done:
            return endpoint_future.result()
        if process_wait in done:
            await asyncio.gather(*output_tasks, return_exceptions=True)
            if endpoint_future.done():
                return endpoint_future.result()
            tail = "\n".join(logs)
            suffix = f"\n{tail}" if tail else ""
            raise RuntimeError(
                f"browser process exited early with {process.returncode}{suffix}"
            )
        tail = "\n".join(logs)
        suffix = f"\n{tail}" if tail else ""
        raise RuntimeError(
            f"timed out waiting for browser CDP endpoint announcement{suffix}"
        )
    finally:
        if not process_wait.done():
            process_wait.cancel()
        await asyncio.gather(process_wait, return_exceptions=True)


@dataclass
class BrowserProcess:
    name: str
    endpoint: str
    process: asyncio.subprocess.Process
    logs: deque[str]
    tasks: list[asyncio.Task[Any]]
    temp_dirs: list[str]
    version: dict[str, Any]

    async def stop(self) -> None:
        if self.process.returncode is None:
            self.process.terminate()
            try:
                await asyncio.wait_for(self.process.wait(), timeout=5)
            except asyncio.TimeoutError:
                self.process.kill()
                await self.process.wait()
        try:
            await asyncio.wait_for(
                asyncio.gather(*self.tasks, return_exceptions=True),
                timeout=1,
            )
        except asyncio.TimeoutError:
            for task in self.tasks:
                task.cancel()
            await asyncio.gather(*self.tasks, return_exceptions=True)
        for directory in self.temp_dirs:
            shutil.rmtree(directory, ignore_errors=True)


async def _start_process(
    *,
    name: str,
    command: list[str],
    endpoint_parser: EndpointParser,
    temp_dirs: list[str],
) -> BrowserProcess:
    try:
        process = await asyncio.create_subprocess_exec(
            *command,
            cwd=str(REPO_ROOT),
            env=clear_proxy_env(os.environ),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
    except Exception:
        for directory in temp_dirs:
            shutil.rmtree(directory, ignore_errors=True)
        raise
    logs: deque[str] = deque(maxlen=500)
    endpoint_future = asyncio.get_running_loop().create_future()
    tasks = [
        asyncio.create_task(
            _collect_output(
                process.stdout,
                logs,
                f"{name}:stdout",
                endpoint_future,
                endpoint_parser,
            )
        ),
        asyncio.create_task(
            _collect_output(
                process.stderr,
                logs,
                f"{name}:stderr",
                endpoint_future,
                endpoint_parser,
            )
        ),
    ]
    endpoint = ""
    deadline = time.monotonic() + _BROWSER_START_TIMEOUT_SECONDS
    try:
        endpoint = await _wait_for_announced_endpoint(
            process,
            tasks,
            endpoint_future,
            logs,
            timeout=max(0.0, deadline - time.monotonic()),
        )
        version = await wait_for_cdp_endpoint(
            endpoint,
            process,
            logs,
            timeout=max(0.0, deadline - time.monotonic()),
        )
    except (Exception, asyncio.CancelledError):
        holder = BrowserProcess(name, endpoint, process, logs, tasks, temp_dirs, {})
        await holder.stop()
        raise
    return BrowserProcess(name, endpoint, process, logs, tasks, temp_dirs, version)


async def start_moli(binary: Path) -> BrowserProcess:
    cache_dir = tempfile.mkdtemp(prefix="moli-frontend-smoke-cache-")
    return await _start_process(
        name="moli",
        endpoint_parser=_moli_endpoint_from_log,
        temp_dirs=[cache_dir],
        command=[
            str(binary),
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--resource",
            "--layout",
            "--http-cache-dir",
            cache_dir,
            "--log-level",
            "info",
        ],
    )


async def start_chromium(binary: Path) -> BrowserProcess:
    profile_dir = tempfile.mkdtemp(prefix="moli-frontend-smoke-chromium-")
    return await _start_process(
        name="chromium",
        endpoint_parser=_chromium_endpoint_from_log,
        temp_dirs=[profile_dir],
        command=[
            str(binary),
            "--headless=new",
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-default-apps",
            "--disable-extensions",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
            f"--user-data-dir={profile_dir}",
            "about:blank",
        ],
    )
