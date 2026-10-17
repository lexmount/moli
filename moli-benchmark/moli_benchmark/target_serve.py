from __future__ import annotations

import asyncio
import os
import shutil
import subprocess
import tempfile
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, ReservedPort, clear_proxy_env, reserve_port
from .sampling import ResourceSampler, _stat_read, error_evidence
from .serve import probe_url
from .synthetic_compare import target_enables_all_resource_fetch, target_metadata


class TargetServeError(RuntimeError):
    def __init__(self, message: str, *, evidence: dict | None = None):
        super().__init__(message)
        self.evidence = evidence


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
    root_identity: dict | None = None
    log_state: dict[str, dict] = field(default_factory=dict)


def _append_log(logs: list[str], line: str) -> None:
    logs.append(line)
    if len(logs) > 400:
        del logs[: len(logs) - 400]


def _read_available(stream: Any, label: str, logs: list[str], state: dict) -> None:
    state.update(
        eof=False, error=None, lines_read=0, started_monotonic_ns=time.monotonic_ns()
    )
    try:
        if stream is None:
            state["error"] = {"type": "MissingStream", "message": "pipe unavailable"}
            return
        while True:
            line = stream.readline()
            if not line:
                state["eof"] = True
                return
            state["lines_read"] += 1
            _append_log(
                logs, f"{label}: {line.decode('utf-8', errors='replace').rstrip()}"
            )
    except Exception as error:  # noqa: BLE001 -- Record failure and continue bounded cleanup.
        state["error"] = error_evidence(error)
    finally:
        state["finished_monotonic_ns"] = time.monotonic_ns()


def _terminate_process(process: subprocess.Popen[bytes]) -> dict:
    record = {"actions": [], "returncode_before": process.poll()}

    def action(name: str, function: Any) -> bool:
        item = {"action": name, "started_monotonic_ns": time.monotonic_ns()}
        record["actions"].append(item)
        try:
            item["result"] = function()
            return True
        except (OSError, subprocess.TimeoutExpired) as error:
            item["error"] = error_evidence(error)
            return False
        finally:
            item["finished_monotonic_ns"] = time.monotonic_ns()

    if (
        record["returncode_before"] is None
        and action("terminate", process.terminate)
        and not action("wait_after_terminate", lambda: process.wait(timeout=2))
        and action("kill", process.kill)
    ):
        action("wait_after_kill", lambda: process.wait(timeout=2))
    record.update(returncode=process.poll(), exited=process.poll() is not None)
    return record


def _root_state(handle: TargetServeHandle) -> dict:
    return {"returncode": handle.process.poll(), "stat": _stat_read(handle.process.pid)}


def _serve_command(
    target: str,
    binary: Path,
    port: int,
    temp_dir: Path | None,
    extra_args: tuple[str, ...] = (),
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
) -> TargetServeHandle:
    engine = target_metadata(target)["engine"]
    temp_dir = (
        Path(tempfile.mkdtemp(prefix=f"moli-benchmark-{target}-"))
        if engine == "chrome"
        else None
    )
    logs: list[str] = []
    reserved_port = reserve_port()
    try:
        port = reserved_port.port
        endpoint = f"http://127.0.0.1:{port}"
        command = _serve_command(target, binary, port, temp_dir, extra_args)
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
    handle = None
    stage = "create_handle"
    try:
        # Publish cleanup ownership before any post-Popen startup operation.
        handle = TargetServeHandle(
            target=target,
            process=process,
            endpoint=endpoint,
            command=command,
            logs=logs,
            sampler=None,
            log_threads=[],
            port_lease=reserved_port,
            temp_dir=temp_dir,
        )
        stage = "root_identity"
        handle.root_identity = _stat_read(process.pid)
        handle.log_state = {label: {} for label in ("stdout", "stderr")}
        stage = "create_log_threads"
        for stream, label in ((process.stdout, "stdout"), (process.stderr, "stderr")):
            handle.log_threads.append(
                threading.Thread(
                    target=_read_available,
                    args=(stream, label, logs, handle.log_state[label]),
                    daemon=True,
                )
            )
        for label, thread in zip(("stdout", "stderr"), handle.log_threads):
            stage = f"start_{label}_reader"
            thread.start()
        stage = "create_sampler"
        handle.sampler = (
            ResourceSampler(process.pid)
            if sample_interval_seconds is None
            else ResourceSampler(process.pid, interval_seconds=sample_interval_seconds)
        )
        stage = "start_sampler"
        handle.sampler.start()
        stage = "readiness"
        started = time.perf_counter()
        deadline = started + timeout_seconds
        version_url = endpoint + "/json/version"
        while time.perf_counter() < deadline:
            if process.poll() is not None:
                raise TargetServeProcessExit(
                    f"{target} CDP server exited early with {process.returncode}: "
                    f"{'; '.join(logs[-20:])}"
                )
            if probe_url(version_url):
                handle.ready_ms = (time.perf_counter() - started) * 1000.0
                return handle
            time.sleep(0.025)
        raise TargetServeReadinessTimeout(
            f"timed out waiting for {target} CDP server at {version_url}: "
            f"{'; '.join(logs[-20:])}"
        )
    except BaseException as error:
        # Returning a handle is not the boundary of resource ownership. Preserve
        # this original startup error even when individual cleanup actions fail.
        evidence = {"instrumentation_schema": 2, "events": [], "errors": []}
        if handle is not None:
            try:
                evidence = stop_target_serve(handle, include_resource_samples=True)
            except BaseException as cleanup_error:  # noqa: BLE001 -- Preserve the original startup exception.
                evidence["errors"].append(
                    {"action": "stop_target_serve", **error_evidence(cleanup_error)}
                )
        if handle is None or "termination" not in evidence:
            # Handle construction/cleanup itself failed. Use the same process
            # termination policy and still attempt the independently owned lease.
            def cleanup_action(name, function):
                event = {"action": name, "started_monotonic_ns": time.monotonic_ns()}
                evidence["events"].append(event)
                try:
                    result = function()
                    event["result"] = result
                    return result
                except BaseException as cleanup_error:  # noqa: BLE001 -- Keep original startup failure.
                    detail = {"action": name, **error_evidence(cleanup_error)}
                    event["error"] = detail
                    evidence["errors"].append(detail)
                    return None
                finally:
                    event["finished_monotonic_ns"] = time.monotonic_ns()

            evidence["termination"] = cleanup_action(
                "terminate_root", lambda: _terminate_process(process)
            )
            cleanup_action("release_port", reserved_port.close)
            if temp_dir is not None:
                cleanup_action("remove_profile", lambda: shutil.rmtree(temp_dir))
            evidence.update(
                cleanup_complete=False,
                stop_protocol_verified=False,
                sampler_sealed_before_termination=False,
                descendants_exited="unknown",
                returncode=cleanup_action("root_returncode", process.poll),
            )
        evidence.update(
            startup_stage=stage, startup_error=error_evidence(error), command=command
        )
        error.evidence = evidence
        raise


def stop_target_serve(
    handle: TargetServeHandle | None,
    *,
    include_resource_samples: bool = False,
) -> dict[str, Any]:
    if handle is None:
        return {}
    report = {"instrumentation_schema": 2, "events": [], "errors": []}

    def call(name, function):
        event = {"action": name, "started_monotonic_ns": time.monotonic_ns()}
        report["events"].append(event)
        try:
            result = function()
            event["result"] = result
            return result
        except Exception as error:  # noqa: BLE001 -- Record failure and continue bounded cleanup.
            evidence = {"action": name, **error_evidence(error)}
            report["errors"].append(evidence)
            event["error"] = evidence
            return None
        finally:
            event["finished_monotonic_ns"] = time.monotonic_ns()

    before = call("root_before_sampler_stop", lambda: _root_state(handle))
    sampler = handle.sampler
    report["sampler_constructed"] = sampler is not None
    resources = (
        call("stop_and_seal_sampler", sampler.stop) if sampler is not None else None
    )
    first_stop = resources
    after = call("root_after_sampler_stop", lambda: _root_state(handle))
    # This changes only teardown ordering. Every already-started sample is kept;
    # no periodic tail row is dropped or excused after an engine exit.
    termination = call("terminate_root", lambda: _terminate_process(handle.process))
    if sampler is not None and (
        resources is None or resources.get("thread_alive_after_stop")
    ):
        resources = call("sampler_join_after_process_cleanup", sampler.stop)
    resources = dict(resources or {})
    if include_resource_samples:
        resources["samples"] = (
            call("snapshot_committed_samples", sampler.export_samples)
            if sampler is not None
            else None
        )
    reader_states = []
    for label, thread in zip(("stdout", "stderr"), handle.log_threads):
        call(f"join_{label}", lambda thread=thread: thread.join(timeout=0.2))
        reader_states.append(
            {
                "stream": label,
                **handle.log_state.get(label, {}),
                "thread_alive_after_join": thread.is_alive(),
            }
        )
    call("release_port", handle.port_lease.close)
    if handle.temp_dir is not None:
        call("remove_profile", lambda: shutil.rmtree(handle.temp_dir))
    expected = (handle.root_identity or {}).get("identity")
    alive = expected is not None and all(
        point
        and point["returncode"] is None
        and point["stat"].get("identity") == expected
        and point["stat"].get("state") not in ("Z", "X")
        for point in (before, after)
    )
    sealed_before = bool(
        first_stop
        and first_stop.get("sealed") is True
        and first_stop.get("thread_alive_after_stop") is False
        and not first_stop.get("join_ever_timed_out")
    )
    readers_complete = len(reader_states) == 2 and all(
        state.get("eof") is True
        and state.get("error") is None
        and not state["thread_alive_after_join"]
        for state in reader_states
    )
    cleanup = bool(
        termination
        and termination["exited"]
        and resources.get("thread_alive_after_stop") is False
        and readers_complete
        and not report["errors"]
    )
    report.update(
        root_identity=handle.root_identity,
        root_observed_alive_at_stop_boundaries=alive,
        sampler_sealed_before_termination=sealed_before,
        stop_protocol_verified=alive
        and sealed_before
        and resources.get("observer_error") is None
        and not report["errors"],
        cleanup_complete=cleanup,
        descendants_exited="unknown",
        termination=termination,
        log_readers=reader_states,
        log_lines_dropped=max(
            0,
            sum(state.get("lines_read", 0) for state in reader_states)
            - len(handle.logs),
        ),
        returncode=handle.process.returncode,
        resources=resources,
        log_tail=list(handle.logs[-40:]),
    )
    return report


async def finish_target_serve_run(client, handle, record, *, key="server") -> None:
    """Preserve workload and close failures independently; always stop the target."""
    record["business_ok"] = bool(record.get("ok"))
    errors = record.setdefault("cleanup_errors", [])
    record.setdefault(key, {})
    try:
        if client is not None:
            try:
                await client.websocket.close()
            except asyncio.CancelledError as error:
                errors.append({"action": "websocket_close", **error_evidence(error)})
                raise
            except Exception as error:  # noqa: BLE001 -- Record failure and continue bounded cleanup.
                errors.append({"action": "websocket_close", **error_evidence(error)})
    finally:
        # Cancellation at the awaited close must not skip the synchronous
        # sampler seal/engine stop attempts or overwrite the workload evidence.
        try:
            if handle is not None:
                record[key] = stop_target_serve(handle, include_resource_samples=True)
            else:
                record.setdefault(key, {})
        except Exception as error:  # noqa: BLE001 -- Record failure and continue bounded cleanup.
            errors.append({"action": "stop_target_serve", **error_evidence(error)})
        stopped = record[key]
        resources = stopped.get("resources", {})
        record["instrumentation_ok"] = bool(
            stopped.get("stop_protocol_verified")
            and stopped.get("cleanup_complete")
            and resources.get("pss_complete")
            and resources.get("resource_reads_complete")
            and not errors
            and record.get("checkpoint_reads_complete", False)
            and record.get("cpu_evidence_complete", False)
            and record.get("affinity_evidence_complete", False)
        )
        record["ok"] = record["business_ok"] and record["instrumentation_ok"]
        # Caller saves the complete failed sample before stopping later cohorts.
        record["safe_to_continue"] = bool(
            stopped.get("cleanup_complete")
            and stopped.get("stop_protocol_verified")
            and not resources.get("join_ever_timed_out")
            and not resources.get("observer_error")
            and not errors
        )


def stop_fixture_server(server, thread) -> dict:
    record = {"actions": [], "errors": []}
    for name, action in (
        ("shutdown", server.shutdown),
        ("server_close", server.server_close),
        ("join", lambda: thread.join(timeout=2)),
    ):
        event = {"action": name, "started_monotonic_ns": time.monotonic_ns()}
        record["actions"].append(event)
        try:
            action()
        except Exception as error:  # noqa: BLE001 -- Record failure and continue bounded cleanup.
            record["errors"].append({"action": name, **error_evidence(error)})
        finally:
            event["finished_monotonic_ns"] = time.monotonic_ns()
    record["thread_alive_after_join"] = thread.is_alive()
    record["complete"] = not record["thread_alive_after_join"] and not record["errors"]
    return record
