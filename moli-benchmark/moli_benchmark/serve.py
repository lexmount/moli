from __future__ import annotations

import os
import signal
import subprocess
import threading
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, ReservedPort, clear_proxy_env, reserve_port
from .process import read_time_verbose_file, time_verbose_command
from .sampling import ResourceSampler, collect_cgroup_artifacts

MAX_SERVE_LOG_LINES = 4096


@dataclass
class ServeHandle:
    process: subprocess.Popen[bytes]
    endpoint: str
    port: int
    logs: list[str]
    sampler: ResourceSampler
    port_lease: ReservedPort
    ready_ms: float | None = None
    time_verbose_path: Path | None = None
    cgroup_artifact_dir: Path | None = None
    log_threads: list[threading.Thread] | None = None


def _append_log(logs: list[str], line: str) -> None:
    logs.append(line)
    overflow = len(logs) - MAX_SERVE_LOG_LINES
    if overflow > 0:
        del logs[:overflow]


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


def _drain_stream(stream: Any, label: str, logs: list[str]) -> None:
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


def _start_log_drain_threads(process: subprocess.Popen[bytes], logs: list[str]) -> list[threading.Thread]:
    threads: list[threading.Thread] = []
    for label, stream in (("stdout", process.stdout), ("stderr", process.stderr)):
        thread = threading.Thread(
            target=_drain_stream,
            args=(stream, label, logs),
            name=f"moli-benchmark-{label}-drain",
            daemon=True,
        )
        thread.start()
        threads.append(thread)
    return threads


def _join_log_drain_threads(threads: list[threading.Thread] | None) -> None:
    for thread in threads or []:
        thread.join(timeout=1)


def _signal_process_group(process: subprocess.Popen[bytes], sig: signal.Signals) -> None:
    # start_moli_serve gives every invocation its own session, including when
    # /usr/bin/time is the group leader and the browser is its child.
    if os.name == "posix":
        try:
            os.killpg(process.pid, sig)
            return
        except ProcessLookupError:
            return
        except OSError:
            pass
    if process.poll() is None:
        try:
            if sig == signal.SIGTERM:
                process.terminate()
            else:
                process.kill()
        except OSError:
            pass


def _terminate_process(process: subprocess.Popen[bytes]) -> bool:
    _signal_process_group(process, signal.SIGTERM)
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        _signal_process_group(process, signal.SIGKILL)
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            return False
    # The wrapper may exit before a child, or may have exited before teardown.
    # Retire any remaining members of the dedicated group as well.
    _signal_process_group(process, signal.SIGKILL)
    return True


def probe_url(url: str, timeout_seconds: float = 0.5) -> bool:
    try:
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with opener.open(url, timeout=timeout_seconds) as response:
            response.read()
        return True
    except (urllib.error.URLError, TimeoutError, OSError):
        return False


def start_moli_serve(
    moli_bin: Path,
    timeout_seconds: float,
    *,
    time_verbose_path: Path | None = None,
    cgroup_artifact_dir: Path | None = None,
) -> ServeHandle:
    logs: list[str] = []
    reserved_port = reserve_port()
    try:
        port = reserved_port.port
        endpoint = f"http://127.0.0.1:{port}"
        command = time_verbose_command(
            [
                str(moli_bin),
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                str(port),
            ],
            time_verbose_path,
        )
        reserved_port.release_socket()
        process = subprocess.Popen(
            command,
            cwd=REPO_ROOT,
            env=clear_proxy_env(os.environ),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        log_threads = _start_log_drain_threads(process, logs)
    except BaseException:
        reserved_port.close()
        raise
    sampler = ResourceSampler(process.pid)
    sampler.start()
    handle = ServeHandle(
        process=process,
        endpoint=endpoint,
        port=port,
        logs=logs,
        sampler=sampler,
        port_lease=reserved_port,
        time_verbose_path=time_verbose_path,
        cgroup_artifact_dir=cgroup_artifact_dir,
        log_threads=log_threads,
    )

    started = time.perf_counter()
    deadline = started + timeout_seconds
    version_url = endpoint + "/json/version"
    while time.perf_counter() < deadline:
        if process.poll() is not None:
            _join_log_drain_threads(handle.log_threads)
            sampler.stop()
            reserved_port.close()
            raise RuntimeError(f"moli serve exited early with {process.returncode}: {'; '.join(logs[-20:])}")
        if probe_url(version_url):
            handle.ready_ms = (time.perf_counter() - started) * 1000.0
            return handle
        time.sleep(0.025)

    process_exited = _terminate_process(process)
    sampler.stop()
    reserved_port.close()
    if process_exited:
        _join_log_drain_threads(handle.log_threads)
    else:
        logs.append("process did not exit after SIGKILL; skipped pipe drain")
    raise RuntimeError(f"timed out waiting for moli serve at {version_url}: {'; '.join(logs[-20:])}")


def stop_moli_serve(handle: ServeHandle | None) -> dict[str, Any]:
    if handle is None:
        return {}
    cgroup = (
        collect_cgroup_artifacts(handle.process.pid, handle.cgroup_artifact_dir)
        if handle.cgroup_artifact_dir is not None
        else None
    )
    process_exited = _terminate_process(handle.process)
    if process_exited:
        _join_log_drain_threads(handle.log_threads)
    else:
        handle.logs.append("process did not exit after SIGKILL; skipped pipe drain")
    resources = handle.sampler.stop()
    handle.port_lease.close()
    return {
        "returncode": handle.process.returncode,
        "resources": resources,
        "time_verbose": (
            read_time_verbose_file(handle.time_verbose_path) if handle.time_verbose_path is not None else None
        ),
        "cgroup": cgroup,
        "log_tail": handle.logs[-40:],
    }
