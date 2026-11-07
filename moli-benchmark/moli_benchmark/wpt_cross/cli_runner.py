"""Per-engine WPT case runner using each engine's ``fetch`` CLI plus the
in-page report bridge.

The fixture server's ``/resources/testharnessreport.js`` stores the final
testharness payload in a hidden DOM node and POSTs bounded payloads back to the
fixture server as a fallback. This runner reads the serialized DOM from a
successful CLI process first, then consults the fixture's
:class:`ResultsStore` when stdout has no final payload.

Compared to the CDP runner this is:
  * Process-per-case — no cross-case state leakage, no relaunch logic.
  * No CDP keepalive / awaitPromise / per-engine websocket quirks.
  * The fairness cost is real but small: each engine uses its own native
    fetch CLI, which is the most "natural" entry-point each project ships.

Engines whose CLI does not actually execute JavaScript (currently obscura)
expose ``cli_fetch_command = None`` and must use the CDP runner instead.
"""

from __future__ import annotations

import json
import multiprocessing
import os
import subprocess
import sys
import time
from concurrent.futures import Future, ProcessPoolExecutor, as_completed
from dataclasses import dataclass
from html.parser import HTMLParser
from pathlib import Path

from ..config import clear_proxy_env
from ..versions import sha256_file
from .engine import EngineDriver, build_driver
from .runner import CaseResult, EngineRunResult, classify_payload
from .runner import FINAL_PAYLOAD_SOURCES
from .server import WptFixtureServer

CaseRun = tuple[str, str] | tuple[str, str, float] | tuple[str, str, float, float]
MAX_RECORDED_STDERR_CHARS = 2000
BENCH_WPT_PAYLOAD_ELEMENT_ID = "__bench_wpt_payload"
WPT_TEMPLATE_HOSTS = (
    "localhost",
    "www.localhost",
    "www1.localhost",
    "www2.localhost",
    "alt.localhost",
    "www.alt.localhost",
)
MOLI_WPT_USER_AGENT = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36"
)


@dataclass(frozen=True)
class _CliSubprocessResult:
    duration_ms: float
    proc_error: str | None
    proc_returncode: int | None
    proc_stderr: str
    proc_stdout: bytes
    wait_script_timeout: bool


@dataclass(frozen=True)
class _CliCaseWorkerInput:
    engine: str
    binary: str
    wpt_root: str
    case_path: str
    external: bool
    timeout_seconds: float
    harness_timeout_multiplier: float
    env: dict[str, str]
    process_timeout_margin_seconds: float
    payload_grace_seconds: float
    successful_process_payload_grace_seconds: float


def _run_cli_subprocess(
    argv: list[str],
    env: dict[str, str],
    proc_timeout: float,
) -> _CliSubprocessResult:
    started = time.perf_counter()
    proc_error: str | None = None
    proc_returncode: int | None = None
    proc_stderr = ""
    proc_stdout = b""
    wait_script_timeout = False
    try:
        cp = subprocess.run(
            argv,
            env=env,
            capture_output=True,
            timeout=proc_timeout,
            check=False,
        )
        proc_returncode = cp.returncode
        proc_stdout = cp.stdout or b""
        proc_stderr = cp.stderr.decode("utf-8", errors="replace")
        wait_script_timeout = "timed out waiting for script to become truthy" in proc_stderr
    except subprocess.TimeoutExpired:
        proc_error = f"engine subprocess wall timeout after {proc_timeout:.1f}s"
    except OSError as error:
        proc_error = f"engine subprocess failed: {error}"

    return _CliSubprocessResult(
        duration_ms=(time.perf_counter() - started) * 1000.0,
        proc_error=proc_error,
        proc_returncode=proc_returncode,
        proc_stderr=proc_stderr,
        proc_stdout=proc_stdout,
        wait_script_timeout=wait_script_timeout,
    )


def _bridge_key_for_case(case_path: str) -> str:
    return case_path if case_path.startswith("/") else "/" + case_path


class _BenchPayloadParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self._depth = 0
        self.chunks: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attrs_dict = dict(attrs)
        if attrs_dict.get("id") == BENCH_WPT_PAYLOAD_ELEMENT_ID:
            self._depth += 1
        elif self._depth:
            self._depth += 1

    def handle_endtag(self, tag: str) -> None:
        if self._depth:
            self._depth -= 1

    def handle_data(self, data: str) -> None:
        if self._depth:
            self.chunks.append(data)


def _payload_from_stdout_html(stdout: bytes) -> dict | None:
    if not stdout:
        return None
    text = stdout.decode("utf-8", errors="replace")
    parser = _BenchPayloadParser()
    try:
        parser.feed(text)
    except Exception:
        return None
    if not parser.chunks:
        return None
    raw = "".join(parser.chunks).strip()
    if not raw:
        return None
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError:
        return None
    return payload if isinstance(payload, dict) else None


def _stderr_tail(stderr: str) -> str:
    stderr = stderr.strip()
    if len(stderr) <= MAX_RECORDED_STDERR_CHARS:
        return stderr
    return stderr[-MAX_RECORDED_STDERR_CHARS:]


def _nonzero_exit_status(returncode: int, stderr: str) -> str:
    if returncode < 0:
        return "crash"
    if "panicked at" in stderr or ("thread '" in stderr and " panicked at " in stderr):
        return "crash"
    return "error"


def _payload_grace_for_process_result(
    *,
    proc_error: str | None,
    proc_returncode: int | None,
    payload_grace_seconds: float,
    successful_process_payload_grace_seconds: float,
) -> float:
    if proc_error is None and proc_returncode == 0:
        return successful_process_payload_grace_seconds
    return payload_grace_seconds


def _curl_resolve_address(address: str) -> str:
    if ":" in address and not (address.startswith("[") and address.endswith("]")):
        return f"[{address}]"
    return address


def _moli_fixture_host_resolve_args(fixture_server: WptFixtureServer) -> list[str]:
    """Return curl resolve overrides for WPT template hostnames.

    Non-secure Moli CLI WPT cases normally run against the fixture
    server's external IPv6 address so localhost is not treated as a trustworthy
    origin. WPT templates still contain origin-distinguishing hostnames such as
    alt.localhost and www.localhost; those names must stay in the URL for
    browser-origin semantics, but libcurl needs to connect them back to the
    external fixture listener.
    """

    external_host = getattr(fixture_server, "external_host", None)
    if not external_host:
        return []
    ports = [
        getattr(fixture_server, "external_port", None),
        getattr(fixture_server, "external_alternate_port", None),
        getattr(fixture_server, "external_remote_port", None),
    ]
    address = _curl_resolve_address(str(external_host))
    args: list[str] = []
    seen: set[tuple[str, int]] = set()
    for port in ports:
        if port is None:
            continue
        port = int(port)
        for host in WPT_TEMPLATE_HOSTS:
            key = (host, port)
            if key in seen:
                continue
            seen.add(key)
            args.extend(["--http-host-resolve", f"{host}:{port}:{address}"])
    return args


def _classify_cli_case_result(
    *,
    case_path: str,
    url: str,
    bridge_key: str,
    fixture_server: WptFixtureServer,
    subprocess_result: _CliSubprocessResult,
    payload_grace_seconds: float,
    successful_process_payload_grace_seconds: float,
) -> CaseResult:
    payload_grace = _payload_grace_for_process_result(
        proc_error=subprocess_result.proc_error,
        proc_returncode=subprocess_result.proc_returncode,
        payload_grace_seconds=payload_grace_seconds,
        successful_process_payload_grace_seconds=successful_process_payload_grace_seconds,
    )
    payload = _payload_from_stdout_html(subprocess_result.proc_stdout)
    if payload is None:
        payload = fixture_server.results.wait_for_final(bridge_key, timeout=payload_grace)
        if payload is None:
            payload = fixture_server.results.get(bridge_key)

    bridge_installed = payload is not None
    case_status_error = subprocess_result.proc_error
    if isinstance(payload, dict) and payload.get("source") in FINAL_PAYLOAD_SOURCES:
        case_status_error = None
    if payload is None and subprocess_result.wait_script_timeout:
        case_status_error = "testharness did not complete within timeout"
    elif (
        payload is None
        and subprocess_result.proc_error is None
        and subprocess_result.proc_returncode != 0
    ):
        case_status_error = (
            f"engine exited code={subprocess_result.proc_returncode} with no harness payload"
        )
        if subprocess_result.proc_stderr.strip():
            case_status_error += f"; stderr tail: {_stderr_tail(subprocess_result.proc_stderr)}"

    case_result = classify_payload(
        payload=payload,
        case_path=case_path,
        url=url,
        duration_ms=subprocess_result.duration_ms,
        bridge_installed=bridge_installed,
        error=case_status_error,
    )
    if (
        payload is None
        and subprocess_result.proc_returncode is not None
        and subprocess_result.proc_returncode != 0
        and subprocess_result.proc_error is None
        and not subprocess_result.wait_script_timeout
    ):
        case_result.status = _nonzero_exit_status(
            subprocess_result.proc_returncode,
            subprocess_result.proc_stderr,
        )
    return case_result


def _run_cli_case_worker(job: _CliCaseWorkerInput) -> CaseResult:
    driver = build_driver(job.engine)
    if driver.cli_fetch_command is None:
        return CaseResult(
            case_path=job.case_path,
            url="",
            status="error",
            duration_ms=0.0,
            error=f"engine `{driver.name}` has no cli_fetch_command; use the CDP runner",
        )

    binary = Path(job.binary)
    with WptFixtureServer(Path(job.wpt_root), primary_hostname="web-platform.localhost") as server:
        if getattr(server, "tls", None) is not None:
            driver = driver.with_fixture_tls(server.tls)
        server.set_harness_timeout_multipliers(
            {job.case_path: job.harness_timeout_multiplier},
            default_multiplier=job.harness_timeout_multiplier,
        )
        url = server.url_for_case(job.case_path, external=job.external)
        bridge_key = _bridge_key_for_case(job.case_path)
        server.results.clear(bridge_key)
        argv = driver.cli_fetch_command(binary, url, job.timeout_seconds)
        if driver.name == "moli":
            argv.extend(["--user-agent", MOLI_WPT_USER_AGENT])
            argv.extend(_moli_fixture_host_resolve_args(server))
        subprocess_result = _run_cli_subprocess(
            argv,
            job.env,
            job.timeout_seconds + job.process_timeout_margin_seconds,
        )
        return _classify_cli_case_result(
            case_path=job.case_path,
            url=url,
            bridge_key=bridge_key,
            fixture_server=server,
            subprocess_result=subprocess_result,
            payload_grace_seconds=job.payload_grace_seconds,
            successful_process_payload_grace_seconds=job.successful_process_payload_grace_seconds,
        )


def run_engine_on_cases_cli(
    *,
    driver: EngineDriver,
    fixture_server: WptFixtureServer,
    cases: list[CaseRun],
    execution_cases: list[CaseRun] | None = None,
    binary_override: str | None = None,
    case_timeout_seconds: float = 8.0,
    process_timeout_margin_seconds: float = 4.0,
    payload_grace_seconds: float = 2.0,
    successful_process_payload_grace_seconds: float = 8.0,
    parallelism: int = 1,
    progress_every: int = 25,
) -> EngineRunResult:
    """Run all ``cases`` through ``driver`` in CLI report-bridge mode.

    ``cases`` is ``[(case_path, url)]`` or
    ``[(case_path, url, timeout_seconds)]`` where ``case_path`` matches the
    bridge payload's ``case_path`` (i.e. ``location.pathname`` = leading-slash
    WPT-relative path).

    ``case_timeout_seconds`` is passed to the engine's CLI as its in-page wait
    deadline; the runner adds ``process_timeout_margin_seconds`` on top for
    the subprocess wall-clock. ``execution_cases`` may provide a different
    submission order, while ``cases`` remains the canonical result order. Each
    case runs entirely inside a worker process, including its own fixture server
    and harness result collection. This keeps the parent process out of
    per-case execution and avoids Python threads in the CLI runner.
    """

    if driver.cli_fetch_command is None:
        raise RuntimeError(
            f"engine `{driver.name}` has no cli_fetch_command; use the CDP runner"
        )

    binary = driver.resolve_binary(binary_override)
    binary_version = None
    try:
        completed = subprocess.run(
            [str(binary), *driver.version_args],
            capture_output=True, text=True, timeout=5, check=False,
        )
        out = (completed.stdout or completed.stderr).strip()
        binary_version = out.splitlines()[0] if out else None
    except (OSError, subprocess.SubprocessError):
        pass

    result = EngineRunResult(
        engine=driver.name,
        binary=str(binary),
        binary_sha256=sha256_file(binary),
        binary_version=binary_version,
        endpoint=f"cli:{binary}",
        ready_ms=None,
    )
    result.shutdown_info = {"mode": "cli", "cases_run": 0, "scheduler": "process-pool"}

    env = clear_proxy_env(os.environ)
    env.update(driver.extra_env)
    # Belt-and-braces NO_PROXY for the local fixture endpoints created by each
    # worker process.
    no_proxy_extra = "127.0.0.1,localhost,::1"
    if fixture_server.external_host:
        no_proxy_extra += "," + fixture_server.external_host
    env["NO_PROXY"] = no_proxy_extra
    env["no_proxy"] = no_proxy_extra

    def case_parts(case: CaseRun) -> tuple[str, bool, float, float]:
        external_base_url = getattr(fixture_server, "external_base_url", None)
        external = bool(external_base_url and case[1].startswith(external_base_url))
        if len(case) == 4:
            return case[0], external, case[2], case[3]
        if len(case) == 3:
            timeout_seconds = case[2]
            return (case[0], external, timeout_seconds, 1.0)
        return (
            case[0],
            external,
            case_timeout_seconds,
            1.0,
        )

    results_by_path: dict[str, CaseResult] = {}
    completed_count = 0
    submission_cases = execution_cases if execution_cases is not None else cases
    total = len(cases)

    max_workers = max(1, parallelism)
    # The parent may already have fixture-server threads; spawn avoids forking
    # that multi-threaded state into workers.
    process_context = multiprocessing.get_context("spawn")
    with ProcessPoolExecutor(max_workers=max_workers, mp_context=process_context) as pool:
        future_to_path: dict[Future[CaseResult], str] = {}
        for case in submission_cases:
            case_path, external, timeout_seconds, harness_timeout_multiplier = case_parts(case)
            job = _CliCaseWorkerInput(
                engine=driver.name,
                binary=str(binary),
                wpt_root=str(fixture_server.wpt_root),
                case_path=case_path,
                external=external,
                timeout_seconds=timeout_seconds,
                harness_timeout_multiplier=harness_timeout_multiplier,
                env=env,
                process_timeout_margin_seconds=process_timeout_margin_seconds,
                payload_grace_seconds=payload_grace_seconds,
                successful_process_payload_grace_seconds=successful_process_payload_grace_seconds,
            )
            future_to_path[pool.submit(_run_cli_case_worker, job)] = case_path

        for fut in as_completed(future_to_path):
            cp = future_to_path[fut]
            try:
                results_by_path[cp] = fut.result()
            except Exception as exc:  # pragma: no cover - defensive
                results_by_path[cp] = CaseResult(
                    case_path=cp,
                    url="",
                    status="error",
                    duration_ms=0.0,
                    error=f"runner exception: {exc!r}",
                )
            completed_count += 1
            if progress_every and completed_count % progress_every == 0:
                print(
                    f"[wpt-cross] {driver.name}: {completed_count}/{total}",
                    file=sys.stderr,
                    flush=True,
                )

    # Preserve original case order in result.cases.
    for case in cases:
        case_path, _, _, _ = case_parts(case)
        if case_path in results_by_path:
            result.cases.append(results_by_path[case_path])
    result.shutdown_info["cases_run"] = len(result.cases)
    result.shutdown_info["parallelism"] = parallelism

    return result
