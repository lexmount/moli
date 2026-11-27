"""Render standalone figures for the controlled relayout comparison."""

from __future__ import annotations

import argparse
import json
import re
import xml.etree.ElementTree as ET
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np
from summarize_layout_changes import NAMES, SCENARIOS, SITES, TITLES, VARIANTS


def render(folder):
    if (folder / "layout-changes.json").exists():
        data = json.loads((folder / "layout-changes.json").read_text())
    else:
        records = [
            json.loads(line)
            for line in (folder / "layout-changes.jsonl").read_text().splitlines()
        ]
        data = {"summary": [r for r in records if r["kind"] == "summary"]}

    rows = data["summary"]
    table = {(r["site"], r["scenario"], r["variant"]): r for r in rows}
    sites = [
        s
        for s in dict.fromkeys(r["site"] for r in rows)
        if all(
            (s, scenario, variant) in table
            for scenario in SCENARIOS
            for variant in VARIANTS
        )
    ]
    if not sites:
        raise RuntimeError("No complete site/variant/scenario cohort to plot")

    def number(site, scenario, variant, metric):
        return table[(site, scenario, variant)]["metrics"][metric]["median"]

    def save(fig, name):
        fig.savefig(folder / (name + ".png"), dpi=170, bbox_inches="tight")
        svg = folder / (name + ".svg")
        fig.savefig(svg, bbox_inches="tight")
        text = re.sub(r"\s+", " ", svg.read_text()).replace("> <", "><").strip() + "\n"
        ET.fromstring(text)
        svg.write_text(text)
        plt.close(fig)

    plt.rcParams.update(
        {"font.size": 10, "axes.spines.top": False, "axes.spines.right": False}
    )
    fig, axes = plt.subplots(1, 5, figsize=(17, 5.8), sharey=True)
    last = None
    for ax, scenario in zip(axes, SCENARIOS, strict=True):
        matrix = np.array(
            [
                [
                    (
                        number(site, scenario, v, "cpu_ms_per_step")
                        / number(site, scenario, "baseline", "cpu_ms_per_step")
                        - 1
                    )
                    * 100
                    for v in VARIANTS[1:]
                ]
                for site in sites
            ]
        )
        last = ax.imshow(matrix, cmap="RdYlGn_r", vmin=-30, vmax=30, aspect="auto")
        ax.set_xticks([0, 1], ["SlotMap", "DenseSlotMap"], rotation=20)
        ax.set_yticks(np.arange(len(sites)), [SITES[s] for s in sites])
        ax.set_title(TITLES[scenario].replace(" / ", "\n"), fontsize=11)
        for y in range(len(sites)):
            for x in range(2):
                ax.text(
                    x,
                    y,
                    f"{matrix[y, x]:+.1f}%",
                    ha="center",
                    va="center",
                    fontsize=9,
                    color="black",
                )
        ax.tick_params(axis="both", length=0)
    fig.suptitle(
        "Real-page relayout CPU change vs Vec (lower is better)", fontsize=15, y=1.02
    )
    fig.colorbar(
        last,
        ax=axes.ravel().tolist(),
        label="CPU time change (%)",
        fraction=0.018,
        pad=0.025,
    )
    fig.text(
        0.04,
        -0.035,
        "3 fresh processes per variant/page; 32 steps per scenario (Wiki: 8); exact sampled geometry parity.\nFrozen page scripts; pinned base d7e0a70f. Relative CPU time, not CPU utilization or live app initialization.",
        fontsize=10,
    )
    save(fig, "layout-changes-cpu")
    fig, axes = plt.subplots(1, 2, figsize=(14, 7))
    y = np.arange(len(sites))
    colors = ["#606c76", "#347ba0", "#cc8140"]
    for j, v in enumerate(VARIANTS):
        cpu = np.array([number(s, "viewport", v, "cpu_ms_per_step") for s in sites])
        lo = np.array(
            [
                table[(s, "viewport", v)]["metrics"]["cpu_ms_per_step"]["min"]
                for s in sites
            ]
        )
        hi = np.array(
            [
                table[(s, "viewport", v)]["metrics"]["cpu_ms_per_step"]["max"]
                for s in sites
            ]
        )
        axes[0].barh(
            y + (j - 1) * 0.23,
            cpu,
            height=0.22,
            label=NAMES[v],
            color=colors[j],
            xerr=[cpu - lo, hi - cpu],
            error_kw={"lw": 0.7, "capsize": 2},
        )
        rss = [number(s, "viewport", v, "rss_mib") for s in sites]
        axes[1].plot(
            rss, y + (j - 1) * 0.17, "o", label=NAMES[v], color=colors[j], markersize=5
        )
    axes[0].set_yticks(y, [SITES[s] for s in sites])
    axes[0].invert_yaxis()
    axes[1].set_yticks(y, [SITES[s] for s in sites])
    axes[1].invert_yaxis()
    axes[0].set_xscale("log")
    axes[0].set_xlabel("Process CPU ms / viewport step (log scale; min/max bars)")
    axes[1].set_xlabel("RSS MiB after viewport workload")
    axes[0].set_title("Desktop ↔ tablet ↔ phone viewport resize")
    axes[1].set_title("Observed resident memory; no forced GC")
    for ax in axes:
        ax.grid(axis="x", alpha=0.2)
        ax.legend(loc="lower right")
    fig.tight_layout()
    fig.text(
        0.03,
        -0.035,
        "Medians of 3 runs; 100% CPU activity = one core (activity itself is reported in the data table).\n1280×720 / 1024×768 / 768×1024 / 390×844; DPR 1, mobile:false; same captured DOM/CSS/assets.",
        fontsize=10,
    )
    save(fig, "layout-changes-viewport")


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("folder", type=Path)
    render(p.parse_args().folder)
