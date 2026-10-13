"""Run one bounded public-API retention diagnostic, preserving every failure.

Build/verify the example separately. This driver never invokes Cargo or changes
source. This is a diagnostic run, not the formal equal-task CPU benchmark.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import signal
import subprocess
import sys
import time
import traceback
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from moli_benchmark import sampling

logger = logging.getLogger(__name__)


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def save(path: Path, value: dict) -> None:
    temporary = path.with_name(path.name + ".tmp")
    created = False
    try:
        with temporary.open("x", encoding="utf-8") as target:
            created = True
            json.dump(value, target, ensure_ascii=False, indent=2)
            target.write("\n")
        temporary.replace(path)
    finally:
        if created:
            temporary.unlink(missing_ok=True)


def terminate(process: subprocess.Popen) -> int:
    if process.poll() is not None:
        return process.returncode
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        return process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        return process.wait(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    events = output.with_suffix(".events.jsonl")
    stderr = output.with_suffix(".stderr.log")
    for path in (output, events, stderr, output.with_name(output.name + ".tmp")):
        if path.exists():
            parser.error(f"refusing to replace existing evidence: {path}")
    output.parent.mkdir(parents=True, exist_ok=True)
    # Reserve this run before any subprocess can start. Partial raw logs survive
    # crashes and are never silently reused by a later invocation.
    with output.open("x", encoding="utf-8") as target:
        json.dump({"status": "reserved", "formal_cost_sample": False}, target)
    record = {
        "status": "started",
        "formal_cost_sample": False,
        "profile_enabled": False,
        "build_manifest": str(args.build.resolve()),
        "driver_sha256": digest(Path(__file__)),
        "sampler_sha256": digest(Path(sampling.__file__)),
        "events": str(events),
        "stderr": str(stderr),
        "started_unix": time.time(),
        "timeout_seconds": 120,
        "errors": [],
        "limitations": [
            "Estimated retained charge is not RSS/PSS or a pass peak bound.",
            "Public budget snapshots are aggregate: exact Page/slot FIFO identity is not exposed.",
            "Source/build/frame and retained reuse counters are not exposed by this public release harness.",
            "Two finite static Pages do not establish long-term starvation freedom or a busy-owner SLA.",
            "CPU and sampled process-tree PSS include the helper and journal overhead; this is not a formal cost comparison.",
        ],
    }
    process = sampler = None
    try:
        build = json.loads(args.build.read_text())
        binary = Path(build["binary"]).resolve()
        source = Path(build["probe_source"]).resolve()
        if build.get("tracked_diff") != []:
            raise ValueError("build manifest must record a clean committed source tree")
        if build.get("profile") != "release":
            raise ValueError("build manifest must identify a release example")
        command = build.get("command", "")
        if "--example layout_working_set_pressure" not in command:
            raise ValueError("manifest does not identify the diagnostic example target")
        if "--all-features" in command or "test-support" in command:
            raise ValueError(
                "diagnostic release must not enable test-support/all-features"
            )
        if digest(binary) != build["binary_sha256"]:
            raise ValueError("binary hash mismatch")
        if digest(source) != build["probe_source_sha256"]:
            raise ValueError("frozen example source hash mismatch")
        record["build"] = build
        record["command"] = [str(binary)]
        record["binary_sha256_before"] = digest(binary)
        save(output, record)
        env = os.environ.copy()
        env.pop("MOLI_CPU_PROFILE", None)
        with events.open("xb") as raw, stderr.open("xb") as errors:
            process = subprocess.Popen(
                [str(binary)],
                stdout=raw,
                stderr=errors,
                env=env,
                start_new_session=True,
            )
            record["pid"] = process.pid
            sampler = sampling.ResourceSampler(process.pid, interval_seconds=0.05)
            sampler.start()
            try:
                record["returncode"] = process.wait(timeout=120)
                record["timed_out"] = False
            except subprocess.TimeoutExpired:
                record["timed_out"] = True
                record["errors"].append(
                    "process exceeded the hard 120-second diagnostic deadline"
                )
                record["returncode"] = terminate(process)
        record["resources"] = sampler.stop()
        record["resource_samples"] = list(sampler.samples)
        sampler = None
        rows = []
        for line_number, line in enumerate(events.read_text().splitlines(), 1):
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as error:
                record["errors"].append(f"event line {line_number}: {error}")
        record["event_count"] = len(rows)
        record["events_sha256"] = digest(events)
        record["stderr_sha256"] = digest(stderr)
        starts = [row for row in rows if row.get("event") == "start"]
        terminals = [row for row in rows if row.get("event") == "result"]
        if len(starts) != 1 or starts[0]["data"]["source_sha"] != build["source_sha"]:
            record["errors"].append(
                "binary embedded source SHA did not match its build manifest"
            )
        if len(terminals) != 1 or terminals[0]["data"].get("ok") is not True:
            record["errors"].append("missing or unsuccessful terminal result")
        if [row.get("sequence") for row in rows] != list(range(len(rows))):
            record["errors"].append("event sequence is incomplete or out of order")
        record["phase_events"] = rows
        record["binary_sha256_after"] = digest(binary)
        if record["binary_sha256_after"] != build["binary_sha256"]:
            record["errors"].append("binary changed during the diagnostic")
        if record["returncode"] != 0:
            record["errors"].append(f"diagnostic exited {record['returncode']}")
        if record["resources"].get("observer_error"):
            record["errors"].append(
                f"resource sampler: {record['resources']['observer_error']}"
            )
        if record["resources"].get("thread_alive_after_stop"):
            record["errors"].append("resource sampler did not stop")
    except BaseException:
        logger.exception("Pressure diagnostic failed; retaining partial evidence")
        record["errors"].append(traceback.format_exc())
    finally:
        if process is not None and process.poll() is None:
            try:
                record["returncode"] = terminate(process)
            except BaseException:
                logger.exception("Pressure diagnostic process cleanup failed")
                record["errors"].append(traceback.format_exc())
        if sampler is not None:
            record["resources"] = sampler.stop()
            record["resource_samples"] = list(sampler.samples)
        record["finished_unix"] = time.time()
        record["status"] = "passed" if not record["errors"] else "failed"
        save(output, record)
    print(json.dumps({"output": str(output), "status": record["status"]}))
    return 0 if record["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
