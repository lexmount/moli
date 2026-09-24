from __future__ import annotations

import json
import os
import shutil
from pathlib import Path
from typing import Any

from .artifacts import write_json, write_text
from .config import REPO_ROOT, clear_proxy_env
from .process import run_process

CDP_SMOKE_PROFILES = ("smoke", "formal")
CDP_CLIENTS = ("raw_cdp", "playwright", "puppeteer")
PUPPETEER_MODULE_ENV = "PUPPETEER_CORE_MODULE"


def _extract_json_payload(stdout: bytes) -> dict[str, Any] | None:
    text = stdout.decode("utf-8", errors="replace").strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        start = text.find("{")
        end = text.rfind("}")
        if start >= 0 and end > start:
            return json.loads(text[start : end + 1])
    return None


def _default_command() -> list[str]:
    if shutil.which("uv"):
        return ["uv", "run", "moli-cdp-smoke"]
    return ["python3", "-m", "moli_cdp_smoke"]


def _supervisor_payload(output_dir: Path, groups: tuple[str, ...]) -> dict[str, Any]:
    records: list[Any] = []
    errors: list[str] = []
    try:
        summary = json.loads((output_dir / "summary.json").read_text())
        outcomes = summary["groups"]
        if (summary.get("ok") is not True or len(outcomes) != len(groups)
                or {row["group"] for row in outcomes} != set(groups)
                or any(row["status"] != "passed" for row in outcomes)):
            errors.append("supervisor did not pass every selected group")
    except (OSError, ValueError, KeyError, TypeError) as error:
        errors.append(f"invalid supervisor summary: {error}")
    for group in groups:
        try:
            payload = json.loads((output_dir / f"{group}.json").read_text())
            if not isinstance(payload, dict):
                raise ValueError("worker result must be an object")
            if payload.get("group") != group or payload.get("ok") is not True:
                errors.append(f"{group}: unsuccessful worker result")
            if not isinstance(payload["results"], list):
                raise ValueError("worker results must be a list")
            records.extend(payload["results"])
        except (OSError, ValueError, KeyError, TypeError) as error:
            errors.append(f"{group}: invalid worker result: {error}")
    return {"ok": not errors, "results": records, "error": "; ".join(errors) or None}


def _discover_group_listing(smoke_command: list[str], timeout_seconds: float, env: dict[str, str]) -> list[dict[str, Any]]:
    result = run_process(
        [*smoke_command, "--list-groups"],
        cwd=REPO_ROOT / "moli-cdp-smoke",
        timeout_seconds=min(timeout_seconds, 30.0),
        env=env,
        sample_resources=False,
    )
    payload = _extract_json_payload(result.stdout)
    if payload is None and result.stderr:
        payload = _extract_json_payload(result.stderr)
    groups = payload.get("groups") if isinstance(payload, dict) else None
    if result.returncode != 0 or result.timed_out or not isinstance(groups, list):
        stderr = result.stderr[-2048:].decode("utf-8", errors="replace")
        raise RuntimeError(f"failed to list CDP smoke groups: returncode={result.returncode} timed_out={result.timed_out} {stderr}")
    return [group for group in groups if isinstance(group, dict)]


def _group_client(group: dict[str, Any]) -> str | None:
    name = str(group.get("name", ""))
    phase = str(group.get("phase", ""))
    if phase in {"raw", "process"}:
        return "raw_cdp"
    if phase in {"page", "browser"}:
        return "playwright"
    if phase == "external" and name == "puppeteer":
        return "puppeteer"
    return None


def _effective_cdp_smoke_groups(
    profile: str,
    groups: tuple[str, ...],
    group_listing: list[dict[str, Any]],
) -> tuple[str, ...]:
    if groups:
        return groups
    selected: list[str] = []
    if profile == "formal":
        for wanted_client in CDP_CLIENTS:
            for group in group_listing:
                name = group.get("name")
                if isinstance(name, str) and name and _group_client(group) == wanted_client:
                    selected.append(name)
        return tuple(selected)
    for group in group_listing:
        name = group.get("name")
        if isinstance(name, str) and name and group.get("default") is True:
            selected.append(name)
    return tuple(selected)


def _selected_groups_by_client(
    selected_groups: tuple[str, ...],
    group_listing: list[dict[str, Any]],
) -> dict[str, list[str]]:
    by_name = {str(group.get("name")): group for group in group_listing if group.get("name") is not None}
    grouped = {client: [] for client in CDP_CLIENTS}
    for group_name in selected_groups:
        client = _group_client(by_name.get(group_name, {"name": group_name}))
        if client in grouped:
            grouped[client].append(group_name)
    return grouped


def _record_client(record: Any) -> str | None:
    if not isinstance(record, dict):
        return None
    name = record.get("name")
    if not isinstance(name, str):
        return None
    if name.startswith("raw_cdp_"):
        return "raw_cdp"
    if name.startswith("puppeteer_"):
        return "puppeteer"
    return "playwright"


def _records_by_client(records: list[Any]) -> dict[str, list[dict[str, Any]]]:
    grouped = {client: [] for client in CDP_CLIENTS}
    for record in records:
        client = _record_client(record)
        if client in grouped and isinstance(record, dict):
            grouped[client].append(record)
    return grouped


def _executable_available(value: str) -> bool:
    if os.sep in value:
        return Path(value).exists()
    return shutil.which(value) is not None


def _collect_preflight(timeout_seconds: float, env: dict[str, str]) -> dict[str, Any]:
    node = env.get("NODE", "node")
    node_available = _executable_available(node)
    module_name = env.get(PUPPETEER_MODULE_ENV, "puppeteer-core")
    puppeteer_core = {
        "module": module_name,
        "available": False,
        "error": None,
    }
    if node_available:
        result = run_process(
            [node, "-e", f"require({module_name!r})"],
            cwd=REPO_ROOT / "moli-cdp-smoke",
            timeout_seconds=min(timeout_seconds, 10.0),
            env=env,
            sample_resources=False,
        )
        puppeteer_core["available"] = result.returncode == 0 and not result.timed_out
        if not puppeteer_core["available"]:
            puppeteer_core["error"] = result.stderr[-2048:].decode("utf-8", errors="replace")
    else:
        puppeteer_core["error"] = f"executable `{node}` not found"
    return {
        "node": {"executable": node, "available": node_available},
        "puppeteer_core": puppeteer_core,
    }


def _client_failure_kind(
    *,
    client: str,
    covered: bool,
    selected_groups: list[str],
    required: bool,
    preflight: dict[str, Any],
    process_ok: bool,
    timed_out: bool,
) -> str | None:
    if covered:
        return None
    if client == "puppeteer":
        node = preflight.get("node", {})
        puppeteer_core = preflight.get("puppeteer_core", {})
        if not node.get("available") or not puppeteer_core.get("available"):
            return "dependency-missing"
    if timed_out:
        return "timeout"
    if selected_groups and not process_ok:
        return "process-failed"
    if required:
        return "no-records"
    return "not-selected"


def _client_rows(
    *,
    profile: str,
    selected_groups: tuple[str, ...],
    group_listing: list[dict[str, Any]],
    records: list[Any],
    preflight: dict[str, Any],
    process_ok: bool,
    timed_out: bool,
) -> list[dict[str, Any]]:
    groups_by_client = _selected_groups_by_client(selected_groups, group_listing)
    records_by_client = _records_by_client(records)
    rows = []
    for client in CDP_CLIENTS:
        client_records = records_by_client[client]
        required = profile == "formal"
        covered = bool(client_records)
        failure_kind = _client_failure_kind(
            client=client,
            covered=covered,
            selected_groups=groups_by_client[client],
            required=required,
            preflight=preflight,
            process_ok=process_ok,
            timed_out=timed_out,
        )
        rows.append(
            {
                "client": client,
                "required": required,
                "covered": covered,
                "ok": covered,
                "gate_ok": (not required) or covered,
                "failure_kind": failure_kind,
                "groups": groups_by_client[client],
                "record_count": len(client_records),
                "records": [record.get("name") for record in client_records if record.get("name")],
            }
        )
    return rows


def _client_coverage(client_rows: list[dict[str, Any]]) -> dict[str, bool]:
    return {str(row["client"]): bool(row.get("covered")) for row in client_rows}


def _formal_requirements(client_rows: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    return {
        str(row["client"]): {
            "required": bool(row.get("required")),
            "ok": bool(row.get("gate_ok")),
            "actual": bool(row.get("covered")),
            "failure_kind": row.get("failure_kind"),
        }
        for row in client_rows
    }


def _markdown(summary: dict[str, Any]) -> str:
    lines = [
        "# CDP smoke benchmark",
        "",
        f"- ok: `{summary['ok']}`",
        f"- profile: `{summary['profile']}`",
        f"- total records: `{summary['total_records']}`",
        f"- elapsed ms: `{summary['elapsed_ms']:.3f}`",
        f"- timeout: `{summary['timed_out']}`",
        f"- raw CDP coverage: `{summary['client_coverage']['raw_cdp']}`",
        f"- Playwright coverage: `{summary['client_coverage']['playwright']}`",
        f"- Puppeteer coverage: `{summary['client_coverage']['puppeteer']}`",
        "",
    ]
    if summary["groups"]:
        lines.append("Groups:")
        for group in summary["groups"]:
            lines.append(f"- `{group}`")
        lines.append("")
    if summary["error"]:
        lines.append("Error:")
        lines.append("")
        lines.append("```text")
        lines.append(str(summary["error"])[-4000:])
        lines.append("```")
        lines.append("")
    return "\n".join(lines)


def run_cdp_smoke_suite(
    *,
    output_dir: Path,
    moli_bin: Path,
    timeout_seconds: float,
    groups: tuple[str, ...],
    profile: str = "smoke",
    command: tuple[str, ...] | None = None,
) -> dict[str, Any]:
    if profile not in CDP_SMOKE_PROFILES:
        raise RuntimeError(f"unknown CDP smoke profile `{profile}`")
    suite_dir = output_dir / "cdp-smoke"
    smoke_command = list(command or _default_command())
    env = clear_proxy_env(os.environ)
    env["MOLI_BIN"] = str(moli_bin)
    group_listing = _discover_group_listing(smoke_command, timeout_seconds, env)
    preflight = _collect_preflight(timeout_seconds, env)
    effective_groups = _effective_cdp_smoke_groups(profile, groups, group_listing)
    supervisor_dir = None
    if not command:
        supervisor_dir = (suite_dir / "workers").resolve()
        # A fresh directory prevents a crashed runner from reusing stale passes.
        supervisor_dir.mkdir(parents=True, exist_ok=False)
        smoke_command.extend(["--output-dir", str(supervisor_dir)])
    for group in effective_groups:
        smoke_command.extend(["--group", group])

    result = run_process(
        smoke_command,
        cwd=REPO_ROOT / "moli-cdp-smoke",
        timeout_seconds=timeout_seconds,
        env=env,
    )
    if supervisor_dir is not None:
        payload = _supervisor_payload(supervisor_dir, effective_groups)
    else:
        payload = _extract_json_payload(result.stdout)
        if payload is None and result.stderr:
            payload = _extract_json_payload(result.stderr)

    records = payload.get("results", []) if isinstance(payload, dict) else []
    ok = result.returncode == 0 and not result.timed_out and isinstance(payload, dict) and payload.get("ok") is True
    client_rows = _client_rows(
        profile=profile,
        selected_groups=effective_groups,
        group_listing=group_listing,
        records=records if isinstance(records, list) else [],
        preflight=preflight,
        process_ok=ok,
        timed_out=result.timed_out,
    )
    coverage = _client_coverage(client_rows)
    formal_requirements = _formal_requirements(client_rows)
    profile_failures = sum(1 for requirement in formal_requirements.values() if not bool(requirement["ok"]))
    total_failures = 0 if ok else 1
    error = None if ok else (payload.get("error") if isinstance(payload, dict) else result.stderr[-4096:].decode("utf-8", errors="replace"))
    summary = {
        "suite": "cdp-smoke",
        "ok": ok,
        "profile": profile,
        "total_failures": total_failures,
        "profile_failures": profile_failures,
        "gate_failures": total_failures + profile_failures,
        "groups": list(effective_groups),
        "explicit_groups": bool(groups),
        "group_selection_source": "explicit" if groups else "group-listing",
        "group_listing": group_listing,
        "preflight": preflight,
        "client_rows": client_rows,
        "client_coverage": coverage,
        "formal_requirements": formal_requirements,
        "total_records": len(records) if isinstance(records, list) else 0,
        "elapsed_ms": result.elapsed_ms,
        "timed_out": result.timed_out,
        "returncode": result.returncode,
        "peak_pss_bytes": result.resources.get("peak_pss_bytes"),
        "peak_cpu_percent": result.resources.get("peak_cpu_percent"),
        "error": error,
    }
    write_json(suite_dir / "group-listing.json", {"groups": group_listing})
    write_json(suite_dir / "preflight.json", preflight)
    write_json(suite_dir / "client-rows.json", {"rows": client_rows})
    write_json(suite_dir / "moli-cdp-smoke.json", payload or {})
    write_json(suite_dir / "process.json", result.json_summary(include_output=not ok))
    write_json(suite_dir / "summary.json", summary)
    write_text(suite_dir / "summary.md", _markdown(summary))
    return summary
