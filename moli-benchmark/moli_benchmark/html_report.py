from __future__ import annotations

import html
import json
from pathlib import Path
from typing import Any

from .artifacts import write_json, write_text


def _cell(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, float):
        return f"{value:.2f}"
    return html.escape(str(value))


def _link(path: str, label: str | None = None) -> str:
    return f'<a href="{html.escape(path)}">{html.escape(label or path)}</a>'


def _num(value: Any) -> float | None:
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _status(ok: bool) -> str:
    return '<span class="ok">PASS</span>' if ok else '<span class="fail">FAIL</span>'


def _status_badge(status: str) -> str:
    classes = {
        "PASS": "badge pass",
        "FAIL": "badge fail",
        "PARTIAL": "badge warn",
        "NOT RUN": "badge neutral",
    }
    return f'<span class="{classes.get(status, "badge neutral")}">{html.escape(status)}</span>'


def _mib(value: Any) -> str:
    if value is None:
        return ""
    try:
        return f"{float(value) / (1024 * 1024):.1f} MiB"
    except (TypeError, ValueError):
        return _cell(value)


def _size(value: Any) -> str:
    if value is None:
        return ""
    try:
        raw = float(value)
    except (TypeError, ValueError):
        return _cell(value)
    if raw >= 1024 * 1024:
        return f"{raw / (1024 * 1024):.1f} MiB"
    if raw >= 1024:
        return f"{raw / 1024:.1f} KiB"
    return f"{int(raw)} B"


def _target_names(targets: dict[str, Any]) -> list[str]:
    preferred = ["moli", "lightpanda", "chrome", "obscura"]
    names = [name for name in preferred if name in targets]
    names.extend(name for name in targets if name not in names)
    return names


def _summary_by_suite(summaries: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    return {str(summary.get("suite")): summary for summary in summaries if summary.get("suite")}


def _suite_failures(summary: dict[str, Any] | None) -> int | None:
    if not summary:
        return None
    value = summary.get("gate_failures", summary.get("total_failures"))
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return None


def _suite_status(summary: dict[str, Any] | None) -> str:
    failures = _suite_failures(summary)
    if failures is None:
        return "NOT RUN"
    return "PASS" if failures == 0 else "FAIL"


def _synthetic_status(by_suite: dict[str, dict[str, Any]]) -> tuple[str, dict[str, Any] | None]:
    matrix = by_suite.get("synthetic-matrix")
    compare = by_suite.get("synthetic-compare")
    synthetic = matrix or compare
    failures = _suite_failures(synthetic)
    if failures is None:
        return "NOT RUN", None
    if failures > 0:
        return "FAIL", synthetic
    if matrix and matrix.get("profile") == "formal":
        return "PASS", matrix
    return "PARTIAL", synthetic


def _amiibo_status(summary: dict[str, Any] | None) -> str:
    failures = _suite_failures(summary)
    if failures is None:
        return "NOT RUN"
    if failures > 0:
        return "FAIL"
    return "PASS" if summary and summary.get("profile") == "formal" else "PARTIAL"


def _p0_rows(summaries: list[dict[str, Any]]) -> list[tuple[str, str, dict[str, Any] | None, str]]:
    by_suite = _summary_by_suite(summaries)
    synthetic_status, synthetic_summary = _synthetic_status(by_suite)
    return [
        (synthetic_status, "Synthetic", synthetic_summary, "Formal local fixture matrix, correctness markers, stability drift."),
        (_suite_status(by_suite.get("startup")), "Startup / Size", by_suite.get("startup"), "Serve ready, binary size, optional CDP first page and idle footprint."),
        (_suite_status(by_suite.get("cdp-smoke") or by_suite.get("cdp-session")), "CDP", by_suite.get("cdp-smoke") or by_suite.get("cdp-session"), "Raw CDP session and client smoke coverage."),
        (_suite_status(by_suite.get("wpt")), "WPT", by_suite.get("wpt"), "Local WPT compat report archival and gate result."),
        (_amiibo_status(by_suite.get("amiibo-crawler")), "Amiibo Crawler", by_suite.get("amiibo-crawler"), "Lightpanda demo crawler workload."),
        (_suite_status(by_suite.get("wild-web")), "Wild Web", by_suite.get("wild-web"), "Real-site seed classification and extraction readiness."),
    ]


def _overall_p0_status(summaries: list[dict[str, Any]]) -> str:
    if not summaries:
        return "NOT RUN"
    statuses = [status for status, _, _, _ in _p0_rows(summaries)]
    if any(status == "FAIL" for status in statuses):
        return "FAIL"
    if all(status == "PASS" for status in statuses):
        return "PASS"
    return "PARTIAL"


def _target_count(versions: dict[str, Any]) -> int:
    targets = versions.get("targets") or {}
    return sum(1 for target in targets.values() if isinstance(target, dict) and target.get("available"))


def _kpi_cards(versions: dict[str, Any], summaries: list[dict[str, Any]]) -> str:
    suite_count = len(summaries)
    total_gate_failures = sum(_suite_failures(summary) or 0 for summary in summaries)
    target_count = _target_count(versions)
    overall = _overall_p0_status(summaries)
    cards = [
        ("P0 Gate", _status_badge(overall), "Overall scorecard status across required benchmark areas."),
        ("Suites Run", str(suite_count), "Benchmark suites with summary data."),
        ("Gate Failures", str(total_gate_failures), "Selected gate target failures plus profile failures."),
        ("Targets Available", str(target_count), "Moli / Lightpanda / Chrome / Obscura binaries detected."),
    ]
    return "".join(
        "<section class=\"kpi\">"
        f"<div class=\"kpi-label\">{html.escape(label)}</div>"
        f"<div class=\"kpi-value\">{value}</div>"
        f"<div class=\"kpi-note\">{html.escape(note)}</div>"
        "</section>"
        for label, value, note in cards
    )


def _p0_scorecard(summaries: list[dict[str, Any]]) -> str:
    body = []
    for status, area, summary, requirement in _p0_rows(summaries):
        failures = _suite_failures(summary)
        suite_name = summary.get("suite") if summary else ""
        body.append(
            "<tr>"
            f"<th>{html.escape(area)}</th>"
            f"<td>{_status_badge(status)}</td>"
            f"<td>{_cell(suite_name)}</td>"
            f"<td>{_cell(failures)}</td>"
            f"<td>{html.escape(requirement)}</td>"
            "</tr>"
        )
    return (
        "<table class=\"scorecard\">"
        "<thead><tr><th>area</th><th>status</th><th>source suite</th><th>failures</th><th>P0 requirement</th></tr></thead>"
        f"<tbody>{''.join(body)}</tbody>"
        "</table>"
    )


def _target_versions(versions: dict[str, Any]) -> str:
    targets = versions.get("targets") or {}
    rows = []
    for name in ("moli", "lightpanda", "chrome", "obscura"):
        target = targets.get(name, {})
        rows.append(
            "<tr>"
            f"<th>{html.escape(name)}</th>"
            f"<td>{_cell(target.get('path'))}</td>"
            f"<td>{_cell(target.get('version'))}</td>"
            f"<td>{_cell(target.get('size_bytes'))}</td>"
            f"<td><code>{_cell(target.get('sha256'))}</code></td>"
            "</tr>"
        )
    return "\n".join(rows)


def _int_value(value: Any) -> int:
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return 0


def _cdp_session_trace_events(summary: dict[str, Any] | None) -> int:
    if not isinstance(summary, dict):
        return 0
    explicit = _int_value(summary.get("total_trace_events"))
    if explicit:
        return explicit
    total = 0
    targets = summary.get("targets")
    if not isinstance(targets, dict):
        return total
    for target in targets.values():
        if not isinstance(target, dict):
            continue
        for key in ("console_errors", "js_exceptions", "network_failures"):
            total += _int_value(target.get(key))
    return total


def _artifact_paths_by_suite(summaries: list[dict[str, Any]]) -> dict[str, tuple[str, ...]]:
    by_suite = {str(summary.get("suite")): summary for summary in summaries if summary.get("suite") is not None}
    known = {
        "startup": (
            "startup/summary.json",
            "startup/gate-rows.json",
            "startup/runs.csv",
            "startup/runs.json",
            "startup/time/",
            "startup/cgroup/",
            "startup/image-size/",
        ),
        "synthetic": ("synthetic/summary.json", "synthetic/runs.csv"),
        "synthetic-matrix": ("synthetic-matrix/summary.json", "synthetic-matrix/matrix.csv"),
        "synthetic-compare": ("synthetic-compare/summary.json", "synthetic-compare/runs.csv"),
        "cdp-session": ("cdp-session/summary.json", "cdp-session/runs.csv"),
        "crawler": ("crawler/summary.json", "crawler/raw-runs.csv"),
        "amiibo-crawler": ("amiibo-crawler/summary.json", "amiibo-crawler/raw-runs.csv"),
        "wild-web": ("wild-web/summary.json", "wild-web/raw-runs.csv"),
        "top-sites": (
            "top-sites/summary.json",
            "top-sites/raw-runs.csv",
            "top-sites/site-outcomes.json",
        ),
        "render-compare": (
            "render-compare/summary.json",
            "render-compare/raw-runs.csv",
            "render-compare/runs.json",
            "render-compare/baseline-sites.csv",
            "render-compare/baseline-sites.json",
            "render-compare/baseline-runs.json",
        ),
        "wpt": ("wpt/summary.json", "wpt/moli-wpt-compat-report.json", "wpt/raw-runs.csv", "wpt/by-tag.csv"),
        "cdp-smoke": ("cdp-smoke/summary.json", "cdp-smoke/moli-cdp-smoke.json"),
    }
    startup = by_suite.get("startup")
    if isinstance(startup, dict) and startup.get("cache_artifacts"):
        known["startup"] = (*known["startup"], "startup/cache/")

    cdp_session = by_suite.get("cdp-session")
    if isinstance(cdp_session, dict) and (
        _int_value(cdp_session.get("total_failures")) or _cdp_session_trace_events(cdp_session)
    ):
        known["cdp-session"] = (*known["cdp-session"], "cdp-session/traces/")

    wild_web = by_suite.get("wild-web")
    if isinstance(wild_web, dict):
        if _int_value(wild_web.get("total_failures")):
            known["wild-web"] = (*known["wild-web"], "wild-web/failures/")
        if _int_value(wild_web.get("replay_artifacts")):
            known["wild-web"] = (*known["wild-web"], "wild-web/replay/manifest.json")

    top_sites = by_suite.get("top-sites")
    if isinstance(top_sites, dict) and _int_value(top_sites.get("total_failures")):
        known["top-sites"] = (*known["top-sites"], "top-sites/failures/")

    render_compare = by_suite.get("render-compare")
    if isinstance(render_compare, dict) and _int_value(render_compare.get("total_failures")):
        known["render-compare"] = (*known["render-compare"], "render-compare/failures/")

    wpt = by_suite.get("wpt")
    if isinstance(wpt, dict) and wpt.get("diff") is not None:
        known["wpt"] = (*known["wpt"], "wpt/diff.json", "wpt/diff.csv")
    return known


def _artifact_index(summaries: list[dict[str, Any]]) -> str:
    known = _artifact_paths_by_suite(summaries)
    rows = []
    for summary in summaries:
        suite = str(summary.get("suite"))
        paths = known.get(suite, ())
        rows.append(
            "<tr>"
            f"<th>{html.escape(suite)}</th>"
            f"<td>{' · '.join(_link(path) for path in paths)}</td>"
            "</tr>"
        )
    return (
        "<table>"
        "<thead><tr><th>suite</th><th>artifacts</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _horizontal_comparisons(summaries: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_suite = _summary_by_suite(summaries)
    comparisons: list[dict[str, Any]] = []
    synthetic_compare = by_suite.get("synthetic-compare")
    cdp_session = by_suite.get("cdp-session")
    if synthetic_compare and cdp_session:
        synthetic_cases = [str(case) for case in synthetic_compare.get("cases", [])]
        cdp_cases = {str(case) for case in cdp_session.get("cases", [])}
        cases = [case for case in synthetic_cases if case in cdp_cases]
        targets: dict[str, Any] = {}
        for source in (synthetic_compare, cdp_session):
            source_targets = source.get("targets", {})
            if not isinstance(source_targets, dict):
                continue
            for target, target_summary in source_targets.items():
                if not isinstance(target_summary, dict):
                    continue
                source_cases = target_summary.get("cases", {})
                if not isinstance(source_cases, dict):
                    continue
                selected_cases = {case: source_cases[case] for case in cases if case in source_cases}
                if selected_cases:
                    targets[str(target)] = {
                        **target_summary,
                        "cases": selected_cases,
                    }
        if cases and targets:
            comparisons.append(
                {
                    "suite": "web-scraping-variants",
                    "sources": ["synthetic-compare", "cdp-session"],
                    "gate_target": synthetic_compare.get("gate_target", "moli"),
                    "gate_failures": synthetic_compare.get("gate_failures", 0),
                    "total_failures": sum(int(target.get("failures", 0) or 0) for target in targets.values()),
                    "cases": cases,
                    "targets": targets,
                    "note": "Derived comparison that joins fetch-style and CDP session variants for shared synthetic cases.",
                }
            )
    for summary in summaries:
        if isinstance(summary.get("targets"), dict) and summary.get("cases"):
            comparisons.append(summary)
    return comparisons


def _read_json_artifact(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def _render_compare_payload(output_dir: Path) -> dict[str, Any] | None:
    suite_dir = output_dir / "render-compare"
    runs = _read_json_artifact(suite_dir / "runs.json")
    baseline_sites = _read_json_artifact(suite_dir / "baseline-sites.json")
    if not isinstance(runs, list) and not isinstance(baseline_sites, list):
        return None

    run_fields = (
        "target",
        "engine",
        "driver",
        "label",
        "rank",
        "domain",
        "url",
        "baseline_target",
        "baseline_category",
        "target_fetch_category",
        "category",
        "ok",
        "excluded",
        "elapsed_ms",
        "baseline_elapsed_ms",
        "peak_pss_bytes",
        "peak_rss_bytes",
        "render_quality_score",
        "raw_content_score",
        "ngram_containment",
        "raw_ngram_containment",
        "key_phrase_hit_rate",
        "raw_key_phrase_hit_rate",
        "visible_text_ratio",
        "baseline_visible_text_length",
        "target_visible_text_length",
        "failure_artifact",
    )
    baseline_fields = (
        "rank",
        "domain",
        "url",
        "baseline_target",
        "baseline_fetch_category",
        "category",
        "usable",
        "baseline_title",
        "baseline_visible_text_length",
        "baseline_elapsed_ms",
        "baseline_peak_pss_bytes",
        "baseline_peak_rss_bytes",
    )
    slim_runs = [
        {field: row.get(field) for field in run_fields if field in row}
        for row in runs
        if isinstance(row, dict)
    ] if isinstance(runs, list) else []
    slim_baselines = [
        {field: row.get(field) for field in baseline_fields if field in row}
        for row in baseline_sites
        if isinstance(row, dict)
    ] if isinstance(baseline_sites, list) else []
    return {
        "runs": slim_runs,
        "baseline_sites": slim_baselines,
        "run_count": len(slim_runs),
        "baseline_site_count": len(slim_baselines),
    }


def _report_payload(
    *,
    output_dir: Path,
    versions: dict[str, Any],
    summaries: list[dict[str, Any]],
    publish_readiness: dict[str, Any] | None,
    report_diff: dict[str, Any] | None,
) -> dict[str, Any]:
    all_artifact_paths = _artifact_paths_by_suite(summaries)
    present_suites = [str(summary.get("suite")) for summary in summaries if summary.get("suite") is not None]
    return {
        "schema_version": 1,
        "output_dir": str(output_dir),
        "versions": versions,
        "summaries": summaries,
        "horizontal_comparisons": _horizontal_comparisons(summaries),
        "render_compare": _render_compare_payload(output_dir),
        "publish_readiness": publish_readiness,
        "report_diff": report_diff,
        "artifact_paths_by_suite": {suite: all_artifact_paths.get(suite, ()) for suite in present_suites},
    }


def _json_script(payload: dict[str, Any]) -> str:
    return json.dumps(payload, ensure_ascii=False, sort_keys=True).replace("</", "<\\/")


def _chartjs_document(payload: dict[str, Any]) -> str:
    data = _json_script(payload)
    document = """<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Moli Benchmark Report</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.9/dist/chart.umd.min.js"></script>
<style>
:root {
  color-scheme: light;
  --page: #f6f7f9;
  --panel: #fff;
  --ink: #111827;
  --muted: #64748b;
  --soft: #f8fafc;
  --line: #d8dee8;
  --line-strong: #aeb8c6;
  --blue: #1f5aa6;
  --teal: #287c74;
  --amber: #a16618;
  --violet: #6d4fc2;
  --green: #177245;
  --red: #b42318;
  --yellow: #935a00;
  --green-bg: #eaf7ef;
  --red-bg: #fdecec;
  --yellow-bg: #fff4d8;
  --neutral-bg: #eef2f7;
}
* {
  box-sizing: border-box;
}
body {
  margin: 0;
  background: var(--page);
  color: var(--ink);
  font: 14px/1.45 Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  overflow-x: hidden;
}
.topbar {
  position: sticky;
  top: 0;
  z-index: 10;
  border-bottom: 1px solid var(--line);
  background: rgba(255, 255, 255, 0.94);
  backdrop-filter: blur(10px);
}
.topbar-inner {
  max-width: 1220px;
  margin: 0 auto;
  padding: 12px 24px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
}
.brand {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-weight: 760;
}
.brand small {
  color: var(--muted);
  font-weight: 520;
}
.nav {
  display: flex;
  flex-wrap: wrap;
  gap: 14px;
}
.nav a {
  color: var(--muted);
  text-decoration: none;
  font-size: 13px;
}
.nav a:hover {
  color: var(--ink);
}
main {
  max-width: 1220px;
  margin: 0 auto;
  padding: 28px 24px 56px;
  min-width: 0;
}
h1, h2, h3 {
  margin: 0;
  letter-spacing: 0;
}
h1 {
  font-size: 31px;
  line-height: 1.12;
}
h2 {
  font-size: 18px;
  margin-bottom: 12px;
}
h3 {
  font-size: 15px;
}
.muted {
  color: var(--muted);
}
.hero {
  display: grid;
  grid-template-columns: minmax(0, 1fr) max-content;
  gap: 18px;
  align-items: start;
  margin: 6px 0 18px;
}
.eyebrow {
  color: var(--muted);
  font-size: 12px;
  font-weight: 700;
  letter-spacing: .08em;
  text-transform: uppercase;
}
.hero p {
  max-width: 820px;
  margin: 8px 0 0;
}
.badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 68px;
  border-radius: 999px;
  padding: 4px 9px;
  font-size: 12px;
  font-weight: 750;
}
.badge.pass {
  color: var(--green);
  background: var(--green-bg);
}
.badge.fail {
  color: var(--red);
  background: var(--red-bg);
}
.badge.warn {
  color: var(--yellow);
  background: var(--yellow-bg);
}
.badge.neutral {
  color: var(--muted);
  background: var(--neutral-bg);
}
.status-tooltip {
  position: relative;
  display: inline-flex;
}
.status-tooltip[data-tooltip]:hover::after,
.status-tooltip[data-tooltip]:focus::after {
  content: attr(data-tooltip);
  position: absolute;
  left: 0;
  top: calc(100% + 8px);
  z-index: 30;
  width: max-content;
  max-width: min(520px, 70vw);
  padding: 10px 12px;
  border: 1px solid #cbd5e1;
  border-radius: 8px;
  background: #0f172a;
  color: #f8fafc;
  box-shadow: 0 18px 44px rgba(15, 23, 42, .22);
  font-size: 12px;
  font-weight: 500;
  line-height: 1.45;
  white-space: normal;
  overflow-wrap: anywhere;
  text-transform: none;
  pointer-events: none;
}
.grid {
  display: grid;
  gap: 12px;
}
.kpis {
  grid-template-columns: repeat(4, minmax(0, 1fr));
  margin: 16px 0;
}
.two {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}
.three {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}
.panel {
  margin: 12px 0;
  padding: 16px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--panel);
  min-width: 0;
}
.panel.compact {
  padding: 14px;
}
.panel.wide {
  grid-column: 1 / -1;
}
.takeaway-grid {
  display: grid;
  grid-template-columns: 1.2fr .8fr;
  gap: 12px;
}
.stat {
  padding: 14px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--panel);
}
.stat-label {
  color: var(--muted);
  font-size: 12px;
  text-transform: uppercase;
}
.stat-value {
  margin-top: 6px;
  font-size: 24px;
  font-weight: 780;
}
.stat-note {
  margin-top: 3px;
  color: var(--muted);
  font-size: 12px;
}
.target-card {
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 13px;
  background: var(--soft);
}
.target-card h3 {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 8px;
}
.target-meta {
  display: grid;
  gap: 6px;
  color: var(--muted);
  font-size: 12px;
}
.chart-box {
  height: 320px;
}
.failure-box {
  height: 260px;
}
.page-chart-box {
  height: 420px;
}
canvas {
  width: 100%;
  max-width: 100%;
  height: 250px;
}
.table-wrap {
  width: 100%;
  overflow-x: auto;
}
table {
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
}
th, td {
  padding: 9px 10px;
  border-bottom: 1px solid var(--line);
  text-align: left;
  vertical-align: top;
  overflow-wrap: anywhere;
}
thead th {
  color: var(--muted);
  background: var(--soft);
  font-size: 12px;
  text-transform: uppercase;
}
tbody tr:hover td, tbody tr:hover th {
  background: #fafcff;
}
code {
  font-size: 12px;
}
a {
  color: var(--blue);
  text-decoration: none;
}
a:hover {
  text-decoration: underline;
}
.pill-list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.pill {
  display: inline-flex;
  align-items: center;
  border-radius: 999px;
  padding: 5px 9px;
  background: var(--neutral-bg);
  color: var(--muted);
  font-size: 12px;
}
.notice {
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 10px 12px;
  background: var(--soft);
}
.finding-list {
  display: grid;
  gap: 9px;
}
.finding {
  padding: 10px 12px;
  border: 1px solid var(--line);
  border-left: 4px solid var(--blue);
  border-radius: 6px;
  background: #fff;
}
.finding.warn {
  border-left-color: var(--yellow);
}
.finding.fail {
  border-left-color: var(--red);
}
.score-cell {
  min-width: 112px;
}
.score-value {
  display: flex;
  justify-content: space-between;
  gap: 10px;
  font-variant-numeric: tabular-nums;
}
.score-track {
  margin-top: 5px;
  height: 7px;
  border-radius: 999px;
  background: #e6ebf1;
  overflow: hidden;
}
.score-fill {
  height: 100%;
  border-radius: 999px;
  background: var(--green);
}
.score-fill.warn {
  background: var(--yellow);
}
.score-fill.fail {
  background: var(--red);
}
.url-cell {
  min-width: 260px;
}
.metric-tight {
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
#webPageTable table {
  min-width: 1520px;
  table-layout: auto;
}
@media (max-width: 900px) {
  .topbar-inner {
    align-items: flex-start;
    flex-direction: column;
  }
  main {
    padding: 20px 12px 40px;
  }
  .hero, .two, .three, .kpis, .takeaway-grid {
    grid-template-columns: 1fr;
  }
}
</style>
<body>
<header class="topbar">
  <div class="topbar-inner">
  <div class="brand">Moli Benchmark <small>rendered from report-data.json</small></div>
  <nav class="nav">
    <a href="#overview">Overview</a>
    <a href="#findings">Findings</a>
    <a href="#comparison">Comparison</a>
    <a href="#web-pages">Web Pages</a>
    <a href="#targets">Targets</a>
    <a href="#readiness">Readiness</a>
    <a href="#artifacts">Artifacts</a>
  </nav>
</div>
</header>
<main>
  <section id="overview" class="hero">
    <div>
      <div class="eyebrow">Benchmark report</div>
      <h1>Benchmark Comparison Report</h1>
      <p class="muted">Lower latency and memory are better. Correctness gates remain the primary publish condition.</p>
      <p class="muted">Output: <code id="outputDir"></code></p>
    </div>
    <div id="publishBadge"></div>
  </section>
  <section class="grid kpis" id="kpis"></section>
  <section id="findings" class="takeaway-grid">
    <div class="panel compact">
      <h2>Executive Summary</h2>
      <div id="executiveSummary"></div>
    </div>
    <div class="panel compact">
      <h2>Run Notes</h2>
      <div id="runNotes"></div>
    </div>
  </section>
  <section id="comparison" class="grid two">
    <div class="panel">
      <h2>Latency P50 by Case</h2>
      <div class="chart-box"><canvas id="latencyChart"></canvas></div>
    </div>
    <div class="panel">
      <h2>Memory PSS P50 by Case</h2>
      <div class="chart-box"><canvas id="memoryChart"></canvas></div>
    </div>
    <div class="panel wide">
      <h2>Memory RSS P50 by Case</h2>
      <div class="chart-box"><canvas id="rssChart"></canvas></div>
    </div>
  </section>
  <section class="panel">
    <h2>Target Failure Counts</h2>
    <div class="failure-box"><canvas id="failureChart"></canvas></div>
  </section>
  <section class="panel">
    <h2>Horizontal Details</h2>
    <div class="table-wrap" id="comparisonTable"></div>
  </section>
  <section id="web-pages" class="panel">
    <h2>Web Page Request Scores</h2>
    <div class="grid kpis" id="webPageKpis"></div>
    <div class="grid two">
      <div>
        <h3>Lowest Render Quality Scores</h3>
        <div class="page-chart-box"><canvas id="pageScoreChart"></canvas></div>
      </div>
      <div>
        <h3>Slowest Page Requests</h3>
        <div class="page-chart-box"><canvas id="pageLatencyChart"></canvas></div>
      </div>
    </div>
    <div class="table-wrap" id="webPageTable"></div>
  </section>
  <section id="targets" class="panel">
    <h2>Targets</h2>
    <div class="grid three" id="targetCards"></div>
  </section>
  <section id="readiness" class="panel">
    <h2>Publication Readiness</h2>
    <div class="table-wrap" id="readinessTable"></div>
  </section>
  <section class="panel">
    <h2>Suites</h2>
    <div class="table-wrap" id="suiteTable"></div>
  </section>
  <section id="artifacts" class="panel">
    <h2>Artifacts</h2>
    <div class="table-wrap" id="artifactTable"></div>
  </section>
  <section class="panel">
    <h2>Previous Report Diff</h2>
    <div id="reportDiff"></div>
  </section>
</main>
<script id="report-data" type="application/json">__REPORT_DATA__</script>
<script>
const report = JSON.parse(document.getElementById('report-data').textContent);
const targetOrder = ['moli', 'moli-cdp', 'moli-full', 'moli-full-cdp', 'lightpanda', 'lightpanda-cdp', 'chrome', 'chrome-cdp', 'obscura', 'obscura-cdp'];
const colors = {
  moli: '#2356a5',
  lightpanda: '#257a73',
  chrome: '#9a6419',
  obscura: '#6d4cc2',
};

function htmlEscape(value) {
  return String(value ?? '').replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
}
function number(value, digits = 2) {
  const n = Number(value);
  return Number.isFinite(n) ? n.toFixed(digits) : '';
}
function mib(value) {
  const n = Number(value);
  return Number.isFinite(n) ? `${(n / 1024 / 1024).toFixed(1)} MiB` : '';
}
function size(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return '';
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MiB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${Math.round(n)} B`;
}
function badge(status) {
  const s = String(status || 'NOT RUN').toUpperCase();
  const cls = s === 'PASS' || s === 'PUBLISHABLE' ? 'pass' : s === 'FAIL' ? 'fail' : s === 'INVESTIGATION' || s === 'PARTIAL' ? 'warn' : 'neutral';
  return `<span class="badge ${cls}">${htmlEscape(s)}</span>`;
}
function statusBadge(status, tooltip) {
  const rendered = badge(status);
  if (!tooltip) return rendered;
  return `<span class="status-tooltip" tabindex="0" data-tooltip="${htmlEscape(tooltip)}">${rendered}</span>`;
}
function suiteMap() {
  return new Map((report.summaries || []).map(summary => [summary.suite, summary]));
}
function targetNames(targets) {
  const names = Object.keys(targets || {});
  return [...targetOrder.filter(name => names.includes(name)), ...names.filter(name => !targetOrder.includes(name))];
}
function targetMeta(targets, target) {
  const meta = targets?.[target] || {};
  return typeof meta === 'object' && meta ? meta : {};
}
function targetEngine(targets, target) {
  return targetMeta(targets, target).engine || String(target).replace(/-cdp$/, '');
}
function targetLabel(targets, target) {
  return targetMeta(targets, target).label || target;
}
function targetColor(targets, target) {
  return colors[targetEngine(targets, target)] || '#64748b';
}
function horizontalSummaries() {
  const comparisons = report.horizontal_comparisons || [];
  const summaries = report.summaries || [];
  const result = [...comparisons];
  const seen = new Set(result.map(summary => summary.suite));
  for (const summary of summaries) {
    if (!summary?.targets || seen.has(summary.suite)) continue;
    result.push(summary);
    seen.add(summary.suite);
  }
  return result;
}
function firstHorizontalSummary() {
  const summaries = horizontalSummaries();
  for (const suite of ['web-scraping-variants', 'synthetic-compare', 'top-sites', 'wild-web', 'cdp-session']) {
    const summary = summaries.find(candidate => candidate.suite === suite);
    if (summary) return summary;
  }
  return summaries[0];
}
function summaryCases(summary) {
  const cases = summary?.cases || [];
  return cases.length ? cases : ['aggregate'];
}
function isAggregateSummary(summary) {
  return summary?.targets && !(summary?.cases || []).length;
}
function resultFor(summary, target, caseName) {
  if (isAggregateSummary(summary)) return summary?.targets?.[target] || {};
  return summary?.targets?.[target]?.cases?.[caseName] || {};
}
function performanceResultFor(summary, target, caseName) {
  if (!isAggregateSummary(summary)) return resultFor(summary, target, caseName);
  const common = summary?.common_success?.targets?.[target];
  if (common && Number(common?.elapsed_ms?.count || 0) > 0) return common;
  const result = resultFor(summary, target, caseName);
  return {
    ...result,
    elapsed_ms: result?.successful_elapsed_ms || result?.elapsed_ms,
    peak_pss_bytes: result?.successful_peak_pss_bytes || result?.peak_pss_bytes,
    peak_rss_bytes: result?.successful_peak_rss_bytes || result?.peak_rss_bytes,
  };
}
function metricFor(summary, target, caseName, metric) {
  const result = performanceResultFor(summary, target, caseName);
  return result?.[metric]?.p50 ?? null;
}
function targetFailures(summary, target) {
  const targetSummary = summary?.targets?.[target] || {};
  if (Number.isFinite(Number(targetSummary.failures))) return Number(targetSummary.failures);
  return Object.values(targetSummary.cases || {}).reduce((total, result) => total + Number(result.failures || 0), 0);
}
function resultPasses(result) {
  return Number(result?.passes ?? result?.successes ?? result?.categories?.success ?? 0);
}
function resultFailures(result) {
  return Number(result?.failures ?? result?.total_failures ?? 0);
}
function horizontalCells(summary) {
  const targets = targetNames(summary?.targets || {});
  const cases = summaryCases(summary);
  return cases.flatMap(caseName => targets.map(target => {
    const result = performanceResultFor(summary, target, caseName);
    return {caseName, target, result};
  }));
}
function fastestCell(summary) {
  return horizontalCells(summary)
    .filter(item => Number.isFinite(Number(item.result.elapsed_ms?.p50)))
    .sort((a, b) => Number(a.result.elapsed_ms.p50) - Number(b.result.elapsed_ms.p50))[0];
}
function lowestMemoryCell(summary) {
  return horizontalCells(summary)
    .filter(item => Number.isFinite(Number(item.result.peak_pss_bytes?.p50)))
    .sort((a, b) => Number(a.result.peak_pss_bytes.p50) - Number(b.result.peak_pss_bytes.p50))[0];
}
function passRate(result) {
  const passes = resultPasses(result);
  const failures = resultFailures(result);
  const attempts = passes + failures;
  const denominator = attempts || Number(result.pages ?? result.seeds ?? result.sites ?? result.runs ?? 0);
  if (!denominator) return '';
  return `${passes}/${denominator} (${(passes / denominator * 100).toFixed(1)}%)`;
}
function rawPassRate(result) {
  const passes = Number(result?.raw_passes);
  const attempts = Number(result?.raw_comparable_attempts);
  if (!Number.isFinite(passes) || !Number.isFinite(attempts) || attempts <= 0) return '';
  return `${passes}/${attempts} (${(passes / attempts * 100).toFixed(1)}%)`;
}
function siteStability(result) {
  const sites = Number(result?.unique_sites);
  if (!Number.isFinite(sites) || sites <= 0) return '';
  const runs = Number(result?.runs || 0);
  if (!result?.repeat_validated) return `${sites} sites; ${runs || 1} run(s), not repeat-validated`;
  return `${Number(result?.all_pass_sites || 0)} all-pass / ${Number(result?.flaky_sites || 0)} flaky / ${Number(result?.all_fail_sites || 0)} all-fail`;
}
function statusCoverage(result) {
  const observed = Number(result?.status_observed_attempts);
  const attempts = Number(result?.observed_sites ?? result?.attempts);
  const denominator = Number.isFinite(attempts) && attempts > 0 ? attempts : Number(result?.seeds);
  if (!Number.isFinite(observed) || !Number.isFinite(denominator) || denominator <= 0) return '';
  const sources = Object.entries(result?.response_status_sources || {})
    .map(([source, count]) => `${source}: ${count}`)
    .join(', ');
  const coverage = `${observed}/${denominator} (${(observed / denominator * 100).toFixed(1)}%)`;
  return sources ? `${coverage}; ${sources}` : coverage;
}
function breakdown(items) {
  if (!items || typeof items !== 'object') return '';
  return Object.entries(items).map(([key, value]) => `${htmlEscape(key)}: ${htmlEscape(value)}`).join(', ');
}
function shortUrl(value) {
  const text = String(value || '');
  if (text.length <= 96) return text;
  return `${text.slice(0, 56)}...${text.slice(-34)}`;
}
function scoreClass(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return 'fail';
  if (n >= 80) return '';
  if (n >= 50) return 'warn';
  return 'fail';
}
function scoreBar(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return '';
  const width = Math.max(0, Math.min(100, n));
  const cls = scoreClass(n);
  return `<div class="score-cell"><div class="score-value"><strong>${number(n)}</strong><span class="muted">/100</span></div><div class="score-track"><div class="score-fill ${cls}" style="width:${width}%"></div></div></div>`;
}
function renderCategory(category, ok) {
  if (ok) return badge('PASS');
  const text = String(category || 'FAIL');
  const cls = text === 'render-partial' ? 'warn' : 'fail';
  return `<span class="badge ${cls}">${htmlEscape(text)}</span>`;
}
function renderKpis() {
  const readiness = report.publish_readiness || {};
  const targets = report.versions?.targets || {};
  const availableTargets = Object.values(targets).filter(t => t && t.available).length;
  const gateFailures = (report.summaries || []).reduce((total, summary) => total + Number(summary.gate_failures ?? summary.total_failures ?? 0), 0);
  const horizontal = firstHorizontalSummary();
  const cards = [
    ['Publication', badge(readiness.status || 'investigation'), 'Machine-readable publish-readiness gate.'],
    ['Suites Run', String((report.summaries || []).length), 'Benchmark suites present in this artifact.'],
    ['Gate Failures', String(gateFailures), 'Selected target gates, not competitor-only failures.'],
    ['Targets Available', String(availableTargets), 'Detected target binaries in versions.json.'],
  ];
  document.getElementById('kpis').innerHTML = cards.map(([label, value, note]) => `
    <div class="stat"><div class="stat-label">${label}</div><div class="stat-value">${value}</div><div class="stat-note">${note}</div></div>
  `).join('');
  document.getElementById('publishBadge').innerHTML = badge(readiness.status || 'investigation');
  document.getElementById('outputDir').textContent = report.output_dir || '';
}
function renderExecutive() {
  const readiness = report.publish_readiness || {};
  const horizontal = firstHorizontalSummary();
  if (horizontal) {
    const fastest = fastestCell(horizontal);
    const memory = lowestMemoryCell(horizontal);
    const commonAttempts = Number(horizontal?.common_success?.attempts || 0);
    const cohortNote = commonAttempts > 0 ? ` over ${commonAttempts} common-success attempt(s)` : '';
    const failureTargets = targetNames(horizontal.targets || {}).filter(target => targetFailures(horizontal, target) > 0);
    document.getElementById('executiveSummary').innerHTML = `
      <div class="finding-list">
        <div class="finding">Horizontal source: <strong>${htmlEscape(horizontal.suite)}</strong>. Gate target: <strong>${htmlEscape(horizontal.gate_target || 'moli')}</strong>.</div>
        <div class="finding">${fastest ? `Fastest comparable-success p50: <strong>${htmlEscape(targetLabel(horizontal.targets, fastest.target))}</strong> / ${htmlEscape(fastest.caseName)} at ${number(fastest.result.elapsed_ms.p50)} ms${cohortNote}.` : 'No comparable successful latency cell is available.'}</div>
        <div class="finding">${memory ? `Lowest comparable-success PSS: <strong>${htmlEscape(targetLabel(horizontal.targets, memory.target))}</strong> / ${htmlEscape(memory.caseName)} at ${mib(memory.result.peak_pss_bytes.p50)}${cohortNote}.` : 'No comparable successful memory cell is available.'}</div>
        <div class="finding ${failureTargets.length ? 'warn' : ''}">${failureTargets.length ? `Targets with comparison failures: <strong>${htmlEscape(failureTargets.map(target => targetLabel(horizontal.targets, target)).join(', '))}</strong>.` : 'No comparison target failures in this suite.'}</div>
      </div>
    `;
    document.getElementById('runNotes').innerHTML = `
      <div class="finding-list">
        <div class="finding">Publication status: ${badge(readiness.status || 'investigation')}</div>
        <div class="finding">Total target failures: <strong>${htmlEscape(horizontal.total_failures ?? 0)}</strong>. Gate failures: <strong>${htmlEscape(horizontal.gate_failures ?? 0)}</strong>.</div>
        <div class="finding">Open <a href="report-data.json">report-data.json</a> for renderer-independent data.</div>
      </div>
    `;
  } else {
    document.getElementById('executiveSummary').innerHTML = '<p class="notice">No horizontal comparison suite is present in this report.</p>';
    document.getElementById('runNotes').innerHTML = '<p class="notice">No chartable horizontal data is present.</p>';
  }
}
function renderTargets() {
  const targets = report.versions?.targets || {};
  document.getElementById('targetCards').innerHTML = targetNames(targets).map(name => {
    const target = targets[name] || {};
    return `<article class="target-card">
      <h3><span>${htmlEscape(name)}</span>${badge(target.available ? 'PASS' : 'NOT RUN')}</h3>
      <div class="target-meta">
        <div><strong>Version</strong> ${htmlEscape(target.version || '')}</div>
        <div><strong>Size</strong> ${size(target.size_bytes)}</div>
        <div><strong>Path</strong> <code>${htmlEscape(target.path || '')}</code></div>
        <div><strong>SHA256</strong> <code>${htmlEscape(target.sha256 || '')}</code></div>
      </div>
    </article>`;
  }).join('');
}
function table(headers, rows) {
  return `<table><thead><tr>${headers.map(h => `<th>${htmlEscape(h)}</th>`).join('')}</tr></thead><tbody>${rows.join('')}</tbody></table>`;
}
function renderReadiness() {
  const checks = report.publish_readiness?.checks || [];
  const rows = checks.map(check => `<tr>
    <th>${htmlEscape(check.name)}</th>
    <td>${badge(check.ok ? 'PASS' : 'FAIL')}</td>
    <td>${htmlEscape(JSON.stringify(check.actual ?? ''))}</td>
    <td>${htmlEscape(JSON.stringify(check.required ?? ''))}</td>
    <td>${htmlEscape(check.detail || '')}</td>
  </tr>`);
  document.getElementById('readinessTable').innerHTML = table(['check', 'gate', 'actual', 'required', 'detail'], rows);
}
function renderSuites() {
  const rows = (report.summaries || []).map(summary => `<tr>
    <th>${htmlEscape(summary.suite)}</th>
    <td>${htmlEscape(summary.profile || '')}</td>
    <td>${htmlEscape(summary.gate_target || '')}</td>
    <td>${htmlEscape(summary.gate_failures ?? '')}</td>
    <td>${htmlEscape(summary.total_failures ?? '')}</td>
    <td>${htmlEscape(summary.runs ?? '')}</td>
    <td>${htmlEscape(summary.timeout_seconds ?? '')}</td>
  </tr>`);
  document.getElementById('suiteTable').innerHTML = table(['suite', 'profile', 'gate target', 'gate failures', 'all failures', 'runs', 'timeout'], rows);
}
function comparisonSummaryHtml(summary) {
  const targets = targetNames(summary.targets);
  if (isAggregateSummary(summary)) {
    const rows = targets.map(target => {
      const result = summary.targets[target] || {};
      const successfulElapsed = result.successful_elapsed_ms || result.elapsed_ms || {};
      const successfulPss = result.successful_peak_pss_bytes || result.peak_pss_bytes || {};
      const common = summary?.common_success?.targets?.[target] || {};
      const commonAttempts = Number(summary?.common_success?.attempts || 0);
      const exclusions = [
        Number(result.excluded_runs || 0) ? `excluded: ${result.excluded_runs}` : '',
        Number(result.non_comparable_attempts || 0) ? `neutral: ${result.non_comparable_attempts}` : '',
      ].filter(Boolean).join('; ');
      return `<tr>
        <th>${htmlEscape(targetLabel(summary.targets, target))}</th>
        <td>${passRate(result)}</td>
        <td>${rawPassRate(result)}</td>
        <td>${htmlEscape(exclusions)}</td>
        <td>${htmlEscape(siteStability(result))}</td>
        <td>${htmlEscape(statusCoverage(result))}</td>
        <td>${htmlEscape(result.successes ?? result.categories?.success ?? result.passes ?? '')}</td>
        <td>${htmlEscape(result.failures ?? '')}</td>
        <td>${number(successfulElapsed.p50)} / ${number(successfulElapsed.p90)} / ${number(successfulElapsed.p95)}</td>
        <td>${mib(successfulPss.p50)} / ${mib(successfulPss.p95)}</td>
        <td>${commonAttempts || ''}</td>
        <td>${number(common.elapsed_ms?.p50)} / ${number(common.elapsed_ms?.p90)} / ${number(common.elapsed_ms?.p95)}</td>
        <td>${mib(common.peak_pss_bytes?.p50)} / ${mib(common.peak_pss_bytes?.p95)}</td>
        <td>${breakdown(result.failure_kinds)}</td>
      </tr>`;
    });
    const pairwise = Array.isArray(summary.pairwise) ? summary.pairwise : [];
    const pairRows = pairwise.map(item => `<tr>
      <th>${htmlEscape(targetLabel(summary.targets, item.left))} vs ${htmlEscape(targetLabel(summary.targets, item.right))}</th>
      <td>${badge(item.repeat_validated ? 'REPEAT VALIDATED' : 'SINGLE SAMPLE')}</td>
      <td>${htmlEscape(item.both_all_pass ?? '')}</td>
      <td>${htmlEscape(item.left_all_pass_right_all_fail ?? '')}</td>
      <td>${htmlEscape(item.right_all_pass_left_all_fail ?? '')}</td>
      <td>${htmlEscape(item.both_all_fail ?? '')}</td>
      <td>${htmlEscape(item.flaky_or_incomplete ?? '')}</td>
      <td>${htmlEscape(item.not_comparable ?? '')}</td>
      <td>${htmlEscape(item.common_success?.attempts ?? '')}</td>
    </tr>`);
    const pairTable = pairRows.length
      ? `<h4>Pairwise site outcomes</h4>${table(['targets', 'validation', 'both pass', 'left only', 'right only', 'both fail', 'flaky / incomplete', 'not comparable', 'common-success attempts'], pairRows)}`
      : '';
    const contract = summary.snapshot_contract
      ? `<p class="muted">Snapshot contract: ${htmlEscape(summary.snapshot_contract)}</p>`
      : '';
    return contract + table(
      ['target', 'counted pass rate', 'raw pass rate', 'excluded / neutral', 'site stability', 'HTTP status observed', 'success', 'failures', 'successful p50 / p90 / p95 ms', 'successful PSS p50 / p95', 'common n', 'common p50 / p90 / p95 ms', 'common PSS p50 / p95', 'failure kinds'],
      rows,
    ) + pairTable;
  }
  const cases = summaryCases(summary);
  const rows = cases.map(caseName => {
    const cells = targets.map(target => {
      const result = resultFor(summary, target, caseName);
      const failures = Number(result.failures || 0);
      const count = Number(result.elapsed_ms?.count || 0);
      const status = failures === 0 && count > 0 ? 'PASS' : 'FAIL';
      const sample = (result.failure_samples || [])[0];
      return `<td>${statusBadge(status, sample)}<br><span class="muted">p50</span> ${number(result.elapsed_ms?.p50)} ms<br><span class="muted">PSS</span> ${mib(result.peak_pss_bytes?.p50)}<br><span class="muted">RSS</span> ${mib(result.peak_rss_bytes?.p50)}<br><span class="muted">failures</span> ${failures}</td>`;
    }).join('');
    return `<tr><th>${htmlEscape(caseName)}</th>${cells}</tr>`;
  });
  return table(['case', ...targets.map(target => targetLabel(summary.targets, target))], rows);
}
function renderComparisonTable() {
  const summaries = horizontalSummaries();
  if (!summaries.length) {
    document.getElementById('comparisonTable').innerHTML = '<p class="muted">No horizontal comparison data.</p>';
    return;
  }
  document.getElementById('comparisonTable').innerHTML = summaries.map(summary => `
    <section class="comparison-summary">
      <h3>${htmlEscape(summary.suite || 'comparison')}</h3>
      ${comparisonSummaryHtml(summary)}
    </section>
  `).join('');
}
function renderWebPages() {
  const section = document.getElementById('web-pages');
  const data = report.render_compare || {};
  const runs = Array.isArray(data.runs) ? data.runs : [];
  const baselineSites = Array.isArray(data.baseline_sites) ? data.baseline_sites : [];
  if (!runs.length && !baselineSites.length) {
    section.style.display = 'none';
    return;
  }
  const evaluatedSites = new Set(runs.map(row => `${row.rank}|${row.domain}`)).size;
  const targetRequests = runs.length;
  const failures = runs.filter(row => !row.ok && !row.excluded).length;
  const worst = runs
    .filter(row => Number.isFinite(Number(row.render_quality_score)))
    .sort((a, b) => Number(a.render_quality_score) - Number(b.render_quality_score))[0];
  const cards = [
    ['Baseline URLs', String(baselineSites.length || evaluatedSites), 'Chrome baseline requests used to build the evaluated set.'],
    ['Evaluated Pages', String(evaluatedSites), 'Pages that reached target scoring after baseline filtering.'],
    ['Target Requests', String(targetRequests), 'Browser/page request rows shown below.'],
    ['Worst Score', worst ? `${number(worst.render_quality_score)} ${worst.target}` : '', worst ? shortUrl(worst.url || worst.domain) : 'No scored target rows.'],
  ];
  document.getElementById('webPageKpis').innerHTML = cards.map(([label, value, note]) => `
    <div class="stat"><div class="stat-label">${htmlEscape(label)}</div><div class="stat-value">${htmlEscape(value)}</div><div class="stat-note">${htmlEscape(note)}</div></div>
  `).join('');

  const baselineByKey = new Map(baselineSites.map(row => [`${row.rank}|${row.domain}`, row]));
  const rows = runs
    .slice()
    .sort((a, b) => Number(a.rank) - Number(b.rank) || String(a.target).localeCompare(String(b.target)))
    .map(row => {
      const baseline = baselineByKey.get(`${row.rank}|${row.domain}`) || {};
      const url = row.url || row.domain;
      const artifact = row.failure_artifact ? `<br><a href="render-compare/${htmlEscape(row.failure_artifact)}">failure artifact</a>` : '';
      return `<tr>
        <th class="metric-tight">${htmlEscape(row.rank)}</th>
        <td class="url-cell"><a href="${htmlEscape(url)}">${htmlEscape(shortUrl(url))}</a><br><span class="muted">${htmlEscape(baseline.baseline_title || '')}</span>${artifact}</td>
        <td>${htmlEscape(row.label || row.target)}</td>
        <td>${renderCategory(row.category, row.ok)}</td>
        <td>${scoreBar(row.render_quality_score)}</td>
        <td>${scoreBar(row.raw_content_score)}</td>
        <td class="metric-tight">${number(row.elapsed_ms)} ms</td>
        <td class="metric-tight">${mib(row.peak_rss_bytes)}</td>
        <td class="metric-tight">${mib(row.peak_pss_bytes)}</td>
        <td class="metric-tight">${number(row.baseline_elapsed_ms)} ms</td>
        <td>${htmlEscape(row.target_fetch_category || '')}<br><span class="muted">${htmlEscape(row.baseline_category || '')}</span></td>
        <td class="metric-tight">${htmlEscape(row.target_visible_text_length ?? '')} / ${htmlEscape(row.baseline_visible_text_length ?? '')}</td>
        <td class="metric-tight">${number(row.ngram_containment)} / ${number(row.key_phrase_hit_rate)}</td>
      </tr>`;
    });
  document.getElementById('webPageTable').innerHTML = table(
    ['rank', 'web page', 'target', 'result', 'render score', 'raw score', 'elapsed', 'RSS', 'PSS', 'Chrome elapsed', 'fetch categories', 'visible chars', 'ngram / key hits'],
    rows,
  );
}
function renderArtifacts() {
  const artifacts = report.artifact_paths_by_suite || {};
  const rows = Object.entries(artifacts).map(([suite, paths]) => `<tr>
    <th>${htmlEscape(suite)}</th>
    <td>${(paths || []).map(path => `<a href="${htmlEscape(path)}">${htmlEscape(path)}</a>`).join(' · ')}</td>
  </tr>`);
  const top = ['report-data.json', 'summary.json', 'summary.md', 'versions.json', 'publish-readiness.json', 'index.html'];
  rows.unshift(`<tr><th>top-level</th><td>${top.map(path => `<a href="${path}">${path}</a>`).join(' · ')}</td></tr>`);
  document.getElementById('artifactTable').innerHTML = table(['suite', 'artifacts'], rows);
}
function renderDiff() {
  const diff = report.report_diff;
  if (!diff) {
    document.getElementById('reportDiff').innerHTML = '<p class="muted">No baseline report was provided.</p>';
    return;
  }
  const summary = diff.summary || {};
  document.getElementById('reportDiff').innerHTML = table(['metric', 'value'], [
    `<tr><th>baseline</th><td><code>${htmlEscape(diff.baseline || '')}</code></td></tr>`,
    `<tr><th>added suites</th><td>${htmlEscape(summary.added ?? '')}</td></tr>`,
    `<tr><th>removed suites</th><td>${htmlEscape(summary.removed ?? '')}</td></tr>`,
    `<tr><th>changed suites</th><td>${htmlEscape(summary.changed ?? '')}</td></tr>`,
    `<tr><th>gate failures delta</th><td>${htmlEscape(summary.gate_failures_delta ?? '')}</td></tr>`,
    `<tr><th>total failures delta</th><td>${htmlEscape(summary.total_failures_delta ?? '')}</td></tr>`,
  ]);
}
function renderCharts() {
  const summary = firstHorizontalSummary();
  if (!summary || !window.Chart) return;
  const targets = targetNames(summary.targets || {});
  const cases = summaryCases(summary);
  const comparisonChartHeight = Math.max(420, cases.length * Math.max(34, targets.length * 7));
  document.getElementById('latencyChart').parentElement.style.height = `${comparisonChartHeight}px`;
  document.getElementById('memoryChart').parentElement.style.height = `${comparisonChartHeight}px`;
  document.getElementById('rssChart').parentElement.style.height = `${comparisonChartHeight}px`;
  const datasets = targets.map(target => ({
    label: targetLabel(summary.targets, target),
    backgroundColor: targetColor(summary.targets, target),
    borderColor: targetColor(summary.targets, target),
    data: cases.map(caseName => metricFor(summary, target, caseName, 'elapsed_ms')),
  }));
  const memoryDatasets = targets.map(target => ({
    label: targetLabel(summary.targets, target),
    backgroundColor: targetColor(summary.targets, target),
    borderColor: targetColor(summary.targets, target),
    data: cases.map(caseName => {
      const value = metricFor(summary, target, caseName, 'peak_pss_bytes');
      return value == null ? null : value / 1024 / 1024;
    }),
  }));
  const rssDatasets = targets.map(target => ({
    label: targetLabel(summary.targets, target),
    backgroundColor: targetColor(summary.targets, target),
    borderColor: targetColor(summary.targets, target),
    data: cases.map(caseName => {
      const value = metricFor(summary, target, caseName, 'peak_rss_bytes');
      return value == null ? null : value / 1024 / 1024;
    }),
  }));
  const chartOptions = (label) => ({
    indexAxis: 'y',
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: {
        position: 'top',
        align: 'end',
        labels: {
          boxWidth: 10,
          boxHeight: 10,
          padding: 14,
          usePointStyle: true,
          pointStyle: 'rectRounded',
        },
      },
      tooltip: { callbacks: { label: ctx => `${ctx.dataset.label}: ${number(ctx.parsed.x)} ${label}` } },
    },
    scales: {
      x: { beginAtZero: true, grid: { color: '#e6ebf1' }, title: { display: true, text: label } },
      y: { grid: { display: false }, ticks: { autoSkip: false } },
    },
  });
  new Chart(document.getElementById('latencyChart'), {
    type: 'bar',
    data: { labels: cases, datasets },
    options: chartOptions('ms'),
  });
  new Chart(document.getElementById('memoryChart'), {
    type: 'bar',
    data: { labels: cases, datasets: memoryDatasets },
    options: chartOptions('MiB'),
  });
  new Chart(document.getElementById('rssChart'), {
    type: 'bar',
    data: { labels: cases, datasets: rssDatasets },
    options: chartOptions('MiB'),
  });
  new Chart(document.getElementById('failureChart'), {
    type: 'bar',
    data: {
      labels: targets.map(target => targetLabel(summary.targets, target)),
      datasets: [{
        label: 'failures',
        backgroundColor: targets.map(target => targetColor(summary.targets, target)),
        data: targets.map(target => targetFailures(summary, target)),
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: { legend: { display: false } },
      scales: { y: { beginAtZero: true, precision: 0 }, x: { grid: { display: false }, ticks: { display: false } } },
    },
  });
}
function renderWebPageCharts() {
  const data = report.render_compare || {};
  const runs = Array.isArray(data.runs) ? data.runs : [];
  if (!runs.length || !window.Chart) return;
  const chartRows = runs
    .filter(row => Number.isFinite(Number(row.render_quality_score)))
    .slice()
    .sort((a, b) => Number(a.render_quality_score) - Number(b.render_quality_score))
    .slice(0, 32);
  const labels = chartRows.map(row => `#${row.rank} ${row.target} ${shortUrl(row.domain || row.url)}`);
  const scoreColors = chartRows.map(row => targetColor({[row.target]: row}, row.target));
  new Chart(document.getElementById('pageScoreChart'), {
    type: 'bar',
    data: {
      labels,
      datasets: [{
        label: 'render quality score',
        data: chartRows.map(row => Number(row.render_quality_score)),
        backgroundColor: scoreColors,
      }],
    },
    options: {
      indexAxis: 'y',
      responsive: true,
      maintainAspectRatio: false,
      plugins: { legend: { display: false } },
      scales: {
        x: { min: 0, max: 100, grid: { color: '#e6ebf1' }, title: { display: true, text: 'score' } },
        y: { grid: { display: false }, ticks: { autoSkip: false } },
      },
    },
  });
  const latencyRows = runs
    .filter(row => Number.isFinite(Number(row.elapsed_ms)))
    .slice()
    .sort((a, b) => Number(b.elapsed_ms) - Number(a.elapsed_ms))
    .slice(0, 32);
  new Chart(document.getElementById('pageLatencyChart'), {
    type: 'bar',
    data: {
      labels: latencyRows.map(row => `#${row.rank} ${row.target} ${shortUrl(row.domain || row.url)}`),
      datasets: [{
        label: 'elapsed ms',
        data: latencyRows.map(row => Number(row.elapsed_ms)),
        backgroundColor: latencyRows.map(row => targetColor({[row.target]: row}, row.target)),
      }],
    },
    options: {
      indexAxis: 'y',
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: { display: false },
        tooltip: { callbacks: { afterLabel: ctx => `RSS: ${mib(latencyRows[ctx.dataIndex]?.peak_rss_bytes)}` } },
      },
      scales: {
        x: { beginAtZero: true, grid: { color: '#e6ebf1' }, title: { display: true, text: 'ms' } },
        y: { grid: { display: false }, ticks: { autoSkip: false } },
      },
    },
  });
}
renderKpis();
renderExecutive();
renderTargets();
renderReadiness();
renderSuites();
renderComparisonTable();
renderWebPages();
renderArtifacts();
renderDiff();
renderCharts();
renderWebPageCharts();
</script>
</body>
</html>
"""
    return document.replace("__REPORT_DATA__", data)


def _publish_readiness(readiness: dict[str, Any] | None) -> str:
    if not readiness:
        return ""
    rows = []
    for check in readiness.get("checks", []):
        if not isinstance(check, dict):
            continue
        rows.append(
            "<tr>"
            f"<th>{_cell(check.get('name'))}</th>"
            f"<td>{_status(bool(check.get('ok')))}</td>"
            f"<td>{_cell(check.get('actual'))}</td>"
            f"<td>{_cell(check.get('required'))}</td>"
            f"<td>{_cell(check.get('detail'))}</td>"
            "</tr>"
        )
    status = str(readiness.get("status", "investigation")).upper()
    return (
        "<h2>Publication Readiness</h2>"
        f"<p>Publication status: {_status_badge('PASS' if status == 'PUBLISHABLE' else 'PARTIAL')} "
        f"<code>{html.escape(status.lower())}</code></p>"
        "<table>"
        "<thead><tr><th>check</th><th>gate</th><th>actual</th><th>required</th><th>detail</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _report_diff(report_diff: dict[str, Any] | None) -> str:
    if not report_diff:
        return ""
    summary = report_diff.get("summary", {})
    rows = [
        ("added suites", summary.get("added")),
        ("removed suites", summary.get("removed")),
        ("changed suites", summary.get("changed")),
        ("unchanged suites", summary.get("unchanged")),
        ("gate failures delta", summary.get("gate_failures_delta")),
        ("total failures delta", summary.get("total_failures_delta")),
    ]
    body = "".join(f"<tr><th>{html.escape(label)}</th><td>{_cell(value)}</td></tr>" for label, value in rows)
    return (
        "<h3>Previous Report Diff</h3>"
        f"<p>Baseline: <code>{_cell(report_diff.get('baseline'))}</code></p>"
        "<table>"
        "<thead><tr><th>metric</th><th>value</th></tr></thead>"
        f"<tbody>{body}</tbody>"
        "</table>"
    )


def _bar(width_percent: float, label: str, *, class_name: str = "") -> str:
    width = max(0.0, min(100.0, width_percent))
    return (
        f'<div class="bar-track {html.escape(class_name)}">'
        f'<div class="bar-fill" style="width:{width:.2f}%"></div>'
        f'<span>{html.escape(label)}</span>'
        "</div>"
    )


def _synthetic_compare(summary: dict[str, Any]) -> str:
    targets = summary.get("targets", {})
    cases = summary.get("cases", [])
    rows = []
    for case in cases:
        cells = [f"<th>{html.escape(str(case))}</th>"]
        for target_name in _target_names(targets):
            result = targets[target_name].get("cases", {}).get(case, {})
            elapsed = result.get("elapsed_ms", {})
            pss = result.get("peak_pss_bytes", {})
            failures = int(result.get("failures", 0) or 0)
            count = int(elapsed.get("count", 0) or 0)
            ok = failures == 0 and count > 0
            cells.append(
                "<td>"
                f"{_status(ok)}"
                f"<div class=\"metric\">p50 {_cell(elapsed.get('p50'))} ms</div>"
                f"<div class=\"metric\">p95 {_cell(elapsed.get('p95'))} ms</div>"
                f"<div class=\"metric\">PSS p50 {_mib(pss.get('p50'))}</div>"
                f"<div class=\"metric\">failures {failures}</div>"
                "</td>"
            )
        rows.append("<tr>" + "".join(cells) + "</tr>")
    header = "".join(f"<th>{html.escape(str(name))}</th>" for name in _target_names(targets))
    return (
        "<table>"
        f"<thead><tr><th>case</th>{header}</tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _synthetic_compare_chart(summary: dict[str, Any]) -> str:
    targets = summary.get("targets", {})
    cases = summary.get("cases", [])
    rows = []
    for case in cases:
        values: list[tuple[str, float]] = []
        for target_name in _target_names(targets):
            elapsed = targets.get(target_name, {}).get("cases", {}).get(case, {}).get("elapsed_ms", {})
            value = _num(elapsed.get("p50"))
            if value is not None:
                values.append((target_name, value))
        if not values:
            continue
        max_value = max(value for _, value in values) or 1.0
        bars = "".join(
            "<div class=\"chart-row\">"
            f"<span class=\"chart-target\">{html.escape(target_name)}</span>"
            f"{_bar((value / max_value) * 100.0, f'{value:.2f} ms', class_name=target_name)}"
            "</div>"
            for target_name, value in values
        )
        rows.append(
            "<section class=\"chart-group\">"
            f"<h3>{html.escape(str(case))}</h3>"
            f"{bars}"
            "</section>"
        )
    if not rows:
        return ""
    return "<div class=\"chart-grid\">" + "".join(rows) + "</div>"


def _synthetic_advantages(summary: dict[str, Any]) -> str:
    targets = summary.get("targets", {})
    cases = summary.get("cases", [])
    wins = {
        target: {"speed": 0, "memory": 0, "passes": 0, "total": 0}
        for target in targets
    }
    for case in cases:
        speed_values: list[tuple[str, float]] = []
        memory_values: list[tuple[str, float]] = []
        for target_name, target_summary in targets.items():
            result = target_summary.get("cases", {}).get(case, {})
            elapsed = result.get("elapsed_ms", {})
            pss = result.get("peak_pss_bytes", {})
            failures = int(result.get("failures", 0) or 0)
            count = int(elapsed.get("count", 0) or 0)
            wins[target_name]["total"] += 1
            if failures == 0 and count > 0:
                wins[target_name]["passes"] += 1
                if elapsed.get("p50") is not None:
                    speed_values.append((target_name, float(elapsed["p50"])))
                if pss.get("p50") is not None:
                    memory_values.append((target_name, float(pss["p50"])))
        if speed_values:
            fastest = min(speed_values, key=lambda item: item[1])[0]
            wins[fastest]["speed"] += 1
        if memory_values:
            lowest_memory = min(memory_values, key=lambda item: item[1])[0]
            wins[lowest_memory]["memory"] += 1

    rows = []
    for target_name in _target_names(targets):
        result = wins[target_name]
        rows.append(
            "<tr>"
            f"<th>{html.escape(str(target_name))}</th>"
            f"<td>{result['passes']} / {result['total']}</td>"
            f"<td>{result['speed']}</td>"
            f"<td>{result['memory']}</td>"
            "</tr>"
        )
    return (
        "<table>"
        "<thead><tr><th>target</th><th>passes</th><th>fastest cases</th><th>lowest PSS cases</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _target_metric_table(summary: dict[str, Any], *, label: str) -> str:
    targets = summary.get("targets", {})
    rows = []
    for target_name in _target_names(targets):
        result = targets[target_name]
        elapsed = result.get("elapsed_ms", {})
        pss = result.get("peak_pss_bytes", result.get("browser_peak_pss_bytes", {}))
        failures = int(result.get("failures", 0) or 0)
        passes = result.get("passes")
        total = result.get("pages", result.get("seeds", result.get("runs")))
        rows.append(
            "<tr>"
            f"<th>{html.escape(str(target_name))}</th>"
            f"<td>{_status(failures == 0 and int(elapsed.get('count', 0) or 0) > 0)}</td>"
            f"<td>{_cell(passes)} / {_cell(total)}</td>"
            f"<td>{_cell(elapsed.get('p50'))}</td>"
            f"<td>{_cell(elapsed.get('p95'))}</td>"
            f"<td>{_mib(pss.get('p50'))}</td>"
            f"<td>{_mib(pss.get('p95'))}</td>"
            f"<td>{failures}</td>"
            "</tr>"
        )
    return (
        "<table>"
        f"<thead><tr><th>{html.escape(label)}</th><th>gate</th><th>passes</th><th>p50 ms</th><th>p95 ms</th><th>PSS p50</th><th>PSS p95</th><th>failures</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _wpt(summary: dict[str, Any]) -> str:
    counts = summary.get("summary", {})
    rows = [
        ("total", counts.get("total")),
        ("pass", counts.get("pass")),
        ("known-fail", counts.get("known_fail")),
        ("unexpected-fail", counts.get("unexpected_fail")),
        ("unexpected-pass", counts.get("unexpected_pass")),
        ("skip", counts.get("skip")),
        ("pass rate", f"{float(counts['pass_rate_percent']):.2f}%" if counts.get("pass_rate_percent") is not None else None),
        (
            "unexpected-fail rate",
            f"{float(counts['unexpected_fail_rate_percent']):.2f}%" if counts.get("unexpected_fail_rate_percent") is not None else None,
        ),
        ("skip rate", f"{float(counts['skip_rate_percent']):.2f}%" if counts.get("skip_rate_percent") is not None else None),
    ]
    summary_rows = "".join(
        "<tr>"
        f"<th>{html.escape(label)}</th>"
        f"<td>{_cell(value)}</td>"
        "</tr>"
        for label, value in rows
    )
    tag_rows = []
    for tag, tag_counts in sorted((counts.get("by_tag") or {}).items()):
        pass_rate = tag_counts.get("pass_rate_percent")
        tag_rows.append(
            "<tr>"
            f"<th>{html.escape(str(tag))}</th>"
            f"<td>{_cell(tag_counts.get('total'))}</td>"
            f"<td>{_cell(tag_counts.get('pass'))}</td>"
            f"<td>{_cell(tag_counts.get('unexpected_fail'))}</td>"
            f"<td>{_cell(tag_counts.get('skip'))}</td>"
            f"<td>{_cell(f'{float(pass_rate):.2f}%' if pass_rate is not None else None)}</td>"
            "</tr>"
        )
    tag_table = (
        "<h3>By Tag</h3>"
        "<table>"
        "<thead><tr><th>tag</th><th>total</th><th>pass</th><th>unexpected fail</th><th>skip</th><th>pass rate</th></tr></thead>"
        f"<tbody>{''.join(tag_rows)}</tbody>"
        "</table>"
        if tag_rows
        else ""
    )
    diff = summary.get("diff")
    diff_table = ""
    if isinstance(diff, dict):
        diff_rows = [
            ("added", diff.get("added")),
            ("removed", diff.get("removed")),
            ("expectation changes", diff.get("expectation_changes")),
            ("category changes", diff.get("category_changes")),
            ("total changes", diff.get("total_changes")),
        ]
        diff_body = "".join(
            "<tr>"
            f"<th>{html.escape(label)}</th>"
            f"<td>{_cell(value)}</td>"
            "</tr>"
            for label, value in diff_rows
        )
        diff_table = (
            "<h3>Baseline Diff</h3>"
            f"<p>Baseline: <code>{_cell(summary.get('baseline'))}</code></p>"
            "<table>"
            "<thead><tr><th>change</th><th>count</th></tr></thead>"
            f"<tbody>{diff_body}</tbody>"
            "</table>"
        )
    return (
        "<table>"
        "<thead><tr><th>metric</th><th>value</th></tr></thead>"
        f"<tbody>{summary_rows}</tbody>"
        "</table>"
        f"{tag_table}"
        f"{diff_table}"
    )


def _wild_web(summary: dict[str, Any]) -> str:
    targets = summary.get("targets", {})
    rows = []
    replay_note = (
        f"<p>Replay capture: <code>{_cell(summary.get('replay_capture'))}</code>. "
        f"Replay artifacts: <code>{_cell(summary.get('replay_artifacts'))}</code>.</p>"
    )
    for target_name in _target_names(targets):
        result = targets[target_name]
        elapsed = result.get("elapsed_ms", {})
        pss = result.get("peak_pss_bytes", {})
        failures = int(result.get("failures", 0) or 0)
        failure_kinds = result.get("failure_kinds", {})
        categories = result.get("categories", {})
        rows.append(
            "<tr>"
            f"<th>{html.escape(str(target_name))}</th>"
            f"<td>{_status(failures == 0 and int(result.get('seeds', 0) or 0) > 0)}</td>"
            f"<td>{_cell(result.get('passes'))} / {_cell(result.get('seeds'))}</td>"
            f"<td>{_cell(elapsed.get('p50'))}</td>"
            f"<td>{_mib(pss.get('p50'))}</td>"
            f"<td>{_cell(result.get('extraction_failures'))}</td>"
            f"<td>{_cell(failure_kinds)}</td>"
            f"<td>{_cell(categories)}</td>"
            "</tr>"
        )
    return replay_note + (
        "<table>"
        "<thead><tr><th>target</th><th>gate</th><th>passes</th><th>p50 ms</th><th>PSS p50</th><th>extraction failures</th><th>failure kinds</th><th>categories</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _metric_cell(result: dict[str, Any]) -> str:
    elapsed = result.get("elapsed_ms", {})
    pss = result.get("peak_pss_bytes", result.get("browser_peak_pss_bytes", {}))
    failures = int(result.get("failures", 0) or 0)
    ok = failures == 0 and int(elapsed.get("count", 0) or 0) > 0
    pss_metric = f"<div class=\"metric\">PSS p50 {_mib(pss.get('p50'))}</div>" if pss.get("p50") is not None else ""
    return (
        "<td>"
        f"{_status(ok)}"
        f"<div class=\"metric\">p50 {_cell(elapsed.get('p50'))} ms</div>"
        f"<div class=\"metric\">p95 {_cell(elapsed.get('p95'))} ms</div>"
        f"{pss_metric}"
        f"<div class=\"metric\">failures {failures}</div>"
        "</td>"
    )


def _headline_compare(summaries: list[dict[str, Any]]) -> str:
    target_names: list[str] = []
    for summary in summaries:
        for target_name in _target_names(summary.get("targets", {})):
            if target_name not in target_names:
                target_names.append(target_name)
    if not target_names:
        return ""

    rows = []
    for summary in summaries:
        suite = summary.get("suite")
        targets = summary.get("targets", {})
        if not targets:
            continue
        if suite in {"synthetic-compare", "cdp-session"}:
            for case in summary.get("cases", []):
                cells = [f"<th>{html.escape(str(suite))}<div class=\"metric\">{html.escape(str(case))}</div></th>"]
                for target_name in target_names:
                    result = targets.get(target_name, {}).get("cases", {}).get(case, {})
                    cells.append(_metric_cell(result) if result else "<td></td>")
                rows.append("<tr>" + "".join(cells) + "</tr>")
        else:
            cells = [f"<th>{html.escape(str(suite))}</th>"]
            for target_name in target_names:
                result = targets.get(target_name, {})
                cells.append(_metric_cell(result) if result else "<td></td>")
            rows.append("<tr>" + "".join(cells) + "</tr>")
    if not rows:
        return ""
    header = "".join(f"<th>{html.escape(str(name))}</th>" for name in target_names)
    return (
        "<h2>Headline Compare</h2>"
        "<table class=\"headline\">"
        f"<thead><tr><th>suite / case</th>{header}</tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _headline_charts(summaries: list[dict[str, Any]]) -> str:
    sections = []
    synthetic_compare = next((summary for summary in summaries if summary.get("suite") == "synthetic-compare"), None)
    if synthetic_compare:
        chart = _synthetic_compare_chart(synthetic_compare)
        if chart:
            sections.append("<h2>Headline Charts</h2><p class=\"section-note\">Lower elapsed time is better. Bars are normalized per case.</p>" + chart)
    return "".join(sections)


def _cdp_session(summary: dict[str, Any]) -> str:
    targets = summary.get("targets", {})
    cases = summary.get("cases", [])
    rows = []
    for case in cases:
        cells = [f"<th>{html.escape(str(case))}</th>"]
        for target_name in _target_names(targets):
            result = targets[target_name].get("cases", {}).get(case, {})
            elapsed = result.get("elapsed_ms", {})
            failures = int(result.get("failures", 0) or 0)
            console_errors = int(result.get("console_errors", 0) or 0)
            js_exceptions = int(result.get("js_exceptions", 0) or 0)
            network_failures = int(result.get("network_failures", 0) or 0)
            cells.append(
                "<td>"
                f"{_status(failures == 0 and int(elapsed.get('count', 0) or 0) > 0)}"
                f"<div class=\"metric\">p50 {_cell(elapsed.get('p50'))} ms</div>"
                f"<div class=\"metric\">p95 {_cell(elapsed.get('p95'))} ms</div>"
                f"<div class=\"metric\">failures {failures}</div>"
                f"<div class=\"metric\">console errors {console_errors}</div>"
                f"<div class=\"metric\">JS exceptions {js_exceptions}</div>"
                f"<div class=\"metric\">network failures {network_failures}</div>"
                "</td>"
            )
        rows.append("<tr>" + "".join(cells) + "</tr>")
    header = "".join(f"<th>{html.escape(str(name))}</th>" for name in _target_names(targets))
    return (
        "<table>"
        f"<thead><tr><th>case</th>{header}</tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _cdp_smoke(summary: dict[str, Any]) -> str:
    client_rows = summary.get("client_rows", [])
    if not isinstance(client_rows, list):
        client_rows = []
    rows = []
    for row in client_rows:
        if not isinstance(row, dict):
            continue
        rows.append(
            "<tr>"
            f"<th>{_cell(row.get('client'))}</th>"
            f"<td>{_status(bool(row.get('covered')))}</td>"
            f"<td>{_status(bool(row.get('gate_ok')))}</td>"
            f"<td>{_cell(row.get('required'))}</td>"
            f"<td>{_cell(row.get('record_count'))}</td>"
            f"<td>{_cell(row.get('failure_kind'))}</td>"
            "</tr>"
        )
    groups = ", ".join(str(group) for group in summary.get("groups", []) if group)
    return (
        f"<p>Profile: <strong>{_cell(summary.get('profile', 'smoke'))}</strong>. "
        f"Records: <strong>{_cell(summary.get('total_records'))}</strong>. "
        f"Groups: {html.escape(groups)}.</p>"
        "<table>"
        "<thead><tr><th>client family</th><th>covered</th><th>formal gate</th><th>required</th><th>records</th><th>failure kind</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _comparison_notice(summaries: list[dict[str, Any]]) -> str:
    target_sets = [set((summary.get("targets") or {}).keys()) for summary in summaries if summary.get("targets")]
    if not target_sets:
        return ""
    all_targets = set().union(*target_sets)
    expected = {"moli", "lightpanda", "chrome", "obscura"}
    if expected.issubset(all_targets):
        return '<p class="notice ok-bg">This report includes moli, lightpanda, chrome, and obscura comparison data.</p>'
    missing = ", ".join(sorted(expected - all_targets))
    present = ", ".join(_target_names({target: {} for target in all_targets}))
    return (
        '<p class="notice warn-bg">'
        f"This report is not a full horizontal comparison. Present targets: {html.escape(present)}. "
        f"Missing targets: {html.escape(missing)}."
        "</p>"
    )


def _synthetic_matrix(summary: dict[str, Any]) -> str:
    cases = summary.get("cases", {})
    gate_rows = summary.get("formal_gate_rows", [])
    gate_table_rows = []
    if isinstance(gate_rows, list):
        for result in gate_rows:
            if not isinstance(result, dict):
                continue
            gate_table_rows.append(
                "<tr>"
                f"<th>{_cell(result.get('gate'))}</th>"
                f"<td>{_status(bool(result.get('ok')))}</td>"
                f"<td>{_cell(result.get('actual'))}</td>"
                f"<td>{_cell(result.get('required'))}</td>"
                f"<td>{_cell(result.get('failure_kind'))}</td>"
                "</tr>"
            )
    profile = _cell(summary.get("profile", "smoke"))
    profile_failures = int(summary.get("profile_failures", 0) or 0)
    requirements_table = (
        "<h3>Formal Gate Rows</h3>"
        "<table>"
        "<thead><tr><th>gate</th><th>status</th><th>actual</th><th>required</th><th>failure kind</th></tr></thead>"
        f"<tbody>{''.join(gate_table_rows)}</tbody>"
        "</table>"
        if gate_table_rows
        else ""
    )
    rows = []
    for case, by_concurrency in cases.items():
        if not isinstance(by_concurrency, dict):
            continue
        for concurrency, result in by_concurrency.items():
            elapsed = result.get("elapsed_p50_ms", {})
            failures = int(result.get("failures", 0) or 0)
            stable = bool(result.get("stable", True)) and failures == 0
            rows.append(
                "<tr>"
                f"<th>{html.escape(str(case))}</th>"
                f"<td>{_cell(concurrency)}</td>"
                f"<td>{_status(stable)}</td>"
                f"<td>{_cell(elapsed.get('p50'))}</td>"
                f"<td>{_cell(elapsed.get('p95'))}</td>"
                f"<td>{_cell(result.get('median_drift_percent'))}</td>"
                f"<td>{failures}</td>"
                "</tr>"
            )
    return (
        f"<p>Profile: <code>{profile}</code>. Profile failures: {profile_failures}.</p>"
        f"{requirements_table}"
        "<table>"
        "<thead><tr><th>case</th><th>concurrency</th><th>gate</th><th>p50 ms</th><th>p95 ms</th><th>median drift %</th><th>failures</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _startup_summary(summary: dict[str, Any]) -> str:
    cases = summary.get("cases", {})
    profile = str(summary.get("profile", "smoke"))
    gate_rows = summary.get("formal_gate_rows", [])
    time_note = (
        "available"
        if summary.get("time_verbose_available")
        else "unavailable in this environment; raw unavailability markers archived"
    )
    artifact_count = len(summary.get("time_verbose_artifacts") or [])
    cgroup_note = (
        "available"
        if summary.get("cgroup_available")
        else "unavailable in this environment; proc cgroup marker archived when possible"
    )
    cgroup_artifact_count = len(summary.get("cgroup_artifacts") or [])
    effective_kernel_modes = sorted(
        {
            mode
            for result in cases.values()
            for mode in (result.get("kernel_cache_modes") or [])
            if mode and mode != "not-applicable"
        }
    )
    effective_cache_note = ", ".join(str(mode) for mode in effective_kernel_modes) or "not recorded"
    rows = []
    for case, result in cases.items():
        elapsed = result.get("elapsed_ms", {})
        pss = result.get("pss_bytes", {})
        rss = result.get("rss_bytes", {})
        threads = result.get("thread_count", {})
        fds = result.get("fd_count", {})
        time_rss = result.get("time_max_rss_bytes", {})
        failures = int(result.get("failures", 0) or 0)
        cdp_page_p50 = result.get("cdp_page_elapsed_p50_ms")
        cdp_page_p95 = result.get("cdp_page_elapsed_p95_ms")
        binary_size = result.get("binary_bytes")
        stripped_size = result.get("stripped_binary_bytes")
        tar_gz_size = result.get("tar_gz_bytes")
        image_uncompressed_size = result.get("image_uncompressed_bytes")
        image_compressed_size = result.get("image_compressed_bytes")
        deploy_size = ""
        if binary_size is not None or stripped_size is not None or tar_gz_size is not None:
            deploy_size = (
                f"<div>bin {_size(binary_size)}</div>"
                f"<div class=\"metric\">stripped {_size(stripped_size)}</div>"
                f"<div class=\"metric\">tar.gz {_size(tar_gz_size)}</div>"
            )
        if image_uncompressed_size is not None or image_compressed_size is not None:
            deploy_size += (
                f"<div>rootfs {_size(image_uncompressed_size)}</div>"
                f"<div class=\"metric\">rootfs.tar.gz {_size(image_compressed_size)}</div>"
            )
        rows.append(
            "<tr>"
            f"<th>{html.escape(str(case))}</th>"
            f"<td>{_status(failures == 0)}</td>"
            f"<td>{_cell(elapsed.get('p50'))}</td>"
            f"<td>{_cell(elapsed.get('p95'))}</td>"
            f"<td>{_cell(cdp_page_p50)}</td>"
            f"<td>{_cell(cdp_page_p95)}</td>"
            f"<td>{deploy_size}</td>"
            f"<td>{_mib(pss.get('p50'))}</td>"
            f"<td>{_mib(rss.get('p50'))}</td>"
            f"<td>{_mib(time_rss.get('p50'))}</td>"
            f"<td>{_cell(threads.get('p50'))}</td>"
            f"<td>{_cell(fds.get('p50'))}</td>"
            f"<td>{failures}</td>"
            "</tr>"
        )
    gate_table = ""
    if isinstance(gate_rows, list) and gate_rows:
        rows_html = []
        for row in gate_rows:
            if not isinstance(row, dict):
                continue
            rows_html.append(
                "<tr>"
                f"<th>{html.escape(str(row.get('gate')))}</th>"
                f"<td>{_status(row.get('ok') is True)}</td>"
                f"<td>{html.escape(str(row.get('actual')))}</td>"
                f"<td>{html.escape(str(row.get('required')))}</td>"
                f"<td>{html.escape(str(row.get('failure_kind') or ''))}</td>"
                "</tr>"
            )
        if rows_html:
            gate_table = (
                "<h3>Startup Formal Gates</h3>"
                "<table>"
                "<thead><tr><th>gate</th><th>status</th><th>actual</th><th>required</th><th>failure kind</th></tr></thead>"
                f"<tbody>{''.join(rows_html)}</tbody>"
                "</table>"
            )
    return (
        f"<p class=\"section-note\">Profile: <code>{html.escape(profile)}</code>. "
        f"/usr/bin/time -v: {html.escape(time_note)}; artifacts: {artifact_count}. "
        f"cgroup raw files: {html.escape(cgroup_note)}; artifacts: {cgroup_artifact_count}. "
        f"OS cache mode: {html.escape(effective_cache_note)}"
        f"{' (drop-cache requested)' if summary.get('drop_os_cache') else ''}.</p>"
        f"{gate_table}"
        "<table>"
        "<thead><tr><th>case</th><th>gate</th><th>p50 ms</th><th>p95 ms</th><th>CDP page p50 ms</th><th>CDP page p95 ms</th><th>deploy size</th><th>PSS p50</th><th>RSS p50</th><th>/usr/bin/time max RSS p50</th><th>threads p50</th><th>fds p50</th><th>failures</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def _amiibo_crawler(summary: dict[str, Any]) -> str:
    targets = summary.get("targets", {})
    profile = _cell(summary.get("profile", "smoke"))
    profile_failures = int(summary.get("profile_failures", 0) or 0)
    requirements = summary.get("formal_requirements", {})
    requirement_rows = []
    if isinstance(requirements, dict):
        for name, result in requirements.items():
            if not isinstance(result, dict):
                continue
            requirement_rows.append(
                "<tr>"
                f"<th>{html.escape(str(name))}</th>"
                f"<td>{_status(bool(result.get('ok')))}</td>"
                f"<td>{_cell(result.get('actual'))}</td>"
                f"<td>{_cell(result.get('required'))}</td>"
                "</tr>"
            )
    requirements_table = (
        "<h3>Formal Profile Requirements</h3>"
        "<table>"
        "<thead><tr><th>requirement</th><th>gate</th><th>actual</th><th>required</th></tr></thead>"
        f"<tbody>{''.join(requirement_rows)}</tbody>"
        "</table>"
        if requirement_rows
        else ""
    )
    rows = []
    for target in _target_names(targets):
        target_result = targets[target]
        mode_results = target_result.get("modes")
        result_items = mode_results.items() if isinstance(mode_results, dict) and mode_results else [("", target_result)]
        for mode, result in result_items:
            elapsed = result.get("elapsed_ms", {})
            browser_pss = result.get("browser_peak_pss_bytes", {})
            failures = int(result.get("failures", 0) or 0)
            failure_kinds = result.get("failure_kinds", {})
            failure_note = ""
            if isinstance(failure_kinds, dict) and failure_kinds:
                failure_note = ", ".join(f"{html.escape(str(kind))}: {_cell(count)}" for kind, count in sorted(failure_kinds.items()))
            rows.append(
                "<tr>"
                f"<th>{html.escape(str(target))}</th>"
                f"<td>{html.escape(str(mode))}</td>"
                f"<td>{_status(failures == 0 and int(result.get('runs', 0) or 0) > 0)}</td>"
                f"<td>{_cell(result.get('passes'))} / {_cell(result.get('runs'))}</td>"
                f"<td>{_cell(elapsed.get('p50'))}</td>"
                f"<td>{_cell(elapsed.get('p95'))}</td>"
                f"<td>{_mib(browser_pss.get('p50'))}</td>"
                f"<td>{_cell(result.get('assertion_failures'))}</td>"
                f"<td>{failures}</td>"
                f"<td>{failure_note}</td>"
                "</tr>"
            )
    return (
        f"<p>Profile: <code>{profile}</code>. Profile failures: {profile_failures}.</p>"
        f"{requirements_table}"
        "<table>"
        "<thead><tr><th>target</th><th>mode</th><th>gate</th><th>passes</th><th>p50 ms</th><th>p95 ms</th><th>browser PSS p50</th><th>assertion failures</th><th>failures</th><th>failure kinds</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody>"
        "</table>"
    )


def write_benchmark_html(
    *,
    output_dir: Path,
    versions: dict[str, Any],
    summaries: list[dict[str, Any]],
    publish_readiness: dict[str, Any] | None = None,
    report_diff: dict[str, Any] | None = None,
) -> None:
    payload = _report_payload(
        output_dir=output_dir,
        versions=versions,
        summaries=summaries,
        publish_readiness=publish_readiness,
        report_diff=report_diff,
    )
    write_json(output_dir / "report-data.json", payload)
    write_text(output_dir / "index.html", _chartjs_document(payload))
