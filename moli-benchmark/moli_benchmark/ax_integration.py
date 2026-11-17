"""Run 20 AX integration cases against Moli and default Chromium.

uv run --project moli-benchmark --no-sync python -m moli_benchmark.ax_integration \
    --moli-bin /path/to/release/moli --chrome-bin /path/to/chrome \
    --output-dir /path/to/results --runs 5

Durations measure the complete CDP round trip, including serialization. Raw
trees, independent expectation failures and cross-browser differences survive
a failed run. Samples are measurements, never retries of a failed assertion.
"""
from __future__ import annotations

import argparse
import asyncio
from collections import Counter
from dataclasses import asdict
import hashlib
import http.server
import json
from pathlib import Path
import statistics
import subprocess
import threading
import time
from typing import Any

from .ax_cases import AX_CASES, AxCase, Probe
from .cdp_session import _create_page_session, _navigate_and_wait_marker
from .raw_cdp import connect_raw_cdp
from .target_serve import start_target_serve, stop_target_serve


PROPERTIES = ("url", "checked", "selected", "disabled", "required", "readonly", "focusable",
              "expanded", "level", "valuemin", "valuemax", "valuetext")
FALSE_DEFAULTS = {"selected", "disabled", "required", "readonly", "focusable"}


def facts(node: dict[str, Any], origin: str) -> dict[str, Any]:
    if node.get("ignored"):
        return {"ignored": True}
    result = {key: node.get(key, {}).get("value") for key in ("role", "name", "value", "description")}
    result.update(ignored=False)
    properties = {p["name"]: p["value"].get("value") for p in node.get("properties", [])}
    for key in PROPERTIES:
        result[key] = properties.get(key, False if key in FALSE_DEFAULTS else None)
    if isinstance(result["url"], str) and result["url"].startswith(origin + "/"):
        result["url"] = "{origin}" + result["url"][len(origin):]
    return result


def expectation_errors(probes: tuple[Probe, ...], observed: dict[str, Any]) -> list[dict[str, Any]]:
    errors = []
    for probe in probes:
        if probe.id not in observed:
            errors.append({"probe": probe.id, "error": "missing AX node"})
            continue
        for key, expected in probe.expected.items():
            actual = observed[probe.id].get(key)
            if actual != expected:
                errors.append({"probe": probe.id, "field": key, "expected": expected, "actual": actual})
    return errors


def compare_observations(chrome: dict[str, Any], moli: dict[str, Any]) -> dict[str, Any]:
    return {key: {"chrome": chrome.get(key), "moli": moli.get(key)}
            for key in chrome.keys() | moli.keys() if chrome.get(key) != moli.get(key)}


def tree_signature(tree: dict[str, Any], origin: str) -> list[Any]:
    return sorted((node["nodeId"], node.get("parentId"), tuple(node.get("childIds", [])),
                   node.get("backendDOMNodeId"), json.dumps(facts(node, origin), sort_keys=True))
                  for node in tree["nodes"])


class FixtureHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path == "/image.svg":
            body = b'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16"/></svg>'
            mime = "image/svg+xml"
        else:
            fixture = next((case for case in AX_CASES if self.path == "/" + case.name), None)
            if fixture is None:
                self.send_error(404)
                return
            body = fixture.html().encode()
            mime = "text/html; charset=utf-8"
        self.send_response(200)
        self.send_header("Content-Type", mime)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: Any) -> None:
        pass


async def measure_case(client: Any, target: str, case: AxCase, origin: str,
                       directory: Path, runs: int, timeout: float) -> list[dict[str, Any]]:
    page = await _create_page_session(client)

    async def call(method: str, params: dict[str, Any] | None = None) -> tuple[dict[str, Any], float]:
        started = time.perf_counter()
        command = await client.send(method, params or {}, session_id=page.session_id)
        result, _ = await client.recv_until_id(command, method=method, timeout=timeout)
        return result["result"], (time.perf_counter() - started) * 1000

    async def evaluate(script: str) -> dict[str, Any]:
        result, _ = await call("Runtime.evaluate", {"expression": f"(() => {{ {script} }})()", "returnByValue": True})
        if "exceptionDetails" in result:
            raise RuntimeError(f"fixture script failed: {result}")
        return result

    rows: list[dict[str, Any]] = []
    identities: dict[str, tuple[Any, Any]] = {}
    try:
        directory.mkdir(parents=True, exist_ok=True)
        ok, navigation_ms, _, trace = await _navigate_and_wait_marker(
            client, page.session_id, origin + "/" + case.name, case.name, timeout, target)
        (directory / "navigation.json").write_text(json.dumps({"ready": ok, "trace": trace}, indent=2) + "\n")
        if not ok:
            raise RuntimeError(f"fixture did not complete: {trace}")
        if any(event.get("method") == "Runtime.exceptionThrown" for event in trace):
            raise RuntimeError(f"fixture raised a JavaScript exception: {trace}")
        root, _ = await call("DOM.getDocument", {"depth": 0})
        for step in case.steps:
            if step.script:
                await evaluate(step.script)
            backends = {}
            for probe in step.probes:
                query, _ = await call("DOM.querySelector", {"nodeId": root["root"]["nodeId"], "selector": "#" + probe.id})
                node, _ = await call("DOM.describeNode", {"nodeId": query["nodeId"]})
                backends[probe.id] = node["node"]["backendNodeId"]
            first = step.probes[0]
            partial, partial_ms = await call("Accessibility.getPartialAXTree", {
                "backendNodeId": backends[first.id], "fetchRelatives": False})
            tree, full_ms = await call("Accessibility.getFullAXTree")
            (directory / f"{step.name}-partial.json").write_text(json.dumps(partial, indent=2) + "\n")
            (directory / f"{step.name}-full.json").write_text(json.dumps(tree, indent=2) + "\n")
            by_backend = {node.get("backendDOMNodeId"): node for node in tree["nodes"]}
            observed, errors = {}, []
            for probe in step.probes:
                node = by_backend.get(backends[probe.id])
                if node is None or probe.expected.get("ignored"):
                    local, _ = await call("Accessibility.getPartialAXTree", {
                        "backendNodeId": backends[probe.id], "fetchRelatives": False})
                    (directory / f"{step.name}-{probe.id}-partial.json").write_text(json.dumps(local, indent=2) + "\n")
                    node = next((node for node in local["nodes"] if node.get("backendDOMNodeId") == backends[probe.id]), None)
                    if node is not None and not node.get("ignored") and backends[probe.id] not in by_backend:
                        errors.append({"probe": probe.id, "error": "visible partial node missing from full tree"})
                if node is None:
                    errors.append({"probe": probe.id, "error": "missing partial and full node"})
                    continue
                observed[probe.id] = facts(node, origin)
                identity = (backends[probe.id], node["nodeId"])
                if probe.id in identities and identities[probe.id] != identity:
                    errors.append({"probe": probe.id, "error": "DOM/AX reference changed during mutation"})
                identities[probe.id] = identity
            errors += expectation_errors(step.probes, observed)
            roles = Counter(node["role"]["value"] for node in tree["nodes"] if not node.get("ignored"))
            for role, count in (case.role_counts or {}).items():
                if roles[role] != count:
                    errors.append({"role": role, "expected_count": count, "actual_count": roles[role]})
            local_first = next((node for node in partial["nodes"] if node.get("backendDOMNodeId") == backends[first.id]), None)
            if local_first is None or facts(local_first, origin) != observed.get(first.id):
                errors.append({"probe": first.id, "error": "partial/full observations disagree"})
            query, query_ms = await call("Accessibility.queryAXTree", {
                "nodeId": root["root"]["nodeId"], "accessibleName": first.expected["name"], "role": first.expected["role"]})
            (directory / f"{step.name}-query.json").write_text(json.dumps(query, indent=2) + "\n")
            if backends[first.id] not in [node.get("backendDOMNodeId") for node in query["nodes"]]:
                errors.append({"probe": first.id, "error": "queryAXTree did not return the expected target"})
            else:
                query_node = next(node for node in query["nodes"] if node.get("backendDOMNodeId") == backends[first.id])
                if facts(query_node, origin) != observed.get(first.id):
                    errors.append({"probe": first.id, "error": "query/full observations disagree"})
            warm_ms = []
            initial_signature = tree_signature(tree, origin)
            for repeat in range(runs):
                warm, elapsed = await call("Accessibility.getFullAXTree")
                warm_ms.append(elapsed)
                (directory / f"{step.name}-warm-{repeat}.json").write_text(json.dumps(warm) + "\n")
                if tree_signature(warm, origin) != initial_signature:
                    errors.append({"repeat": repeat, "error": "complete warm tree changed without mutation"})
                warm_nodes = {node.get("backendDOMNodeId"): node for node in warm["nodes"]}
                for probe in step.probes:
                    if probe.expected.get("ignored"):
                        continue
                    node = warm_nodes.get(backends[probe.id])
                    if node is None or facts(node, origin) != observed.get(probe.id) or node["nodeId"] != identities[probe.id][1]:
                        errors.append({"probe": probe.id, "repeat": repeat, "error": "warm tree changed without mutation"})
            rows.append({"target": target, "case": case.name, "step": step.name,
                         "navigation_ms": navigation_ms, "partial_ms": partial_ms,
                         "full_first_ms": full_ms, "full_warm_ms": warm_ms,
                         "full_warm_median_ms": statistics.median(warm_ms), "query_ms": query_ms,
                         "nodes": len(tree["nodes"]), "roles": dict(roles),
                         "observed": observed, "errors": errors})
    finally:
        if page.browser_context_id:
            command = await client.send("Target.disposeBrowserContext", {"browserContextId": page.browser_context_id})
            await client.recv_until_id(command, timeout=timeout)
    return rows


async def run_target(target: str, binary: Path, origin: str, output: Path,
                     runs: int, timeout: float) -> dict[str, Any]:
    handle = start_target_serve(target, binary, timeout)
    rows = []
    client = None
    try:
        client = await connect_raw_cdp(handle.endpoint)
        command = await client.send("Browser.getVersion")
        version, _ = await client.recv_until_id(command, timeout=timeout)
        for case in AX_CASES:
            try:
                case_rows = await measure_case(client, target, case, origin, output / target / case.name, runs, timeout)
            except Exception as error:
                case_rows = [{"target": target, "case": case.name, "step": "error", "errors": [{"error": repr(error)}]}]
                (output / f"{target}-{case.name}-error.json").write_text(json.dumps({
                    "error": repr(error), "receive": getattr(error, "cdp_receive_evidence", None)}, indent=2) + "\n")
            rows += case_rows
            (output / f"{target}.json").write_text(json.dumps({"version": version["result"], "rows": rows}, indent=2) + "\n")
            print(json.dumps({"target": target, "case": case.name, "errors": sum(len(row["errors"]) for row in case_rows)}), flush=True)
    finally:
        if client is not None:
            await client.websocket.close()
        details = stop_target_serve(handle)
        (output / f"{target}-process.json").write_text(json.dumps(details, indent=2) + "\n")
    return {"version": version["result"], "rows": rows}


def build_summary(reports: dict[str, Any]) -> dict[str, Any]:
    indexed = {target: {(row["case"], row["step"]): row for row in report["rows"]}
               for target, report in reports.items()}
    comparisons = []
    for case in AX_CASES:
        for step in case.steps:
            chrome = indexed["chrome-cdp"].get((case.name, step.name), {})
            moli = indexed["moli-cdp"].get((case.name, step.name), {})
            comparisons.append({"case": case.name, "step": step.name,
                                "differences": compare_observations(chrome.get("observed", {}), moli.get("observed", {})),
                                "missing_target": not chrome or not moli})
    passing = []
    for case in AX_CASES:
        matches = [row for row in comparisons if row["case"] == case.name]
        if all(not row["differences"] and not row["missing_target"] for row in matches) and all(
            not indexed[target].get((case.name, step.name), {"errors": ["missing"]})["errors"]
            for target in reports for step in case.steps
        ):
            passing.append(case.name)
    return {"cases": len(AX_CASES), "phases": len(comparisons), "passing_cases": passing,
            "expectation_failures": {target: sum(len(row["errors"]) for row in report["rows"]) for target, report in reports.items()},
            "comparisons": comparisons,
            "matching_phases": sum(not row["differences"] and not row["missing_target"] for row in comparisons)}


def write_report(output: Path, reports: dict[str, Any], summary: dict[str, Any]) -> None:
    lines = ["# AX integration benchmark", "",
             f"{summary['cases']} independent documents; {summary['phases']} phases. "
             f"Passing cases: {len(summary['passing_cases'])}/{summary['cases']}. "
             f"Matching Chromium observations: {summary['matching_phases']}/{summary['phases']}.", "",
             "CDP round-trip milliseconds. Cold partial precedes the first full tree; warm values are medians. "
             "Mutation phases retain the document. Each failed assertion is retained; measurements do not retry assertions.", "",
             "| Case / phase | Moli first full | Moli warm full | Chromium first full | Chromium warm full | Checks |",
             "|---|---:|---:|---:|---:|---|"]
    for comparison in summary["comparisons"]:
        data = {target: next((row for row in report["rows"] if (row["case"], row["step"]) ==
                             (comparison["case"], comparison["step"])), {}) for target, report in reports.items()}
        times = [f"{data[target][key]:.2f}" if key in data[target] else "—"
                 for target in ("moli-cdp", "chrome-cdp") for key in ("full_first_ms", "full_warm_median_ms")]
        ok = not comparison["differences"] and not comparison["missing_target"] and all(row and not row["errors"] for row in data.values())
        lines.append(f"| {comparison['case']} / {comparison['step']} | {' | '.join(times)} | {'pass' if ok else 'FAIL'} |")
    lines += ["", "Expectation failures: " + json.dumps(summary["expectation_failures"]), "",
              "Full observations and differences: `summary.json`, `moli-cdp.json`, `chrome-cdp.json`. "
              "Per-case artifacts retain partial/full/query trees and every warm sample.", ""]
    differences = [row for row in summary["comparisons"] if row["differences"]]
    if differences:
        lines += ["## Chromium comparison differences", "",
                  "| Case / phase | Probe | Differing fields |", "|---|---|---|"]
        for row in differences:
            for probe, observations in row["differences"].items():
                chrome, moli = observations["chrome"] or {}, observations["moli"] or {}
                fields = [key for key in sorted(chrome.keys() | moli.keys()) if chrome.get(key) != moli.get(key)]
                lines.append(f"| {row['case']} / {row['step']} | {probe} | {', '.join(fields)} |")
        lines.append("")
    for target, report in reports.items():
        failures = [row for row in report["rows"] if row["errors"]]
        if failures:
            lines += [f"## {target} failures", ""]
            for row in failures:
                for error in row["errors"]:
                    lines.append(f"- `{row['case']} / {row['step']}`: {json.dumps(error, ensure_ascii=False)}")
            lines.append("")
    (output / "report.md").write_text("\n".join(lines))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--moli-bin", type=Path, required=True)
    parser.add_argument("--chrome-bin", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=15)
    args = parser.parse_args()
    if args.runs < 1 or args.timeout <= 0:
        parser.error("runs and timeout must be positive")
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    binaries = {"chrome-cdp": args.chrome_bin.resolve(), "moli-cdp": args.moli_bin.resolve()}
    manifest = {"runs": args.runs, "timeout": args.timeout, "binaries": {},
                "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
                "fixtures": {case.name: hashlib.sha256(json.dumps(
                    {"html": case.html(), "contract": asdict(case)}, sort_keys=True).encode()).hexdigest()
                    for case in AX_CASES}}
    for target, path in binaries.items():
        with path.open("rb") as binary:
            manifest["binaries"][target] = {"path": str(path), "sha256": hashlib.file_digest(binary, "sha256").hexdigest()}
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        origin = f"http://127.0.0.1:{server.server_port}"
        reports = {}
        for target, binary in binaries.items():
            try:
                reports[target] = asyncio.run(run_target(target, binary, origin, output, args.runs, args.timeout))
            except Exception as error:
                reports[target] = {"version": None, "rows": [
                    {"target": target, "case": case.name, "step": "error", "errors": [{"error": repr(error)}]}
                    for case in AX_CASES]}
                (output / f"{target}.json").write_text(json.dumps(reports[target], indent=2) + "\n")
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
    summary = build_summary(reports)
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    write_report(output, reports, summary)
    print(json.dumps({key: value for key, value in summary.items() if key != "comparisons"}, indent=2))
    return int(any(summary["expectation_failures"].values()) or summary["matching_phases"] != summary["phases"])


if __name__ == "__main__":
    raise SystemExit(main())
