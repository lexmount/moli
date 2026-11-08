from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
import time
from dataclasses import dataclass, replace
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Sequence

from .config import REPO_ROOT, TRACE_BACKGROUND_PROCESS_ENV, clear_proxy_env
from .process import INHERIT_PROCESS_GROUP_ENV, terminate_process_tree
from .runner import SmokeGroup, group_listing, resolve_group_selection


_DEFAULT_JOBS = 1
_DEFAULT_GROUP_TIMEOUT_SECONDS = 120.0


@dataclass(frozen=True)
class WorkerJob:
    group: SmokeGroup
    attempt: int
    repeat: int
    endpoint: str | None = None
    browser: str | None = None
    chromium_bin: str | None = None

    @property
    def label(self) -> str:
        if self.repeat == 1:
            return self.group.name
        return f"{self.group.name}#{self.attempt}"

    @property
    def file_stem(self) -> str:
        if self.repeat == 1:
            return self.group.name
        return f"{self.group.name}-run-{self.attempt:02d}"


@dataclass(frozen=True)
class WorkerOutcome:
    job: WorkerJob
    status: str
    duration_seconds: float
    exit_code: int | None
    log_path: Path
    result_path: Path
    scenario_count: int
    endpoint: str | None = None
    error: str | None = None
    browser: dict[str, Any] | None = None
    previous_attempts: tuple[WorkerOutcome, ...] = ()
    chromium_bin: str | None = None

    @property
    def passed(self) -> bool:
        return self.status == "passed"

    @property
    def flaky(self) -> bool:
        return self.passed and bool(self.previous_attempts)

    def summary(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "group": self.job.group.name,
            "attempt": self.job.attempt,
            "status": self.status,
            "durationSeconds": round(
                self.duration_seconds + sum(attempt.duration_seconds for attempt in self.previous_attempts), 3
            ),
            "exitCode": self.exit_code,
            "scenarioCount": self.scenario_count,
            "log": str(self.log_path),
            "result": str(self.result_path),
            "flaky": self.flaky,
            "retriesUsed": len(self.previous_attempts),
        }
        if self.previous_attempts:
            payload["executions"] = [
                attempt.summary()
                for attempt in (*self.previous_attempts, replace(self, previous_attempts=()))
            ]
        if self.error:
            payload["error"] = self.error
        if self.endpoint:
            payload["endpoint"] = self.endpoint
        if self.browser:
            payload["browser"] = self.browser
        if self.job.browser == "chromium":
            payload["chromiumBin"] = self.chromium_bin or self.job.chromium_bin
        return payload


def _positive_int(raw: str) -> int:
    value = int(raw)
    if value <= 0:
        raise argparse.ArgumentTypeError("value must be positive")
    return value


def _positive_float(raw: str) -> float:
    value = float(raw)
    if not 0 < value < float("inf"):
        raise argparse.ArgumentTypeError("value must be a finite positive number")
    return value


def _nonnegative_int(raw: str) -> int:
    value = int(raw)
    if value < 0:
        raise argparse.ArgumentTypeError("value must be nonnegative")
    return value


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run CDP smoke groups in isolated worker processes."
    )
    parser.add_argument(
        "--group",
        action="append",
        default=[],
        help=(
            "Select a group. May be repeated or comma-separated. Defaults to "
            "every repository-managed group."
        ),
    )
    parser.add_argument(
        "--jobs",
        type=_positive_int,
        default=_DEFAULT_JOBS,
        help="Maximum isolated workers to run concurrently (default: 1).",
    )
    parser.add_argument(
        "--timeout",
        type=_positive_float,
        default=_DEFAULT_GROUP_TIMEOUT_SECONDS,
        help="Wall-clock timeout for each worker in seconds (default: 120).",
    )
    parser.add_argument(
        "--repeat",
        type=_positive_int,
        default=1,
        help="Run every selected group this many times; every repetition must pass.",
    )
    parser.add_argument(
        "--retries",
        type=_nonnegative_int,
        default=0,
        help="Retry each failed Chromium group in a fresh worker/browser (default: 0).",
    )
    parser.add_argument(
        "--fail-fast",
        action="store_true",
        help="Do not start queued workers after the first failure.",
    )
    parser.add_argument(
        "--strict-full-suite",
        action="store_true",
        help="Require all default groups in owned browsers, without filters or fail-fast.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        help="Directory for per-worker logs/results and summary.json.",
    )
    parser.add_argument(
        "--endpoint",
        help=(
            "Use an existing HTTP CDP endpoint. Workers remain separate, but "
            "the external browser process is shared."
        ),
    )
    parser.add_argument(
        "--list-groups",
        action="store_true",
        help="List available groups as JSON and exit.",
    )
    parser.add_argument("--browser", choices=("moli", "chromium"), help="Managed browser engine (default: moli).")
    parser.add_argument("--chromium-bin", help="Override the locked Playwright Chromium executable.")
    parser.add_argument(
        "--chromium-bin-for-group", action="append", default=[], metavar="GROUP=PATH",
        help="Override Chromium for one group without changing group selection. May be repeated.",
    )
    args = parser.parse_args(argv)
    overrides: dict[str, str] = {}
    if args.chromium_bin_for_group and (args.browser != "chromium" or args.endpoint):
        parser.error("--chromium-bin-for-group requires managed --browser chromium")
    known_groups = {group["name"] for group in group_listing()}
    for raw in args.chromium_bin_for_group:
        group, separator, binary = raw.partition("=")
        if not separator or not group or not binary.strip():
            parser.error("--chromium-bin-for-group requires GROUP=PATH with a nonempty path")
        if group not in known_groups:
            parser.error(f"--chromium-bin-for-group: unknown group {group!r}")
        if group in overrides:
            parser.error(f"--chromium-bin-for-group: duplicate group {group!r}")
        overrides[group] = binary
    args.chromium_bin_for_group = overrides
    return args


def _default_output_dir() -> Path:
    timestamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    return REPO_ROOT / "target" / "smoke" / "cdp" / f"{timestamp}-{os.getpid()}"


def _worker_argv(job: WorkerJob, result_path: Path) -> list[str]:
    argv = [
        sys.executable,
        "-m",
        "moli_cdp_smoke.worker",
        "--group",
        job.group.name,
        "--result",
        str(result_path),
    ]
    if job.endpoint:
        argv.extend(("--endpoint", job.endpoint))
    if job.browser:
        argv.extend(("--browser", job.browser))
    if job.chromium_bin:
        argv.extend(("--chromium-bin", job.chromium_bin))
    return argv


def _worker_environment() -> dict[str, str]:
    env = clear_proxy_env(os.environ)
    env[INHERIT_PROCESS_GROUP_ENV] = "1"
    # The supervisor already persists worker stdout/stderr in the artifact
    # directory. Stream each worker's browser logs into that same file so a
    # process-level timeout does not destroy the only copy of the renderer
    # diagnostics held in the worker's memory.
    env[TRACE_BACKGROUND_PROCESS_ENV] = "1"
    env["PYTHONUNBUFFERED"] = "1"
    return env


def _read_worker_payload(result_path: Path) -> tuple[dict[str, Any] | None, str | None]:
    try:
        payload = json.loads(result_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None, "worker did not publish a result file"
    except (OSError, json.JSONDecodeError) as error:
        return None, f"worker result is unreadable: {error}"
    if not isinstance(payload, dict):
        return None, "worker result root is not an object"
    return payload, None


def _coverage_error(job: WorkerJob, payload: dict[str, Any]) -> str | None:
    results = payload.get("results")
    if not isinstance(results, list) or not results:
        return "worker executed no scenarios"
    for result in results:
        if (
            not isinstance(result, dict)
            or result.get("ok") is not True
            or result.get("applicable") is False
            or "not_applicable" in str(result.get("name", ""))
        ):
            return f"worker did not validate scenario: {result}"
    if job.browser:
        identity = payload.get("browser") or {}
        if identity.get("engine") != job.browser:
            return f"worker browser mismatch: requested {job.browser}, got {identity}"
        launches = payload.get("browserLaunches") or []
        if not job.endpoint and not launches:
            return "managed worker published no browser process evidence"
        if any(launch.get("engine") != job.browser for launch in launches):
            return f"worker mixed browser engines: {launches}"
        if any((launch.get("browser") or {}).get("engine") != job.browser for launch in launches):
            return "worker did not verify every launched browser's discovery identity"
    return None


async def _wait_for_worker(
    process: asyncio.subprocess.Process,
    timeout_seconds: float,
) -> tuple[int | None, bool]:
    wait_task = asyncio.create_task(process.wait())
    try:
        done, _ = await asyncio.wait((wait_task,), timeout=timeout_seconds)
    except BaseException:
        await terminate_process_tree(
            process,
            terminate_timeout_seconds=2,
            kill_timeout_seconds=2,
        )
        raise
    if wait_task in done:
        exit_code = wait_task.result()
        # A successful worker should have cleaned up its children. Clear any
        # process which escaped worker teardown before the process group can be
        # reused by another isolated job.
        await terminate_process_tree(
            process,
            terminate_timeout_seconds=0,
            kill_timeout_seconds=1,
        )
        return exit_code, False

    await terminate_process_tree(
        process,
        terminate_timeout_seconds=2,
        kill_timeout_seconds=2,
    )
    return process.returncode, True


async def _run_worker_job(
    job: WorkerJob,
    output_dir: Path,
    timeout_seconds: float,
) -> WorkerOutcome:
    log_path = output_dir / f"{job.file_stem}.log"
    result_path = output_dir / f"{job.file_stem}.json"
    result_path.unlink(missing_ok=True)
    started_at = time.monotonic()
    print(f"[moli-cdp-supervisor] START {job.label}", file=sys.stderr, flush=True)
    with log_path.open("wb", buffering=0) as log_stream:
        environment = _worker_environment()
        environment["MOLI_SMOKE_WORKER_TIMEOUT_SECONDS"] = str(timeout_seconds)
        process = await asyncio.create_subprocess_exec(
            *_worker_argv(job, result_path),
            cwd=str(REPO_ROOT),
            env=environment,
            stdout=log_stream,
            stderr=asyncio.subprocess.STDOUT,
            start_new_session=os.name == "posix",
        )
        exit_code, timed_out = await _wait_for_worker(process, timeout_seconds)

    duration = time.monotonic() - started_at
    payload, payload_error = _read_worker_payload(result_path)
    scenario_count = 0
    endpoint = None
    if payload is not None and isinstance(payload.get("results"), list):
        scenario_count = len(payload["results"])
    if payload is not None and isinstance(payload.get("endpoint"), str):
        endpoint = payload["endpoint"]
    chromium_bin = None
    for launch in (payload.get("browserLaunches") or []) if payload else []:
        command = launch.get("command")
        if launch.get("engine") == "chromium" and isinstance(command, list) and command:
            chromium_bin = command[0]
            break

    if timed_out:
        status = "timed_out"
        error = f"worker exceeded {timeout_seconds:g}s"
    elif payload_error:
        status = "crashed"
        error = payload_error
    elif payload is None:
        status = "crashed"
        error = "worker result is unavailable"
    elif payload.get("group") != job.group.name:
        status = "crashed"
        error = f"worker reported the wrong group: {payload.get('group')!r}"
    elif exit_code == 0 and payload.get("ok") is True:
        error = _coverage_error(job, payload)
        status = "failed" if error else "passed"
    else:
        status = "failed"
        worker_error = payload.get("error")
        error = str(worker_error) if worker_error else (
            _coverage_error(job, payload) or f"worker exited with {exit_code}"
        )

    outcome = WorkerOutcome(
        job=job,
        status=status,
        duration_seconds=duration,
        exit_code=exit_code,
        log_path=log_path,
        result_path=result_path,
        scenario_count=scenario_count,
        endpoint=endpoint,
        error=error,
        browser=payload.get("browser") if payload else None,
        chromium_bin=chromium_bin,
    )
    print(
        f"[moli-cdp-supervisor] {status.upper()} {job.label} "
        f"elapsed={duration:.3f}s",
        file=sys.stderr,
        flush=True,
    )
    return outcome


def _log_tail(path: Path, *, lines: int = 40) -> str:
    try:
        content = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as error:
        return f"<failed to read log: {error}>"
    return "\n".join(content[-lines:])


def _write_json_atomic(path: Path, payload: dict[str, Any]) -> None:
    rendered = json.dumps(payload, indent=2, ensure_ascii=False) + "\n"
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(rendered, encoding="utf-8")
    os.replace(temporary, path)


def _validate_full_suite(args: argparse.Namespace) -> None:
    if not args.strict_full_suite:
        return
    forbidden = [
        name for name, enabled in [
            ("--group", args.group), ("--endpoint", args.endpoint),
            ("--fail-fast", args.fail_fast), ("--list-groups", args.list_groups),
            *[(name, os.environ.get(name)) for name in (
                "MOLI_SMOKE_GROUPS", "MOLI_MULTI_PAGE_CASES", "MOLI_MULTI_PAGE_SHARD",
            )],
        ] if enabled
    ]
    if forbidden:
        raise RuntimeError("strict full-suite mode forbids " + ", ".join(forbidden))


async def run_supervisor(args: argparse.Namespace) -> tuple[int, dict[str, Any]]:
    _validate_full_suite(args)
    if args.retries and (args.browser != "chromium" or args.endpoint):
        raise RuntimeError("--retries requires managed --browser chromium")
    selection = resolve_group_selection(args.group)
    output_dir = (args.output_dir or _default_output_dir()).resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    jobs = [
        WorkerJob(
            group=group,
            attempt=attempt,
            repeat=args.repeat,
            endpoint=args.endpoint.rstrip("/") if args.endpoint else None,
            browser=args.browser or ("moli" if args.endpoint is None else None),
            chromium_bin=args.chromium_bin_for_group.get(group.name, args.chromium_bin),
        )
        for group in selection.groups
        for attempt in range(1, args.repeat + 1)
    ]
    fixed_port = os.environ.get("MOLI_CDP_PORT")
    if (
        args.endpoint is None
        and fixed_port is not None
        and args.jobs > 1
        and len(jobs) > 1
    ):
        raise RuntimeError(
            "MOLI_CDP_PORT cannot be shared by parallel isolated workers; "
            "unset it to use atomic OS-assigned ports or run with --jobs 1"
        )
    semaphore = asyncio.Semaphore(args.jobs)
    stop_starting = asyncio.Event()

    async def execute(job: WorkerJob) -> WorkerOutcome:
        async with semaphore:
            if args.fail_fast and stop_starting.is_set():
                return WorkerOutcome(
                    job=job,
                    status="skipped",
                    duration_seconds=0,
                    exit_code=None,
                    log_path=output_dir / f"{job.file_stem}.log",
                    result_path=output_dir / f"{job.file_stem}.json",
                    scenario_count=0,
                    error="not started after an earlier failure",
                )
            attempts: list[WorkerOutcome] = []
            for retry in range(args.retries + 1):
                attempt_dir = output_dir
                if retry:
                    attempt_dir = output_dir / "retries" / job.file_stem / f"retry-{retry:02d}"
                    print(
                        f"[moli-cdp-supervisor] RETRY {job.label} {retry}/{args.retries}; "
                        f"previous {attempts[-1].status}, log={attempts[-1].log_path}",
                        file=sys.stderr,
                        flush=True,
                    )
                attempt_dir.mkdir(parents=True, exist_ok=True)
                try:
                    outcome = await _run_worker_job(job, attempt_dir, args.timeout)
                except Exception as error:
                    outcome = WorkerOutcome(
                        job=job,
                        status="crashed",
                        duration_seconds=0,
                        exit_code=None,
                        log_path=attempt_dir / f"{job.file_stem}.log",
                        result_path=attempt_dir / f"{job.file_stem}.json",
                        scenario_count=0,
                        error=f"failed to start or supervise worker: {error}",
                    )
                attempts.append(outcome)
                if outcome.passed:
                    break
            outcome = replace(outcome, previous_attempts=tuple(attempts[:-1]))
            if outcome.flaky:
                print(
                    f"[moli-cdp-supervisor] FLAKY {job.label} passed after {len(attempts) - 1} retries",
                    file=sys.stderr,
                    flush=True,
                )
            if not outcome.passed:
                stop_starting.set()
            return outcome

    outcomes = await asyncio.gather(*(execute(job) for job in jobs))
    ok = all(outcome.passed for outcome in outcomes)
    summary = {
        "ok": ok,
        "jobs": args.jobs,
        "timeoutSeconds": args.timeout,
        "repeat": args.repeat,
        "retries": args.retries,
        "flakyGroups": sum(outcome.flaky for outcome in outcomes),
        "strictFullSuite": args.strict_full_suite,
        "externalEndpoint": args.endpoint,
        "browser": args.browser or ("external" if args.endpoint else "moli"),
        "outputDirectory": str(output_dir),
        "groups": [outcome.summary() for outcome in outcomes],
    }
    _write_json_atomic(output_dir / "summary.json", summary)

    for outcome in outcomes:
        if outcome.passed or outcome.status == "skipped":
            continue
        if outcome.error:
            print(
                f"\n--- {outcome.job.label} ({outcome.status}) error ---\n{outcome.error}",
                file=sys.stderr,
            )
        print(
            f"\n--- {outcome.job.label} ({outcome.status}) log tail ---\n"
            f"{_log_tail(outcome.log_path)}",
            file=sys.stderr,
        )
    passed = sum(outcome.passed for outcome in outcomes)
    print(
        f"[moli-cdp-supervisor] summary {passed}/{len(outcomes)} passed; "
        f"{summary['flakyGroups']} flaky; "
        f"artifacts={output_dir}",
        file=sys.stderr,
    )
    return (0 if ok else 1), summary


async def async_main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    if args.list_groups:
        _validate_full_suite(args)
        print(json.dumps({"groups": group_listing()}, indent=2, ensure_ascii=False))
        return 0
    exit_code, _ = await run_supervisor(args)
    return exit_code


def main(argv: Sequence[str] | None = None) -> None:
    raise SystemExit(asyncio.run(async_main(argv)))
