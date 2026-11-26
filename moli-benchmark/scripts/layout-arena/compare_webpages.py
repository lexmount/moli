"""Compare one Vec baseline and two stable-key stores on captured public pages."""

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
from pathlib import Path
from types import SimpleNamespace

from web_bench import one_run

PHASES = [
    "box_tree_us",
    "list_marker_us",
    "form_control_us",
    "inline_preparation_us",
    "numeric_layout_us",
    "embedded_frame_us",
    "projection_us",
]


def native_metrics(folder, result):
    start, end = (result[k]["unix_time"] for k in ["before_pulse", "after_pulse"])
    passes = []
    retention = []
    cache_traces = []
    for line in (folder / "engine.log").read_text().splitlines():
        layout_pass = 'stage="layout_pass"' in line
        retention_pass = any(
            f'stage="{name}"' in line
            for name in ["layout_box_compaction", "layout_box_retention"]
        )
        cache_pass = 'stage="layout_numeric_cache"' in line
        if not (layout_pass or retention_pass or cache_pass):
            continue
        stamp = datetime.datetime.fromisoformat(line.split()[0]).timestamp()
        if not start <= stamp <= end:
            continue
        values = dict(re.findall(r"(\w+)=(\d+)", line))
        target = (
            passes if layout_pass else retention if retention_pass else cache_traces
        )
        if layout_pass:
            values["retained_numeric_tree"] = (
                "1" if "retained_numeric_tree=true" in line else "0"
            )
        target.append({k: int(v) for k, v in values.items()})
    if not passes:
        raise RuntimeError(f"No layout traces in pulse window: {folder}")
    return {
        "passes": len(passes),
        "retained_box_visits": sum(p.get("retained_box_count", 0) for p in passes),
        "retained_tree_passes": sum(p.get("retained_numeric_tree", 0) for p in passes),
        "box_retention_ms": sum(p["total_us"] for p in retention) / 1000,
        "retention_calls": len(retention),
        "inline_layout_hits": statistics.median(
            p.get("retained_inline_layout_hits", 0) for p in cache_traces
        )
        if cache_traces
        else 0,
        "inline_measurement_hits": statistics.median(
            p.get("retained_inline_measurement_hits", 0) for p in cache_traces
        )
        if cache_traces
        else 0,
        "layout_ms": sum(p["total_us"] for p in passes) / 1000,
        "phase_ms": {k: sum(p.get(k, 0) for p in passes) / 1000 for k in PHASES},
        "retained_numeric_hits": sum(
            p.get("retained_numeric_layout_hits", 0) for p in passes
        ),
        "reused_inline_contexts": sum(
            p.get("reused_inline_contexts", 0) for p in passes
        ),
        "max_box_count": max(p["box_count"] for p in passes),
    }


def metrics(folder, result):
    before, after = result["before_pulse"], result["after_pulse"]
    return {
        "pulse_cpu_ms": (after["cpu_ticks"] - before["cpu_ticks"])
        * 1000
        / result["clock_ticks"],
        "pulse_wall_ms": result["pulse"]["total"],
        "load_event_ms": result["load_event_seconds"] * 1000
        if result["load_event_seconds"] is not None
        else None,
        "initialization_cpu_ms": (
            result["after_initialization"]["cpu_ticks"] - result["before"]["cpu_ticks"]
        )
        * 1000
        / result["clock_ticks"],
        "rss_mib": after["memory_kib"]["Rss"] / 1024,
        "pss_mib": after["memory_kib"]["Pss"] / 1024,
        "peak_rss_mib": after["peak_rss_kib"] / 1024,
        "native": native_metrics(folder, result),
    }


def distribution(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


def summarize(rows):
    result = {}
    for site in dict.fromkeys(r["site"] for r in rows):
        result[site] = {}
        for variant in dict.fromkeys(r["variant"] for r in rows):
            group = [
                r["metrics"]
                for r in rows
                if r["site"] == site and r["variant"] == variant
            ]
            if not group:
                continue
            result[site][variant] = {
                key: distribution([g[key] for g in group if g[key] is not None])
                for key in group[0]
                if key != "native" and any(g[key] is not None for g in group)
            }
            result[site][variant]["native"] = {
                key: distribution([g["native"][key] for g in group])
                for key in group[0]["native"]
                if key != "phase_ms"
            }
            result[site][variant]["native"]["phase_ms"] = {
                key: distribution([g["native"]["phase_ms"][key] for g in group])
                for key in PHASES
            }
    return result


async def compare(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    binaries = {
        key: getattr(args, key).resolve() for key in ["baseline", "slotmap", "dense"]
    }
    archive = args.archives.resolve()
    manifest = {
        "started_at": datetime.datetime.now(datetime.UTC).isoformat(),
        "platform": platform.platform(),
        "python": platform.python_version(),
        "affinity": sorted(os.sched_getaffinity(0)),
        "engine_cpus": args.cpus,
        "repetitions": args.repetitions,
        "sites": args.sites,
        "binaries": {
            key: {
                "path": str(value),
                "sha256": hashlib.sha256(value.read_bytes()).hexdigest(),
            }
            for key, value in binaries.items()
        },
        "snapshot_hashes": {
            site: hashlib.sha256(
                (archive / ("snapshot-archive-" + site) / "snapshot.json").read_bytes()
            ).hexdigest()
            for site in args.sites
        },
        "method": "Fresh process; captured public DOM/CSS/assets; original scripts inert; 96 alternating structural edits and synchronous layout reads; rotating variant order.",
    }
    rows = []
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    for repeat in range(args.repetitions):
        variants = list(binaries)
        order = variants[repeat % 3 :] + variants[: repeat % 3]
        for site in args.sites:
            for variant in order:
                name = f"{repeat + 1:02d}-{site}-{variant}"
                folder = output / name
                print("running", name, flush=True)
                await one_run(
                    SimpleNamespace(
                        binary=binaries[variant],
                        site=site,
                        mode="snapshot",
                        archive=archive / ("snapshot-archive-" + site),
                        output=folder,
                        seconds=args.seconds,
                        cpus=args.cpus,
                    )
                )
                result = json.loads((folder / "result.json").read_text())
                assert (
                    result["binary_sha256"] == manifest["binaries"][variant]["sha256"]
                )
                assert result["snapshot_sha256"] == manifest["snapshot_hashes"][site]
                reference = next((r for r in rows if r["site"] == site), None)
                fingerprint = {
                    key: result["observation"][key]
                    for key in ["title", "nodes", "text", "boxes"]
                }
                if reference and (
                    fingerprint != reference["fingerprint"]
                    or result["pulse"]["geometry"] != reference["pulse_geometry"]
                ):
                    (folder / "comparison-error.txt").write_text(
                        "Input-equivalent runs produced different geometry or DOM."
                    )
                    raise RuntimeError(f"DOM or geometry mismatch: {name}")
                rows.append(
                    {
                        "run": name,
                        "variant": variant,
                        "site": site,
                        "metrics": metrics(folder, result),
                        "fingerprint": fingerprint,
                        "pulse_geometry": result["pulse"]["geometry"],
                    }
                )
                (output / "runs.json").write_text(
                    json.dumps(rows, indent=2, ensure_ascii=False)
                )
                (output / "summary.json").write_text(
                    json.dumps(summarize(rows), indent=2)
                )
    manifest["finished_at"] = datetime.datetime.now(datetime.UTC).isoformat()
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print("completed", len(rows), "runs", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["baseline", "slotmap", "dense", "archives", "output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--sites", nargs="+", default=["finance", "baidu", "rust"])
    parser.add_argument("--repetitions", type=int, default=5)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--cpus", default="")
    asyncio.run(compare(parser.parse_args()))
