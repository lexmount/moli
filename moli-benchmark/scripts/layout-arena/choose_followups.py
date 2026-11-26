"""Predeclared memory followups: mandatory cases, largest baseline peaks, largest deviations."""

import argparse
import json
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("root", type=Path)
a = p.parse_args()
out = a.root
summary = json.loads((out / "live-round1/summary.json").read_text())
plan = json.loads((out / "live-plan-v2.json").read_text())
valid = {
    site: vs
    for site, vs in summary.items()
    if all(v in vs for v in ["baseline", "slotmap", "dense"])
}
selected = []
reasons = {}


def add(site, reason):
    if site not in valid:
        return
    if site not in selected:
        selected.append(site)
    reasons.setdefault(site, []).append(reason)


for site in ["finance", "wikipedia", "hh"]:
    add(site, "mandatory prior concern / large document")
for site, vs in sorted(
    valid.items(),
    key=lambda item: item[1]["baseline"]["metrics"]["init_peak_rss_mib"]["median"],
    reverse=True,
)[:5]:
    add(site, "top 5 baseline initialization peak RSS")
deltas = []
for site, vs in valid.items():
    baseline = vs["baseline"]["metrics"]["idle_pss_mib"]["median"]
    delta = max(
        abs(vs[v]["metrics"]["idle_pss_mib"]["median"] / baseline - 1)
        for v in ["slotmap", "dense"]
    )
    if delta >= 0.05:
        deltas.append((delta, site))
for delta, site in sorted(deltas, reverse=True)[:3]:
    add(site, "top 3 absolute candidate idle PSS deviation >=5%")
followups = [
    dict(r, repetitions=2, selection_reasons=reasons[r["site"]])
    for r in plan
    if r["site"] in selected
]
(out / "live-followup-plan.json").write_text(json.dumps(followups, indent=2))
print("followups", [(r["site"], r["selection_reasons"]) for r in followups])
