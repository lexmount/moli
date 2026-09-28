from __future__ import annotations

import os
import shutil
import signal
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, ReservedPort, clear_proxy_env, reserve_port
from .sampling import ResourceSampler
from .process import _kill_process_group
from .serve import probe_url
from .synthetic_compare import target_enables_all_resource_fetch, target_metadata


class TargetServeError(RuntimeError):
    pass


class TargetServeProcessExit(TargetServeError):
    pass


class TargetServeReadinessTimeout(TargetServeError):
    pass


@dataclass
class TargetServeHandle:
    target: str
    process: subprocess.Popen[bytes]
    endpoint: str
    command: list[str]
    logs: list[str]
    sampler: ResourceSampler | None
    log_threads: list[threading.Thread]
    port_lease: ReservedPort
    temp_dir: Path | None = None
    ready_ms: float | None = None
    native: bool = False


def _append_log(logs: list[str], line: str) -> None:
    logs.append(line)
    if len(logs) > 400:
        del logs[: len(logs) - 400]


def _read_available(stream: Any, label: str, logs: list[str]) -> None:
    if stream is None:
        return
    try:
        while True:
            line = stream.readline()
            if not line:
                return
            _append_log(logs, f"{label}: {line.decode('utf-8', errors='replace').rstrip()}")
    except OSError:
        return


def _terminate_process(process: subprocess.Popen[bytes]) -> bool:
    if process.poll() is not None:
        return True
    try:
        process.terminate()
    except OSError:
        return process.poll() is not None
    try:
        process.wait(timeout=2)
        return True
    except subprocess.TimeoutExpired:
        try:
            process.kill()
        except OSError:
            return process.poll() is not None
        try:
            process.wait(timeout=2)
            return True
        except subprocess.TimeoutExpired:
            return False


def _serve_command(
    target: str,
    binary: Path,
    port: int,
    temp_dir: Path | None,
    extra_args: tuple[str, ...] = (),
    *,
    native: bool = False,
) -> list[str]:
    engine = target_metadata(target)["engine"]
    if engine == "moli" or engine == "lightpanda":
        compatibility_args = (
            ["--layout", "--resource"]
            if target_enables_all_resource_fetch(target)
            else []
        )
        return [
            str(binary),
            "serve",
            *compatibility_args,
            *extra_args,
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
        ]
    if engine == "obscura":
        return [str(binary), "serve", "--port", str(port)]
    if engine == "chrome":
        if temp_dir is None:
            raise RuntimeError("chrome target requires a temporary profile directory")
        if native:
            if extra_args:
                raise ValueError("native browser launches do not accept identity or feature overrides")
            return [
                str(binary), "--no-first-run", "--no-default-browser-check",
                "--no-proxy-server", "--password-store=basic", "--window-size=1920,1080",
                "--remote-debugging-address=127.0.0.1",
                f"--user-data-dir={temp_dir}", f"--remote-debugging-port={port}", "about:blank",
            ]
        return [
            str(binary),
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--no-first-run",
            f"--user-data-dir={temp_dir}",
            f"--remote-debugging-port={port}",
            *extra_args,
            "about:blank",
        ]
    raise RuntimeError(f"unknown CDP target: {target}")


def start_target_serve(
    target: str,
    binary: Path,
    timeout_seconds: float,
    extra_args: tuple[str, ...] = (),
    *,
    sample_interval_seconds: float | None = None,
    native: bool = False,
) -> TargetServeHandle:
    if native and (target not in {"moli-full-cdp", "chrome-cdp"} or extra_args):
        raise ValueError("native mode requires moli-full-cdp or chrome-cdp without extra flags")
    engine = target_metadata(target)["engine"]
    temp_dir = Path(tempfile.mkdtemp(prefix=f"moli-benchmark-{target}-")) if engine == "chrome" else None
    logs: list[str] = []
    reserved_port = reserve_port()
    try:
        port = reserved_port.port
        endpoint = f"http://127.0.0.1:{port}"
        command = _serve_command(target, binary, port, temp_dir, extra_args, native=native)
        reserved_port.release_socket()
        process = subprocess.Popen(
            command,
            cwd=REPO_ROOT,
            env=clear_proxy_env(os.environ),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
    except BaseException:
        reserved_port.close()
        if temp_dir is not None:
            shutil.rmtree(temp_dir, ignore_errors=True)
        raise
    log_threads = [
        threading.Thread(target=_read_available, args=(process.stdout, "stdout", logs), daemon=True),
        threading.Thread(target=_read_available, args=(process.stderr, "stderr", logs), daemon=True),
    ]
    for thread in log_threads:
        thread.start()
    sampler = None if native else (
        ResourceSampler(process.pid)
        if sample_interval_seconds is None
        else ResourceSampler(process.pid, interval_seconds=sample_interval_seconds)
    )
    if sampler is not None:
        sampler.start()
    handle = TargetServeHandle(
        target=target,
        process=process,
        endpoint=endpoint,
        command=command,
        logs=logs,
        sampler=sampler,
        log_threads=log_threads,
        port_lease=reserved_port,
        temp_dir=temp_dir,
        native=native,
    )
    started = time.perf_counter()
    deadline = started + timeout_seconds
    version_url = endpoint + "/json/version"
    while time.perf_counter() < deadline:
        if process.poll() is not None:
            stop_target_serve(handle)
            raise TargetServeProcessExit(
                f"{target} CDP server exited early with {process.returncode}: "
                f"{'; '.join(logs[-20:])}"
            )
        if probe_url(version_url):
            handle.ready_ms = (time.perf_counter() - started) * 1000.0
            return handle
        time.sleep(0.025)
    stop_target_serve(handle)
    raise TargetServeReadinessTimeout(
        f"timed out waiting for {target} CDP server at {version_url}: "
        f"{'; '.join(logs[-20:])}"
    )


def stop_target_serve(
    handle: TargetServeHandle | None,
    *,
    include_resource_samples: bool = False,
) -> dict[str, Any]:
    if handle is None:
        return {}
    process_exited = False
    resources: dict[str, Any] = {}
    try:
        process_exited = _terminate_process(handle.process)
        if handle.native and os.name == "posix":
            # Native sessions own a fresh process group. Reap residual children
            # even if Chromium's root crashed or Browser.close already exited it.
            _kill_process_group(handle.process.pid, signal.SIGKILL)
        if process_exited:
            for thread in handle.log_threads:
                thread.join(timeout=0.2)
        else:
            _append_log(handle.logs, "process did not exit after SIGKILL; skipped pipe drain")
        resources = handle.sampler.stop() if handle.sampler is not None else {}
        if include_resource_samples and handle.sampler is not None:
            resources["samples"] = list(handle.sampler.samples)
    finally:
        handle.port_lease.close()
        if handle.temp_dir is not None:
            shutil.rmtree(handle.temp_dir, ignore_errors=True)
    return {
        "process_exited": process_exited,
        "returncode": handle.process.returncode,
        "resources": resources,
        "log_tail": handle.logs[-40:],
    }
