"""Export the paired old/new arena relayout matrix without page content."""

from __future__ import annotations

import argparse
import csv
import json
import math
import statistics
from pathlib import Path

V = ("baseline", "slotmap_before", "dense_before", "slotmap", "dense")
S = ("viewport", "container_width", "text", "display", "detach")
NAMES = {
    "baseline": "Vec",
    "slotmap_before": "SlotMap before",
    "dense_before": "DenseSlotMap before",
    "slotmap": "SlotMap after",
    "dense": "DenseSlotMap after",
}
TITLES = {
    "viewport": "Viewport resize",
    "container_width": "Container width",
    "text": "Text changes",
    "display": "Display changes",
    "detach": "Detach / reinsert",
}
FIELDS = (
    "cpu_ms_per_step",
    "wall_ms_per_step",
    "average_cpu_percent",
    "rss_mib",
    "js_heap_used_mib",
    "js_heap_total_mib",
    "pss_mib",
    "kernel_peak_rss_mib",
    "retained_boxes_per_update",
    "reused_ifcs_per_update",
    "retained_tree_passes_per_update",
    "box_tree_ms_per_update",
    "native_layout_ms_per_update",
    "numeric_ms_per_update",
    "inline_preparation_ms_per_update",
    "projection_ms_per_update",
)


def distribution(a):
    return {"median": statistics.median(a), "min": min(a), "max": max(a)}


def run(args):
    args.output.mkdir(parents=True, exist_ok=False)
    m = json.loads((args.input / "manifest.json").read_text())
    raw = json.loads((args.input / "runs.json").read_text())
    rows = []
    groups = {}
    paired = {}
    for r in raw:
        row = {
            k: r[k] for k in ["run", "site", "variant", "repeat", "status", "seconds"]
        }
        if r["status"] != "ok":
            row["error"] = r.get("error", "failed")
            rows.append(row)
            continue
        row["metrics"] = []
        row.update({k: r[k] for k in ["idle_rss_mib", "closed_rss_mib"]})
        for metric in r["metrics"]:
            x = dict(metric)
            n = x["native"]
            steps = x["steps"]
            x.update(
                retained_boxes_per_update=n["retained_box_visits"] / steps,
                reused_ifcs_per_update=n["reused_inline_contexts"] / steps,
                retained_tree_passes_per_update=n["retained_tree_passes"] / steps,
                box_tree_ms_per_update=n["phase_ms"]["box_tree_us"] / steps,
                native_layout_ms_per_update=n["layout_ms"] / steps,
                numeric_ms_per_update=n["phase_ms"]["numeric_layout_us"] / steps,
                inline_preparation_ms_per_update=n["phase_ms"]["inline_preparation_us"]
                / steps,
                projection_ms_per_update=n["phase_ms"]["projection_us"] / steps,
            )
            row["metrics"].append(x)
            groups.setdefault((r["site"], x["scenario"], r["variant"]), []).append(x)
            paired[(r["site"], x["scenario"], r["repeat"], r["variant"])] = x[
                "cpu_ms_per_step"
            ]
        rows.append(row)
    summary = [
        {
            "site": site,
            "scenario": scenario,
            "variant": variant,
            "runs": len(a),
            "metrics": {key: distribution([x[key] for x in a]) for key in FIELDS},
        }
        for (site, scenario, variant), a in groups.items()
    ]
    table = {(r["site"], r["scenario"], r["variant"]): r for r in summary}
    sites = [
        p["site"]
        for p in m["plan"]
        if all(
            (p["site"], s, v) in table
            and table[(p["site"], s, v)]["runs"] == m["repetitions"]
            for s in S
            for v in V
        )
    ]

    if not sites:
        raise RuntimeError("No complete matched cohort for all five variants")

    def med(site, scenario, variant, key="cpu_ms_per_step"):
        return table[(site, scenario, variant)]["metrics"][key]["median"]

    geo = []
    changes = []
    for scenario in S:
        item = {"scenario": scenario, "pages": len(sites)}
        for v, base in [
            ("slotmap", "slotmap_before"),
            ("dense", "dense_before"),
            ("slotmap", "baseline"),
            ("dense", "baseline"),
        ]:
            ratios = [
                med(site, scenario, v) / med(site, scenario, base) for site in sites
            ]
            item[v + "_vs_" + base + "_percent"] = (
                (math.exp(statistics.mean(math.log(x) for x in ratios)) - 1) * 100
                if ratios
                else None
            )
            item[v + "_vs_" + base + "_paired_rounds"] = (
                [
                    (
                        math.exp(
                            statistics.mean(
                                math.log(
                                    paired[(site, scenario, r, v)]
                                    / paired[(site, scenario, r, base)]
                                )
                                for site in sites
                            )
                        )
                        - 1
                    )
                    * 100
                    for r in range(1, m["repetitions"] + 1)
                ]
                if sites
                else []
            )
        geo.append(item)
    for site in sites:
        for scenario in S:
            for v, base in [("slotmap", "slotmap_before"), ("dense", "dense_before")]:
                a = [
                    (
                        paired[(site, scenario, r, v)]
                        / paired[(site, scenario, r, base)]
                        - 1
                    )
                    * 100
                    for r in range(1, m["repetitions"] + 1)
                ]
                changes.append(
                    {
                        "site": site,
                        "scenario": scenario,
                        "variant": v,
                        "reference": base,
                        "paired_cpu_percent": distribution(a),
                        "rounds": a,
                    }
                )
    metadata = {
        key: m[key]
        for key in [
            "started_at",
            "finished_at",
            "repetitions",
            "cpus",
            "platform",
            "binary_sha256",
            "control_sha256",
            "inputs",
            "steps_normal",
            "steps_large",
            "viewport_sequence",
        ]
    }
    metadata.update(
        base="d7e0a70f28e42db46b04f716b750466c272bad02",
        fresh_processes=len(raw),
        successful_processes=sum(r["status"] == "ok" for r in raw),
        timed_updates=sum(x["steps"] for r in rows for x in r.get("metrics", [])),
    )
    data = {
        "metadata": metadata,
        "summary": summary,
        "runs": rows,
        "aggregate_changes": geo,
        "paired_changes": changes,
    }
    (args.output / "followup.json").write_text(json.dumps(data, indent=2) + "\n")
    with (args.output / "reuse-followup.jsonl").open("w") as f:
        for x in (
            [{"kind": "metadata", **metadata}]
            + [{"kind": "run", **r} for r in rows]
            + [{"kind": "summary", **r} for r in summary]
            + [{"kind": "aggregate", **g} for g in geo]
            + [{"kind": "paired_change", **r} for r in changes]
        ):
            f.write(json.dumps(x) + "\n")
    with (args.output / "reuse-followup.csv").open("w", newline="") as f:
        w = csv.DictWriter(
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
        w.writeheader()
        for r in rows:
            for x in r.get("metrics", []) or [{}]:
                w.writerow(
                    {
                        **{
                            key: r[key]
                            for key in ["run", "site", "variant", "repeat", "status"]
                        },
                        **{key: x.get(key) for key in ["scenario", "steps", *FIELDS]},
                    }
                )
    lines = [
        "# Viewport retention and direct storage scans",
        "",
        f"{metadata['successful_processes']}/{len(raw)} fresh-process trials; {len(sites)} admitted page snapshots; {metadata['timed_updates']:,} measured updates. Vec, both previous arena binaries and both updated binaries are tested together in the same cohort.",
        "",
        "Viewport changes retain compatible box identities and shaped IFC inputs, revalidate all styles, and invalidate old numeric constraints. Root style changes can refresh compatible containers. Clearly unordered scans now use storage iteration; numeric feedback snapshots restore the same physical order with a revision check. IFC owner indices use compact generational arrays. Geometry publication keeps the existing ordered traversal and parent-ID conversion. No new numeric IDs or dense-reference array are introduced.",
        "",
        "The same initialized DOM, CSS and assets are replayed with application scripts inert. All five versions use fresh processes, rotated order, identical settings, DPR 1, and the same five scenario families as the prior report. Each scenario warms once, then runs 8 updates (two cycles). These bounded renderer workloads do not measure live application resize handlers or long-term leaks.",
        "",
        "Each update forces geometry. Up to 1,000 visible rectangles, body/target bounds, viewport and DOM/text counts match exactly between variants and rounds. Each family restores its starting state. Missing assets, failures and geometry differences remain failed records. No forced GC or allocator trim is used.",
        "",
        "CPU comes from the POSIX all-thread process clock; CPU activity uses 100%=one logical CPU. Full geometry collection is outside timed steps, but deferred/background work inside those windows is counted. RSS/PSS are sampled every 100 ms and at boundaries. Later memory phases include earlier work; kernel high-water is since process startup. Small differences and broad/zero-crossing paired ranges are not statistical proof.",
        "",
        "## CPU changes",
        "",
        "Equal-page geometric mean of per-page median CPU ratios. Negative is faster. Before/after percentages compare the same backend within this new cohort, not historical measurements.",
        "",
        "| Scenario | Slot after vs before | Dense after vs before | Slot after vs Vec | Dense after vs Vec |",
        "|---|---:|---:|---:|---:|",
    ]
    for g in geo:
        vals = [
            g[k]
            for k in [
                "slotmap_vs_slotmap_before_percent",
                "dense_vs_dense_before_percent",
                "slotmap_vs_baseline_percent",
                "dense_vs_baseline_percent",
            ]
        ]
        lines.append(
            "| "
            + TITLES[g["scenario"]]
            + " | "
            + " | ".join(f"{x:+.1f}%" if x is not None else "n/a" for x in vals)
            + " |"
        )
    memory_changes = {}
    for v, base in [("slotmap", "slotmap_before"), ("dense", "dense_before")]:
        delta = [
            max(med(site, s, v, "rss_mib") for s in S)
            - max(med(site, s, base, "rss_mib") for s in S)
            for site in sites
        ]
        memory_changes[v] = distribution(delta)
    consistent = [
        TITLES[g["scenario"]]
        for g in geo
        if all(
            x < 0
            for v, base in [("slotmap", "slotmap_before"), ("dense", "dense_before")]
            for x in g[v + "_vs_" + base + "_paired_rounds"]
        )
    ]
    memory_case = "wikipedia" if "wikipedia" in sites else sites[0]
    lines += [
        "",
        "## Reading the results",
        "",
        "Aggregate same-backend CPU time improves in every paired round for both backends for: "
        + (", ".join(consistent) or "none")
        + ". See the paired-round table for the direction and range of other changes; this does not isolate an intrinsic container effect.",
        "",
        "There is no broad RSS reduction. Across pages, the median change in maximum scenario-boundary RSS is "
        + "; ".join(
            f"{NAMES[v]} {d['median']:+.2f} MiB (page range {d['min']:+.2f} … {d['max']:+.2f})"
            for v, d in memory_changes.items()
        )
        + ".",
        "",
        f"Viewport residence can increase substantially earlier in the sequence. For {memory_case}, "
        + "; ".join(
            f"{NAMES[v]} viewport-boundary RSS {med(memory_case, 'viewport', base, 'rss_mib'):.2f} → {med(memory_case, 'viewport', v, 'rss_mib'):.2f} MiB, maximum across all scenario boundaries {max(med(memory_case, s, base, 'rss_mib') for s in S):.2f} → {max(med(memory_case, s, v, 'rss_mib') for s in S):.2f} MiB"
            for v, base in [("slotmap", "slotmap_before"), ("dense", "dense_before")]
        )
        + ". Both observations matter when choosing a CPU/memory tradeoff.",
    ]
    lines += [
        "",
        "## Memory across the five scenarios",
        "",
        "MiB; the maximum of the five scenario-boundary RSS medians for each page/version. This is a bounded workload with no forced GC, not a retained-object or long-term leak measurement. PSS, kernel high-water and JS heap used/total are also exported per scenario in the numeric files.",
        "",
        "| Page | Vec | Slot before | Slot after | Dense before | Dense after |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for site in sites:
        values = [
            max(med(site, s, v, "rss_mib") for s in S)
            for v in ["baseline", "slotmap_before", "slotmap", "dense_before", "dense"]
        ]
        lines.append(
            "| " + site + " | " + " | ".join(f"{x:.2f}" for x in values) + " |"
        )
    for scenario in S:
        lines += [
            "",
            f"## {TITLES[scenario]}",
            "",
            "Process CPU ms per update; medians of 3 independent processes per variant/page.",
            "",
            "| Page | Vec | Slot before → after | Dense before → after | Slot RSS before → after MiB | Dense RSS before → after MiB |",
            "|---|---:|---:|---:|---:|---:|",
        ]
        for site in sites:
            values = [med(site, scenario, v) for v in V]
            rss = [med(site, scenario, v, "rss_mib") for v in V]
            lines.append(
                f"| {site} | {values[0]:.3f} | {values[1]:.3f} → {values[3]:.3f} | {values[2]:.3f} → {values[4]:.3f} | {rss[1]:.2f} → {rss[3]:.2f} | {rss[2]:.2f} → {rss[4]:.2f} |"
            )
    lines += [
        "",
        "## Work retained during viewport updates",
        "",
        "Average retained boxes / reused IFC contexts per update, then median across runs. Numeric outputs are invalidated on viewport changes; preserving shaped inputs does not authorize reuse of old sizes.",
        "",
        "| Page | Slot before | Slot after | Dense before | Dense after |",
        "|---|---:|---:|---:|---:|",
    ]
    for site in sites:
        vals = [
            f"{med(site, 'viewport', v, 'retained_boxes_per_update'):.1f} / {med(site, 'viewport', v, 'reused_ifcs_per_update'):.1f}"
            for v in V[1:]
        ]
        lines.append("| " + site + " | " + " | ".join(vals) + " |")
    lines += [
        "",
        "## Viewport native phase timings",
        "",
        "Median native wall milliseconds per update, before → after. These instrumented wall timers locate work; they do not partition all-thread process CPU time. The bundle includes viewport reuse, direct scans and compact IFC owner lookup, so this does not isolate an intrinsic container effect.",
        "",
        "| Page | Backend | Box tree | Inline preparation | Numeric layout | Projection |",
        "|---|---|---:|---:|---:|---:|",
    ]
    for site in sites:
        for v, base in [("slotmap", "slotmap_before"), ("dense", "dense_before")]:
            cells = [
                f"{med(site, 'viewport', base, key):.3f} → {med(site, 'viewport', v, key):.3f}"
                for key in [
                    "box_tree_ms_per_update",
                    "inline_preparation_ms_per_update",
                    "numeric_ms_per_update",
                    "projection_ms_per_update",
                ]
            ]
            lines.append(
                "| " + site + " | " + NAMES[v] + " | " + " | ".join(cells) + " |"
            )
    lines += [
        "",
        "## Aggregate paired rounds",
        "",
        "Equal-page geometric mean of each round's same-backend before/after CPU ratios. All three rounds are shown; these are not confidence intervals.",
        "",
        "| Scenario | SlotMap rounds 1 / 2 / 3 | DenseSlotMap rounds 1 / 2 / 3 |",
        "|---|---:|---:|",
    ]
    for g in geo:
        cells = [
            " / ".join(f"{x:+.1f}%" for x in g[v + "_vs_" + base + "_paired_rounds"])
            for v, base in [("slotmap", "slotmap_before"), ("dense", "dense_before")]
        ]
        lines.append("| " + TITLES[g["scenario"]] + " | " + " | ".join(cells) + " |")
    lines += [
        "",
        "## Same-round CPU ratios",
        "",
        "Median (min … max) percentage change versus the same backend before the followup. All original trials remain included.",
        "",
        "| Page | Scenario | SlotMap | DenseSlotMap |",
        "|---|---|---:|---:|",
    ]
    ct = {(r["site"], r["scenario"], r["variant"]): r for r in changes}
    for site in sites:
        for scenario in S:
            cells = []
            for v in ["slotmap", "dense"]:
                d = ct[(site, scenario, v)]["paired_cpu_percent"]
                cells.append(f"{d['median']:+.1f}% ({d['min']:+.1f} … {d['max']:+.1f})")
            lines.append(
                "| "
                + site
                + " | "
                + TITLES[scenario]
                + " | "
                + " | ".join(cells)
                + " |"
            )
    lines += [
        "",
        "## Validation and scope",
        "",
        "The Rust variants independently pass formatting, workspace/all-target/all-feature Clippy and full default Nextest before their Rust commits. Exact counts and source revisions are recorded in the PR. Snapshot admission is separate from formal medians. All binary/control/input hashes, run-level observations, phase timings, CPU activity and RSS/PSS ranges are retained in the numeric files; original bodies/cookies stay local. The base remains d7e0a70f, so current-main synchronization and revalidation are required before merging either alternative.",
        "",
        "![Paired CPU changes](reuse-followup-cpu.png)",
        "",
        "![Memory comparison](reuse-followup-memory.png)",
        "",
    ]
    for r in rows:
        if r["status"] != "ok":
            lines.append(f"- Failed: {r['run']}: {r.get('error', 'failed')}")
    (args.output / "results-reuse-followup-d7e0a70f.md").write_text("\n".join(lines))
    print(
        json.dumps(
            {
                "processes": len(raw),
                "passed": metadata["successful_processes"],
                "updates": metadata["timed_updates"],
                "aggregate_changes": geo,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("input", type=Path)
    p.add_argument("--output", type=Path, required=True)
    run(p.parse_args())
