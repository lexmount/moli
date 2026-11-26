"""Render standalone memory/CPU figures after measurements finish."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

LABELS = {
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
    "google": "Google",
    "bing": "Bing",
    "github": "GitHub",
    "mdn": "MDN",
    "bbc": "BBC",
    "cnn": "CNN",
    "sina": "Sina News",
    "163": "NetEase News",
    "qq": "Tencent News",
    "sohu": "Sohu",
    "douban": "Douban",
    "v2ex": "V2EX",
    "bilibili": "Bilibili",
    "jd": "JD",
    "taobao": "Taobao",
    "hh": "hh.ru",
    "vue": "Vue docs",
    "react": "React Learn",
}
COLORS = {"baseline": "#596579", "slotmap": "#25876d", "dense": "#cc7831"}
NAMES = {"baseline": "Vec", "slotmap": "SlotMap", "dense": "DenseSlotMap"}


def plot_static(report):
    data = json.loads((report / "snapshots-summary.json").read_text())
    sites = sorted(
        data,
        key=lambda site: data[site]["baseline"]["metrics"]["idle_rss_mib"]["median"],
        reverse=True,
    )
    y = np.arange(len(sites))
    fig, axes = plt.subplots(
        1, 3, figsize=(16, 6), gridspec_kw={"width_ratios": [1.6, 1.1, 1.3]}
    )
    for index, v in enumerate(COLORS):
        vals = [data[site][v]["metrics"]["idle_rss_mib"]["median"] for site in sites]
        axes[0].barh(
            y + (index - 1) * 0.23, vals, height=0.22, color=COLORS[v], label=NAMES[v]
        )
    axes[0].set_yticks(y, [LABELS[s] for s in sites])
    axes[0].invert_yaxis()
    axes[0].set_xlabel("Idle RSS after 3 update batches (MiB)")
    axes[0].legend(frameon=False)
    for index, v in enumerate(["slotmap", "dense"]):
        delta = [
            data[s][v]["metrics"]["idle_pss_anon_mib"]["median"]
            - data[s]["baseline"]["metrics"]["idle_pss_anon_mib"]["median"]
            for s in sites
        ]
        axes[1].barh(
            y + (index - 0.5) * 0.3, delta, height=0.28, color=COLORS[v], label=NAMES[v]
        )
        ratio = [
            100
            * (
                data[s][v]["metrics"]["cpu_us_per_update"]["median"]
                / data[s]["baseline"]["metrics"]["cpu_us_per_update"]["median"]
                - 1
            )
            for s in sites
        ]
        axes[2].barh(y + (index - 0.5) * 0.3, ratio, height=0.28, color=COLORS[v])
    for ax in axes[1:]:
        ax.set_yticks(y, [])
        ax.invert_yaxis()
        ax.axvline(0, color="#6e7781", linewidth=0.8)
    axes[1].set_xlabel("Anonymous PSS change vs Vec (MiB)")
    axes[2].set_xlabel("CPU per update change vs Vec (%)")
    for ax in axes:
        ax.grid(axis="x", alpha=0.2)
        ax.set_axisbelow(True)
        ax.spines[["top", "right"]].set_visible(False)
    fig.suptitle("Matched webpage snapshots: stable layout-box arenas", fontsize=15)
    fig.text(
        0.01,
        0.015,
        "Fresh processes; matched inputs/geometry; 3 batches/run, n=3 or 5; no forced GC. Medians shown; ranges in report. Negative CPU change means faster.",
        fontsize=9,
    )
    fig.tight_layout(rect=(0, 0.045, 1, 0.94))
    fig.savefig(report / "memory-snapshots.svg")
    fig.savefig(report / "memory-snapshots.png", dpi=160)
    plt.close(fig)


def plot_live(report):
    data = json.loads((report / "live-summary.json").read_text())
    sites = [s for s, vs in data.items() if all(v in vs for v in COLORS)]
    sites.sort(
        key=lambda s: data[s]["baseline"]["metrics"]["init_peak_rss_mib"]["median"],
        reverse=True,
    )
    y = np.arange(len(sites))
    fig, axes = plt.subplots(1, 2, figsize=(14, max(8, len(sites) * 0.38)))
    for index, v in enumerate(COLORS):
        for ax, field in zip(
            axes, ["init_peak_rss_mib", "closed_rss_mib"], strict=True
        ):
            vals = np.array([data[s][v]["metrics"][field]["median"] for s in sites])
            low = np.array([data[s][v]["metrics"][field]["min"] for s in sites])
            high = np.array([data[s][v]["metrics"][field]["max"] for s in sites])
            ax.barh(
                y + (index - 1) * 0.24,
                vals,
                height=0.23,
                color=COLORS[v],
                label=NAMES[v],
                xerr=np.array([vals - low, high - vals]),
                error_kw={"elinewidth": 0.7, "capsize": 1},
            )
    axes[0].set_yticks(
        y,
        [
            LABELS[s] + " (n=" + "/".join(str(data[s][v]["runs"]) for v in COLORS) + ")"
            for s in sites
        ],
    )
    axes[1].set_yticks(y, [])
    for ax in axes:
        ax.invert_yaxis()
        ax.grid(axis="x", alpha=0.2)
        ax.set_axisbelow(True)
        ax.spines[["top", "right"]].set_visible(False)
    axes[0].set_xlabel("Peak RSS through navigation window (MiB)")
    axes[1].set_xlabel("RSS 2s after Target.closeTarget reply (MiB)")
    axes[0].legend(frameon=False)
    fig.suptitle("Live webpages: observed whole-process memory", fontsize=15)
    fig.text(
        0.01,
        0.008,
        "Unmodified page JS; dynamic responses/load state vary. These are observations, not paired causal performance rankings. Bars: median; lines: min-max; n is Vec/SlotMap/Dense.",
        fontsize=9,
    )
    fig.tight_layout(rect=(0, 0.035, 1, 0.97))
    fig.savefig(report / "memory-live.svg")
    fig.savefig(report / "memory-live.png", dpi=150)
    plt.close(fig)


def plot_traces(root, report):
    fig, axes = plt.subplots(2, 3, figsize=(15, 8))
    for ax, site in zip(
        axes.flat, ["finance", "wikipedia", "hh", "google", "cnn", "jd"], strict=True
    ):
        for v, color in COLORS.items():
            folder = root / "live-round1" / f"01-{site}-{v}"
            if not (folder / "result.json").exists():
                continue
            result = json.loads((folder / "result.json").read_text())
            origin = result["before"]["monotonic"]
            points = [
                json.loads(line)
                for line in (folder / "memory-samples.jsonl").read_text().splitlines()
            ]
            ax.plot(
                [p["monotonic"] - origin for p in points],
                [p["memory_kib"]["Rss"] / 1024 for p in points],
                label=NAMES[v],
                color=color,
                linewidth=1.3,
            )
            closed = next(
                (
                    p["monotonic"] - origin
                    for p in points
                    if p["phase"] == "after_close"
                ),
                None,
            )
            if closed is not None:
                ax.axvline(closed, color=color, linestyle=":", alpha=0.5, linewidth=0.7)
        ax.set_title(LABELS[site])
        ax.set_xlabel("Seconds since navigation")
        ax.set_ylabel("Sampled RSS (MiB)")
        ax.set_xlim(left=-0.5)
        ax.grid(alpha=0.2)
        ax.spines[["top", "right"]].set_visible(False)
    axes.flat[0].legend(frameon=False)
    fig.suptitle("Live first-trial RSS traces (100 ms samples)", fontsize=15)
    fig.text(
        0.01,
        0.008,
        "One predetermined first trial per variant; live inputs/loading differ. Dotted lines mark the first sample after target-close reply.",
        fontsize=9,
    )
    fig.tight_layout(rect=(0, 0.035, 1, 0.95))
    fig.savefig(report / "memory-live-traces.svg")
    fig.savefig(report / "memory-live-traces.png", dpi=150)
    plt.close(fig)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--raw-root", type=Path)
    args = parser.parse_args()
    plot_static(args.report)
    plot_live(args.report)
    if args.raw_root:
        plot_traces(args.raw_root, args.report)

# Normalize generated XML so vector figures do not add path-whitespace noise.
if __name__ == "__main__":
    for svg in args.report.glob("memory-*.svg"):
        svg.write_text(
            re.sub(r"\s+", " ", svg.read_text()).replace("> <", "><").strip() + "\n"
        )
