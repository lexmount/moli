"""Measure matched layout snapshots and separately observed live-page memory."""

from __future__ import annotations

import argparse
import asyncio
import datetime
import hashlib
import json
import os
import platform
import re
import statistics
import time
from pathlib import Path
from types import SimpleNamespace

from compare_webpages import native_metrics
from web_bench import one_run

VARIANTS = ("baseline", "slotmap", "dense")
POINTS = (
    "before",
    "after_initialization",
    "after_workload",
    "after_idle",
    "after_close",
)
BLOCKED = re.compile(
    r"just a moment|access denied|403 forbidden|404 not found|安全验证|请完成验证|robot check|ошибка доступа",
    re.IGNORECASE,
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def scalar_metrics(folder, result):
    values = {}
    for name in POINTS:
        point = result.get(name)
        if not point:
            continue
        mem = point["memory_kib"]
        prefix = {
            "before": "blank",
            "after_initialization": "init",
            "after_workload": "workload",
            "after_idle": "idle",
            "after_close": "closed",
        }[name]
        for field in ["Rss", "Pss", "Pss_Anon", "Anonymous"]:
            if field in mem:
                values[prefix + "_" + field.lower() + "_mib"] = mem[field] / 1024
        values[prefix + "_uss_mib"] = (
            mem["Private_Clean"] + mem["Private_Dirty"]
        ) / 1024
    values["init_peak_rss_mib"] = result["after_initialization"]["peak_rss_kib"] / 1024
    values["init_window_seconds"] = (
        result["after_initialization"]["monotonic"] - result["before"]["monotonic"]
    )
    values["after_idle_seconds"] = (
        result["after_idle"]["monotonic"] - result["before"]["monotonic"]
    )
    values["lifetime_peak_rss_mib"] = (
        max(p["peak_rss_kib"] for k in POINTS if (p := result.get(k))) / 1024
    )
    values["init_cpu_ms"] = (
        (result["after_initialization"]["cpu_ticks"] - result["before"]["cpu_ticks"])
        * 1000
        / result["clock_ticks"]
    )
    values["load_ms"] = (
        result["load_event_seconds"] * 1000
        if result["load_event_seconds"] is not None
        else None
    )
    values["root_complete"] = int(result["observation"]["ready"] == "complete")
    values["nodes"] = result["observation"]["nodes"]
    values["retained_rss_delta_mib"] = values["idle_rss_mib"] - values["blank_rss_mib"]
    values["after_close_rss_delta_mib"] = (
        values["closed_rss_mib"] - values["blank_rss_mib"]
    )
    samples = [
        json.loads(line)
        for line in (folder / "memory-samples.jsonl").read_text().splitlines()
    ]
    if not samples or (folder / "sampling-error.json").exists():
        raise RuntimeError("Memory sampling failed")
    values["sampled_peak_pss_mib"] = max(p["memory_kib"]["Pss"] for p in samples) / 1024
    for phase in ["initialization", "idle", "live_followup", "after_close"]:
        group = [p for p in samples if p["phase"] == phase]
        if group:
            values[phase + "_sampled_peak_rss_mib"] = (
                max(p["memory_kib"]["Rss"] for p in group) / 1024
            )
    batches = []
    for batch in result.get("pulse_batches", []):
        before, after = batch["before"], batch["after"]
        updates = len(batch["pulse"]["samples"])
        native = native_metrics(folder, {"before_pulse": before, "after_pulse": after})
        batches.append(
            {
                "cpu_ms": (after["cpu_ticks"] - before["cpu_ticks"])
                * 1000
                / result["clock_ticks"],
                "wall_ms": batch["pulse"]["total"],
                "rss_mib": after["memory_kib"]["Rss"] / 1024,
                "pss_mib": after["memory_kib"]["Pss"] / 1024,
                "updates": updates,
                "native": native,
            }
        )
    if batches:
        updates = sum(b["updates"] for b in batches)
        values["cpu_us_per_update"] = sum(b["cpu_ms"] for b in batches) * 1000 / updates
        values["wall_us_per_update"] = (
            sum(b["wall_ms"] for b in batches) * 1000 / updates
        )
        values["first_cpu_us_per_update"] = (
            batches[0]["cpu_ms"] * 1000 / batches[0]["updates"]
        )
        values["late_cpu_us_per_update"] = (
            sum(b["cpu_ms"] for b in batches[1:])
            * 1000
            / sum(b["updates"] for b in batches[1:])
        )
        values["batch_rss_growth_mib"] = batches[-1]["rss_mib"] - batches[0]["rss_mib"]
        values["box_retention_us_per_update"] = (
            sum(b["native"]["box_retention_ms"] for b in batches) * 1000 / updates
        )
        values["numeric_us_per_update"] = (
            sum(b["native"]["phase_ms"]["numeric_layout_us"] for b in batches)
            * 1000
            / updates
        )
        values["projection_us_per_update"] = (
            sum(b["native"]["phase_ms"]["projection_us"] for b in batches)
            * 1000
            / updates
        )
        values["max_box_count"] = max(b["native"]["max_box_count"] for b in batches)
    return values, batches


def summarize(rows):
    result = {}
    for row in rows:
        if row["status"] != "ok":
            continue
        group = result.setdefault(row["site"], {}).setdefault(
            row["variant"], {"runs": 0, "values": {}}
        )
        group["runs"] += 1
        for key, value in row["metrics"].items():
            if value is not None:
                group["values"].setdefault(key, []).append(value)
    for vs in result.values():
        for group in vs.values():
            group["metrics"] = {
                key: {
                    "median": statistics.median(value),
                    "min": min(value),
                    "max": max(value),
                }
                for key, value in group.pop("values").items()
            }
    return result


async def run(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    plan = json.loads(args.plan.read_text())
    parent = args.binaries.resolve()
    binaries = {v: parent / (v + "-moli") for v in VARIANTS}
    control = Path(__file__).resolve().parent
    manifest = {
        "started_at": datetime.datetime.now(datetime.UTC).isoformat(),
        "mode": args.mode,
        "plan": plan,
        "repetitions": args.repetitions,
        "repeat_offset": args.repeat_offset,
        "platform": platform.platform(),
        "python": platform.python_version(),
        "clock_ticks": os.sysconf("SC_CLK_TCK"),
        "cpus": args.cpus,
        "sample_interval_seconds": 0.1,
        "idle_seconds": 2,
        "after_close_seconds": 2,
        "rss_limit_mib": 4096,
        "binary_sha256": {v: digest(p) for v, p in binaries.items()},
        "control_sha256": {
            n: digest(control / n)
            for n in [
                "web_bench.py",
                "snapshot_capture.js",
                "compare_webpages.py",
                "extended_compare.py",
            ]
        },
        "inputs": {
            r["site"]: {
                name: digest(Path(r["archive"]) / name)
                for name in ["snapshot.json", "archive.json"]
            }
            for r in plan
            if args.mode == "snapshot"
        },
        "method": "Snapshots: original-fidelity check, 3 batches, 48 updates per batch (12 for >5000 DOM elements), root font/style/layout data unchanged. Live: no script injection/interception/pulse, fixed 8s window (20s finance/hh), additional 2s observation, then target close and 2s observation. Both: fresh process, three variants, rotating order, memory sampled at 100ms, no forced GC.",
    }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    rows = []
    references = {}
    for repeat in range(args.repetitions):
        for site_index, item in enumerate(plan):
            if repeat >= item.get("repetitions", args.repetitions):
                continue
            site = item["site"]
            rotation = (
                repeat + item.get("order_index", site_index) + args.repeat_offset
            ) % 3
            order = VARIANTS[rotation:] + VARIANTS[:rotation]
            for variant in order:
                name = f"{repeat + 1 + args.repeat_offset:02d}-{site}-{variant}"
                folder = output / name
                start = time.monotonic()
                row = {
                    "run": name,
                    "site": site,
                    "variant": variant,
                    "repeat": repeat + 1 + args.repeat_offset,
                    "status": "failed",
                }
                print("running", name, flush=True)
                options = SimpleNamespace(
                    binary=binaries[variant],
                    site=site,
                    mode=args.mode,
                    archive=Path(item.get("archive", str(output / "unused"))),
                    output=folder,
                    seconds=2
                    if args.mode == "snapshot"
                    else (20 if site in ["finance", "hh"] else 8),
                    cpus=args.cpus,
                    network_mode=item["network_mode"],
                    sample_interval=0.1,
                    max_rss_mib=4096,
                    idle_seconds=2,
                    close_seconds=2,
                    pulse_batches=3,
                    pulse_updates=item.get("pulse_updates", 96),
                    close_page=True,
                    observe_only=args.mode == "live",
                    load_timeout=30 if args.mode == "snapshot" else 0,
                )
                try:
                    await asyncio.wait_for(one_run(options), 180)
                    result = json.loads((folder / "result.json").read_text())
                    if result["binary_sha256"] != manifest["binary_sha256"][variant]:
                        raise RuntimeError("Executable changed during experiment")
                    if (
                        result["navigation"].get("errorText")
                        or result["observation"]["nodes"] < 20
                        or BLOCKED.search(result["observation"]["title"])
                    ):
                        raise RuntimeError(
                            "Navigation failed or challenge/error document"
                        )
                    if args.mode == "snapshot":
                        if any(
                            result[key] != manifest["inputs"][site][name]
                            for key, name in [
                                ("archive_sha256", "archive.json"),
                                ("snapshot_sha256", "snapshot.json"),
                            ]
                        ):
                            raise RuntimeError(
                                "Input archive changed during experiment"
                            )
                        fingerprint = {
                            key: result["observation"][key]
                            for key in ["title", "nodes", "text", "boxes"]
                        }
                        geometry = result["pulse"]["geometry"]
                        if (
                            site in references
                            and (fingerprint, geometry) != references[site]
                        ):
                            raise RuntimeError(
                                "DOM or geometry differs between engine variants"
                            )
                        references[site] = (fingerprint, geometry)
                    values, batches = scalar_metrics(folder, result)
                    row.update(
                        status="ok",
                        metrics=values,
                        batches=batches,
                        title=result["observation"]["title"],
                        ready=result["observation"]["ready"],
                        url=result["observation"]["url"],
                    )
                except Exception as error:  # noqa: BLE001 - failures are explicit rows, never silently retried
                    row["error"] = repr(error)
                    folder.mkdir(exist_ok=True)
                    (folder / "run-error.json").write_text(json.dumps(row, indent=2))
                    print("failed", name, repr(error), flush=True)
                row["seconds"] = time.monotonic() - start
                rows.append(row)
                (output / "runs.json").write_text(
                    json.dumps(rows, indent=2, ensure_ascii=False)
                )
                (output / "summary.json").write_text(
                    json.dumps(summarize(rows), indent=2)
                )
    manifest["finished_at"] = datetime.datetime.now(datetime.UTC).isoformat()
    manifest["successes"] = sum(r["status"] == "ok" for r in rows)
    manifest["failures"] = sum(r["status"] != "ok" for r in rows)
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(
        "complete",
        manifest["successes"],
        "successful",
        manifest["failures"],
        "failed",
        flush=True,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["snapshot", "live"], required=True)
    for name in ["plan", "binaries", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--repeat-offset", type=int, default=0)
    parser.add_argument("--cpus", default="3,4")
    asyncio.run(run(parser.parse_args()))
