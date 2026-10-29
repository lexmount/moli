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
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from playwright.async_api import async_playwright

from .config import (
    REPO_ROOT,
    TRACE_BACKGROUND_PROCESS_ENV,
    clear_proxy_env,
    moli_binary,
)
from .process import subprocess_starts_new_session, terminate_process_tree


@dataclass
class BrowserProcess:
    process: asyncio.subprocess.Process
    logs: list[str]
    tasks: list[asyncio.Task[Any]]
    data_dir: str
    endpoint_ready: asyncio.Future[str]
    engine: str = "moli"


BROWSER_ENV = "MOLI_SMOKE_BROWSER"
BROWSER_LAUNCHES: list[dict[str, Any]] = []


_ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")
_LISTENING_ADDRESS = re.compile(
    r"\bprotocol server listening\b.*\baddr=127\.0\.0\.1:(?P<port>[0-9]{1,5})\b"
)


def _endpoint_from_log_line(line: str) -> str | None:
    chrome = re.search(r"DevTools listening on ws://127\.0\.0\.1:([0-9]+)/", line)
    if chrome is not None:
        port = int(chrome.group(1))
        return f"http://127.0.0.1:{port}" if 0 < port <= 65535 else None
    match = _LISTENING_ADDRESS.search(_ANSI_ESCAPE.sub("", line))
    if match is None:
        return None
    port = int(match.group("port"))
    if not 0 < port <= 65535:
        return None
    return f"http://127.0.0.1:{port}"


def _consume_task_result(task: asyncio.Task[Any]) -> None:
    try:
        task.exception()
    except asyncio.CancelledError:
        pass


async def _collect_process_output(
    stream: asyncio.StreamReader | None,
    logs: list[str],
    label: str,
    endpoint_ready: asyncio.Future[str],
    engine: str = "moli",
) -> None:
    if stream is None:
        return
    while True:
        line = await stream.readline()
        if not line:
            return
        text = line.decode("utf-8", errors="replace").rstrip()
        logs.append(f"{label}: {text}")
        endpoint = _endpoint_from_log_line(text)
        if endpoint is not None and not endpoint_ready.done():
            endpoint_ready.set_result(endpoint)
        if os.environ.get(TRACE_BACKGROUND_PROCESS_ENV) == "1":
            print(f"[{engine} {label}] {text}", file=sys.stderr, flush=True)


async def start_moli_serve(
    port: int = 0,
    *,
    layout: bool = True,
    extra_args: tuple[str, ...] = (),
    env_overrides: dict[str, str | None] | None = None,
) -> BrowserProcess:
    if os.environ.get(BROWSER_ENV) == "chromium":
        raise RuntimeError("a Chromium smoke worker must not launch Moli")
    binary = moli_binary()
    data_dir = tempfile.mkdtemp(prefix="moli-cdp-smoke-cache-")
    command = [
        str(binary),
        "serve",
        "--debugger",
        "--host",
        "127.0.0.1",
        "--port",
        str(port),
        "--resource",
        "--log-level",
        os.environ.get("MOLI_SMOKE_LOG_LEVEL", "info"),
    ]
    if layout:
        command.append("--layout")
    command.extend(("--http-cache-dir", data_dir, *extra_args))
    env = clear_proxy_env(os.environ)
    for key, value in (env_overrides or {}).items():
        if value is None:
            env.pop(key, None)
        else:
            env[key] = value
    try:
        process = await asyncio.create_subprocess_exec(
            *command,
            cwd=str(REPO_ROOT),
            env=env,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            start_new_session=subprocess_starts_new_session(),
        )
    except Exception:
        shutil.rmtree(data_dir, ignore_errors=True)
        raise
    logs: list[str] = []
    endpoint_ready = asyncio.get_running_loop().create_future()
    tasks = [
        asyncio.create_task(
            _collect_process_output(process.stdout, logs, "stdout", endpoint_ready)
        ),
        asyncio.create_task(
            _collect_process_output(process.stderr, logs, "stderr", endpoint_ready)
        ),
    ]
    BROWSER_LAUNCHES.append({"engine": "moli", "command": command, "pid": process.pid})
    return BrowserProcess(
        process=process,
        logs=logs,
        tasks=tasks,
        data_dir=data_dir,
        endpoint_ready=endpoint_ready,
    )


async def start_browser(port: int = 0, *, extra_args: tuple[str, ...] = ()) -> BrowserProcess:
    if os.environ.get(BROWSER_ENV, "moli") == "moli":
        return await start_moli_serve(port, extra_args=extra_args)
    async with async_playwright() as playwright:
        binary = Path(
            os.environ.get("CHROMIUM_BIN") or playwright.chromium.executable_path
        ).expanduser().resolve()
    if not binary.is_file():
        raise RuntimeError("missing Chromium; run `playwright install chromium` or set CHROMIUM_BIN")
    profile = tempfile.mkdtemp(prefix="moli-cdp-smoke-chromium-")
    command = [
        str(binary), "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
        "--disable-back-forward-cache",
        # Keep profile services from adding unrelated network descriptors during
        # local-fixture resource accounting (also used by Playwright's launcher).
        "--disable-background-networking", "--disable-component-update", "--disable-sync",
        "--no-first-run", "--no-default-browser-check", "--remote-allow-origins=*",
        "--remote-debugging-address=127.0.0.1", f"--remote-debugging-port={port}",
        f"--user-data-dir={profile}", *extra_args, "about:blank",
    ]
    try:
        process = await asyncio.create_subprocess_exec(
            *command, cwd=str(REPO_ROOT), env=clear_proxy_env(os.environ),
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            start_new_session=subprocess_starts_new_session(),
        )
    except BaseException:
        shutil.rmtree(profile, ignore_errors=True)
        raise
    logs: list[str] = []
    endpoint_ready = asyncio.get_running_loop().create_future()
    tasks = [asyncio.create_task(_collect_process_output(stream, logs, label, endpoint_ready, "chromium"))
             for stream, label in [(process.stdout, "stdout"), (process.stderr, "stderr")]]
    BROWSER_LAUNCHES.append({"engine": "chromium", "command": command, "pid": process.pid})
    return BrowserProcess(process, logs, tasks, profile, endpoint_ready, "chromium")


async def browser_identity(endpoint: str, expected: str | None) -> dict[str, Any]:
    def read_version() -> dict[str, Any]:
        with urllib.request.urlopen(endpoint.rstrip("/") + "/json/version", timeout=2) as response:
            return json.load(response)
    version = await asyncio.to_thread(read_version)
    engine = "moli" if version.get("webSocketDebuggerUrl", "").endswith("/moli-browser") else "chromium"
    if engine == "chromium" and not str(version.get("Browser", "")).startswith(("Chrome/", "HeadlessChrome/", "Chromium/")):
        raise RuntimeError(f"endpoint is not a Chromium browser: {version}")
    if expected is not None and engine != expected:
        raise RuntimeError(f"requested {expected}, but endpoint belongs to {engine}: {version}")
    return {"engine": engine, "version": version}


async def stop_browser(serve: BrowserProcess | None) -> None:
    if serve is None:
        return
    stopped = await terminate_process_tree(serve.process)
    for task in serve.tasks:
        task.cancel()
    done, pending = await asyncio.wait(serve.tasks, timeout=2)
    for task in done:
        _consume_task_result(task)
    for task in pending:
        task.add_done_callback(_consume_task_result)
    shutil.rmtree(serve.data_dir, ignore_errors=True)
    if not stopped:
        raise RuntimeError(f"{serve.engine} process {serve.process.pid} did not exit after SIGKILL")


def _probe_url(url: str) -> bool:
    try:
        with urllib.request.urlopen(url, timeout=0.5) as response:
            response.read()
        return True
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


async def wait_for_cdp_server(
    endpoint: str,
    serve: BrowserProcess | None,
) -> None:
    version_url = endpoint.rstrip("/") + "/json/version"
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if serve is not None and serve.process.returncode is not None:
            tail = "\n".join(serve.logs[-80:])
            raise RuntimeError(f"{serve.engine} exited early with {serve.process.returncode}\n{tail}")
        if await asyncio.to_thread(_probe_url, version_url):
            return
        await asyncio.sleep(0.05)
    tail_text = "\n".join(serve.logs[-80:]) if serve is not None else ""
    tail = f"\n{tail_text}" if tail_text else ""
    raise RuntimeError(f"timed out waiting for CDP server at {endpoint}{tail}")


async def wait_for_browser_endpoint(
    serve: BrowserProcess,
    *,
    timeout_seconds: float = 10,
) -> str:
    process_exit = asyncio.create_task(serve.process.wait())
    try:
        done, _ = await asyncio.wait(
            (serve.endpoint_ready, process_exit),
            timeout=timeout_seconds,
            return_when=asyncio.FIRST_COMPLETED,
        )
        if serve.endpoint_ready in done:
            endpoint = serve.endpoint_ready.result()
            await wait_for_cdp_server(endpoint, serve)
            identity = await browser_identity(endpoint, serve.engine)
            for launch in BROWSER_LAUNCHES:
                if launch["pid"] == serve.process.pid:
                    launch["browser"] = identity
            return endpoint
        if process_exit in done:
            tail = "\n".join(serve.logs[-80:])
            raise RuntimeError(
                f"{serve.engine} exited early with {process_exit.result()}\n{tail}"
            )
        tail_text = "\n".join(serve.logs[-80:])
        tail = f"\n{tail_text}" if tail_text else ""
        raise RuntimeError(
            f"timed out waiting for {serve.engine} to report its bound endpoint" + tail
        )
    finally:
        if not process_exit.done():
            process_exit.cancel()
            await asyncio.gather(process_exit, return_exceptions=True)
