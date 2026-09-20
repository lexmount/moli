"""Engine drivers for cross-engine WPT runs.

An :class:`EngineDriver` knows how to launch a single headless browser engine
in CDP mode, expose its CDP HTTP endpoint (e.g. ``http://127.0.0.1:9222``),
and report enough metadata to be recorded in ``environment.json``.

The drivers are intentionally thin wrappers around the existing
``target_serve.start_target_serve`` machinery. They do **not** know anything
about WPT cases or the testharness — that lives in :mod:`runner`.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from ..config import REPO_ROOT, ReservedPort, clear_proxy_env, reserve_port
from ..sampling import ResourceSampler
from ..serve import _join_log_drain_threads, _start_log_drain_threads, probe_url
from ..versions import sha256_file


@dataclass
class EngineDriverHandle:
    """Live state of a launched engine driver."""

    engine: str
    process: subprocess.Popen[bytes]
    endpoint: str
    sampler: ResourceSampler
    port_lease: ReservedPort
    logs: list[str] = field(default_factory=list)
    temp_dir: Path | None = None
    ready_ms: float | None = None
    binary: Path | None = None
    binary_sha256: str | None = None
    binary_version: str | None = None
    log_threads: list[threading.Thread] = field(default_factory=list, repr=False)


def _binary_version(binary: Path, version_args: tuple[str, ...]) -> str | None:
    try:
        completed = subprocess.run(
            [str(binary), *version_args],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    output = (completed.stdout or completed.stderr).strip()
    return output.splitlines()[0] if output else None


def _terminate(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        process.terminate()
        process.wait(timeout=2)
        return
    except subprocess.TimeoutExpired:
        pass
    except OSError:
        return
    try:
        process.kill()
        process.wait(timeout=2)
    except (OSError, subprocess.TimeoutExpired):
        return


@dataclass
class EngineDriver:
    """Definition of how to launch a single engine.

    ``build_command`` returns the argv to launch the engine in CDP serve mode
    on the given port. ``needs_profile_dir`` indicates whether a temp profile
    directory should be provisioned before launch.
    """

    name: str
    binary_env_var: str
    default_binary_names: tuple[str, ...]
    build_command: Callable[[Path, int, Path | None], list[str]]
    version_args: tuple[str, ...]
    needs_profile_dir: bool = False
    extra_env: dict[str, str] = field(default_factory=dict)
    # Optional CLI-mode fetch invocation: (binary, url, timeout_seconds) -> argv.
    # When set, the runner prefers HTTP-callback CLI mode and skips CDP launch
    # entirely. Engines whose CLI does not execute JavaScript (e.g. obscura)
    # leave this as None and fall back to CDP.
    cli_fetch_command: Callable[[Path, str, float], list[str]] | None = None

    def resolve_binary(self, override: str | None = None) -> Path:
        raw = override or os.environ.get(self.binary_env_var)
        if raw:
            path = Path(raw).expanduser().resolve()
            if not path.exists():
                raise RuntimeError(f"{self.binary_env_var} does not exist: {path}")
            return path
        for name in self.default_binary_names:
            located = shutil.which(name)
            if located:
                return Path(located).resolve()
        raise RuntimeError(
            f"missing {self.name} binary; set {self.binary_env_var} or install one of {self.default_binary_names}"
        )

    def launch(self, *, binary_override: str | None = None, ready_timeout_seconds: float = 30.0) -> EngineDriverHandle:
        binary = self.resolve_binary(binary_override)
        last_error: RuntimeError | None = None
        for attempt in range(1, 4):
            temp_dir: Path | None = None
            if self.needs_profile_dir:
                temp_dir = Path(tempfile.mkdtemp(prefix=f"moli-bench-wpt-{self.name}-"))
            reserved = reserve_port()
            try:
                port = reserved.port
                command = self.build_command(binary, port, temp_dir)
                env = clear_proxy_env(os.environ)
                env.update(self.extra_env)
                reserved.release_socket()
                process = subprocess.Popen(
                    command,
                    cwd=REPO_ROOT,
                    env=env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    start_new_session=True,
                )
                logs: list[str] = []
                log_threads = _start_log_drain_threads(process, logs)
            except BaseException:
                reserved.close()
                if temp_dir is not None:
                    shutil.rmtree(temp_dir, ignore_errors=True)
                raise
            endpoint = f"http://127.0.0.1:{port}"
            sampler = ResourceSampler(process.pid)
            sampler.start()
            handle = EngineDriverHandle(
                engine=self.name,
                process=process,
                endpoint=endpoint,
                sampler=sampler,
                port_lease=reserved,
                logs=logs,
                log_threads=log_threads,
                temp_dir=temp_dir,
                binary=binary,
                binary_sha256=sha256_file(binary),
                binary_version=_binary_version(binary, self.version_args),
            )
            started = time.perf_counter()
            deadline = started + ready_timeout_seconds
            version_url = endpoint + "/json/version"
            while time.perf_counter() < deadline:
                if process.poll() is not None:
                    _join_log_drain_threads(handle.log_threads)
                    log_tail = "; ".join(handle.logs[-20:])
                    self.shutdown(handle)
                    last_error = RuntimeError(
                        f"{self.name} CDP server exited early with code {process.returncode}"
                        f" on launch attempt {attempt}: {log_tail}"
                    )
                    if "Address already in use" in log_tail and attempt < 3:
                        time.sleep(0.05 * attempt)
                        break
                    raise last_error
                if probe_url(version_url):
                    handle.ready_ms = (time.perf_counter() - started) * 1000.0
                    return handle
                time.sleep(0.025)
            else:
                self.shutdown(handle)
                last_error = RuntimeError(
                    f"timed out waiting for {self.name} CDP server at {version_url}: "
                    + "; ".join(handle.logs[-20:])
                )
                raise last_error
            # Early exit due to a retryable port race.
            continue
        if last_error is not None:
            raise last_error
        raise RuntimeError(f"failed to launch {self.name} CDP server")

    def shutdown(self, handle: EngineDriverHandle | None) -> dict[str, Any]:
        if handle is None:
            return {}
        try:
            _terminate(handle.process)
            _join_log_drain_threads(handle.log_threads)
            resources = handle.sampler.stop()
        finally:
            handle.port_lease.close()
            if handle.temp_dir is not None:
                shutil.rmtree(handle.temp_dir, ignore_errors=True)
        return {
            "returncode": handle.process.returncode,
            "ready_ms": handle.ready_ms,
            "resources": resources,
            "log_tail": handle.logs[-40:],
        }


def _moli_command(binary: Path, port: int, _tmp: Path | None) -> list[str]:
    return [
        str(binary),
        "serve",
        "--layout",
        "--resource",
        "--host",
        "127.0.0.1",
        "--port",
        str(port),
    ]


def _moli_fetch(binary: Path, url: str, timeout_seconds: float) -> list[str]:
    timeout_ms = max(1000, int(timeout_seconds * 1000))
    return [
        str(binary), "fetch", "--layout", "--resource", url,
        "--wait-until", "done",
        "--wait-script",
        "globalThis.__bench_wpt__ && globalThis.__bench_wpt__.source !== 'incremental'",
        "--timeout", str(timeout_ms),
        "--log-level", "error",
    ]


def _lightpanda_command(binary: Path, port: int, _tmp: Path | None) -> list[str]:
    return [str(binary), "serve", "--host", "127.0.0.1", "--port", str(port)]


def _lightpanda_fetch(binary: Path, url: str, timeout_seconds: float) -> list[str]:
    # lightpanda's --wait-until done often never fires (no DOMContentLoaded
    # equivalent in the testharness completion path), so wait-ms acts as the
    # real in-page deadline. Keep it aligned with the configured case timeout.
    wait_ms = max(1000, int(timeout_seconds * 1000))
    return [
        str(binary), "fetch", url,
        "--dump", "html",
        "--wait-until", "done",
        "--wait-ms", str(wait_ms),
        "--http-timeout", str(wait_ms),
        "--terminate-ms", str(wait_ms),
    ]


def _obscura_command(binary: Path, port: int, _tmp: Path | None) -> list[str]:
    return [str(binary), "serve", "--port", str(port)]


def _chrome_command(binary: Path, port: int, tmp: Path | None) -> list[str]:
    if tmp is None:
        raise RuntimeError("chrome driver requires a profile directory")
    return [
        str(binary),
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        "--disable-dev-shm-usage",
        # WPT opens auxiliary test windows without user activation.
        "--disable-popup-blocking",
        "--no-first-run",
        f"--user-data-dir={tmp}",
        f"--remote-debugging-port={port}",
        "about:blank",
    ]


ENGINES: dict[str, EngineDriver] = {
    "moli": EngineDriver(
        name="moli",
        binary_env_var="MOLI_BIN",
        default_binary_names=("moli",),
        build_command=_moli_command,
        cli_fetch_command=_moli_fetch,
        version_args=("version",),
    ),
    "lightpanda": EngineDriver(
        name="lightpanda",
        binary_env_var="LIGHTPANDA_BIN",
        default_binary_names=("lightpanda",),
        build_command=_lightpanda_command,
        cli_fetch_command=_lightpanda_fetch,
        version_args=("version",),
    ),
    "obscura": EngineDriver(
        name="obscura",
        binary_env_var="OBSCURA_BIN",
        default_binary_names=("obscura",),
        build_command=_obscura_command,
        version_args=("--help",),
    ),
    "chrome": EngineDriver(
        name="chrome",
        binary_env_var="CHROME_BIN",
        default_binary_names=("google-chrome", "google-chrome-stable", "chrome", "chromium", "chromium-browser"),
        build_command=_chrome_command,
        version_args=("--version",),
        needs_profile_dir=True,
    ),
}


def build_driver(engine: str) -> EngineDriver:
    if engine not in ENGINES:
        raise RuntimeError(f"unknown engine `{engine}`; known: {sorted(ENGINES)}")
    return ENGINES[engine]
