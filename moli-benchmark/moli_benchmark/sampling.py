from __future__ import annotations

import copy
import hashlib
import os
import subprocess
import threading
import time
import traceback
from dataclasses import dataclass, field
from itertools import pairwise
from pathlib import Path
from typing import Any

INSTRUMENTATION_SCHEMA = 2


def error_evidence(error: BaseException) -> dict[str, Any]:
    result = {"type": type(error).__name__, "message": str(error)}
    if isinstance(error, OSError):
        result.update(errno=error.errno, filename=error.filename)
    return result


def _text_read(path: Path) -> dict[str, Any]:
    record = {"path": str(path), "started_monotonic_ns": time.monotonic_ns()}
    try:
        record.update(
            status="ok", raw=path.read_text(encoding="utf-8", errors="replace")
        )
    except OSError as error:
        record.update(status="os_error", error=error_evidence(error), raw=None)
    record["finished_monotonic_ns"] = time.monotonic_ns()
    return record


def _stat_read(pid: int, tid: int | None = None) -> dict[str, Any]:
    path = Path(f"/proc/{pid}/stat" if tid is None else f"/proc/{pid}/task/{tid}/stat")
    record = _text_read(path)
    record["identity"] = None
    if record["status"] == "ok":
        try:
            raw = record["raw"]
            close = raw.rfind(")")
            fields = raw[close + 2 :].split()
            observed_id = int(raw[: raw.index("(")].strip())
            if close < 0 or observed_id != (pid if tid is None else tid):
                raise ValueError("proc stat ID/format mismatch")
            user, system, start = (int(fields[index]) for index in (11, 12, 19))
            if min(user, system, start) < 0:
                raise ValueError("negative proc stat counter")
            record.update(
                pid=pid,
                tid=tid,
                start_ticks=start,
                identity=f"{pid}:{start}" if tid is None else f"{pid}:{tid}:{start}",
                state=fields[0],
                ppid=int(fields[1]),
                user_ticks=user,
                system_ticks=system,
                total_ticks=user + system,
            )
        except (ValueError, IndexError) as error:
            record.update(status="parse_error", error=error_evidence(error))
    return record


def _same_identity(before: dict, after: dict) -> bool:
    return before.get("identity") is not None and before["identity"] == after.get(
        "identity"
    )


def _process_tree_evidence(root_pid: int) -> tuple[list[int], dict[int, dict], dict]:
    stats, failures = {}, {}
    enumeration_error = None
    try:
        paths = list(Path("/proc").glob("[0-9]*/stat"))
    except OSError as error:
        paths, enumeration_error = [], error_evidence(error)
    for path in paths:
        pid = int(path.parent.name)
        record = _stat_read(pid)
        if record["status"] == "ok":
            stats[pid] = record
        else:
            key = f"{record['status']}:{record.get('error', {}).get('errno')}"
            failures[key] = failures.get(key, 0) + 1
    pids, seen = [root_pid], {root_pid}
    for parent in pids:
        for pid, record in stats.items():
            if record["ppid"] == parent and pid not in seen:
                pids.append(pid)
                seen.add(pid)
    # Root is kept even when unreadable: never turn missing coverage into 0/0.
    if root_pid not in stats:
        stats[root_pid] = _stat_read(root_pid)
    return (
        pids,
        {pid: stats[pid] for pid in pids},
        {
            "scope": "best_effort_non_atomic_proc_tree",
            "enumerated_stat_paths": len(paths),
            "scan_failures_by_status_errno": failures,
            "enumeration_error": enumeration_error,
            "all_descendants_observed": "unknown",
        },
    )


def _process_tree(root_pid: int) -> list[int]:
    return _process_tree_evidence(root_pid)[0]


def _field_read(source: dict, field_name: str, multiplier: int = 1) -> dict:
    record = {key: value for key, value in source.items() if key != "raw"}
    record.update(field=field_name, value=None)
    if source["status"] != "ok":
        return record
    for line in source["raw"].splitlines():
        fields = line.split()
        if fields and fields[0] == field_name:
            record["raw_line"] = line
            try:
                value = int(fields[1])
                if value < 0:
                    raise ValueError("negative procfs field")
                record["value"] = value * multiplier
            except (ValueError, IndexError) as error:
                record.update(status="parse_error", error=error_evidence(error))
            return record
    record["status"] = "missing_field"
    return record


def _fd_read(pid: int) -> dict:
    path = Path(f"/proc/{pid}/fd")
    record = {
        "path": str(path),
        "value": None,
        "started_monotonic_ns": time.monotonic_ns(),
    }
    try:
        record.update(status="ok", value=sum(1 for _ in path.iterdir()))
    except OSError as error:
        record.update(status="os_error", error=error_evidence(error))
    record["finished_monotonic_ns"] = time.monotonic_ns()
    return record


def _read_pss_bytes(pid: int) -> int | None:
    return _field_read(_text_read(Path(f"/proc/{pid}/smaps_rollup")), "Pss:", 1024)[
        "value"
    ]


def _read_rss_bytes(pid: int) -> int | None:
    return _field_read(_text_read(Path(f"/proc/{pid}/status")), "VmRSS:", 1024)["value"]


def _read_thread_count(pid: int) -> int | None:
    return _field_read(_text_read(Path(f"/proc/{pid}/status")), "Threads:")["value"]


def _read_fd_count(pid: int) -> int | None:
    return _fd_read(pid)["value"]


def _read_cpu_identity(pid: int) -> tuple[tuple[int, int], int] | None:
    value = _stat_read(pid)
    if value["status"] != "ok":
        return None
    return (pid, value["start_ticks"]), value["total_ticks"]


def _read_cpu_percent(pids: list[int]) -> float | None:
    # Compatibility for other callers: this is ps lifetime %CPU, not phase CPU.
    try:
        completed = subprocess.run(
            ["ps", "-o", "%cpu=", "-p", ",".join(map(str, pids))],
            capture_output=True,
            text=True,
            timeout=1,
            check=False,
        )
        return (
            sum(float(line.strip()) for line in completed.stdout.splitlines())
            if completed.returncode == 0 and completed.stdout.strip()
            else None
        )
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return None


def snapshot_resources(
    root_pid: int, *, include_lifetime_cpu: bool = True
) -> dict[str, Any]:
    started = time.monotonic_ns()
    pids, before, tree = _process_tree_evidence(root_pid)
    processes = []
    for pid in pids:
        status = _text_read(Path(f"/proc/{pid}/status"))
        row = {
            "pid": pid,
            "identity_before": before[pid],
            "pss": _field_read(
                _text_read(Path(f"/proc/{pid}/smaps_rollup")), "Pss:", 1024
            ),
            "rss": _field_read(status, "VmRSS:", 1024),
            "threads": _field_read(status, "Threads:"),
            "fds": _fd_read(pid),
        }
        row["identity_after"] = _stat_read(pid)
        row["identity_valid"] = _same_identity(before[pid], row["identity_after"])
        processes.append(row)
    result = {
        "instrumentation_schema": INSTRUMENTATION_SCHEMA,
        "pids": pids,
        "process_count": len(pids),
        "tree": tree,
        "processes": processes,
        "started_monotonic_ns": started,
        "finished_monotonic_ns": time.monotonic_ns(),
        "cpu_percent": _read_cpu_percent(pids) if include_lifetime_cpu else None,
        "cpu_percent_method": "ps_lifetime" if include_lifetime_cpu else None,
    }
    for name, total, count in (
        ("pss", "pss_bytes", "pss_process_count"),
        ("rss", "rss_bytes", "rss_process_count"),
        ("threads", "thread_count", "thread_process_count"),
        ("fds", "fd_count", "fd_process_count"),
    ):
        values = [
            row[name]["value"]
            for row in processes
            if row["identity_valid"] and row[name]["status"] == "ok"
        ]
        result[count] = len(values)
        result[total] = sum(values) if pids and len(values) == len(pids) else None
    result["selected_process_reads_complete"] = (
        bool(processes)
        and all(
            row["identity_valid"]
            and all(
                row[name]["status"] == "ok" for name in ("pss", "rss", "threads", "fds")
            )
            for row in processes
        )
        and tree.get("enumeration_error") is None
    )
    result["finished_monotonic_ns"] = time.monotonic_ns()
    return result


def clock_ticks_evidence() -> dict:
    result = {"source": "os.sysconf(SC_CLK_TCK)", "ticks_per_second": None}
    try:
        rate = os.sysconf("SC_CLK_TCK")
        if not isinstance(rate, int) or rate <= 0:
            raise ValueError("invalid SC_CLK_TCK")
        result.update(ticks_per_second=rate, counter_tick_ms=1000 / rate)
    except (OSError, ValueError) as error:
        result["error"] = error_evidence(error)
    return result


def cpu_snapshot(pid: int) -> dict:
    """Root process ticks and its observed live threads; no descendant CPU."""
    result = {
        "started_monotonic_ns": time.monotonic_ns(),
        "root_before": _stat_read(pid),
        "threads": {},
        "observations": [],
        "enumeration_error": None,
    }
    try:
        tasks = list(Path(f"/proc/{pid}/task").iterdir())
    except OSError as error:
        tasks = []
        result["enumeration_error"] = error_evidence(error)
    result["enumerated_tids"] = sorted(int(task.name) for task in tasks)
    for tid in result["enumerated_tids"]:
        before = _stat_read(pid, tid)
        runtime = _text_read(Path(f"/proc/{pid}/task/{tid}/schedstat"))
        runtime["value"] = None
        if runtime["status"] == "ok":
            try:
                runtime["value"] = int(runtime["raw"].split()[0])
                if runtime["value"] < 0:
                    raise ValueError("negative schedstat runtime")
            except (ValueError, IndexError) as error:
                runtime.update(
                    status="parse_error", error=error_evidence(error), value=None
                )
        after = _stat_read(pid, tid)
        valid = _same_identity(before, after) and runtime["status"] == "ok"
        result["observations"].append(
            {
                "tid": tid,
                "before": before,
                "runtime": runtime,
                "after": after,
                "valid": valid,
            }
        )
        if valid:
            result["threads"][before["identity"]] = runtime["value"]
    result["root_after"] = _stat_read(pid)
    result["finished_monotonic_ns"] = time.monotonic_ns()
    return result


def cpu_delta(
    before: dict, after: dict, clock_ticks: dict, expected_root: str | None
) -> dict:
    old, new = before["threads"], after["threads"]
    common, created, retired = (
        old.keys() & new.keys(),
        new.keys() - old.keys(),
        old.keys() - new.keys(),
    )
    regressed = sorted(key for key in common if new[key] < old[key])
    identities = [
        point[key].get("identity")
        for point in (before, after)
        for key in ("root_before", "root_after")
    ]
    root_valid = expected_root is not None and all(
        key == expected_root for key in identities
    )
    boundary_complete = root_valid and all(
        point["enumeration_error"] is None
        and bool(point["observations"])
        and all(row["valid"] for row in point["observations"])
        for point in (before, after)
    )
    known_ns = (
        None
        if regressed or not root_valid
        else sum(new[key] - old[key] for key in common)
        + sum(new[key] for key in created)
    )
    first, last = before["root_after"], after["root_after"]
    process_valid = root_valid and all(
        last.get(key, -1) >= first.get(key, 0) for key in ("user_ticks", "system_ticks")
    )
    ticks = last["total_ticks"] - first["total_ticks"] if process_valid else None
    rate = clock_ticks["ticks_per_second"]
    process_cpu = {
        "method": "proc_root_stat_utime_plus_stime",
        "scope": "root_process_all_threads_no_descendants",
        "before": first,
        "after": last,
        "clock_ticks": clock_ticks,
        "identity_valid": root_valid,
        "counter_valid": process_valid,
        "delta_ticks": ticks,
        "cpu_ms": ticks / rate * 1000 if ticks is not None and rate else None,
        "zero_ticks_mean": "no_exported_tick_change_not_zero_cpu",
        "remote_process_clock": {"status": "unsupported_not_runtime_validated"},
    }
    return {
        # Historical field stays an explicitly named thread estimate. Never
        # silently substitute process ticks when thread evidence is incomplete.
        "cpu_ms": known_ns / 1e6
        if known_ns is not None and boundary_complete
        else None,
        "thread_cpu_observed": {
            "method": "root_threads_schedstat_boundary_estimate",
            "before": before,
            "after": after,
            "known_cpu_ms": known_ns / 1e6 if known_ns is not None else None,
            "boundary_read_complete": boundary_complete,
            "counter_regressions": regressed,
            "lifetime_coverage": "not_proven_by_boundary_sampling",
        },
        "created_threads": sorted(created),
        "retired_threads": sorted(retired),
        "process_cpu": process_cpu,
    }


def affinity_snapshot(
    pid: int, requested_cpu: int | None, expected_root: str | None
) -> dict:
    result = {
        "started_monotonic_ns": time.monotonic_ns(),
        "root_before": _stat_read(pid),
        "requested_cpu": requested_cpu,
        "threads": [],
        "enumeration_error": None,
    }
    try:
        tids = sorted(int(path.name) for path in Path(f"/proc/{pid}/task").iterdir())
    except OSError as error:
        tids = []
        result["enumeration_error"] = error_evidence(error)
    for tid in tids:
        before = _stat_read(pid, tid)
        row = {"tid": tid, "before": before, "cpus": None, "error": None}
        try:
            row["cpus"] = sorted(os.sched_getaffinity(tid))
        except OSError as error:
            row["error"] = error_evidence(error)
        row["after"] = _stat_read(pid, tid)
        row["identity_valid"] = _same_identity(before, row["after"])
        result["threads"].append(row)
    result["root_after"] = _stat_read(pid)
    result["boundary_complete"] = (
        bool(tids)
        and result["enumeration_error"] is None
        and expected_root is not None
        and all(
            result[key].get("identity") == expected_root
            for key in ("root_before", "root_after")
        )
        and all(
            row["identity_valid"] and row["error"] is None for row in result["threads"]
        )
    )
    result["all_observed_match"] = (
        result["boundary_complete"]
        and requested_cpu is not None
        and all(row["cpus"] == [requested_cpu] for row in result["threads"])
    )
    result["lifetime_affinity"] = "not_proven_by_boundary_sampling"
    result["finished_monotonic_ns"] = time.monotonic_ns()
    return result


def instrumentation_manifest(probe_path: Path, *input_paths: Path) -> dict:
    paths = [
        probe_path,
        Path(__file__),
        Path(__file__).with_name("target_serve.py"),
        *input_paths,
    ]
    return {
        "schema": INSTRUMENTATION_SCHEMA,
        "input_sha256": {
            str(path.resolve()): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in paths
        },
        "process_cpu_method": "proc_root_stat_utime_plus_stime",
        "process_cpu_precision": "SC_CLK_TCK_quantized_not_schedstat_nanoseconds",
        "thread_cpu_scope": "root_observed_boundary_threads",
        "remote_process_clock": "unsupported_not_runtime_validated",
        "resource_scope": "best_effort_non_atomic_process_tree",
        "observer_clock": "time.monotonic_ns_boundaries_not_atomic_snapshot",
    }


def _sanitize_cgroup_name(name: str) -> str:
    return "".join(
        char if char.isalnum() or char in ("-", "_", ".") else "_" for char in name
    )


def _read_pid_cgroups(pid: int) -> list[dict[str, Any]]:
    try:
        lines = (
            Path(f"/proc/{pid}/cgroup")
            .read_text(encoding="utf-8", errors="replace")
            .splitlines()
        )
    except OSError:
        return []
    entries = []
    for line in lines:
        parts = line.split(":", 2)
        if len(parts) != 3:
            continue
        hierarchy, controllers, relative_path = parts
        entries.append(
            {
                "hierarchy": hierarchy,
                "controllers": [
                    controller for controller in controllers.split(",") if controller
                ],
                "path": relative_path,
            }
        )
    return entries


def _candidate_cgroup_roots(entry: dict[str, Any]) -> list[Path]:
    relative = str(entry.get("path") or "/").lstrip("/")
    controllers = entry.get("controllers") or []
    roots = [Path("/sys/fs/cgroup") / relative]
    for controller in controllers:
        roots.append(Path("/sys/fs/cgroup") / str(controller) / relative)
    return roots


def collect_cgroup_artifacts(pid: int, output_dir: Path) -> dict[str, Any]:
    output_dir.mkdir(parents=True, exist_ok=True)
    entries = _read_pid_cgroups(pid)
    try:
        raw_cgroup = Path(f"/proc/{pid}/cgroup").read_text(
            encoding="utf-8", errors="replace"
        )
    except OSError as error:
        raw_cgroup = f"unavailable: {error}\n"
    proc_cgroup_path = output_dir / "proc-cgroup.txt"
    proc_cgroup_path.write_text(raw_cgroup, encoding="utf-8")

    files = [
        "cgroup.controllers",
        "cgroup.events",
        "cgroup.procs",
        "cgroup.stat",
        "cpu.stat",
        "io.stat",
        "memory.current",
        "memory.events",
        "memory.max",
        "memory.peak",
        "memory.stat",
        "pids.current",
        "pids.max",
    ]
    copied: list[str] = [str(proc_cgroup_path)]
    roots: list[str] = []
    unavailable: list[str] = []
    seen_roots: set[Path] = set()
    for entry_index, entry in enumerate(entries):
        for root in _candidate_cgroup_roots(entry):
            if root in seen_roots or not root.exists():
                continue
            seen_roots.add(root)
            roots.append(str(root))
            label = f"{entry_index}-{_sanitize_cgroup_name(root.name or 'root')}"
            root_dir = output_dir / label
            root_dir.mkdir(parents=True, exist_ok=True)
            for file_name in files:
                source = root / file_name
                target = root_dir / file_name
                try:
                    target.write_text(
                        source.read_text(encoding="utf-8", errors="replace"),
                        encoding="utf-8",
                    )
                    copied.append(str(target))
                except OSError as error:
                    unavailable.append(f"{source}: {error}")
    if unavailable:
        unavailable_path = output_dir / "unavailable.txt"
        unavailable_path.write_text("\n".join(unavailable) + "\n", encoding="utf-8")
        copied.append(str(unavailable_path))
    return {
        "pid": pid,
        "entries": entries,
        "roots": roots,
        "artifact_dir": str(output_dir),
        "artifacts": copied,
        "available": bool(roots),
    }


@dataclass
class ResourceSampler:
    root_pid: int
    interval_seconds: float = 0.1
    samples: list[dict[str, Any]] = field(default_factory=list)
    _stop: threading.Event = field(default_factory=threading.Event)
    _thread: threading.Thread | None = None
    _started_monotonic: float | None = None
    _previous_monotonic: float | None = None
    _previous_cpu: dict[str, int] = field(default_factory=dict)
    _observer_error: str | None = None
    _observer_error_evidence: dict | None = None
    _clock_ticks: dict = field(default_factory=dict)
    _previous_cpu_complete: bool = False
    _samples_lock: threading.Lock = field(default_factory=threading.Lock)
    _sealed_samples: list[dict[str, Any]] | None = None
    _stop_requested_ns: int | None = None
    _join_finished_ns: int | None = None
    _thread_exited_ns: int | None = None
    _last_committed_ns: int | None = None
    _join_ever_timed_out: bool = False

    def start(self) -> None:
        if self.interval_seconds <= 0:
            raise ValueError("resource sample interval must be positive")
        if self._thread is not None:
            raise RuntimeError("resource sampler already started")
        self._clock_ticks = clock_ticks_evidence()
        self._started_monotonic = time.perf_counter()
        self._thread = threading.Thread(
            target=self._run, name="moli-resource-sampler", daemon=True
        )
        self._thread.start()

    def stop(self) -> dict[str, Any]:
        if self._stop_requested_ns is None:
            self._stop_requested_ns = time.monotonic_ns()
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=2)
        alive = self._thread is not None and self._thread.is_alive()
        self._join_finished_ns = time.monotonic_ns()
        self._join_ever_timed_out |= alive
        if not alive:
            with self._samples_lock:
                self._sealed_samples = copy.deepcopy(self.samples)
        return self.summary()

    def export_samples(self) -> list[dict[str, Any]]:
        with self._samples_lock:
            return copy.deepcopy(
                self._sealed_samples
                if self._sealed_samples is not None
                else self.samples
            )

    def _run(self) -> None:
        try:
            next_capture = time.perf_counter()
            while not self._stop.is_set():
                capture_started = time.perf_counter()
                sample = snapshot_resources(self.root_pid, include_lifetime_cpu=False)
                captured = time.perf_counter()
                rows = sample.get("processes", [])
                identities = {
                    row["identity_after"]["identity"]: row["identity_after"][
                        "total_ticks"
                    ]
                    for row in rows
                    if row["identity_valid"]
                }
                complete = bool(rows) and len(identities) == sample["process_count"]
                common = identities.keys() & self._previous_cpu.keys()
                regressed = sorted(
                    key for key in common if identities[key] < self._previous_cpu[key]
                )
                delta_ticks = (
                    None
                    if regressed
                    else sum(
                        identities[key] - self._previous_cpu[key] for key in common
                    )
                )
                elapsed = (
                    captured - self._previous_monotonic
                    if self._previous_monotonic is not None
                    else None
                )
                stable = (
                    complete
                    and self._previous_cpu_complete
                    and identities.keys() == self._previous_cpu.keys()
                )
                rate = self._clock_ticks.get("ticks_per_second")
                sample["cpu_percent"] = (
                    delta_ticks / rate / elapsed * 100
                    if stable
                    and delta_ticks is not None
                    and rate
                    and elapsed
                    and elapsed > 0
                    else None
                )
                sample["cpu_percent_method"] = "proc_tree_common_identity_tick_delta"
                sample["cpu_evidence"] = {
                    "before": self._previous_cpu,
                    "after": identities,
                    "clock_ticks": self._clock_ticks,
                    "elapsed_seconds": elapsed,
                    "known_common_delta_ticks": delta_ticks,
                    "counter_regressions": regressed,
                    "created_processes": sorted(
                        identities.keys() - self._previous_cpu.keys()
                    ),
                    "retired_processes": sorted(
                        self._previous_cpu.keys() - identities.keys()
                    ),
                    "stable_observed_process_set": stable,
                    "lifetime_coverage": "not_proven_by_boundary_sampling",
                }
                self._previous_monotonic = captured
                self._previous_cpu = identities
                self._previous_cpu_complete = complete
                sample["timestamp"] = time.time()
                sample["elapsed_ms"] = (
                    (captured - self._started_monotonic) * 1000
                    if self._started_monotonic is not None
                    else 0.0
                )
                sample["capture_duration_ms"] = (captured - capture_started) * 1000
                sample["kind"] = "periodic"
                with self._samples_lock:
                    sample["sequence"] = len(self.samples)
                    self.samples.append(sample)
                    self._last_committed_ns = time.monotonic_ns()
                next_capture += self.interval_seconds
                now = time.perf_counter()
                if next_capture <= now:
                    missed = int((now - next_capture) / self.interval_seconds) + 1
                    next_capture += missed * self.interval_seconds
                self._stop.wait(max(0.0, next_capture - now))
        except BaseException as error:  # noqa: BLE001 -- Preserve observer termination as evidence.
            self._observer_error = f"{type(error).__name__}: {error}"
            self._observer_error_evidence = {
                **error_evidence(error),
                "traceback": traceback.format_exc(),
            }
        finally:
            self._thread_exited_ns = time.monotonic_ns()

    def summary(self) -> dict[str, Any]:
        samples = self.export_samples()
        pss = [
            sample["pss_bytes"]
            for sample in samples
            if sample.get("pss_bytes") is not None
        ]
        rss = [
            sample["rss_bytes"]
            for sample in samples
            if sample.get("rss_bytes") is not None
        ]
        cpu = [
            sample["cpu_percent"]
            for sample in samples
            if sample.get("cpu_percent") is not None
        ]
        process_counts = [
            sample["process_count"]
            for sample in samples
            if sample.get("process_count") is not None
        ]
        thread_counts = [
            sample["thread_count"]
            for sample in samples
            if sample.get("thread_count") is not None
        ]
        fd_counts = [
            sample["fd_count"]
            for sample in samples
            if sample.get("fd_count") is not None
        ]
        capture_durations = [
            float(sample["capture_duration_ms"])
            for sample in samples
            if sample.get("capture_duration_ms") is not None
        ]
        intervals = [
            float(current["elapsed_ms"]) - float(previous["elapsed_ms"])
            for previous, current in pairwise(samples)
            if previous.get("elapsed_ms") is not None
            and current.get("elapsed_ms") is not None
        ]
        complete_pss_samples = sum(
            1
            for sample in samples
            if sample.get("pss_bytes") is not None
            and sample.get("pss_process_count") == sample.get("process_count")
        )
        return {
            "instrumentation_schema": INSTRUMENTATION_SCHEMA,
            "sample_count": len(samples),
            "sealed": self._sealed_samples is not None,
            "stop_requested_monotonic_ns": self._stop_requested_ns,
            "join_finished_monotonic_ns": self._join_finished_ns,
            "thread_exited_monotonic_ns": self._thread_exited_ns,
            "last_committed_monotonic_ns": self._last_committed_ns,
            "last_committed_sample_sequence": len(samples) - 1 if samples else None,
            "join_ever_timed_out": self._join_ever_timed_out,
            "observer_error_evidence": self._observer_error_evidence,
            "scope": "sampler_start_through_stop_request_inflight_completion",
            "totals_are_final": self._sealed_samples is not None,
            "interval_seconds": self.interval_seconds,
            "peak_pss_bytes": max(pss) if pss else None,
            "peak_rss_bytes": max(rss) if rss else None,
            "peak_cpu_percent": max(cpu) if cpu else None,
            "peak_process_count": max(process_counts) if process_counts else None,
            "peak_thread_count": max(thread_counts) if thread_counts else None,
            "peak_fd_count": max(fd_counts) if fd_counts else None,
            "average_cpu_percent": sum(cpu) / len(cpu) if cpu else None,
            "capture_duration_ms": {
                "average": sum(capture_durations) / len(capture_durations)
                if capture_durations
                else None,
                "max": max(capture_durations) if capture_durations else None,
            },
            "late_sample_count": sum(
                1
                for interval in intervals
                if interval > self.interval_seconds * 1000.0 * 1.5
            ),
            "observed_interval_ms": {
                "average": sum(intervals) / len(intervals) if intervals else None,
                "max": max(intervals) if intervals else None,
            },
            "resource_complete_samples": sum(
                sample.get("selected_process_reads_complete") is True
                for sample in samples
            ),
            "resource_reads_complete": bool(samples)
            and all(
                sample.get("selected_process_reads_complete") is True
                for sample in samples
            ),
            "pss_complete_samples": complete_pss_samples,
            "pss_complete": bool(samples) and complete_pss_samples == len(samples),
            "observer_error": self._observer_error,
            "thread_alive_after_stop": bool(
                self._thread is not None and self._thread.is_alive()
            ),
            "sampling_method": "procfs_process_tree_cpu_ticks_smaps_rollup",
        }
