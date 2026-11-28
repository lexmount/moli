"""Export numeric real-page relayout comparisons without captured page content."""

from __future__ import annotations

import argparse
import csv
import json
import math
import statistics
from pathlib import Path

VARIANTS = ("baseline", "slotmap", "dense")
NAMES = {"baseline": "Vec", "slotmap": "SlotMap", "dense": "DenseSlotMap"}
SCENARIOS = ("viewport", "container_width", "text", "display", "detach")
TITLES = {
    "viewport": "Viewport resize",
    "container_width": "Container width / wrapping",
    "text": "Visible text updates",
    "display": "Display none / restore",
    "detach": "Detach / reinsert",
}
SITES = {
    "finance": "Baidu Finance",
    "baidu": "Baidu",
    "rust": "Rust homepage",
    "pypi": "PyPI requests",
    "hn": "Hacker News",
    "zhihu": "Zhihu",
    "wikipedia": "Wikipedia Rust",
    "python_docs": "Python tutorial",
    "rust_book": "Rust book",
    "typescript": "TypeScript handbook",
}
FIELDS = (
    "cpu_ms_per_step",
    "wall_ms_per_step",
    "js_ms_per_step",
    "average_cpu_percent",
    "rss_mib",
    "pss_mib",
    "kernel_peak_rss_mib",
    "rss_before_mib",
)


def dist(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


def summarize(args):
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((args.input / "manifest.json").read_text())
    rows = json.loads((args.input / "runs.json").read_text())
    groups = {}
    public = []
    for r in rows:
        safe = {
            k: r[k] for k in ["run", "site", "variant", "repeat", "status", "seconds"]
        }
        if r["status"] != "ok":
            safe["error"] = r.get("error", "failed")
            public.append(safe)
            continue
        safe["metrics"] = r["metrics"]
        safe.update({k: r[k] for k in ["idle_rss_mib", "closed_rss_mib"]})
        public.append(safe)
        for m in r["metrics"]:
            groups.setdefault((r["site"], m["scenario"], r["variant"]), []).append(m)
    summary = []
    for (site, scenario, variant), values in groups.items():
        summary.append(
            {
                "site": site,
                "scenario": scenario,
                "variant": variant,
                "runs": len(values),
                "steps_per_run": values[0]["steps"],
                "metrics": {f: dist([v[f] for v in values]) for f in FIELDS},
            }
        )
    table = {(r["site"], r["scenario"], r["variant"]): r for r in summary}
    sites = [p["site"] for p in manifest["plan"]]

    def med(site, scenario, variant, metric):
        return table[(site, scenario, variant)]["metrics"][metric]["median"]

    complete = [
        s
        for s in sites
        if all(
            (s, c, v) in table and table[(s, c, v)]["runs"] == manifest["repetitions"]
            for c in SCENARIOS
            for v in VARIANTS
        )
    ]
    paired = []
    round_values = {}
    for row in rows:
        if row["status"] != "ok":
            continue
        for metric in row["metrics"]:
            round_values[
                (row["site"], metric["scenario"], row["repeat"], row["variant"])
            ] = metric["cpu_ms_per_step"]
    for site in complete:
        for scenario in SCENARIOS:
            for variant in VARIANTS[1:]:
                values = [
                    (
                        round_values[(site, scenario, repeat, variant)]
                        / round_values[(site, scenario, repeat, "baseline")]
                        - 1
                    )
                    * 100
                    for repeat in range(1, manifest["repetitions"] + 1)
                ]
                paired.append(
                    {
                        "site": site,
                        "scenario": scenario,
                        "variant": variant,
                        "cpu_delta_percent": dist(values),
                        "round_deltas_percent": values,
                    }
                )
    paired_table = {(r["site"], r["scenario"], r["variant"]): r for r in paired}

    def percent(value):
        return f"{value:+.1f}%" if value is not None else "n/a"

    geo = []
    for c in SCENARIOS:
        item = {"scenario": c, "sites": len(complete)}
        for variant in VARIANTS[1:]:
            ratios = [
                med(s, c, variant, "cpu_ms_per_step")
                / med(s, c, "baseline", "cpu_ms_per_step")
                for s in complete
            ]
            item[variant + "_cpu_delta_percent"] = (
                (math.exp(statistics.mean(math.log(r) for r in ratios)) - 1) * 100
                if ratios
                else None
            )
        geo.append(item)
    data = {
        "metadata": {
            "base": "d7e0a70f28e42db46b04f716b750466c272bad02",
            "binary_sha256": manifest["binary_sha256"],
            "control_sha256": manifest["control_sha256"],
            "inputs": manifest["inputs"],
            "repetitions": manifest["repetitions"],
            "steps_normal": manifest["steps_normal"],
            "steps_large": manifest["steps_large"],
            "cpus": manifest["cpus"],
            "platform": manifest["platform"],
            "started_at": manifest["started_at"],
            "finished_at": manifest.get("finished_at"),
            "method": manifest["method"],
            "viewport_sequence": manifest["viewport_sequence"],
            "scenario_evaluations": sum(
                len(r.get("metrics", [])) for r in rows if r["status"] == "ok"
            ),
            "timed_steps": sum(
                m["steps"] for r in rows if r["status"] == "ok" for m in r["metrics"]
            ),
        },
        "runs": public,
        "summary": summary,
        "geometric_cpu_deltas": geo,
        "paired_cpu_changes": paired,
    }
    (args.output / "layout-changes.json").write_text(json.dumps(data, indent=2))
    with (args.output / "layout-changes.jsonl").open("w") as f:
        for row in (
            [{"kind": "metadata", **data["metadata"]}]
            + [{"kind": "run", **r} for r in public]
            + [{"kind": "summary", **r} for r in summary]
            + [{"kind": "geometric_cpu_delta", **r} for r in geo]
            + [{"kind": "paired_cpu_change", **r} for r in paired]
        ):
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
    with (args.output / "layout-changes.csv").open("w", newline="") as f:
        writer = csv.DictWriter(
            f,
            fieldnames=[
                "run",
                "site",
                "variant",
                "repeat",
                "status",
                "scenario",
                "steps",
                *FIELDS,
            ],
            lineterminator="\n",
        )
        writer.writeheader()
        for r in rows:
            for m in r.get("metrics", []) or [{}]:
                writer.writerow(
                    {
                        **{
                            k: r[k]
                            for k in ["run", "site", "variant", "repeat", "status"]
                        },
                        **{k: m.get(k) for k in ["scenario", "steps", *FIELDS]},
                    }
                )
    lines = [
        "# Real-page relayout: Vec / SlotMap / DenseSlotMap",
        "",
        f"{sum(r['status'] == 'ok' for r in rows)}/{len(rows)} fresh-process trials passed original-capture fidelity, per-step cross-variant geometry and final-restoration checks. "
        + f"{len(complete)} pages have all {manifest['repetitions']} balanced-order rounds for every variant. The engine base and three release binaries are unchanged from the earlier reports.",
        "",
        "This adds actual layout changes to the previous zero-height structural pulse. The same captured DOM, CSS and assets are replayed with application scripts inert. It measures renderer relayout, not live application resize handlers or network/initialization performance. All variants enable the same native layout tracing. Comparisons against Vec include the shared stable-identity migration; only SlotMap versus DenseSlotMap isolates the storage alias. These are Moli-versus-Moli comparisons; Chromium is not measured here.",
        "",
        "Each process warms each scenario once, then performs "
        + str(manifest["steps_normal"])
        + " measured steps ("
        + str(manifest["steps_large"])
        + " for the 9,920-element Wiki article). Scenarios run in the same fixed sequence in all variants:",
        "",
        "- Viewport: 1280×720 → 1024×768 → 768×1024 → 390×844 → 1280×720, repeating, DPR 1 and `mobile:false`. This changes layout/media-query constraints without changing the UA or emulating a mobile device.",
        "- Container width: body width 960 / 640 / 1180 px / original, forcing wrapping on actual page content.",
        "- Text: duplicate/triplicate 20 visible text nodes (or all available if fewer), then restore the original text.",
        "- Display: hide a visible main-content subtree with `display:none`, then restore its original style.",
        "- Detach: remove and reinsert the same main-content DOM subtree at its original location, exercising retired box identities and slot reuse.",
        "",
        "Every step synchronously reads body and target bounds. Up to 1,000 visible element rectangles, body/target bounds, viewport, element count and text length are checked exactly between variants/repetitions, outside the timed window. Every scenario must change its observed state and restore the exact starting state. Original-capture geometry must still match before the workload, with no missing assets or substitutions. No geometry tolerance, forced GC or allocator trim is used.",
        "",
        "CPU ms/step comes from the POSIX process CPU clock (nanosecond API, all threads), avoiding `/proc/stat`’s 10 ms quantization. Average CPU % = measured CPU time / elapsed time in those same per-step windows × 100; **100% is one core**, with affinity to two logical CPUs. Wall time includes the CDP update, synchronous layout and resource-probe overhead; geometry inspection and between-step idle are excluded. Inspection still allocates in the engine, and deferred GC/background work during a timed window is included in process CPU. CPU % measures activity, while CPU time measures work: similar near-100% busy values can still have very different step durations.",
        "",
        "RSS/PSS are sampled every 100 ms and at phase boundaries. Scenario RSS below is after its final measured step; peak RSS is the kernel high-water since process start, not a per-scenario peak. Scenarios share a process in fixed order, so later memory points include earlier work. Numeric data includes min/max ranges, all formal trials, idle/after-close residency and hashes; response bodies, identifiers and page text remain local.",
        "",
        "## CPU change by scenario",
        "",
        "Equal-page geometric mean of per-page median CPU ratios; positive is slower. This is not a traffic-weighted total.",
        "",
        "| Scenario | Pages | SlotMap CPU Δ | DenseSlotMap CPU Δ |",
        "|---|---:|---:|---:|",
    ]
    for g in geo:
        lines.append(
            f"| {TITLES[g['scenario']]} | {g['sites']} | {percent(g['slotmap_cpu_delta_percent'])} | {percent(g['dense_cpu_delta_percent'])} |"
        )
    for scenario in SCENARIOS:
        lines += [
            "",
            f"## {TITLES[scenario]}",
            "",
            "Medians of independent trials. Times are process CPU ms/step; CPU activity uses the same timed windows.",
            "",
            "| Page | Vec CPU | Slot CPU | Dense CPU | Slot Δ | Dense Δ | CPU % Vec / Slot / Dense | RSS MiB Vec / Slot / Dense |",
            "|---|---:|---:|---:|---:|---:|---:|---:|",
        ]
        for site in complete:
            cpu = [med(site, scenario, v, "cpu_ms_per_step") for v in VARIANTS]
            activity = [med(site, scenario, v, "average_cpu_percent") for v in VARIANTS]
            rss = [med(site, scenario, v, "rss_mib") for v in VARIANTS]
            lines.append(
                f"| {SITES[site]} | {cpu[0]:.3f} | {cpu[1]:.3f} | {cpu[2]:.3f} | {(cpu[1] / cpu[0] - 1) * 100:+.1f}% | {(cpu[2] / cpu[0] - 1) * 100:+.1f}% | "
                + " / ".join(f"{v:.1f}" for v in activity)
                + " | "
                + " / ".join(f"{v:.2f}" for v in rss)
                + " |"
            )
    lines += [
        "",
        "## Same-round CPU ratios",
        "",
        "To expose temporal drift, these are median (min … max) CPU percentage changes versus Vec within the same page/scenario/round. All three original rounds remain included. These can differ from ratios of absolute medians above. Wide or zero-crossing ranges are not evidence of a stable improvement.",
        "",
        "| Page | Scenario | SlotMap median (range) | DenseSlotMap median (range) |",
        "|---|---|---:|---:|",
    ]
    for site in complete:
        for scenario in SCENARIOS:
            cells = []
            for variant in VARIANTS[1:]:
                d = paired_table[(site, scenario, variant)]["cpu_delta_percent"]
                cells.append(f"{d['median']:+.1f}% ({d['min']:+.1f} … {d['max']:+.1f})")
            lines.append(
                f"| {SITES[site]} | {TITLES[scenario]} | " + " | ".join(cells) + " |"
            )
    lines += [
        "",
        "## Ranges and failures",
        "",
        "The independent trials per variant are a bounded local comparison, not statistical proof. Variant order rotates, but workload phases stay in fixed order. Min/max CPU and wall ranges are retained in the JSONL. The same binaries remain pinned to main d7e0a70f; any merge needs rebasing and revalidation on current main.",
        "",
    ]
    for r in rows:
        if r["status"] != "ok":
            lines.append(f"- {r['run']}: {r.get('error', 'failed')}")
    if all(r["status"] == "ok" for r in rows):
        lines.append("No formal trial failed.")
    lines += [
        "",
        "Admission/preflight runs are retained separately and excluded from formal medians. The initial Rust smoke passed all variants. Ten first-attempt admissions stopped on an unavailable Python CPU-clock helper before the workload; using the libc POSIX clock API fixed the harness. All ten subsequent Vec admissions passed every scenario with no asset misses, and no additional assets or exclusions were needed. Harness development failures are not successful engine measurements.",
        "",
        "## Executables",
        "",
        "| Variant | SHA-256 |",
        "|---|---|",
    ]
    for v in VARIANTS:
        if v in manifest["binary_sha256"]:
            lines.append(f"| {NAMES[v]} | `{manifest['binary_sha256'][v]}` |")
    lines += [
        "",
        "![CPU by workload](layout-changes-cpu.png)",
        "",
        "![Viewport CPU and memory](layout-changes-viewport.png)",
        "",
    ]
    (args.output / "results-layout-changes-d7e0a70f.md").write_text("\n".join(lines))
    print(
        json.dumps(
            {
                "runs": len(rows),
                "passed": sum(r["status"] == "ok" for r in rows),
                "complete_pages": len(complete),
                "geometric_cpu_deltas": geo,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("input", type=Path)
    p.add_argument("--output", type=Path, required=True)
    summarize(p.parse_args())
