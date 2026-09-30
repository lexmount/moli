from __future__ import annotations

import asyncio
import os
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any

from .config import REPO_ROOT, clear_proxy_env, moli_binary


SERVE_DIAGNOSTIC_TAIL_LINES = 120


@dataclass
class MoliServe:
    process: asyncio.subprocess.Process
    logs: list[str]
    tasks: list[asyncio.Task[Any]]


async def _collect_process_output(stream: asyncio.StreamReader | None, logs: list[str], label: str) -> None:
    if stream is None:
        return
    while True:
        line = await stream.readline()
        if not line:
            return
        text = line.decode("utf-8", errors="replace").rstrip()
        logs.append(f"{label}: {text}")
        if os.environ.get("MOLI_WEBDRIVER_SMOKE_TRACE_BG") == "1":
            print(f"[moli serve {label}] {text}", file=sys.stderr, flush=True)


async def start_moli_serve(port: int, *, layout: bool = False) -> MoliServe:
    binary = moli_binary()
    process = await asyncio.create_subprocess_exec(
        str(binary),
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        str(port),
        "--image",
        *(["--layout"] if layout else []),
        cwd=str(REPO_ROOT),
        env=clear_proxy_env(os.environ),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    logs: list[str] = []
    tasks = [
        asyncio.create_task(_collect_process_output(process.stdout, logs, "stdout")),
        asyncio.create_task(_collect_process_output(process.stderr, logs, "stderr")),
    ]
    return MoliServe(process=process, logs=logs, tasks=tasks)


async def stop_moli_serve(serve: MoliServe | None) -> None:
    if serve is None:
        return
    if serve.process.returncode is None:
        serve.process.terminate()
        try:
            await asyncio.wait_for(serve.process.wait(), timeout=5)
        except asyncio.TimeoutError:
            serve.process.kill()
            await serve.process.wait()

    # The child has closed its pipe writers, so let both collectors consume
    # the remaining buffered output. Cancelling them here can discard the
    # panic or connection error that caused a smoke scenario to fail.
    await asyncio.gather(*serve.tasks, return_exceptions=True)


def render_moli_serve_diagnostics(
    serve: MoliServe,
    max_lines: int = SERVE_DIAGNOSTIC_TAIL_LINES,
) -> str:
    if max_lines <= 0:
        raise ValueError("moli serve diagnostic line limit must be positive")

    total_lines = len(serve.logs)
    selected_lines = serve.logs[-max_lines:]
    body = "\n".join(selected_lines) if selected_lines else "<no child output captured>"
    return (
        "moli serve diagnostics "
        f"(last {len(selected_lines)} of {total_lines} captured lines):\n{body}"
    )


def _probe_status(url: str) -> bool:
    try:
        with urllib.request.urlopen(url, timeout=0.5) as response:
            response.read()
        return response.status == 200
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


async def wait_for_webdriver_server(
    endpoint: str,
    serve: MoliServe | None,
) -> None:
    status_url = endpoint.rstrip("/") + "/status"
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if serve is not None and serve.process.returncode is not None:
            tail = "\n".join(serve.logs[-80:])
            raise RuntimeError(f"moli serve exited early with {serve.process.returncode}\n{tail}")
        if await asyncio.to_thread(_probe_status, status_url):
            return
        await asyncio.sleep(0.05)
    tail_text = "\n".join(serve.logs[-80:]) if serve is not None else ""
    tail = f"\n{tail_text}" if tail_text else ""
    raise RuntimeError(f"timed out waiting for WebDriver server at {endpoint}{tail}")
