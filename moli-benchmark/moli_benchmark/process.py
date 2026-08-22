from __future__ import annotations

import os
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .sampling import ResourceSampler, collect_cgroup_artifacts


@dataclass(frozen=True)
class ProcessResult:
    command: list[str]
    returncode: int | None
    elapsed_ms: float
    stdout: bytes
    stderr: bytes
    timed_out: bool
    resources: dict[str, Any]
    time_verbose: dict[str, Any] | None = None
    cgroup: dict[str, Any] | None = None
    response_status: int | None = None
    response_mime_type: str | None = None
    main_document_body_capture: str | None = None
    final_url: str | None = None

    def output_digest_material(self) -> bytes:
        return self.stdout + b"\n--- stderr ---\n" + self.stderr

    def json_summary(self, include_output: bool = False) -> dict[str, Any]:
        summary: dict[str, Any] = {
            "command": self.command,
            "returncode": self.returncode,
            "elapsed_ms": self.elapsed_ms,
            "timed_out": self.timed_out,
            "resources": self.resources,
            "stdout_bytes": len(self.stdout),
            "stderr_bytes": len(self.stderr),
        }
        if self.time_verbose is not None:
            summary["time_verbose"] = self.time_verbose
        if self.cgroup is not None:
            summary["cgroup"] = self.cgroup
        if self.response_status is not None:
            summary["response_status"] = self.response_status
        if self.response_mime_type is not None:
            summary["response_mime_type"] = self.response_mime_type
        if self.main_document_body_capture is not None:
            summary["main_document_body_capture"] = self.main_document_body_capture
        if self.final_url is not None:
            summary["final_url"] = self.final_url
        if include_output:
            summary["stdout_tail"] = self.stdout[-4096:].decode("utf-8", errors="replace")
            summary["stderr_tail"] = self.stderr[-4096:].decode("utf-8", errors="replace")
        return summary


def _kill_process_group(pid: int, sig: signal.Signals) -> bool:
    try:
        os.killpg(pid, sig)
        return True
    except OSError:
        return False


def _parse_elapsed_seconds(value: str) -> float | None:
    parts = value.strip().split(":")
    try:
        if len(parts) == 3:
            hours, minutes, seconds = parts
            return int(hours) * 3600 + int(minutes) * 60 + float(seconds)
        if len(parts) == 2:
            minutes, seconds = parts
            return int(minutes) * 60 + float(seconds)
        return float(value)
    except ValueError:
        return None


def parse_time_verbose_output(text: str) -> dict[str, Any]:
    parsed: dict[str, Any] = {}
    if text.startswith("unavailable:"):
        return {"available": False, "error": text.strip()}
    for line in text.splitlines():
        if ": " not in line:
            continue
        key, value = line.split(": ", 1)
        key = key.strip()
        value = value.strip()
        try:
            integer = int(value)
        except ValueError:
            integer = None
        if key == "User time (seconds)":
            try:
                parsed["user_seconds"] = float(value)
            except ValueError:
                pass
        elif key == "System time (seconds)":
            try:
                parsed["system_seconds"] = float(value)
            except ValueError:
                pass
        elif key == "Elapsed (wall clock) time (h:mm:ss or m:ss)":
            parsed["elapsed_seconds"] = _parse_elapsed_seconds(value)
        elif key == "Maximum resident set size (kbytes)" and integer is not None:
            parsed["max_rss_bytes"] = integer * 1024
        elif key == "Major (requiring I/O) page faults" and integer is not None:
            parsed["major_page_faults"] = integer
        elif key == "Minor (reclaiming a frame) page faults" and integer is not None:
            parsed["minor_page_faults"] = integer
        elif key == "Voluntary context switches" and integer is not None:
            parsed["voluntary_context_switches"] = integer
        elif key == "Involuntary context switches" and integer is not None:
            parsed["involuntary_context_switches"] = integer
        elif key == "File system inputs" and integer is not None:
            parsed["file_system_inputs"] = integer
        elif key == "File system outputs" and integer is not None:
            parsed["file_system_outputs"] = integer
        elif key == "Exit status" and integer is not None:
            parsed["exit_status"] = integer
    if parsed:
        parsed["available"] = True
    return parsed


def read_time_verbose_file(path: Path) -> dict[str, Any] | None:
    try:
        raw = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    parsed = parse_time_verbose_output(raw)
    parsed["raw_path"] = str(path)
    parsed["raw_bytes"] = len(raw.encode("utf-8"))
    return parsed


def time_verbose_command(command: list[str], output_path: Path | None) -> list[str]:
    if output_path is None:
        return command
    output_path.parent.mkdir(parents=True, exist_ok=True)
    time_bin = Path("/usr/bin/time")
    if not time_bin.exists():
        output_path.write_text("unavailable: /usr/bin/time executable not found\n", encoding="utf-8")
        return command
    return [str(time_bin), "-v", "-o", str(output_path), *command]


def run_process(
    command: list[str],
    *,
    cwd: Path,
    timeout_seconds: float,
    env: dict[str, str] | None = None,
    sample_resources: bool = True,
    time_verbose_path: Path | None = None,
    cgroup_artifact_dir: Path | None = None,
) -> ProcessResult:
    started = time.perf_counter()
    effective_command = time_verbose_command(command, time_verbose_path)
    process = subprocess.Popen(
        effective_command,
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    sampler = ResourceSampler(process.pid) if sample_resources else None
    cgroup = collect_cgroup_artifacts(process.pid, cgroup_artifact_dir) if cgroup_artifact_dir is not None else None
    if sampler is not None:
        sampler.start()
    timed_out = False
    stdout = b""
    stderr = b""
    try:
        try:
            stdout, stderr = process.communicate(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            _kill_process_group(process.pid, signal.SIGTERM)
            try:
                stdout, stderr = process.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                _kill_process_group(process.pid, signal.SIGKILL)
                try:
                    stdout, stderr = process.communicate(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
    finally:
        elapsed_ms = (time.perf_counter() - started) * 1000.0
        resources = sampler.stop() if sampler is not None else {}
    time_verbose = read_time_verbose_file(time_verbose_path) if time_verbose_path is not None else None
    return ProcessResult(
        command=command,
        returncode=process.returncode,
        elapsed_ms=elapsed_ms,
        stdout=stdout,
        stderr=stderr,
        timed_out=timed_out,
        resources=resources,
        time_verbose=time_verbose,
        cgroup=cgroup,
    )
