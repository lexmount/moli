from __future__ import annotations

import asyncio
from copy import deepcopy
from dataclasses import asdict
from datetime import datetime, timezone
import hashlib
import importlib.metadata
import json
from pathlib import Path
import platform
import subprocess
import time

from ..artifacts import write_json
from ..config import REPO_ROOT
from .browser import browser_session
from .cases import Case, manifest
from .observations import Observer, complete_fp_result, error_record
from .probe import site_probe
from .report import write_report

SCHEMA_VERSION = 1


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def collector_hashes() -> dict[str, str]:
    root = Path(__file__).parent
    paths = [*root.glob("*.py"), *root.glob("*.js"), root.parent / "target_serve.py"]
    return {str(path.relative_to(root.parent)): digest(path) for path in sorted(paths)}


def workload(cases: tuple[Case, ...]) -> dict:
    return {"cases": manifest(cases), "mode": "native-cdp", "runtime_enabled": True,
            "layout": "on-demand", "resources": "all", "identity_override": False,
            "tls_verification": True, "proxy": "disabled", "parallelism": 1,
            "navigation_timeout_seconds": 45, "sample_timeout_seconds": 150,
            "typing_delay_ms": 80, "dummy_credentials": True, "screenshot": False,
            "playwright_version": importlib.metadata.version("playwright"),
            "collector": collector_hashes()}


def workload_hash(value: dict) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def classify(row: dict) -> str:
    if not row.get("process_cleaned_up"):
        return "cleanup_error"
    if row.get("browser_crashed"):
        return "browser_crash"
    errors = row.get("errors", [])
    if errors:
        if any(error.get("network_codes") for error in errors):
            return "network_error"
        if any("Timeout" in error["type"] for error in errors):
            return "timeout"
        return "collector_error"
    if (row.get("navigation_status") or 0) >= 400:
        return "network_error"
    report = row.get("report") or {}
    if report.get("valid") is True:
        return "complete"
    evidence = [value for key, value in report.items() if key not in {"valid", "readyState", "collector", "kind", "source"}]
    return "partial" if any(value not in (None, False, "", {}, []) for value in evidence) else "missing"


async def act(page, case: Case):
    if case.action == "demo_login":
        await page.locator('input[type=email]').first.click(timeout=10000)
        await page.keyboard.type("moli-cdp-demo@example.com", delay=80)
        await page.locator('input[type=password]').first.click(timeout=10000)
        await page.keyboard.type("Moli-Demo-Only-2026!", delay=80)
        await page.get_by_role("button", name="Login", exact=True).click(timeout=10000)
    elif case.action == "start_scan":
        link = page.locator('a[href="/fingerprint-check"]').filter(has_text="Scan My Browser Now")
        if await link.count() != 1:
            raise ValueError("public scan link is absent or ambiguous")
        await link.click(timeout=10000)
    elif case.action is not None:
        raise ValueError("unknown site action")


async def capture(case: Case, engine: str, binary: Path, output: Path, *, probe: str | None = None, run: int = 1) -> dict:
    row = {"site": case.id, "engine": engine, "run": run, "case": asdict(case), "errors": [],
           "started_at": datetime.now(timezone.utc).isoformat(), "report": None,
           "browser_crashed": False}
    stage = "session"
    try:
        async with browser_session(binary, engine, output) as (page, cdp):
            def crashed(_page):
                row["browser_crashed"] = True
            page.on("crash", crashed)
            started = time.monotonic()
            clock = lambda: time.monotonic() - started
            observer = Observer(cdp, clock)
            try:
                async with asyncio.timeout(150):
                    stage = "navigation"
                    response = await page.goto(case.url, wait_until="domcontentloaded", timeout=45000)
                    row["navigation_status"] = response.status if response else None
                    row["domcontentloaded_at"] = clock()
                    await asyncio.sleep(case.wait_seconds)
                    if case.action:
                        stage = case.action
                        await act(page, case)
                        row["action_completed_at"] = clock()
                        await asyncio.sleep(case.action_wait_seconds)
                    stage = "projection"
                    cutoff = clock()
                    frozen = observer.snapshot(cutoff)
                    row["cutoff"] = cutoff
                    report = await asyncio.wait_for(page.evaluate(probe or site_probe(), {"site": case.id}), 15)
                    if not isinstance(report, dict) or type(report.get("valid")) is not bool:
                        raise ValueError("projector returned an invalid report envelope")
                    row["report"] = report
                    row["snapshot_completed_at"] = clock()
                    stage = "network_evidence"
                    row["observations"] = await observer.resolve(frozen)
                    if case.id == "fingerprint-pro":
                        completed = [entry["result"] for entry in row["observations"]["responses"]
                                     if entry["within_window"] and complete_fp_result(entry.get("result", {}))]
                        # Multiple contradictory replies are not a license to select the best score.
                        unique = {json.dumps(value, sort_keys=True) for value in completed}
                        row["report"] = {"valid": len(unique) == 1 and not row["observations"]["result_limit_reached"], "results": completed,
                                         "ambiguous": len(unique) > 1}
                    row["elapsed_seconds"] = clock()
            except Exception as error:
                row["errors"].append(error_record(stage, error))
    except Exception as error:
        row["errors"].append(error_record(stage, error))
    metadata_path = output / "environment.json"
    metadata = json.loads(metadata_path.read_text()) if metadata_path.exists() else {}
    row["process_cleaned_up"] = metadata.get("process_cleaned_up", False)
    row["browser"] = metadata.get("browser", {})
    row["status"] = classify(row)
    row["finished_at"] = datetime.now(timezone.utc).isoformat()
    write_json(output / "result.json", row)
    return row


def build_summary(rows: list[dict], cases: tuple[Case, ...], engines: list[str], *, runs: int = 1) -> dict:
    states = ("complete", "partial", "missing", "timeout", "network_error", "collector_error", "browser_crash", "cleanup_error")
    coverage = {engine: {"planned": len(cases) * runs, "attempted": sum(r["engine"] == engine for r in rows),
                        **{state: sum(r["engine"] == engine and r["status"] == state for r in rows) for state in states}}
                for engine in engines}
    return {"suite": "fingerprint", "observational": True, "availability_is_not_pass_count": True,
            "cases": [case.id for case in cases], "runs": runs, "coverage": coverage, "samples": deepcopy(rows)}


async def run_suite(output: Path, binaries: dict[str, Path], cases: tuple[Case, ...], *, runs: int = 1, baseline: dict | None = None) -> dict:
    if not cases or not binaries or set(binaries) - {"moli", "chromium"} or not 1 <= runs <= 10:
        raise ValueError("supply cases, native engines and 1..10 predeclared runs")
    binaries = {engine: path.resolve(strict=True) for engine, path in binaries.items()}
    config = workload(cases)
    provenance = {"schema_version": SCHEMA_VERSION, "suite": "fingerprint", "workload": config,
                  "workload_hash": workload_hash(config), "started_at": datetime.now(timezone.utc).isoformat(),
                  "environment": {"platform": platform.platform(), "machine": platform.machine(),
                                  "python": platform.python_version()},
                  "binaries": {engine: {"path": str(path), "sha256": digest(path)} for engine, path in binaries.items()}}
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=REPO_ROOT, text=True, capture_output=True, check=True)
    provenance["revision"] = revision.stdout.strip()
    output.mkdir(parents=True, exist_ok=False)
    write_json(output / "manifest.json", provenance)
    rows: list[dict] = []
    summary = {"schema_version": SCHEMA_VERSION, "manifest": provenance, "suites": []}
    try:
        for run in range(1, runs + 1):
            for index, case in enumerate(cases):
                engines = list(binaries)
                if (index + run - 1) % 2:
                    engines.reverse()
                for engine in engines:
                    print(f"fingerprint {run}/{runs} {engine} {case.id}", flush=True)
                    row = await capture(case, engine, binaries[engine], output / "samples" / str(run) / engine / case.id, run=run)
                    rows.append(row)
                    summary["suites"] = [build_summary(rows, cases, list(binaries), runs=runs)]
                    write_json(output / "summary.json", summary)
                    write_report(output, summary, baseline)
                    if not row["process_cleaned_up"]:
                        raise RuntimeError("browser cleanup failed; refusing to start another public sample")
    finally:
        summary["suites"] = [build_summary(rows, cases, list(binaries), runs=runs)]
        summary["inputs_unchanged"] = collector_hashes() == config["collector"] and all(
            digest(path) == provenance["binaries"][engine]["sha256"] for engine, path in binaries.items())
        summary["finished_at"] = datetime.now(timezone.utc).isoformat()
        summary["matrix_complete"] = len(rows) == len(cases) * len(binaries) * runs
        write_json(output / "summary.json", summary)
        write_report(output, summary, baseline)
    return summary
