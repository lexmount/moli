from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from .artifacts import write_csv, write_json
from .chrome_dcl import run_chrome_dcl_dump, run_served_cdp_dcl_dump
from .config import clear_proxy_env
from .process import ProcessResult, run_process
from .public_web import (
    PUBLIC_WEB_SNAPSHOT_CONTRACT,
    WILD_WEB_ARTIFACT_POLICY,
    WILD_WEB_CLASSIFIER,
    PublicWebAttempt,
    PublicWebResult,
    PublicWebScheduledCase,
    PublicWebScheduler,
    build_public_web_fetch_command,
    count_public_web_row_values,
    public_web_target_metadata as _wild_web_target_metadata,
    rotated_target_order as _rotated_target_order,
    run_public_web_target,
    schedule_public_web_cases,
    successful_public_web_attempt_cohort,
    unavailable_public_web_row,
    write_public_web_failure_artifacts,
    write_public_web_replay_artifact,
)
from .stats import summarize
from .synthetic_compare import WEBFETCH_TARGETS


WILD_WEB_SEEDS: dict[str, str] = {
    "baidu-home": "https://www.baidu.com/",
    "bilibili-home": "https://www.bilibili.com/",
    "zhihu-home": "https://www.zhihu.com/",
    "toutiao-home": "https://www.toutiao.com/",
}

WILD_WEB_SEED_ASSERTIONS: dict[str, dict[str, Any]] = {
    "baidu-home": {"title_any": ("百度", "baidu"), "min_text_length": 20},
    "bilibili-home": {"title_any": ("哔哩", "bilibili"), "min_text_length": 20},
    "zhihu-home": {"title_any": ("知乎", "zhihu"), "min_text_length": 20},
    "toutiao-home": {"title_any": ("头条", "toutiao"), "min_text_length": 20},
}

_extract_page_snapshot = WILD_WEB_CLASSIFIER.snapshot


def _wild_web_extraction_failures(seed: str, snapshot: dict[str, Any]) -> list[str]:
    assertions = WILD_WEB_SEED_ASSERTIONS.get(seed, {})
    failures: list[str] = []
    title = str(snapshot.get("title") or "")
    sample = str(snapshot.get("text_sample") or "")
    text_length = int(snapshot.get("text_length") or 0)
    title_lower = title.lower()
    sample_lower = sample.lower()
    title_any = tuple(str(value).lower() for value in assertions.get("title_any", ()))
    text_any = tuple(str(value).lower() for value in assertions.get("text_any", ()))
    min_text_length = int(assertions.get("min_text_length", 1))
    if not title:
        failures.append("missing-title")
    elif title_any and not any(value in title_lower for value in title_any):
        failures.append("title-keyword-mismatch")
    if text_length < min_text_length:
        failures.append("short-body-text")
    elif text_any and not any(value in sample_lower for value in text_any):
        failures.append("text-keyword-mismatch")
    return failures


def _wild_command_for_target(
    target: str,
    binary: Path,
    url: str,
    timeout_seconds: float,
) -> list[str]:
    return build_public_web_fetch_command(
        target,
        binary,
        url,
        timeout_seconds,
        suite_name="wild-web",
    )


def _run_wild_web_target(
    *,
    target: str,
    binary: Path,
    url: str,
    timeout_seconds: float,
    proc_env: dict[str, str],
) -> ProcessResult:
    return run_public_web_target(
        target=target,
        binary=binary,
        url=url,
        timeout_seconds=timeout_seconds,
        proc_env=proc_env,
        command_builder=_wild_command_for_target,
        chrome_runner=run_chrome_dcl_dump,
        served_cdp_runner=run_served_cdp_dcl_dump,
        process_runner=run_process,
    )


def _classify(
    stdout: bytes,
    stderr: bytes,
    returncode: int | None,
    timed_out: bool,
    response_status: int | None = None,
) -> str:
    return WILD_WEB_CLASSIFIER.classify_output(
        stdout=stdout,
        stderr=stderr,
        returncode=returncode,
        timed_out=timed_out,
        response_status=response_status,
    )


def _failure_kind(category: str, extraction_failures: list[str], error: str | None = None) -> str | None:
    return WILD_WEB_CLASSIFIER.failure_kind(
        category,
        error=error,
        extraction_failed=bool(extraction_failures),
    )


def _successful_attempt_cohort(
    rows: list[dict[str, Any]],
    targets: tuple[str, ...],
) -> dict[str, Any]:
    return successful_public_web_attempt_cohort(
        rows,
        targets,
        attempt_key_fields=("run", "seed"),
        unique_key_fields=("seed",),
        unique_count_name="unique_seeds",
        metric_fields=("elapsed_ms", "peak_pss_bytes"),
    )


def _execute_wild_web_attempt(
    *,
    suite_dir: Path,
    target: str,
    metadata: dict[str, str],
    info: dict[str, Any],
    run_id: int,
    seed: str,
    timeout_seconds: float,
    proc_env: dict[str, str],
    schedule_index: int,
    target_order_index: int,
    capture_replay: bool,
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    url = WILD_WEB_SEEDS[seed]
    attempt = PublicWebAttempt.start(
        target=target,
        metadata=metadata,
        target_info=info,
        run=run_id,
        case_fields={"seed": seed},
        url=url,
        schedule_index=schedule_index,
        target_order_index=target_order_index,
        artifact_stem=f"{target}-run-{run_id}-{seed}",
    )
    binary = attempt.binary_path()
    if binary is None:
        row = unavailable_public_web_row(
            attempt,
            category="error",
            extra_fields={
                "classification_ok": False,
                "extraction_ok": False,
                "extraction_failures": ["target-unavailable"],
                "extraction_failure_count": 1,
            },
        )
        row["failure_artifact"] = write_public_web_failure_artifacts(
            suite_dir=suite_dir,
            attempt=attempt,
            row=row,
            stdout=b"",
            stderr=b"",
            policy=WILD_WEB_ARTIFACT_POLICY,
        )
        return row, None

    process = _run_wild_web_target(
        target=target,
        binary=binary,
        url=url,
        timeout_seconds=timeout_seconds,
        proc_env=proc_env,
    )
    result = PublicWebResult.capture(
        attempt,
        process,
        classifier=WILD_WEB_CLASSIFIER,
    )
    category = WILD_WEB_CLASSIFIER.classify_result(result)
    snapshot = result.snapshot
    extraction_failures = _wild_web_extraction_failures(seed, snapshot)
    classification_ok = WILD_WEB_CLASSIFIER.classification_ok(category)
    extraction_ok = not extraction_failures
    ok = classification_ok and extraction_ok
    row = {
        **result.base_row(),
        **result.evidence_fields(policy=WILD_WEB_ARTIFACT_POLICY),
        "category": category,
        "classification_basis": WILD_WEB_CLASSIFIER.classification_basis(
            result, category
        ),
        "classification_ok": classification_ok,
        "extraction_ok": extraction_ok,
        "ok": ok,
        "title": snapshot["title"],
        "text_length": snapshot["text_length"],
        "text_sample": snapshot["text_sample"],
        "extraction_failures": extraction_failures,
        "extraction_failure_count": len(extraction_failures),
        "failure_kind": None if ok else _failure_kind(category, extraction_failures),
    }
    if not ok:
        row["failure_artifact"] = write_public_web_failure_artifacts(
            suite_dir=suite_dir,
            attempt=attempt,
            row=row,
            stdout=process.stdout,
            stderr=process.stderr,
            policy=WILD_WEB_ARTIFACT_POLICY,
        )
        return row, None
    if not capture_replay:
        return row, None
    row["replay_artifact"] = write_public_web_replay_artifact(
        suite_dir=suite_dir,
        attempt=attempt,
        stdout=process.stdout,
    )
    return row, {
        "target": target,
        **metadata,
        "run": run_id,
        "seed": seed,
        "url": url,
        "category": category,
        "title": snapshot["title"],
        "text_length": snapshot["text_length"],
        "artifact": row["replay_artifact"],
    }


def run_wild_web_suite(
    *,
    output_dir: Path,
    target_matrix: dict[str, Any],
    targets: tuple[str, ...],
    seeds: tuple[str, ...],
    runs: int,
    timeout_seconds: float,
    gate_target: str,
    capture_replay: bool = False,
) -> dict[str, Any]:
    unknown_targets = [target for target in targets if target not in WEBFETCH_TARGETS]
    if unknown_targets:
        raise RuntimeError(f"unknown webfetch target(s): {', '.join(unknown_targets)}")
    if gate_target not in WEBFETCH_TARGETS:
        raise RuntimeError(f"unknown gate target: {gate_target}")

    suite_dir = output_dir / "wild-web"
    selected = seeds or tuple(WILD_WEB_SEEDS.keys())
    for seed in selected:
        if seed not in WILD_WEB_SEEDS:
            raise RuntimeError(f"unknown wild web seed: {seed}")
    proc_env = clear_proxy_env(os.environ)
    metadata_by_target = {
        target: _wild_web_target_metadata(target) for target in targets
    }
    info_by_target = {
        target: target_matrix.get(metadata_by_target[target]["binary_key"], {})
        for target in targets
    }
    scheduled_cases = schedule_public_web_cases(selected, runs=runs)

    def execute_attempt(
        scheduled: PublicWebScheduledCase[str],
        target: str,
        target_order_index: int,
    ) -> tuple[dict[str, Any], dict[str, Any] | None]:
        return _execute_wild_web_attempt(
            suite_dir=suite_dir,
            target=target,
            metadata=metadata_by_target[target],
            info=info_by_target[target],
            run_id=scheduled.run,
            seed=scheduled.case,
            timeout_seconds=timeout_seconds,
            proc_env=proc_env,
            schedule_index=scheduled.schedule_index,
            target_order_index=target_order_index,
            capture_replay=capture_replay,
        )

    executions = PublicWebScheduler[
        str,
        tuple[dict[str, Any], dict[str, Any] | None],
    ](targets).run(scheduled_cases, execute_attempt)
    rows = [row for row, _ in executions]
    replay_manifest = [
        replay_entry
        for _, replay_entry in executions
        if replay_entry is not None
    ]

    gate_failures = sum(1 for row in rows if row["target"] == gate_target and not row.get("ok"))
    summary: dict[str, Any] = {
        "suite": "wild-web",
        "runs": runs,
        "seeds": list(selected),
        "timeout_seconds": timeout_seconds,
        "snapshot_contract": PUBLIC_WEB_SNAPSHOT_CONTRACT,
        "schedule": "site-paired-rotating-target-order",
        "scheduled_site_groups": len(scheduled_cases),
        "gate_target": gate_target,
        "gate_failures": gate_failures,
        "targets": {},
        "total_failures": sum(1 for row in rows if not row.get("ok")),
        "replay_capture": bool(capture_replay),
        "replay_artifacts": len(replay_manifest),
        "common_success": _successful_attempt_cohort(rows, targets),
    }
    for target in targets:
        target_rows = [row for row in rows if row["target"] == target]
        successful_rows = [row for row in target_rows if row.get("ok")]
        value_counts = count_public_web_row_values(target_rows)
        summary["targets"][target] = {
            **_wild_web_target_metadata(target),
            "seeds": len(target_rows),
            "runs": runs,
            "passes": sum(1 for row in target_rows if row.get("ok")),
            "failures": sum(1 for row in target_rows if not row.get("ok")),
            "status_observed_attempts": sum(
                1 for row in target_rows if row.get("response_status") is not None
            ),
            "status_unobserved_attempts": sum(
                1 for row in target_rows if row.get("response_status") is None
            ),
            "response_status_sources": value_counts[
                "response_status_sources"
            ],
            "extraction_failures": sum(int(row.get("extraction_failure_count", 0) or 0) for row in target_rows),
            "categories": value_counts["categories"],
            "failure_kinds": value_counts["failure_kinds"],
            "classification_bases": value_counts["classification_bases"],
            "elapsed_ms": summarize(row["elapsed_ms"] for row in target_rows if row.get("elapsed_ms") is not None),
            "successful_elapsed_ms": summarize(
                row["elapsed_ms"]
                for row in successful_rows
                if row.get("elapsed_ms") is not None
            ),
            "peak_pss_bytes": summarize(row["peak_pss_bytes"] for row in target_rows if row.get("peak_pss_bytes") is not None),
            "successful_peak_pss_bytes": summarize(
                row["peak_pss_bytes"]
                for row in successful_rows
                if row.get("peak_pss_bytes") is not None
            ),
        }
    write_csv(suite_dir / "raw-runs.csv", rows)
    write_json(suite_dir / "runs.json", rows)
    write_json(suite_dir / "summary.json", summary)
    if capture_replay:
        write_json(
            suite_dir / "replay" / "manifest.json",
            {
                "schema_version": 1,
                "note": "Captured only when --capture-replay or --wild-web-capture-replay is explicitly provided. Caller is responsible for robots/ToS review before publishing.",
                "artifacts": replay_manifest,
            },
        )
    return summary
