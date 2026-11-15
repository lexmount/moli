"""Observation reports: collection coverage and site verdicts are separate axes."""
from __future__ import annotations

from html import escape
import json
from pathlib import Path

from ..artifacts import write_json, write_text


def load_summary(path: Path) -> dict:
    if path.is_dir():
        path = path / "summary.json"
    summary = json.loads(path.read_text(encoding="utf-8"))
    if (not isinstance(summary, dict) or summary.get("schema_version") != 1
            or len(summary.get("suites", [])) != 1
            or summary["suites"][0].get("suite") != "fingerprint"):
        raise ValueError("baseline must be a version-1 fingerprint summary.json")
    index_samples(summary)  # Reject ambiguous sample keys before starting browsers.
    return summary


def index_samples(summary: dict) -> dict[tuple, dict]:
    indexed = {}
    for row in summary["suites"][0]["samples"]:
        key = (row["engine"], row["site"], row["run"])
        if key in indexed:
            raise ValueError(f"duplicate fingerprint sample: {key}")
        indexed[key] = row
    return indexed


def metrics(row: dict) -> dict:
    """Use only projected results, never raw page/network evidence or timings."""
    report = row.get("report") or {}
    if row["site"] == "fingerprint-pro" and report.get("valid") is True:
        # The runner verified that all completed responses agree; never select
        # a lowest score out of conflicting responses.
        report = report["results"][0]
    result = {}

    def flatten(value, prefix):
        if isinstance(value, dict):
            for key, child in sorted(value.items()):
                flatten(child, f"{prefix}.{key}" if prefix else key)
        else:
            result[prefix] = value

    for key, value in sorted(report.items()):
        if key not in {"readyState", "collector", "kind", "source", "percentages", "reportLabels"}:
            flatten(value, key)
    return result


def compare(current: dict, baseline: dict) -> dict:
    old, new = index_samples(baseline), index_samples(current)
    reasons = []
    for key in ["workload_hash", "environment"]:
        if current["manifest"].get(key) != baseline["manifest"].get(key):
            reasons.append(f"{key} differs")
    for label, summary in [("current", current), ("baseline", baseline)]:
        if summary.get("matrix_complete") is not True:
            reasons.append(f"{label} matrix is incomplete")
        if summary.get("inputs_unchanged") is not True:
            reasons.append(f"{label} inputs are changed or unverified")
    if set(old) != set(new):
        reasons.append("engine/site/repetition matrix differs")
    changes = []
    for key in sorted(old.keys() | new.keys()):
        before, after = old.get(key), new.get(key)
        entry = {"engine": key[0], "site": key[1], "run": key[2],
                 "old_status": before["status"] if before else "not_sampled",
                 "new_status": after["status"] if after else "not_sampled", "fields": []}
        left, right = metrics(before) if before else {}, metrics(after) if after else {}
        for field in sorted(left.keys() | right.keys()):
            a, b = left.get(field), right.get(field)
            # False != 0 for fingerprint signals, even though Python says otherwise.
            if field in left and field in right and type(a) is type(b) and a == b:
                continue
            change = {"field": field, "old_present": field in left, "new_present": field in right,
                      "old": a, "new": b}
            if type(a) in {int, float} and type(b) in {int, float}:
                change["delta"] = b - a
            entry["fields"].append(change)
        scripts = lambda row: sorted(script["sha256"] for script in (row or {}).get("observations", {}).get("scripts", [])
                                      if "sha256" in script)
        entry["script_set_changed"] = scripts(before) != scripts(after)
        entry["browser_changed"] = (before or {}).get("browser") != (after or {}).get("browser")
        if entry["fields"] or entry["old_status"] != entry["new_status"] or entry["script_set_changed"] or entry["browser_changed"]:
            changes.append(entry)
    return {"observational": True, "comparable_workload": not reasons, "caveats": reasons,
            "baseline_revision": baseline["manifest"].get("revision"), "changes": changes,
            "interpretation": "Differences are observations, not proof of causation or a bot pass-rate change. "
                              "Site scripts, browser versions, IP reputation and remote scoring may change."}


def render_html(summary: dict, comparison: dict | None = None) -> str:
    def pretty(value):
        return escape(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True))

    suite = summary["suites"][0]
    coverage = "".join(f"<tr><td>{escape(engine)}</td><td><pre>{pretty(counts)}</pre></td></tr>"
                       for engine, counts in sorted(suite["coverage"].items()))
    samples = ""
    for row in sorted(suite["samples"], key=lambda r: (r["site"], r["run"], r["engine"])):
        samples += (f"<tr><td>{escape(row['site'])}</td><td>{row['run']}</td><td>{escape(row['engine'])}</td>"
                    f"<td>{escape(row['status'])}</td><td><pre>{pretty(metrics(row))}</pre>"
                    f"<details><summary>Collection diagnostics</summary><pre>{pretty({'errors': row.get('errors', []), 'observations': row.get('observations', {})})}</pre></details></td></tr>")
    history = ("<h2>History comparison</h2><p>No baseline supplied.</p>" if comparison is None else
               f"<h2>History comparison</h2><p>{escape(comparison['interpretation'])}</p><pre>{pretty(comparison)}</pre>")
    return f"""<!doctype html>
<html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Native CDP fingerprint observations</title>
<style>body{{font:15px system-ui,sans-serif;margin:2rem;line-height:1.5;color:#222}}
table{{border-collapse:collapse;width:100%}}th,td{{padding:.6rem;border:1px solid #bbb;text-align:left;vertical-align:top}}
pre{{white-space:pre-wrap;overflow-wrap:anywhere;margin:0}}details{{margin-top:.6rem}}h2{{margin-top:2rem}}</style>
<h1>Native CDP fingerprint observations</h1>
<p><strong>Collection completeness is not a bot pass rate.</strong> Robot/isBot=true are valid site results.
Missing reports, network failures and capability reports cannot be counted as passes. Browser identity differences
are retained; Chromium is a comparison sample, not a universal oracle.</p>
<p>Matrix complete: {summary.get('matrix_complete', False)}. Inputs unchanged: {summary.get('inputs_unchanged', False)}.</p>
<h2>Collection coverage</h2><table><tr><th>Engine</th><th>Sample states</th></tr>{coverage}</table>
<h2>Site observations</h2><table><tr><th>Site</th><th>Run</th><th>Engine</th><th>Collection</th><th>Selected results</th></tr>{samples}</table>
{history}<h2>Provenance and sampling contract</h2><pre>{pretty(summary['manifest'])}</pre></html>
"""


def write_report(output: Path, summary: dict, baseline: dict | None = None) -> None:
    comparison = compare(summary, baseline) if baseline is not None else None
    if comparison is not None:
        write_json(output / "comparison.json", comparison)
    write_text(output / "index.html", render_html(summary, comparison))
