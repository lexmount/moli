"""Select two more rounds for Finance and the two largest observed CPU regressions."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def select(root):
    summary = json.loads((root / "snapshot-rounds/summary.json").read_text())
    original = json.loads((root / "snapshot-plan-v2.json").read_text())
    selected = ["finance"]
    ranking = []
    for site, variants in summary.items():
        if not all(v in variants for v in ["baseline", "slotmap", "dense"]):
            continue
        base = variants["baseline"]["metrics"]["cpu_us_per_update"]["median"]
        regression = max(
            variants[v]["metrics"]["cpu_us_per_update"]["median"] / base - 1
            for v in ["slotmap", "dense"]
        )
        if regression > 0.05:
            ranking.append((regression, site))
    for _, site in sorted(ranking, reverse=True)[:2]:
        if site not in selected:
            selected.append(site)
    plan = [
        dict(
            r,
            repetitions=2,
            selection_reason="Two further balanced rounds after observed regression, retaining the original three",
        )
        for r in original
        if r["site"] in selected
    ]
    (root / "cpu-confirmation-plan.json").write_text(json.dumps(plan, indent=2))
    return plan


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    print([r["site"] for r in select(args.root)])
