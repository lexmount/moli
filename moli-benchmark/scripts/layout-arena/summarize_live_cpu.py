"""Expose CPU activity in the already published, unmodified live-page trials."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import statistics
from pathlib import Path

VARIANTS = ("baseline", "slotmap", "dense")


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    source = list(csv.DictReader(args.measurements.open()))
    source = [r for r in source if r["mode"] == "live"]
    rows = []
    for r in source:
        row = {k: r[k] for k in ["site", "variant", "repeat", "status"]}
        if r["status"] == "ok":
            row.update(
                initial_cpu_ms=float(r["init_cpu_ms"]),
                initial_wall_seconds=float(r["init_window_seconds"]),
                average_cpu_percent=float(r["init_cpu_ms"])
                / float(r["init_window_seconds"])
                / 10,
                load_in_window=int(r["load_in_window"]),
                root_nodes=int(r["nodes"]),
                root_http_status=int(r["root_http_status"]),
            )
        rows.append(row)
    metadata = {
        "source_sha256": hashlib.sha256(args.measurements.read_bytes()).hexdigest(),
        "base": "d7e0a70f28e42db46b04f716b750466c272bad02",
        "method": "Derived from previous live observations, not new runs. Process CPU user+system ticks divided by observed initialization window, 100%=one core. Includes waiting for network/workers/timers. Different content/loading progress can change work; not causal performance comparison.",
    }
    groups = {}
    for r in rows:
        if r["status"] == "ok":
            groups.setdefault((r["site"], r["variant"]), []).append(r)
    summary = []
    for (site, variant), rs in groups.items():
        summary.append(
            {
                "site": site,
                "variant": variant,
                "runs": len(rs),
                "average_cpu_percent": {
                    "median": statistics.median(r["average_cpu_percent"] for r in rs),
                    "min": min(r["average_cpu_percent"] for r in rs),
                    "max": max(r["average_cpu_percent"] for r in rs),
                },
                "initial_cpu_ms_median": statistics.median(
                    r["initial_cpu_ms"] for r in rs
                ),
                "load_events": sum(r["load_in_window"] for r in rs),
                "root_nodes_min": min(r["root_nodes"] for r in rs),
                "root_nodes_max": max(r["root_nodes"] for r in rs),
            }
        )
    with (args.output / "live-cpu.jsonl").open("w") as f:
        for row in (
            [{"kind": "metadata", **metadata}]
            + [{"kind": "run", **r} for r in rows]
            + [{"kind": "summary", **r} for r in summary]
        ):
            f.write(json.dumps(row) + "\n")
    with (args.output / "live-cpu.csv").open("w", newline="") as f:
        fields = [
            "site",
            "variant",
            "repeat",
            "status",
            "initial_cpu_ms",
            "initial_wall_seconds",
            "average_cpu_percent",
            "load_in_window",
            "root_nodes",
            "root_http_status",
        ]
        w = csv.DictWriter(f, fieldnames=fields, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    table = {(r["site"], r["variant"]): r for r in summary}
    sites = list(dict.fromkeys(r["site"] for r in rows))
    lines = [
        "# CPU activity on unmodified live pages",
        "",
        "This is a CPU-focused export of the 132 live trials in [the earlier memory report](results-extended-d7e0a70f.md); it adds no new live navigations. 126 observations across 27 sites are usable. Three HTTP 403/challenge cases, two network failures and one incomplete document stay excluded, with their rows retained.",
        "",
        "**100% means one CPU core.** Activity = process user+system CPU time divided by the measured initialization window. Windows are 8 seconds (20 for Finance and hh.ru), including network and timer waits. CPU time uses the existing Linux process ticks, which have 10 ms resolution. Both candidate binaries and the Vec baseline use affinity to the same two logical CPUs and identical settings.",
        "",
        "The pages execute their original scripts; there is no freezer, Fetch interception or injected benchmark. This measures observed process activity, not the CPU cost of an identical completed workload. Different assets, worker/iframe state and loading progress can change both CPU activity and memory. A lower number on an incomplete or less active page cannot establish an optimization. n=1 pages are exploratory; n=3 pages retain ranges. Do not average these percentages into a performance score.",
        "",
        "| Page | n Vec / Slot / Dense | CPU % Vec / Slot / Dense | CPU seconds Vec / Slot / Dense | load events Vec / Slot / Dense |",
        "|---|---:|---:|---:|---:|",
    ]
    for s in sites:
        if not all((s, v) in table for v in VARIANTS):
            continue
        rs = [table[(s, v)] for v in VARIANTS]
        lines.append(
            "| "
            + s
            + " | "
            + " / ".join(str(r["runs"]) for r in rs)
            + " | "
            + " / ".join(f"{r['average_cpu_percent']['median']:.1f}" for r in rs)
            + " | "
            + " / ".join(f"{r['initial_cpu_ms_median'] / 1000:.3f}" for r in rs)
            + " | "
            + " / ".join(str(r["load_events"]) for r in rs)
            + " |"
        )
    lines += [
        "",
        "Min/max activity ranges, CPU time, load counts, root DOM counts and failures are in `live-cpu.jsonl` / `live-cpu.csv`. Captured page bodies and event logs remain private. For fixed-work CPU comparisons, use [the controlled layout-change report](results-layout-changes-d7e0a70f.md).",
        "",
    ]
    (args.output / "results-live-cpu-d7e0a70f.md").write_text("\n".join(lines))
    print(
        "Exported",
        len(rows),
        "existing live observations,",
        len(summary),
        "site/variant summaries",
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("measurements", type=Path)
    p.add_argument("--output", type=Path, required=True)
    run(p.parse_args())
