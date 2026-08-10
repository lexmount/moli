"""Render a Chart.js HTML report from a wpt_cross output directory.

The report intentionally keeps the runner data model intact: it reads the
``matrix.json`` + ``summary.json`` files emitted by
``python -m moli_benchmark.wpt_cross`` and derives visual summaries from
those files only.
"""

from __future__ import annotations

import argparse
import html
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


ENGINE_ORDER = ("moli", "lightpanda", "chrome", "obscura")
STATUS_ORDER = ("pass", "fail", "timeout", "crash", "harness-stalled", "error", "missing")
STATUS_LABELS = {
    "pass": "Pass",
    "fail": "Fail",
    "timeout": "Timeout",
    "crash": "Crash",
    "harness-stalled": "Harness stalled",
    "error": "Runner error",
    "missing": "Missing",
}
STATUS_COLOR = {
    "pass": "#1f8f4d",
    "fail": "#d64045",
    "timeout": "#b7791f",
    "crash": "#8b1e3f",
    "harness-stalled": "#7353ba",
    "error": "#c05621",
    "missing": "#6b7280",
}
ENGINE_COLOR = {
    "moli": "#2563eb",
    "lightpanda": "#0f766e",
    "chrome": "#d97706",
    "obscura": "#7c3aed",
}


def _group_key(case_path: str) -> str:
    parts = case_path.split("/", 2)
    return "/".join(parts[:2]) if len(parts) >= 2 else case_path


def _engines_from_summary(summary: dict[str, Any], matrix: list[dict[str, Any]]) -> list[str]:
    engines = list(summary.get("engines", {}).keys())
    if not engines and matrix:
        engines = list(matrix[0].get("results", {}).keys())
    ordered = [engine for engine in ENGINE_ORDER if engine in engines]
    ordered.extend(engine for engine in engines if engine not in ordered)
    return ordered


def _status_counts(summary: dict[str, Any], matrix: list[dict[str, Any]], engines: list[str]) -> dict[str, dict[str, int]]:
    out: dict[str, dict[str, int]] = {}
    for engine in engines:
        counts = Counter()
        summary_counts = summary.get("engines", {}).get(engine)
        if isinstance(summary_counts, dict):
            counts.update({str(k): int(v or 0) for k, v in summary_counts.items()})
        else:
            for row in matrix:
                counts[str(row.get("results", {}).get(engine, {}).get("status", "missing"))] += 1
        out[engine] = {status: counts.get(status, 0) for status in STATUS_ORDER if counts.get(status, 0)}
    return out


def _duration_ms(row: dict[str, Any], engine: str) -> float | None:
    value = row.get("results", {}).get(engine, {}).get("duration_ms")
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _median(values: list[float]) -> float | None:
    if not values:
        return None
    values = sorted(values)
    mid = len(values) // 2
    if len(values) % 2:
        return values[mid]
    return (values[mid - 1] + values[mid]) / 2.0


def _engine_cards(
    matrix: list[dict[str, Any]],
    engines: list[str],
    counts: dict[str, dict[str, int]],
) -> list[dict[str, Any]]:
    total = len(matrix)
    cards = []
    for engine in engines:
        passed = counts.get(engine, {}).get("pass", 0)
        non_pass = max(0, total - passed)
        durations = [_duration_ms(row, engine) for row in matrix]
        durations = [value for value in durations if value is not None]
        cards.append(
            {
                "engine": engine,
                "total": total,
                "pass": passed,
                "non_pass": non_pass,
                "pass_rate": (passed / total * 100.0) if total else 0.0,
                "median_ms": _median(durations),
                "counts": counts.get(engine, {}),
            }
        )
    return cards


def _group_rows(matrix: list[dict[str, Any]], engines: list[str]) -> list[dict[str, Any]]:
    groups: dict[str, dict[str, Counter[str]]] = defaultdict(lambda: {engine: Counter() for engine in engines})
    for row in matrix:
        group = _group_key(str(row.get("case_path", "")))
        for engine in engines:
            status = str(row.get("results", {}).get(engine, {}).get("status", "missing"))
            groups[group][engine][status] += 1

    rows = []
    for group in sorted(groups):
        engine_stats = {}
        aggregate_non_pass = 0
        aggregate_total = 0
        for engine in engines:
            counter = groups[group][engine]
            total = sum(counter.values())
            passed = counter.get("pass", 0)
            non_pass = max(0, total - passed)
            aggregate_non_pass += non_pass
            aggregate_total += total
            engine_stats[engine] = {
                "total": total,
                "pass": passed,
                "non_pass": non_pass,
                "pass_rate": (passed / total * 100.0) if total else 0.0,
                "counts": {status: counter.get(status, 0) for status in STATUS_ORDER if counter.get(status, 0)},
            }
        rows.append(
            {
                "group": group,
                "engines": engine_stats,
                "aggregate_non_pass": aggregate_non_pass,
                "aggregate_total": aggregate_total,
            }
        )
    rows.sort(key=lambda row: (-int(row["aggregate_non_pass"]), str(row["group"])))
    return rows


def _pairwise_rows(matrix: list[dict[str, Any]], engines: list[str]) -> list[dict[str, Any]]:
    rows = []
    for left in engines:
        for right in engines:
            if left == right:
                continue
            left_pass_right_not = 0
            right_pass_left_not = 0
            same_status = 0
            different_status = 0
            both_pass = 0
            both_non_pass = 0
            for row in matrix:
                left_status = str(row.get("results", {}).get(left, {}).get("status", "missing"))
                right_status = str(row.get("results", {}).get(right, {}).get("status", "missing"))
                if left_status == right_status:
                    same_status += 1
                else:
                    different_status += 1
                if left_status == "pass" and right_status == "pass":
                    both_pass += 1
                elif left_status != "pass" and right_status != "pass":
                    both_non_pass += 1
                elif left_status == "pass":
                    left_pass_right_not += 1
                elif right_status == "pass":
                    right_pass_left_not += 1
            rows.append(
                {
                    "left": left,
                    "right": right,
                    "left_pass_right_not": left_pass_right_not,
                    "right_pass_left_not": right_pass_left_not,
                    "net_pass_advantage": left_pass_right_not - right_pass_left_not,
                    "same_status": same_status,
                    "different_status": different_status,
                    "both_pass": both_pass,
                    "both_non_pass": both_non_pass,
                    "total": len(matrix),
                }
            )
    return rows


def _top_regression_rows(matrix: list[dict[str, Any]], engines: list[str], primary: str = "moli") -> list[dict[str, Any]]:
    if primary not in engines:
        return []
    rows = []
    for other in engines:
        if other == primary:
            continue
        grouped: Counter[str] = Counter()
        examples: dict[str, list[str]] = defaultdict(list)
        for row in matrix:
            left = str(row.get("results", {}).get(primary, {}).get("status", "missing"))
            right = str(row.get("results", {}).get(other, {}).get("status", "missing"))
            if left != "pass" and right == "pass":
                group = _group_key(str(row.get("case_path", "")))
                grouped[group] += 1
                if len(examples[group]) < 3:
                    examples[group].append(str(row.get("case_path", "")))
        for group, count in grouped.most_common(12):
            rows.append({"engine": other, "group": group, "count": count, "examples": examples[group]})
    rows.sort(key=lambda row: (-int(row["count"]), str(row["engine"]), str(row["group"])))
    return rows[:18]


def _status_label(status: str) -> str:
    return STATUS_LABELS.get(status, status)


def _format_ms(value: Any) -> str:
    try:
        raw = float(value)
    except (TypeError, ValueError):
        return "n/a"
    if raw >= 1000:
        return f"{raw / 1000:.2f}s"
    return f"{raw:.0f}ms"


def _render_cards(cards: list[dict[str, Any]]) -> str:
    pieces = []
    for card in cards:
        engine = str(card["engine"])
        counts = card.get("counts", {})
        chips = "".join(
            f'<span class="mini-chip"><i style="background:{STATUS_COLOR.get(status, "#999")}"></i>'
            f'{html.escape(_status_label(status))} {int(value)}</span>'
            for status, value in counts.items()
        )
        pieces.append(
            '<section class="kpi">'
            f'<div class="kpi-top"><span>{html.escape(engine)}</span>'
            f'<b style="color:{ENGINE_COLOR.get(engine, "#111827")}">{float(card["pass_rate"]):.1f}%</b></div>'
            f'<div class="kpi-main">{int(card["pass"])}<span> / {int(card["total"])} pass</span></div>'
            f'<div class="kpi-sub">{int(card["non_pass"])} non-pass · median {_format_ms(card["median_ms"])}</div>'
            f'<div class="chip-row">{chips}</div>'
            "</section>"
        )
    return "".join(pieces)


def _render_group_table(group_rows: list[dict[str, Any]], engines: list[str]) -> str:
    head = "".join(f'<th>{html.escape(engine)} pass</th><th>{html.escape(engine)} gaps</th>' for engine in engines)
    body = []
    for row in group_rows[:80]:
        cells = []
        for engine in engines:
            stats = row["engines"][engine]
            rate = float(stats["pass_rate"])
            tone = "good" if rate >= 90 else "warn" if rate >= 65 else "bad"
            cells.append(
                f'<td><span class="rate-text {tone}">{rate:.0f}%</span> '
                f'<span class="muted">{int(stats["pass"])}/{int(stats["total"])}</span></td>'
                f'<td>{int(stats["non_pass"])}</td>'
            )
        body.append(f'<tr><th>{html.escape(str(row["group"]))}</th>{"".join(cells)}</tr>')
    return (
        '<div class="table-wrap"><table class="data-table">'
        f'<thead><tr><th>WPT area</th>{head}</tr></thead>'
        f'<tbody>{"".join(body)}</tbody></table></div>'
    )


def _render_pairwise_table(pairwise_rows: list[dict[str, Any]], engines: list[str]) -> str:
    rows = []
    for row in pairwise_rows:
        if row["left"] != "moli":
            continue
        right = str(row["right"])
        net = int(row["net_pass_advantage"])
        net_class = "good" if net > 0 else "bad" if net < 0 else "warn"
        rows.append(
            "<tr>"
            f"<th>moli vs {html.escape(right)}</th>"
            f'<td class="{net_class}">{net:+d}</td>'
            f'<td>{int(row["left_pass_right_not"])}</td>'
            f'<td>{int(row["right_pass_left_not"])}</td>'
            f'<td>{int(row["both_pass"])}</td>'
            f'<td>{int(row["different_status"])}</td>'
            "</tr>"
        )
    if not rows:
        return ""
    return (
        '<div class="table-wrap compact"><table class="data-table">'
        "<thead><tr><th>pair</th><th>net pass advantage</th><th>LM pass / peer not</th>"
        "<th>peer pass / LM not</th><th>both pass</th><th>status disagreements</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody></table></div>"
    )


def _render_regression_table(rows: list[dict[str, Any]]) -> str:
    body = []
    for row in rows:
        examples = "<br>".join(f"<code>{html.escape(example)}</code>" for example in row["examples"])
        body.append(
            "<tr>"
            f'<th>vs {html.escape(str(row["engine"]))}</th>'
            f'<td>{html.escape(str(row["group"]))}</td>'
            f'<td>{int(row["count"])}</td>'
            f"<td>{examples}</td>"
            "</tr>"
        )
    if not body:
        return '<p class="empty">No moli-only non-pass clusters against passing peers in this run.</p>'
    return (
        '<div class="table-wrap compact"><table class="data-table">'
        "<thead><tr><th>comparison</th><th>area</th><th>count</th><th>examples</th></tr></thead>"
        f"<tbody>{''.join(body)}</tbody></table></div>"
    )


def _render_recorded_failure_drift_table(summary: dict[str, Any]) -> str:
    drift = summary.get("recorded_failure_drift")
    if not isinstance(drift, dict):
        return '<p class="empty">No recorded subtest failure drift data in this run.</p>'
    comparisons = drift.get("comparisons")
    if not isinstance(comparisons, list) or not comparisons:
        return '<p class="empty">No recorded subtest failure-name drift between moli and peers.</p>'
    rows = []
    for row in comparisons[:80]:
        if not isinstance(row, dict):
            continue
        primary_only = "<br>".join(
            f"<code>{html.escape(str(name))}</code>"
            for name in (row.get("primary_only_examples") or [])[:4]
        )
        peer_only = "<br>".join(
            f"<code>{html.escape(str(name))}</code>"
            for name in (row.get("peer_only_examples") or [])[:4]
        )
        message_examples = "<br>".join(
            f"<code>{html.escape(str(name))}</code>"
            for name in (row.get("message_diff_examples") or [])[:4]
        )
        rows.append(
            "<tr>"
            f"<th>{html.escape(str(row.get('case_path', '')))}</th>"
            f"<td>{html.escape(str(row.get('primary', 'moli')))} vs {html.escape(str(row.get('peer', 'peer')))}</td>"
            f"<td>{int(row.get('primary_only_count') or 0)}{('<br>' + primary_only) if primary_only else ''}</td>"
            f"<td>{int(row.get('peer_only_count') or 0)}{('<br>' + peer_only) if peer_only else ''}</td>"
            f"<td>{int(row.get('message_diff_count') or 0)}{('<br>' + message_examples) if message_examples else ''}</td>"
            "</tr>"
        )
    if not rows:
        return '<p class="empty">No recorded subtest failure-name drift between moli and peers.</p>'
    limit = drift.get("recorded_failure_limit_per_engine")
    note = (
        f'<p class="panel-note">Failure-name drift uses the full subtest failure-name list when present; '
        f'message differences use recorded failure details, capped at {int(limit)} entries per engine per case.</p>'
        if isinstance(limit, int)
        else ""
    )
    return (
        note
        + '<div class="table-wrap compact"><table class="data-table">'
        "<thead><tr><th>case</th><th>comparison</th><th>moli-only recorded failures</th>"
        "<th>peer-only recorded failures</th><th>same-name message differences</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody></table></div>"
    )


def _render_known_failure_audits(summary: dict[str, Any]) -> str:
    audits = summary.get("known_failure_audits")
    if not isinstance(audits, dict) or not audits:
        return ""

    rows: list[str] = []
    category_rows: list[str] = []
    for engine, audit in sorted(audits.items()):
        if not isinstance(audit, dict):
            continue
        counts = audit.get("counts") if isinstance(audit.get("counts"), dict) else {}
        ok = audit.get("ok") is True
        rows.append(
            "<tr>"
            f"<th>{html.escape(str(engine))}</th>"
            f'<td class="{"good" if ok else "bad"}">{"ok" if ok else "attention"}</td>'
            f'<td><code>{html.escape(str(audit.get("manifest", "")))}</code></td>'
            f'<td><code>{html.escape(str(audit.get("output", "")))}</code></td>'
            f'<td>{int(counts.get("known_failures", 0))}</td>'
            f'<td>{int(counts.get("resolved_known_failures", 0))}</td>'
            f'<td>{int(counts.get("mismatched_known_failures", 0))}</td>'
            f'<td>{int(counts.get("missing_expected_failures", 0))}</td>'
            f'<td>{int(counts.get("skipped_known_failures", 0))}</td>'
            f'<td>{int(counts.get("unexpected_failures", 0))}</td>'
            "</tr>"
        )
        category_counts = audit.get("category_counts")
        category_metadata = audit.get("categories") if isinstance(audit.get("categories"), dict) else {}
        if isinstance(category_counts, dict):
            known = category_counts.get("known_failures")
            if isinstance(known, dict):
                for category, count in sorted(known.items()):
                    metadata = (
                        category_metadata.get(category)
                        if isinstance(category_metadata, dict)
                        else None
                    )
                    tracking_doc = ""
                    scope = ""
                    evidence = ""
                    if isinstance(metadata, dict):
                        tracking_doc = str(metadata.get("tracking_doc", ""))
                        scope = str(metadata.get("scope", ""))
                        raw_evidence = metadata.get("evidence")
                        if isinstance(raw_evidence, list):
                            evidence_items: list[str] = []
                            for item in raw_evidence:
                                if isinstance(item, dict):
                                    evidence_path = html.escape(str(item.get("path", "")))
                                    kind = html.escape(str(item.get("kind", "")))
                                    note = html.escape(str(item.get("note", "")))
                                    evidence_items.append(
                                        f"{kind} <code>{evidence_path}</code>: {note}"
                                    )
                                elif isinstance(item, str):
                                    evidence_items.append(html.escape(item))
                            evidence = "<br>".join(evidence_items)
                    category_rows.append(
                        "<tr>"
                        f"<th>{html.escape(str(engine))}</th>"
                        f"<td>{html.escape(str(category))}</td>"
                        f"<td>{int(count)}</td>"
                        f"<td><code>{html.escape(tracking_doc)}</code></td>"
                        f"<td>{html.escape(scope)}</td>"
                        f"<td>{evidence}</td>"
                        "</tr>"
                    )

    if not rows:
        return ""

    category_table = ""
    if category_rows:
        category_table = (
            '<div class="table-wrap compact audit-categories"><table class="data-table">'
            "<thead><tr><th>engine</th><th>known-failure category</th><th>count</th><th>tracking doc</th><th>scope</th><th>evidence</th></tr></thead>"
            f"<tbody>{''.join(category_rows)}</tbody></table></div>"
        )

    return (
        '<section class="panel">'
        "<h2>Known-failure audit</h2>"
        '<p class="panel-note">Manifest-backed audit of non-pass cases. The runner keeps raw case statuses intact; any resolved, changed, missing, or unexpected failure makes the audit fail. Focused runs may opt into skipped manifest rules.</p>'
        '<div class="table-wrap compact"><table class="data-table">'
        "<thead><tr><th>engine</th><th>audit</th><th>manifest</th><th>output</th>"
        "<th>known</th><th>resolved</th><th>mismatched</th><th>missing</th><th>skipped</th><th>unexpected</th></tr></thead>"
        f"<tbody>{''.join(rows)}</tbody></table></div>"
        f"{category_table}"
        "</section>"
    )


def _render_matrix_section(engines: list[str]) -> str:
    engine_options = "".join(f'<option value="{html.escape(engine)}">{html.escape(engine)}</option>' for engine in engines)
    status_options = "".join(f'<option value="{html.escape(status)}">{html.escape(_status_label(status))}</option>' for status in STATUS_ORDER)
    head = "".join(f"<th>{html.escape(engine)}</th>" for engine in engines)
    return (
        '<div class="matrix-controls">'
        '<input id="caseFilter" type="search" placeholder="Filter case path, e.g. shadow-dom or Range">'
        '<select id="engineFilter"><option value="">Any engine</option>' + engine_options + "</select>"
        '<select id="statusFilter"><option value="">Any status</option>' + status_options + "</select>"
        '<label><input type="checkbox" id="onlyDiff"> disagreements only</label>'
        '<label><input type="checkbox" id="lmGap"> moli non-pass while any peer passes</label>'
        '<span id="shownCount" class="shown-count"></span>'
        "</div>"
        '<div class="table-wrap matrix-wrap">'
        f'<table id="matrix" class="data-table matrix"><thead><tr><th>case</th>{head}</tr></thead><tbody></tbody></table>'
        "</div>"
    )


def _chart_payload(
    *,
    matrix: list[dict[str, Any]],
    summary: dict[str, Any],
    engines: list[str],
    counts: dict[str, dict[str, int]],
    cards: list[dict[str, Any]],
    group_rows: list[dict[str, Any]],
    pairwise_rows: list[dict[str, Any]],
    regression_rows: list[dict[str, Any]],
) -> dict[str, Any]:
    slim_rows = []
    for row in matrix:
        cells = {}
        for engine in engines:
            result = row.get("results", {}).get(engine, {})
            cells[engine] = {
                "status": result.get("status", "missing"),
                "duration_ms": result.get("duration_ms"),
                "subtests": result.get("subtests"),
                "error": result.get("error"),
                "test_type": result.get("test_type", row.get("test_type", "testharness")),
                "reftest_comparisons": result.get("reftest_comparisons", []),
                "artifacts": result.get("artifacts", {}),
            }
        slim_rows.append({"case": row.get("case_path", ""), "group": _group_key(str(row.get("case_path", ""))), "results": cells})
    return {
        "summary": summary,
        "engines": engines,
        "statusOrder": STATUS_ORDER,
        "statusLabels": STATUS_LABELS,
        "statusColor": STATUS_COLOR,
        "engineColor": ENGINE_COLOR,
        "counts": counts,
        "cards": cards,
        "groups": group_rows,
        "pairwise": pairwise_rows,
        "regressions": regression_rows,
        "recordedFailureDrift": summary.get("recorded_failure_drift", {}),
        "rows": slim_rows,
    }


_CSS = r"""
:root {
  color-scheme: light;
  --bg: #f5f7fb;
  --panel: #ffffff;
  --ink: #111827;
  --muted: #64748b;
  --line: #d9e1ec;
  --soft: #eef2f7;
  --good: #15803d;
  --warn: #b7791f;
  --bad: #c0262d;
  --shadow: 0 14px 35px rgba(15, 23, 42, 0.08);
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: var(--bg);
  color: var(--ink);
  font: 14px/1.5 Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
.page { max-width: 1480px; margin: 0 auto; padding: 28px 24px 44px; }
.hero {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 24px;
  align-items: end;
  padding: 24px;
  background: linear-gradient(135deg, #ffffff 0%, #eef6ff 100%);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: var(--shadow);
}
h1 { margin: 0; font-size: 30px; line-height: 1.15; letter-spacing: 0; }
.subtitle { margin-top: 8px; color: var(--muted); max-width: 820px; }
.hero-meta { text-align: right; color: var(--muted); font-size: 13px; }
.hero-meta strong { display: block; color: var(--ink); font-size: 24px; }
.kpis { display: grid; grid-template-columns: repeat(4, minmax(210px, 1fr)); gap: 14px; margin-top: 18px; }
.kpi {
  min-width: 0;
  padding: 16px;
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: 0 8px 22px rgba(15, 23, 42, 0.05);
}
.kpi-top { display: flex; justify-content: space-between; gap: 12px; color: var(--muted); font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }
.kpi-main { margin-top: 6px; font-size: 32px; font-weight: 780; line-height: 1.1; }
.kpi-main span { font-size: 13px; color: var(--muted); font-weight: 500; }
.kpi-sub { margin-top: 6px; color: var(--muted); font-size: 13px; }
.chip-row { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; }
.mini-chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 3px 7px;
  border: 1px solid var(--line);
  border-radius: 999px;
  color: #334155;
  background: #fff;
  font-size: 12px;
}
.mini-chip i { width: 8px; height: 8px; border-radius: 999px; }
.grid { display: grid; gap: 16px; margin-top: 18px; }
.grid.two { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
.panel {
  background: var(--panel);
  border: 1px solid var(--line);
  border-radius: 8px;
  box-shadow: 0 8px 22px rgba(15, 23, 42, 0.05);
  padding: 16px;
  min-width: 0;
}
.panel h2 { margin: 0 0 4px; font-size: 17px; }
.panel-note { margin: 0 0 12px; color: var(--muted); font-size: 13px; }
.chart-box { position: relative; height: 320px; min-height: 260px; }
.chart-box.short { height: 260px; }
.chart-box.tall { height: 440px; }
.table-wrap {
  overflow: auto;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: #fff;
}
.table-wrap.compact { max-height: 420px; }
.audit-categories { margin-top: 12px; }
.data-table { width: 100%; border-collapse: collapse; min-width: 760px; }
.data-table th, .data-table td { padding: 8px 10px; border-bottom: 1px solid #edf1f6; text-align: left; vertical-align: top; }
.data-table thead th { position: sticky; top: 0; z-index: 1; background: #f8fafc; color: #475569; font-size: 12px; text-transform: uppercase; letter-spacing: .04em; }
.data-table tbody th { font-family: ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace; font-weight: 600; }
.rate-text { font-weight: 760; }
.good { color: var(--good); }
.warn { color: var(--warn); }
.bad { color: var(--bad); }
.muted { color: var(--muted); }
code {
  font-family: ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
  font-size: 12px;
  color: #334155;
}
.matrix-controls {
  display: grid;
  grid-template-columns: minmax(280px, 1fr) 160px 160px auto auto auto;
  gap: 10px;
  align-items: center;
  margin-bottom: 12px;
}
.matrix-controls input[type="search"], .matrix-controls select {
  width: 100%;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: #fff;
  padding: 8px 10px;
  font: inherit;
}
.matrix-controls label { white-space: nowrap; color: #334155; font-size: 13px; }
.shown-count { color: var(--muted); font-size: 13px; text-align: right; }
.matrix-wrap { max-height: 760px; }
.matrix { min-width: 980px; }
.matrix .case-cell { min-width: 460px; max-width: 680px; word-break: break-word; font-family: ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace; }
.status-badge {
  display: inline-flex;
  align-items: center;
  min-width: 86px;
  justify-content: center;
  padding: 3px 7px;
  color: #fff;
  border-radius: 5px;
  font-size: 12px;
  font-weight: 720;
}
.cell-sub { margin-top: 3px; color: var(--muted); font-size: 11px; }
.artifact-links { display: flex; flex-wrap: wrap; gap: 5px; margin-top: 5px; }
.artifact-links a { color: #1d4ed8; font-size: 11px; text-decoration: none; }
.artifact-links a:hover { text-decoration: underline; }
.empty { color: var(--muted); margin: 0; }
.notice {
  padding: 10px 12px;
  border: 1px solid #fed7aa;
  background: #fff7ed;
  color: #9a3412;
  border-radius: 8px;
  margin-top: 12px;
  display: none;
}
@media (max-width: 1100px) {
  .hero { grid-template-columns: 1fr; }
  .hero-meta { text-align: left; }
  .kpis, .grid.two { grid-template-columns: 1fr; }
  .matrix-controls { grid-template-columns: 1fr 1fr; }
}
"""


_JS = r"""
const DATA = __DATA__;

function byId(id) { return document.getElementById(id); }
function pct(value) { return `${Number(value || 0).toFixed(1)}%`; }
function statusLabel(status) { return DATA.statusLabels[status] || status; }
function statusColor(status) { return DATA.statusColor[status] || '#64748b'; }

function chartDefaults() {
  if (!window.Chart) {
    byId('chartNotice').style.display = 'block';
    return false;
  }
  Chart.defaults.font.family = 'Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';
  Chart.defaults.color = '#475569';
  return true;
}

function renderCharts() {
  if (!chartDefaults()) return;
  const engines = DATA.engines;
  const statusOrder = DATA.statusOrder.filter(status => engines.some(engine => (DATA.counts[engine] || {})[status]));

  new Chart(byId('statusStackChart'), {
    type: 'bar',
    data: {
      labels: engines,
      datasets: statusOrder.map(status => ({
        label: statusLabel(status),
        data: engines.map(engine => (DATA.counts[engine] || {})[status] || 0),
        backgroundColor: statusColor(status),
        borderWidth: 0,
        borderRadius: 3,
      })),
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: { legend: { position: 'bottom' }, tooltip: { mode: 'index', intersect: false } },
      scales: { x: { stacked: true, grid: { display: false } }, y: { stacked: true, beginAtZero: true, title: { display: true, text: 'cases' } } },
    },
  });

  new Chart(byId('passRateChart'), {
    type: 'bar',
    data: {
      labels: engines,
      datasets: [{
        label: 'Pass rate',
        data: DATA.cards.map(card => card.pass_rate),
        backgroundColor: engines.map(engine => DATA.engineColor[engine] || '#2563eb'),
        borderRadius: 5,
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: { legend: { display: false }, tooltip: { callbacks: { label: ctx => pct(ctx.raw) } } },
      scales: { y: { beginAtZero: true, max: 100, ticks: { callback: value => `${value}%` } }, x: { grid: { display: false } } },
    },
  });

  const topGroups = [...DATA.groups].sort((a, b) => b.aggregate_non_pass - a.aggregate_non_pass).slice(0, 16).reverse();
  new Chart(byId('topGroupsChart'), {
    type: 'bar',
    data: {
      labels: topGroups.map(row => row.group),
      datasets: engines.map(engine => ({
        label: engine,
        data: topGroups.map(row => row.engines[engine]?.non_pass || 0),
        backgroundColor: DATA.engineColor[engine] || '#64748b',
        borderRadius: 3,
      })),
    },
    options: {
      indexAxis: 'y',
      responsive: true,
      maintainAspectRatio: false,
      plugins: { legend: { position: 'bottom' } },
      scales: { x: { beginAtZero: true, title: { display: true, text: 'non-pass cases' } }, y: { grid: { display: false }, ticks: { autoSkip: false } } },
    },
  });

  const lmPairs = DATA.pairwise.filter(row => row.left === 'moli');
  new Chart(byId('pairwiseChart'), {
    type: 'bar',
    data: {
      labels: lmPairs.map(row => `vs ${row.right}`),
      datasets: [
        { label: 'LM pass / peer not', data: lmPairs.map(row => row.left_pass_right_not), backgroundColor: '#2563eb', borderRadius: 4 },
        { label: 'peer pass / LM not', data: lmPairs.map(row => -row.right_pass_left_not), backgroundColor: '#d64045', borderRadius: 4 },
      ],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      plugins: {
        legend: { position: 'bottom' },
        tooltip: { callbacks: { label: ctx => `${ctx.dataset.label}: ${Math.abs(ctx.raw)}` } },
      },
      scales: { y: { grid: { display: false } }, x: { title: { display: true, text: 'case count (negative means peer leads)' } } },
    },
  });
}

function resultTitle(result) {
  const bits = [];
  if (result.duration_ms != null) bits.push(`${Number(result.duration_ms).toFixed(0)} ms`);
  if (result.subtests && result.subtests.total != null) {
    bits.push(`subtests: ${result.subtests.pass || 0}/${result.subtests.total || 0} pass`);
  }
  if (result.error) bits.push(result.error);
  if (result.reftest_comparisons && result.reftest_comparisons.length) {
    const first = result.reftest_comparisons[0];
    bits.push(`reftest ${first.relation}: maxDifference=${first.max_difference}, differentPixels=${first.different_pixels}`);
  }
  return bits.join(' · ');
}

function appendArtifactLinks(cell, artifacts) {
  if (!artifacts || !artifacts.test) return;
  const links = document.createElement('div');
  links.className = 'artifact-links';
  const add = (label, href) => {
    if (!href) return;
    const link = document.createElement('a');
    link.href = href;
    link.target = '_blank';
    link.rel = 'noopener';
    link.textContent = label;
    links.appendChild(link);
  };
  add('test', artifacts.test);
  for (const [index, reference] of (artifacts.references || []).entries()) {
    add(`ref ${index + 1}`, reference.reference);
    add(`diff ${index + 1}`, reference.diff);
  }
  cell.appendChild(links);
}

function renderMatrix() {
  const text = byId('caseFilter').value.trim().toLowerCase();
  const engineFilter = byId('engineFilter').value;
  const statusFilter = byId('statusFilter').value;
  const onlyDiff = byId('onlyDiff').checked;
  const lmGap = byId('lmGap').checked;
  const tbody = document.querySelector('#matrix tbody');
  const frag = document.createDocumentFragment();
  let shown = 0;
  let matched = 0;

  for (const row of DATA.rows) {
    if (text && !row.case.toLowerCase().includes(text)) continue;
    const statuses = DATA.engines.map(engine => row.results[engine]?.status || 'missing');
    if (onlyDiff && new Set(statuses).size <= 1) continue;
    if (engineFilter && statusFilter && (row.results[engineFilter]?.status || 'missing') !== statusFilter) continue;
    if (!engineFilter && statusFilter && !statuses.includes(statusFilter)) continue;
    if (lmGap) {
      const lm = row.results.moli?.status || 'missing';
      const peerPass = DATA.engines.some(engine => engine !== 'moli' && (row.results[engine]?.status || 'missing') === 'pass');
      if (lm === 'pass' || !peerPass) continue;
    }
    matched++;
    if (shown >= 2000) continue;

    const tr = document.createElement('tr');
    const caseCell = document.createElement('td');
    caseCell.className = 'case-cell';
    caseCell.textContent = row.case;
    tr.appendChild(caseCell);

    for (const engine of DATA.engines) {
      const result = row.results[engine] || { status: 'missing' };
      const status = result.status || 'missing';
      const td = document.createElement('td');
      td.title = resultTitle(result);
      const badge = document.createElement('span');
      badge.className = 'status-badge';
      badge.style.background = statusColor(status);
      badge.textContent = statusLabel(status);
      td.appendChild(badge);
      if (result.duration_ms != null || (result.subtests && result.subtests.total != null)) {
        const sub = document.createElement('div');
        sub.className = 'cell-sub';
        const parts = [];
        if (result.duration_ms != null) parts.push(`${Number(result.duration_ms).toFixed(0)}ms`);
        if (result.subtests && result.subtests.total != null) parts.push(`${result.subtests.pass || 0}/${result.subtests.total || 0}`);
        sub.textContent = parts.join(' · ');
        td.appendChild(sub);
      }
      appendArtifactLinks(td, result.artifacts);
      tr.appendChild(td);
    }
    frag.appendChild(tr);
    shown++;
  }
  tbody.replaceChildren(frag);
  byId('shownCount').textContent = `${shown.toLocaleString()} shown / ${matched.toLocaleString()} matched`;
}

document.addEventListener('DOMContentLoaded', () => {
  renderCharts();
  for (const id of ['caseFilter', 'engineFilter', 'statusFilter', 'onlyDiff', 'lmGap']) {
    byId(id).addEventListener('input', renderMatrix);
    byId(id).addEventListener('change', renderMatrix);
  }
  renderMatrix();
});
"""


def render_html(output_dir: Path, *, matrix_name: str = "matrix.json", summary_name: str = "summary.json", html_name: str = "index.html") -> Path:
    matrix_path = output_dir / matrix_name
    summary_path = output_dir / summary_name
    if not matrix_path.exists() or not summary_path.exists():
        raise FileNotFoundError(f"missing {matrix_name} or {summary_name} in {output_dir}")

    matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    engines = _engines_from_summary(summary, matrix)
    counts = _status_counts(summary, matrix, engines)
    cards = _engine_cards(matrix, engines, counts)
    group_rows = _group_rows(matrix, engines)
    pairwise_rows = _pairwise_rows(matrix, engines)
    regression_rows = _top_regression_rows(matrix, engines)
    payload = _chart_payload(
        matrix=matrix,
        summary=summary,
        engines=engines,
        counts=counts,
        cards=cards,
        group_rows=group_rows,
        pairwise_rows=pairwise_rows,
        regression_rows=regression_rows,
    )

    total = len(matrix)
    elapsed = summary.get("total_elapsed_seconds")
    elapsed_text = f"{float(elapsed) / 60.0:.1f} min" if isinstance(elapsed, (int, float)) else "n/a"
    generated_for = ", ".join(html.escape(engine) for engine in engines)
    data_json = (
        json.dumps(payload, separators=(",", ":"))
        .replace("<", "\\u003c")
        .replace(">", "\\u003e")
        .replace("&", "\\u0026")
    )

    html_doc = f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>WPT support: moli vs lightpanda vs chrome vs obscura</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.9/dist/chart.umd.min.js"></script>
<style>{_CSS}</style>
</head>
<body>
<main class="page">
  <header class="hero">
    <div>
      <h1>WPT support comparison</h1>
      <div class="subtitle">Cross-engine view for moli, lightpanda, chrome, and obscura. The report is derived from the same WPT matrix data used by the runner, with no post-hoc status rewriting.</div>
    </div>
    <div class="hero-meta">
      <strong>{total}</strong>
      cases · {len(engines)} engines<br>
      elapsed {html.escape(elapsed_text)}<br>
      {generated_for}
    </div>
  </header>

  <div class="kpis">{_render_cards(cards)}</div>
  <div id="chartNotice" class="notice">Chart.js did not load. Tables and the case matrix are still available below.</div>
  {_render_known_failure_audits(summary)}

  <section class="grid two">
    <div class="panel">
      <h2>Status distribution</h2>
      <p class="panel-note">Stacked case counts by final runner status.</p>
      <div class="chart-box"><canvas id="statusStackChart"></canvas></div>
    </div>
    <div class="panel">
      <h2>Pass rate</h2>
      <p class="panel-note">Top-level pass rate per engine over the selected WPT case set.</p>
      <div class="chart-box"><canvas id="passRateChart"></canvas></div>
    </div>
  </section>

  <section class="grid two">
    <div class="panel">
      <h2>Largest API gaps by area</h2>
      <p class="panel-note">Top WPT directories ranked by aggregate non-pass count across engines.</p>
      <div class="chart-box tall"><canvas id="topGroupsChart"></canvas></div>
    </div>
    <div class="panel">
      <h2>Moli pairwise position</h2>
      <p class="panel-note">Positive bars are cases moli passes while the peer does not; negative bars are peer-only passes.</p>
      <div class="chart-box short"><canvas id="pairwiseChart"></canvas></div>
      {_render_pairwise_table(pairwise_rows, engines)}
    </div>
  </section>

  <section class="panel">
    <h2>Moli gap clusters</h2>
    <p class="panel-note">Areas where moli is non-pass while at least one peer passes. Use these as implementation triage candidates, not as replacement for subtest-level investigation.</p>
    {_render_regression_table(regression_rows)}
  </section>

  <section class="panel">
    <h2>Recorded subtest drift</h2>
    <p class="panel-note">Cases where the recorded failing subtest names differ between moli and a peer, even when top-level case status may match.</p>
    {_render_recorded_failure_drift_table(summary)}
  </section>

  <section class="panel">
    <h2>Directory support table</h2>
    <p class="panel-note">Pass rate and non-pass count by WPT area. The table is sorted by aggregate non-pass count.</p>
    {_render_group_table(group_rows, engines)}
  </section>

  <section class="panel">
    <h2>Case matrix</h2>
    <p class="panel-note">Filter by path, status, disagreement, or moli-only gaps. The table renders at most 2000 rows at once to keep the page responsive.</p>
    {_render_matrix_section(engines)}
  </section>
</main>
<script>{_JS.replace("__DATA__", data_json)}</script>
</body>
</html>
"""
    out_path = output_dir / html_name
    out_path.write_text(html_doc, encoding="utf-8")
    return out_path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="python -m moli_benchmark.wpt_cross.render_html")
    parser.add_argument("output_dir", type=Path, help="Directory containing matrix.json and summary.json")
    parser.add_argument("--matrix", default="matrix.json")
    parser.add_argument("--summary", default="summary.json")
    parser.add_argument("--html", default="index.html")
    args = parser.parse_args(argv)
    out = render_html(args.output_dir, matrix_name=args.matrix, summary_name=args.summary, html_name=args.html)
    print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
