from __future__ import annotations

import asyncio
import os
import re
import shutil
import sys
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any

from .config import REPO_ROOT, clear_proxy_env, moli_binary
from .process import subprocess_starts_new_session, terminate_process_tree


@dataclass
class MoliServe:
    process: asyncio.subprocess.Process
    logs: list[str]
    tasks: list[asyncio.Task[Any]]
    http_cache_dir: str
    endpoint_ready: asyncio.Future[str]


_ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")
_LISTENING_ADDRESS = re.compile(
    r"\bprotocol server listening\b.*\baddr=127\.0\.0\.1:(?P<port>[0-9]{1,5})\b"
)


def _moli_endpoint_from_log_line(line: str) -> str | None:
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
) -> None:
    if stream is None:
        return
    while True:
        line = await stream.readline()
        if not line:
            return
        text = line.decode("utf-8", errors="replace").rstrip()
        logs.append(f"{label}: {text}")
        endpoint = _moli_endpoint_from_log_line(text)
        if endpoint is not None and not endpoint_ready.done():
            endpoint_ready.set_result(endpoint)
        if os.environ.get("MOLI_SMOKE_TRACE_BG") == "1":
            print(f"[moli serve {label}] {text}", file=sys.stderr, flush=True)


async def start_moli_serve(
    port: int = 0,
    *,
    layout: bool = True,
    extra_args: tuple[str, ...] = (),
) -> MoliServe:
    binary = moli_binary()
    http_cache_dir = tempfile.mkdtemp(prefix="moli-cdp-smoke-cache-")
    command = [
        str(binary),
        "serve",
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
    command.extend(("--http-cache-dir", http_cache_dir, *extra_args))
    try:
        process = await asyncio.create_subprocess_exec(
            *command,
            cwd=str(REPO_ROOT),
            env=clear_proxy_env(os.environ),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            start_new_session=subprocess_starts_new_session(),
        )
    except Exception:
        shutil.rmtree(http_cache_dir, ignore_errors=True)
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
    return MoliServe(
        process=process,
        logs=logs,
        tasks=tasks,
        http_cache_dir=http_cache_dir,
        endpoint_ready=endpoint_ready,
    )


async def stop_moli_serve(serve: MoliServe | None) -> None:
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
    shutil.rmtree(serve.http_cache_dir, ignore_errors=True)
    if not stopped:
        raise RuntimeError(f"moli serve process {serve.process.pid} did not exit after SIGKILL")


def _probe_url(url: str) -> bool:
    try:
        with urllib.request.urlopen(url, timeout=0.5) as response:
            response.read()
        return True
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


async def wait_for_cdp_server(
    endpoint: str,
    serve: MoliServe | None,
) -> None:
    version_url = endpoint.rstrip("/") + "/json/version"
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if serve is not None and serve.process.returncode is not None:
            tail = "\n".join(serve.logs[-80:])
            raise RuntimeError(f"moli serve exited early with {serve.process.returncode}\n{tail}")
        if await asyncio.to_thread(_probe_url, version_url):
            return
        await asyncio.sleep(0.05)
    tail_text = "\n".join(serve.logs[-80:]) if serve is not None else ""
    tail = f"\n{tail_text}" if tail_text else ""
    raise RuntimeError(f"timed out waiting for CDP server at {endpoint}{tail}")


async def wait_for_moli_endpoint(
    serve: MoliServe,
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
            return endpoint
        if process_exit in done:
            tail = "\n".join(serve.logs[-80:])
            raise RuntimeError(
                f"moli serve exited early with {process_exit.result()}\n{tail}"
            )
        tail_text = "\n".join(serve.logs[-80:])
        tail = f"\n{tail_text}" if tail_text else ""
        raise RuntimeError(
            "timed out waiting for moli to report its bound endpoint" + tail
        )
    finally:
        if not process_exit.done():
            process_exit.cancel()
            await asyncio.gather(process_exit, return_exceptions=True)
