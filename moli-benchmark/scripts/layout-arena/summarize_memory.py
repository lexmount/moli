"""Audit raw runs and export numeric-only arena memory comparisons."""

from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path

from extended_compare import BLOCKED, scalar_metrics, summarize


def collect(root, groups, live=False):
    rows = []
    for group in groups:
        path = root / group / "runs.json"
        if not path.exists():
            continue
        for original in json.loads(path.read_text()):
            row = dict(original)
            row["group"] = group
            folder = root / group / row["run"]
            if live and (folder / "resource-limit.json").exists():
                row["status"] = "resource_limit"
            if live and (folder / "result.json").exists():
                result = json.loads((folder / "result.json").read_text())
                if "metrics" not in row:
                    try:
                        row["metrics"], _ = scalar_metrics(folder, result)
                    except (RuntimeError, KeyError, ValueError):
                        rows.append(row)
                        continue
                frame = result["navigation"]["frameId"]
                responses = []
                for line in (folder / "events.jsonl").read_text().splitlines():
                    event = json.loads(line)
                    if event.get("method") == "Network.responseReceived":
                        p = event["params"]
                        if p.get("type") == "Document" and p.get("frameId") == frame:
                            responses.append(p["response"])
                status = responses[-1]["status"] if responses else None
                row["metrics"] = dict(row["metrics"], root_http_status=status)
                load = result["load_event_seconds"]
                window = row["metrics"]["init_window_seconds"]
                row["metrics"]["load_in_window"] = int(
                    load is not None and load <= window
                )
                if result["navigation"].get("errorText"):
                    row.update(
                        status="network_error", error=result["navigation"]["errorText"]
                    )
                elif status is not None and status >= 400:
                    row.update(
                        status="http_error",
                        error=f"Root document returned HTTP {status}",
                    )
                elif BLOCKED.search(result["observation"]["title"]):
                    row.update(status="challenge", error="Challenge or error document")
                elif result["observation"]["nodes"] < 20:
                    row.update(
                        status="incomplete",
                        error="No usable document body at the observation point",
                    )
            rows.append(row)
    return rows


def export(root, output):
    output.mkdir(parents=True, exist_ok=True)
    static = collect(root, ["snapshot-rounds", "snapshot-confirmation"])
    live = collect(root, ["live-round1", "live-followups"], live=True)
    result = {
        "snapshots": {"runs": static, "summary": summarize(static)},
        "live": {"runs": live, "summary": summarize(live)},
    }
    for kind, value in result.items():
        (output / (kind + "-summary.json")).write_text(
            json.dumps(value["summary"], indent=2)
        )
    (output / "audit.json").write_text(
        json.dumps(
            {
                kind: {
                    "attempts": len(value["runs"]),
                    "successes": sum(r["status"] == "ok" for r in value["runs"]),
                    "failures": [
                        {
                            "group": r["group"],
                            "run": r["run"],
                            "site": r["site"],
                            "variant": r["variant"],
                            "status": r["status"],
                            "error": r.get("error"),
                        }
                        for r in value["runs"]
                        if r["status"] != "ok"
                    ],
                }
                for kind, value in result.items()
            },
            indent=2,
        )
    )
    records = []
    for kind, value in result.items():
        for site, variants in value["summary"].items():
            for variant, summary in variants.items():
                records.append(
                    {
                        "type": "summary",
                        "mode": kind,
                        "site": site,
                        "variant": variant,
                        **summary,
                    }
                )
        for r in value["runs"]:
            record = {k: r[k] for k in ["site", "variant", "repeat", "status"]}
            record.update(
                type="run", mode=kind, group=r["group"], metrics=r.get("metrics", {})
            )
            if r["status"] != "ok":
                record["failure_kind"] = r["status"]
            records.append(record)
    (output / "results-extended-d7e0a70f.jsonl").write_text(
        "".join(json.dumps(r, separators=(",", ":")) + "\n" for r in records)
    )
    scalar_names = sorted(
        {
            key
            for value in result.values()
            for r in value["runs"]
            for key in r.get("metrics", {})
        }
    )
    with (output / "measurements.csv").open("w", newline="") as file:
        writer = csv.DictWriter(
            file,
            fieldnames=["mode", "site", "variant", "repeat", "status"] + scalar_names,
            lineterminator="\n",
        )
        writer.writeheader()
        for kind, value in result.items():
            for r in value["runs"]:
                writer.writerow(
                    dict(
                        mode=kind,
                        **{k: r[k] for k in ["site", "variant", "repeat", "status"]},
                        **r.get("metrics", {}),
                    )
                )
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = export(args.root, args.output)
    print(
        {
            kind: {
                "attempts": len(value["runs"]),
                "ok": sum(r["status"] == "ok" for r in value["runs"]),
            }
            for kind, value in result.items()
        }
    )
