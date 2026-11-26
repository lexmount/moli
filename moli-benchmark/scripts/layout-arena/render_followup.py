"""Render before/after CPU comparisons from the paired arena matrix."""

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

SCENARIOS = ("viewport", "container_width", "text", "display", "detach")
LABELS = ("Viewport", "Container width", "Text", "Display", "Detach / reinsert")


def render(folder):
    data = json.loads((folder / "followup.json").read_text())
    table = {(r["site"], r["scenario"], r["variant"]): r for r in data["summary"]}
    sites = list(dict.fromkeys(r["site"] for r in data["summary"]))
    sites = [
        site
        for site in sites
        if all(
            (site, s, v) in table
            for s in SCENARIOS
            for v in ["slotmap_before", "dense_before", "slotmap", "dense"]
        )
    ]
    if not sites:
        raise RuntimeError("No complete matched cohort")

    def value(site, scenario, variant):
        return table[(site, scenario, variant)]["metrics"]["cpu_ms_per_step"]["median"]

    plt.rcParams.update({"font.size": 10})
    fig, axes = plt.subplots(1, 2, figsize=(13, 6.8), sharey=True)
    last = None
    for ax, variant, title in zip(
        axes,
        ["slotmap", "dense"],
        ["SlotMap: after vs before", "DenseSlotMap: after vs before"],
        strict=True,
    ):
        matrix = np.array(
            [
                [
                    (value(site, s, variant) / value(site, s, variant + "_before") - 1)
                    * 100
                    for s in SCENARIOS
                ]
                for site in sites
            ]
        )
        last = ax.imshow(matrix, cmap="RdYlGn_r", vmin=-50, vmax=50, aspect="auto")
        ax.set_title(title)
        ax.set_xticks(range(5), LABELS, rotation=30, ha="right")
        ax.set_yticks(range(len(sites)), sites)
        for y in range(len(sites)):
            for x in range(5):
                ax.text(
                    x, y, f"{matrix[y, x]:+.1f}%", ha="center", va="center", fontsize=10
                )
        ax.tick_params(length=0)
    fig.suptitle(
        "Viewport retention and direct storage scans: process CPU time change",
        y=1.01,
        fontsize=14,
    )
    fig.colorbar(
        last,
        ax=axes.ravel().tolist(),
        fraction=0.025,
        pad=0.025,
        label="CPU time change (%); negative is faster",
    )
    fig.text(
        0.03,
        -0.075,
        "Same 5 binaries/cohort; 3 independent processes per page/variant; 8 updates per scenario.\nFrozen real DOM/CSS/assets; exact sampled geometry parity. Min/max and same-round ratios are retained in the report.\nThese are instrumented renderer workloads, pinned base d7e0a70f; small changes are not statistical proof.",
        fontsize=10,
    )
    fig.savefig(folder / "reuse-followup-cpu.png", dpi=170, bbox_inches="tight")
    path = folder / "reuse-followup-cpu.svg"
    fig.savefig(path, bbox_inches="tight")
    text = re.sub(r"\s+", " ", path.read_text()).replace("> <", "><").strip() + "\n"
    ET.fromstring(text)
    path.write_text(text)
    plt.close(fig)

    fig, ax = plt.subplots(figsize=(10, 7.8))
    fig.subplots_adjust(bottom=0.28)
    variants = ["baseline", "slotmap_before", "slotmap", "dense_before", "dense"]
    names = ["Vec", "Slot before", "Slot after", "Dense before", "Dense after"]
    colors = ["#444444", "#8ac2dc", "#176b94", "#f5bd8c", "#c65c14"]
    for offset, variant, name, color in zip(
        np.linspace(-0.28, 0.28, len(variants)), variants, names, colors, strict=True
    ):
        values = [
            max(
                table[(site, s, variant)]["metrics"]["rss_mib"]["median"]
                for s in SCENARIOS
            )
            for site in sites
        ]
        ax.scatter(
            values, np.arange(len(sites)) + offset, label=name, color=color, s=40
        )
    ax.set_yticks(range(len(sites)), sites)
    ax.invert_yaxis()
    ax.set_xlabel("Maximum scenario-boundary median RSS (MiB)")
    ax.set_title("Memory after viewport, width, text, display and detach updates")
    ax.grid(axis="x", alpha=0.25)
    handles, labels = ax.get_legend_handles_labels()
    fig.legend(handles, labels, ncols=3, loc="lower center", bbox_to_anchor=(0.5, 0.07))
    fig.text(
        0.05,
        0.015,
        "3 fresh processes per page/version; no forced GC or allocator trim.\nLater scenarios include earlier work; this does not measure long-term leaks.",
        fontsize=10,
    )
    fig.savefig(folder / "reuse-followup-memory.png", dpi=170, bbox_inches="tight")
    path = folder / "reuse-followup-memory.svg"
    fig.savefig(path, bbox_inches="tight")
    text = re.sub(r"\s+", " ", path.read_text()).replace("> <", "><").strip() + "\n"
    ET.fromstring(text)
    path.write_text(text)
    plt.close(fig)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", type=Path)
    render(parser.parse_args().folder)
