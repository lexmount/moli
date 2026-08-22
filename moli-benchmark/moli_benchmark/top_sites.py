from __future__ import annotations

import csv
import os
import re
from itertools import combinations
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from .artifacts import write_csv, write_json
from .chrome_dcl import run_chrome_dcl_dump, run_served_cdp_dcl_dump
from .config import PROJECT_ROOT, REPO_ROOT, clear_proxy_env
from .process import ProcessResult, run_process
from .public_web import (
    PUBLIC_WEB_SNAPSHOT_CONTRACT,
    TOP_SITES_ARTIFACT_POLICY,
    TOP_SITES_CLASSIFIER,
    PublicWebAttempt,
    PublicWebResult,
    PublicWebScheduledCase,
    PublicWebScheduler,
    build_public_web_fetch_command,
    count_public_web_row_values,
    elapsed_failure_reached_timeout as _elapsed_failure_reached_timeout,
    public_web_target_metadata as _top_sites_target_metadata,
    rotated_target_order as _rotated_target_order,
    run_public_web_target,
    safe_artifact_filename_token as _artifact_filename_token,
    schedule_public_web_cases,
    successful_public_web_attempt_cohort,
    unavailable_public_web_row,
    write_public_web_failure_artifacts,
)
from .stats import summarize
from .synthetic_compare import WEBFETCH_TARGETS


TOP_SITES_FIXTURE_ROOT = PROJECT_ROOT / "fixtures" / "top-sites"
TOP_SITES_LIST_PATH = TOP_SITES_FIXTURE_ROOT / "chinese-community-top100-websites.csv"
GLOBAL_TOP_SITES_LIST_PATH = TOP_SITES_FIXTURE_ROOT / "global-top-websites-seed-list.csv"
WEBFETCH_LONGTAIL_LIST_PATH = TOP_SITES_FIXTURE_ROOT / "webfetch-longtail-seed-list.csv"
RENDER_QUALITY_LIST_PATH = TOP_SITES_FIXTURE_ROOT / "render-quality-seed-list.csv"
LEGACY_ENCODING_LIST_PATH = TOP_SITES_FIXTURE_ROOT / "legacy-encoding-websites-seed-list.csv"

TOP_SITES_SOURCES: dict[str, dict[str, Any]] = {
    "chinese-community": {
        "path": TOP_SITES_LIST_PATH,
        "label": "Chinese community top 100 (curated)",
    },
    "global": {
        "path": GLOBAL_TOP_SITES_LIST_PATH,
        "label": "Tranco-derived English-world top sites",
    },
    "webfetch-longtail": {
        "path": WEBFETCH_LONGTAIL_LIST_PATH,
        "label": "Observed WebFetch longtail URL failures",
    },
    "render-quality": {
        "path": RENDER_QUALITY_LIST_PATH,
        "label": "Curated article/document URLs for rendered-DOM quality checks",
    },
    "legacy-encoding": {
        "path": LEGACY_ENCODING_LIST_PATH,
        "label": "Curated non-UTF-8 public pages for document/script encoding checks",
    },
}

COMPOSITE_TOP_SITES_SOURCES = ("mixed", "webfetch-mix")

DEFAULT_TOP_SITES_SOURCE = "chinese-community"

TOP_SITES_PROFILES: dict[str, dict[str, Any]] = {
    "quick": {"limit": 20, "default_runs": 1},
    "full": {"limit": 100, "default_runs": 1},
    "webfetch": {"limit": 300, "default_runs": 1},
}

DEFAULT_TOP_SITES_PROFILE = "quick"
DEFAULT_TOP_SITES_MIN_BODY_BYTES = 256
DEFAULT_TOP_SITES_PARALLELISM_CAP = 8


def _default_top_sites_parallelism() -> int:
    if hasattr(os, "sched_getaffinity"):
        try:
            return min(DEFAULT_TOP_SITES_PARALLELISM_CAP, max(1, len(os.sched_getaffinity(0))))
        except OSError:
            pass
    return min(DEFAULT_TOP_SITES_PARALLELISM_CAP, max(1, os.cpu_count() or 1))


DEFAULT_TOP_SITES_PARALLELISM = _default_top_sites_parallelism()

_TOP_LIST_HEADING = re.compile(r"^##\s+Top\s+\d+\b", re.IGNORECASE)
_NEXT_SECTION_HEADING = re.compile(r"^##\s+")
_LIST_ENTRY = re.compile(r"^\s*(\d+)\.\s+`([^`]+)`")
_extract_title_and_text = TOP_SITES_CLASSIFIER.snapshot


def _parse_top_sites_sections(path: Path) -> list[tuple[str, list[tuple[int, str]]]]:
    if not path.exists():
        raise RuntimeError(f"top sites list not found: {path}")
    in_section = False
    current_heading = ""
    current_entries: list[tuple[int, str]] = []
    sections: list[tuple[str, list[tuple[int, str]]]] = []
    seen_domains: set[str] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.rstrip()
        if not in_section:
            if _TOP_LIST_HEADING.match(line):
                in_section = True
                current_heading = line
                current_entries = []
                seen_domains = set()
            continue
        if _NEXT_SECTION_HEADING.match(line):
            if current_entries:
                sections.append((current_heading, current_entries))
            top_heading = _TOP_LIST_HEADING.match(line)
            in_section = bool(top_heading)
            current_heading = line if top_heading else ""
            current_entries = []
            seen_domains = set()
            continue
        match = _LIST_ENTRY.match(line)
        if match:
            rank = int(match.group(1))
            domain = match.group(2).strip()
            if domain and domain not in seen_domains:
                seen_domains.add(domain)
                current_entries.append((rank, domain))
    if in_section and current_entries:
        sections.append((current_heading, current_entries))
    if not sections:
        raise RuntimeError(f"no Top entries parsed from {path}")
    return sections


def _parse_top_sites_csv(path: Path) -> list[tuple[int, str]]:
    if not path.exists():
        raise RuntimeError(f"top sites list not found: {path}")
    try:
        with path.open(encoding="utf-8-sig", newline="") as handle:
            reader = csv.DictReader(handle)
            if reader.fieldnames != ["rank", "target"]:
                raise RuntimeError(f"expected CSV columns `rank,target` in {path}")
            entries: list[tuple[int, str]] = []
            seen_targets: set[str] = set()
            for row_number, row in enumerate(reader, start=2):
                rank_raw = (row.get("rank") or "").strip()
                target = (row.get("target") or "").strip()
                try:
                    rank = int(rank_raw)
                except ValueError as error:
                    raise RuntimeError(f"invalid rank on CSV row {row_number} in {path}") from error
                if rank <= 0 or not target:
                    raise RuntimeError(f"invalid entry on CSV row {row_number} in {path}")
                if target not in seen_targets:
                    seen_targets.add(target)
                    entries.append((rank, target))
    except csv.Error as error:
        raise RuntimeError(f"invalid CSV seed list {path}: {error}") from error
    if not entries:
        raise RuntimeError(f"no entries parsed from {path}")
    return entries


def parse_top_sites_list(path: Path) -> list[tuple[int, str]]:
    if path.suffix.lower() == ".csv":
        return _parse_top_sites_csv(path)
    return _parse_top_sites_sections(path)[0][1]


def _parse_top_sites_list_by_count(path: Path, count: int) -> list[tuple[int, str]]:
    if path.suffix.lower() == ".csv":
        entries = _parse_top_sites_csv(path)
        if len(entries) != count:
            raise RuntimeError(f"expected {count} entries in {path}, found {len(entries)}")
        return entries
    heading = re.compile(rf"^##\s+Top\s+{count}\b", re.IGNORECASE)
    for section_heading, entries in _parse_top_sites_sections(path):
        if heading.match(section_heading):
            return entries
    raise RuntimeError(f"no Top {count} entries parsed from {path}")


def resolve_top_sites_source(source: str, list_path: Path | None) -> tuple[str, Path]:
    if list_path is not None:
        return ("custom", list_path)
    if source not in TOP_SITES_SOURCES:
        if source == "mixed":
            return (source, TOP_SITES_FIXTURE_ROOT / "mixed-top-websites")
        if source == "webfetch-mix":
            return (source, TOP_SITES_FIXTURE_ROOT / "webfetch-mix-websites")
        raise RuntimeError(
            f"unknown top-sites source `{source}`; expected one of "
            f"{sorted(TOP_SITES_SOURCES) + list(COMPOSITE_TOP_SITES_SOURCES)} or a --list-path override"
        )
    return (source, TOP_SITES_SOURCES[source]["path"])


def _interleave_entries(*entry_lists: list[tuple[int, str]]) -> list[tuple[int, str]]:
    interleaved: list[tuple[int, str]] = []
    seen: set[str] = set()
    max_len = max((len(entries) for entries in entry_lists), default=0)
    for index in range(max_len):
        for entries in entry_lists:
            if index >= len(entries):
                continue
            _, domain = entries[index]
            if domain in seen:
                continue
            seen.add(domain)
            interleaved.append((len(interleaved) + 1, domain))
    return interleaved


def _append_unique(
    base: list[tuple[int, str]],
    extra: list[tuple[int, str]],
    *,
    limit: int | None = None,
) -> list[tuple[int, str]]:
    combined: list[tuple[int, str]] = []
    seen: set[str] = set()
    for _, domain in [*base, *extra]:
        if domain in seen:
            continue
        seen.add(domain)
        combined.append((len(combined) + 1, domain))
        if limit is not None and len(combined) >= limit:
            break
    return combined


def load_top_sites_entries(source: str, list_path: Path | None) -> tuple[list[tuple[int, str]], list[str]]:
    """Return (entries, source_labels) for the requested source.

    `mixed` interleaves entries from chinese-community and global sources, taking
    one from each in rank order, to make a balanced cross-region list.

    `webfetch-mix` keeps that top-site coverage but caps it at 100 entries, then
    appends observed longtail URL paths from the WebFetch failure corpus.
    """
    if list_path is not None:
        return parse_top_sites_list(list_path), [f"custom:{list_path.name}"]
    if source == "mixed":
        cn = parse_top_sites_list(TOP_SITES_SOURCES["chinese-community"]["path"])
        gl = _parse_top_sites_list_by_count(GLOBAL_TOP_SITES_LIST_PATH, 100)
        interleaved = _interleave_entries(cn, gl)
        labels = [
            f"chinese-community:{TOP_SITES_SOURCES['chinese-community']['path'].name}",
            f"global:{GLOBAL_TOP_SITES_LIST_PATH.name}",
        ]
        return interleaved, labels
    if source == "webfetch-mix":
        cn = parse_top_sites_list(TOP_SITES_SOURCES["chinese-community"]["path"])
        gl = _parse_top_sites_list_by_count(GLOBAL_TOP_SITES_LIST_PATH, 100)
        top_site_mix = _interleave_entries(cn, gl)[:100]
        longtail = parse_top_sites_list(WEBFETCH_LONGTAIL_LIST_PATH)
        entries = _append_unique(top_site_mix, longtail)
        labels = [
            f"chinese-community:{TOP_SITES_SOURCES['chinese-community']['path'].name}",
            f"global:{GLOBAL_TOP_SITES_LIST_PATH.name}",
            f"webfetch-longtail:{WEBFETCH_LONGTAIL_LIST_PATH.name}",
        ]
        return entries, labels
    if source not in TOP_SITES_SOURCES:
        raise RuntimeError(
            f"unknown top-sites source `{source}`; expected one of "
            f"{sorted(TOP_SITES_SOURCES) + list(COMPOSITE_TOP_SITES_SOURCES)} or a --list-path override"
        )
    info = TOP_SITES_SOURCES[source]
    entries = (
        _parse_top_sites_list_by_count(info["path"], 100)
        if source == "global"
        else parse_top_sites_list(info["path"])
    )
    return entries, [f"{source}:{info['path'].name}"]


def _is_explicit_pdf_url(url: str) -> bool:
    return urlsplit(url).path.lower().endswith(".pdf")


def _top_command_for_target(
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
        suite_name="top-sites",
        omit_moli_page_wait=_is_explicit_pdf_url(url),
    )


def _run_top_sites_target(
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
        command_builder=_top_command_for_target,
        chrome_runner=run_chrome_dcl_dump,
        served_cdp_runner=run_served_cdp_dcl_dump,
        process_runner=run_process,
    )


def _classify(
    stdout: bytes,
    stderr: bytes,
    returncode: int | None,
    timed_out: bool,
    min_body_bytes: int,
    snapshot: dict[str, Any] | None = None,
    response_status: int | None = None,
    main_document_body_capture: str | None = None,
) -> str:
    return TOP_SITES_CLASSIFIER.classify_output(
        stdout=stdout,
        stderr=stderr,
        returncode=returncode,
        timed_out=timed_out,
        min_body_bytes=min_body_bytes,
        snapshot=snapshot,
        response_status=response_status,
        main_document_body_capture=main_document_body_capture,
    )


def _ok_categories() -> set[str]:
    return set(TOP_SITES_CLASSIFIER.ok_categories)


_SITE_UNREACHABLE_FAILURE_KINDS = {"network-error", "timeout"}


def _site_unreachable_exclusions(
    rows: list[dict[str, Any]],
    targets: tuple[str, ...],
) -> dict[str, str]:
    rows_by_domain: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        rows_by_domain.setdefault(str(row["domain"]), []).append(row)

    excluded: dict[str, str] = {}
    for domain, domain_rows in rows_by_domain.items():
        if any(row.get("ok") for row in domain_rows):
            continue
        if {str(row.get("target")) for row in domain_rows} != set(targets):
            continue
        failure_kinds = {
            str(
                row.get("failure_kind")
                or TOP_SITES_CLASSIFIER.failure_kind(
                    str(row.get("category"))
                )
                or ""
            )
            for row in domain_rows
        }
        failure_kinds.discard("")
        if (
            failure_kinds
            and failure_kinds <= _SITE_UNREACHABLE_FAILURE_KINDS
        ):
            excluded[domain] = "site-unreachable"
    return excluded


def _build_site_outcomes(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, int, str], list[dict[str, Any]]] = {}
    for row in rows:
        key = (str(row["target"]), int(row["rank"]), str(row["domain"]))
        grouped.setdefault(key, []).append(row)

    outcomes: list[dict[str, Any]] = []
    for (target, rank, domain), attempts in grouped.items():
        comparable_attempts = [
            row for row in attempts if row.get("comparable", True)
        ]
        passes = sum(1 for row in comparable_attempts if row.get("ok"))
        attempt_count = len(comparable_attempts)
        if attempt_count == 0:
            outcome = "not-comparable"
        elif passes == attempt_count:
            outcome = "all-pass"
        elif passes == 0:
            outcome = "all-fail"
        else:
            outcome = "flaky"
        categories: dict[str, int] = {}
        for row in attempts:
            category = str(row.get("category") or "unknown")
            categories[category] = categories.get(category, 0) + 1
        response_statuses: set[int] = set()
        response_status_sources: dict[str, int] = {}
        for row in attempts:
            status = row.get("response_status")
            if isinstance(status, int) and not isinstance(status, bool):
                response_statuses.add(status)
            source = row.get("response_status_source")
            if isinstance(source, str) and source:
                response_status_sources[source] = (
                    response_status_sources.get(source, 0) + 1
                )
        outcomes.append(
            {
                "target": target,
                "rank": rank,
                "domain": domain,
                "url": str(attempts[0].get("url") or domain),
                "attempts": attempt_count,
                "raw_attempts": len(attempts),
                "non_comparable_attempts": len(attempts) - attempt_count,
                "passes": passes,
                "failures": attempt_count - passes,
                "pass_rate_percent": (
                    (passes / attempt_count) * 100.0 if attempt_count else None
                ),
                "outcome": outcome,
                "categories": categories,
                "response_statuses": sorted(response_statuses),
                "response_status_sources": response_status_sources,
                "status_observed_attempts": sum(
                    1 for row in attempts if row.get("response_status") is not None
                ),
            }
        )
    outcomes.sort(
        key=lambda row: (
            str(row["target"]),
            int(row["rank"]),
            str(row["domain"]),
        )
    )
    return outcomes


def _pairwise_site_comparisons(
    outcomes: list[dict[str, Any]],
    targets: tuple[str, ...],
    *,
    runs: int,
) -> list[dict[str, Any]]:
    by_target_site = {
        (str(row["target"]), int(row["rank"]), str(row["domain"])): row
        for row in outcomes
    }
    site_keys = sorted({(int(row["rank"]), str(row["domain"])) for row in outcomes})
    comparisons: list[dict[str, Any]] = []
    for left, right in combinations(targets, 2):
        counts = {
            "both_all_pass": 0,
            "left_all_pass_right_all_fail": 0,
            "right_all_pass_left_all_fail": 0,
            "both_all_fail": 0,
            "flaky_or_incomplete": 0,
            "not_comparable": 0,
        }
        left_only_sites: list[dict[str, Any]] = []
        right_only_sites: list[dict[str, Any]] = []
        evaluated_sites = 0
        for rank, domain in site_keys:
            left_row = by_target_site.get((left, rank, domain))
            right_row = by_target_site.get((right, rank, domain))
            if left_row is None or right_row is None:
                continue
            left_outcome = left_row["outcome"]
            right_outcome = right_row["outcome"]
            if "not-comparable" in {left_outcome, right_outcome}:
                counts["not_comparable"] += 1
                continue
            evaluated_sites += 1
            if left_outcome == "all-pass" and right_outcome == "all-pass":
                counts["both_all_pass"] += 1
            elif left_outcome == "all-pass" and right_outcome == "all-fail":
                counts["left_all_pass_right_all_fail"] += 1
                left_only_sites.append({"rank": rank, "domain": domain})
            elif right_outcome == "all-pass" and left_outcome == "all-fail":
                counts["right_all_pass_left_all_fail"] += 1
                right_only_sites.append({"rank": rank, "domain": domain})
            elif left_outcome == "all-fail" and right_outcome == "all-fail":
                counts["both_all_fail"] += 1
            else:
                counts["flaky_or_incomplete"] += 1
        comparisons.append(
            {
                "left": left,
                "right": right,
                "runs_per_site": runs,
                "repeat_validated": runs >= 3,
                "evaluated_sites": evaluated_sites,
                **counts,
                "left_only_sites": left_only_sites,
                "right_only_sites": right_only_sites,
            }
        )
    return comparisons


def _successful_attempt_cohort(
    rows: list[dict[str, Any]],
    targets: tuple[str, ...],
) -> dict[str, Any]:
    return successful_public_web_attempt_cohort(
        rows,
        targets,
        attempt_key_fields=("run", "rank", "domain"),
        unique_key_fields=("rank", "domain"),
        unique_count_name="unique_sites",
        metric_fields=("elapsed_ms", "peak_pss_bytes", "peak_rss_bytes"),
        require_comparable=True,
    )


def _domain_to_url(domain: str) -> str:
    if domain.startswith("http://") or domain.startswith("https://"):
        return domain
    return f"https://{domain}"


def _execute_one(
    *,
    suite_dir: Path,
    target: str,
    metadata: dict[str, str],
    info: dict[str, Any],
    run_id: int,
    rank: int,
    domain: str,
    timeout_seconds: float,
    min_body_bytes: int,
    proc_env: dict[str, str],
    schedule_index: int,
    target_order_index: int,
) -> dict[str, Any]:
    url = _domain_to_url(domain)
    attempt = PublicWebAttempt.start(
        target=target,
        metadata=metadata,
        target_info=info,
        run=run_id,
        case_fields={"rank": rank, "domain": domain},
        url=url,
        schedule_index=schedule_index,
        target_order_index=target_order_index,
        artifact_stem=(
            f"{target}-run-{run_id}-rank{rank:03d}-"
            f"{_artifact_filename_token(domain)}"
        ),
    )
    binary = attempt.binary_path()
    if binary is None:
        row = unavailable_public_web_row(attempt, category="error")
        row["failure_artifact"] = write_public_web_failure_artifacts(
            suite_dir=suite_dir,
            attempt=attempt,
            row=row,
            stdout=b"",
            stderr=b"",
            policy=TOP_SITES_ARTIFACT_POLICY,
        )
        return row

    process = _run_top_sites_target(
        target=target,
        binary=binary,
        url=url,
        timeout_seconds=timeout_seconds,
        proc_env=proc_env,
    )
    result = PublicWebResult.capture(
        attempt,
        process,
        classifier=TOP_SITES_CLASSIFIER,
    )
    category = TOP_SITES_CLASSIFIER.classify_result(
        result,
        min_body_bytes=min_body_bytes,
    )
    if (
        result.response_status is None
        and not TOP_SITES_CLASSIFIER.classification_ok(category)
        and _elapsed_failure_reached_timeout(process.elapsed_ms, timeout_seconds)
    ):
        category = "timeout"
    comparable = TOP_SITES_CLASSIFIER.comparable(category)
    ok = TOP_SITES_CLASSIFIER.classification_ok(category)
    row: dict[str, Any] = {
        **result.base_row(),
        **result.evidence_fields(policy=TOP_SITES_ARTIFACT_POLICY),
        "command": process.command,
        "category": category,
        "classification_basis": TOP_SITES_CLASSIFIER.classification_basis(
            result, category
        ),
        "ok": ok,
        "comparable": comparable,
        "non_comparable_reason": (
            None if comparable else "main-document-body-not-captured"
        ),
        "failure_kind": (
            None
            if ok or not comparable
            else TOP_SITES_CLASSIFIER.failure_kind(category)
        ),
    }
    if comparable and not ok:
        row["failure_artifact"] = write_public_web_failure_artifacts(
            suite_dir=suite_dir,
            attempt=attempt,
            row=row,
            stdout=process.stdout,
            stderr=process.stderr,
            policy=TOP_SITES_ARTIFACT_POLICY,
        )
    return row


def run_top_sites_suite(
    *,
    output_dir: Path,
    target_matrix: dict[str, Any],
    targets: tuple[str, ...],
    profile: str = DEFAULT_TOP_SITES_PROFILE,
    list_path: Path | None = None,
    source: str = DEFAULT_TOP_SITES_SOURCE,
    runs: int | None = None,
    timeout_seconds: float = 15.0,
    gate_target: str = "moli",
    parallelism: int = DEFAULT_TOP_SITES_PARALLELISM,
    chrome_parallelism: int = 1,
    min_body_bytes: int = DEFAULT_TOP_SITES_MIN_BODY_BYTES,
    limit_override: int | None = None,
) -> dict[str, Any]:
    unknown_targets = [target for target in targets if target not in WEBFETCH_TARGETS]
    if unknown_targets:
        raise RuntimeError(f"unknown webfetch target(s): {', '.join(unknown_targets)}")
    if gate_target not in WEBFETCH_TARGETS:
        raise RuntimeError(f"unknown gate target: {gate_target}")
    if profile not in TOP_SITES_PROFILES:
        raise RuntimeError(f"unknown top-sites profile `{profile}`; expected one of {sorted(TOP_SITES_PROFILES)}")
    profile_config = TOP_SITES_PROFILES[profile]
    limit = int(limit_override if limit_override is not None else profile_config["limit"])
    if limit <= 0:
        raise RuntimeError("top-sites limit must be positive")
    runs_count = int(runs if runs is not None else profile_config["default_runs"])
    if runs_count <= 0:
        raise RuntimeError("top-sites runs must be positive")
    if parallelism <= 0:
        raise RuntimeError("top-sites parallelism must be positive")
    if chrome_parallelism <= 0:
        raise RuntimeError("top-sites chrome parallelism must be positive")

    suite_dir = output_dir / "top-sites"
    resolved_source, primary_path = resolve_top_sites_source(source, list_path)
    entries_all, list_source_labels = load_top_sites_entries(resolved_source, list_path)
    entries = entries_all[:limit]
    list_source = primary_path
    proc_env = clear_proxy_env(os.environ)

    metadata_by_target = {
        target: _top_sites_target_metadata(target) for target in targets
    }
    info_by_target = {
        target: target_matrix.get(metadata_by_target[target]["binary_key"], {})
        for target in targets
    }
    scheduled_cases = schedule_public_web_cases(entries, runs=runs_count)

    def execute_attempt(
        scheduled: PublicWebScheduledCase[tuple[int, str]],
        target: str,
        target_order_index: int,
    ) -> dict[str, Any]:
        rank, domain = scheduled.case
        return _execute_one(
            suite_dir=suite_dir,
            target=target,
            metadata=metadata_by_target[target],
            info=info_by_target[target],
            run_id=scheduled.run,
            rank=rank,
            domain=domain,
            timeout_seconds=timeout_seconds,
            min_body_bytes=min_body_bytes,
            proc_env=proc_env,
            schedule_index=scheduled.schedule_index,
            target_order_index=target_order_index,
        )

    rows = PublicWebScheduler[tuple[int, str], dict[str, Any]](
        targets,
        parallelism=parallelism,
        target_parallelism={"chrome": chrome_parallelism},
    ).run(
        scheduled_cases,
        execute_attempt,
    )

    excluded_domains = (
        _site_unreachable_exclusions(rows, targets) if len(targets) > 1 else {}
    )
    for row in rows:
        exclusion_reason = excluded_domains.get(str(row["domain"]))
        row["excluded"] = exclusion_reason is not None
        row["exclusion_reason"] = exclusion_reason

    counted_rows = [row for row in rows if not row.get("excluded")]
    comparable_counted_rows = [
        row for row in counted_rows if row.get("comparable", True)
    ]
    site_outcomes = _build_site_outcomes(counted_rows)
    pairwise_comparisons = _pairwise_site_comparisons(
        site_outcomes,
        targets,
        runs=runs_count,
    )
    for comparison in pairwise_comparisons:
        comparison["common_success"] = _successful_attempt_cohort(
            counted_rows,
            (str(comparison["left"]), str(comparison["right"])),
        )
    common_success = _successful_attempt_cohort(counted_rows, targets)
    gate_failures = sum(
        1
        for row in comparable_counted_rows
        if row["target"] == gate_target and not row.get("ok")
    )
    gate_site_outcomes = [
        row
        for row in site_outcomes
        if row["target"] == gate_target and row["outcome"] != "not-comparable"
    ]
    summary: dict[str, Any] = {
        "suite": "top-sites",
        "profile": profile,
        "limit": limit,
        "source": resolved_source,
        "list_source": (
            str(list_source.relative_to(REPO_ROOT))
            if list_source.is_relative_to(REPO_ROOT)
            else str(list_source)
        ),
        "list_sources": list_source_labels,
        "site_count": len(entries),
        "counted_site_count": len(entries) - len(excluded_domains),
        "excluded_site_count": len(excluded_domains),
        "excluded_sites": [
            {"domain": domain, "reason": reason}
            for domain, reason in sorted(
                excluded_domains.items(),
                key=lambda item: next((rank for rank, entry_domain in entries if entry_domain == item[0]), 0),
            )
        ],
        "runs": runs_count,
        "timeout_seconds": timeout_seconds,
        "min_body_bytes": min_body_bytes,
        "snapshot_contract": PUBLIC_WEB_SNAPSHOT_CONTRACT,
        "schedule": "site-paired-rotating-target-order",
        "scheduled_site_groups": len(scheduled_cases),
        "parallelism": parallelism,
        "chrome_parallelism": chrome_parallelism,
        "gate_target": gate_target,
        "gate_failures": gate_failures,
        "gate_site_failures": sum(
            1 for row in gate_site_outcomes if row["outcome"] != "all-pass"
        ),
        "gate_flaky_sites": sum(
            1 for row in gate_site_outcomes if row["outcome"] == "flaky"
        ),
        "total_failures": sum(
            1 for row in comparable_counted_rows if not row.get("ok")
        ),
        "total_non_comparable_runs": len(counted_rows)
        - len(comparable_counted_rows),
        "total_excluded_runs": sum(1 for row in rows if row.get("excluded")),
        "repeat_validated": runs_count >= 3,
        "pairwise": pairwise_comparisons,
        "common_success": common_success,
        "targets": {},
    }
    for target in targets:
        all_target_rows = [row for row in rows if row["target"] == target]
        target_observations = [
            row for row in all_target_rows if not row.get("excluded")
        ]
        target_rows = [
            row for row in target_observations if row.get("comparable", True)
        ]
        raw_comparable_rows = [
            row for row in all_target_rows if row.get("comparable", True)
        ]
        successful_rows = [row for row in target_rows if row.get("ok")]
        all_target_site_outcomes = [
            row for row in site_outcomes if row["target"] == target
        ]
        target_site_outcomes = [
            row
            for row in all_target_site_outcomes
            if row["outcome"] != "not-comparable"
        ]
        value_counts = count_public_web_row_values(target_observations)
        status_observed_attempts = sum(
            1
            for row in target_observations
            if row.get("response_status") is not None
        )
        summary["targets"][target] = {
            **_top_sites_target_metadata(target),
            "sites": len(target_rows),
            "raw_sites": len(all_target_rows),
            "observed_sites": len(target_observations),
            "unique_sites": len(target_site_outcomes),
            "observed_unique_sites": len(all_target_site_outcomes),
            "attempts": len(target_rows),
            "raw_observations": len(all_target_rows),
            "raw_comparable_attempts": len(raw_comparable_rows),
            "raw_non_comparable_attempts": len(all_target_rows)
            - len(raw_comparable_rows),
            "raw_passes": sum(1 for row in raw_comparable_rows if row.get("ok")),
            "raw_failures": sum(
                1 for row in raw_comparable_rows if not row.get("ok")
            ),
            "raw_pass_rate_percent": (
                (
                    sum(1 for row in raw_comparable_rows if row.get("ok"))
                    / len(raw_comparable_rows)
                )
                * 100.0
                if raw_comparable_rows
                else None
            ),
            "non_comparable_attempts": len(target_observations) - len(target_rows),
            "excluded_runs": len(all_target_rows) - len(target_observations),
            "runs": runs_count,
            "repeat_validated": runs_count >= 3,
            "passes": sum(1 for row in target_rows if row.get("ok")),
            "failures": sum(1 for row in target_rows if not row.get("ok")),
            "pass_rate_percent": (
                (sum(1 for row in target_rows if row.get("ok")) / len(target_rows))
                * 100.0
                if target_rows
                else None
            ),
            "all_pass_sites": sum(
                1 for row in target_site_outcomes if row["outcome"] == "all-pass"
            ),
            "all_fail_sites": sum(
                1 for row in target_site_outcomes if row["outcome"] == "all-fail"
            ),
            "flaky_sites": sum(
                1 for row in target_site_outcomes if row["outcome"] == "flaky"
            ),
            "status_observed_attempts": status_observed_attempts,
            "status_unobserved_attempts": len(target_observations)
            - status_observed_attempts,
            "response_status_sources": value_counts[
                "response_status_sources"
            ],
            "status_coverage_percent": (
                (status_observed_attempts / len(target_observations)) * 100.0
                if target_observations
                else None
            ),
            "content_inferred_http_failures": sum(
                1
                for row in target_rows
                if row.get("response_status") is None
                and row.get("returncode") == 0
                and row.get("category")
                in {"blocked-or-forbidden", "not-found", "http-error"}
            ),
            "categories": value_counts["categories"],
            "failure_kinds": value_counts["failure_kinds"],
            "classification_bases": value_counts["classification_bases"],
            "elapsed_ms": summarize(
                row["elapsed_ms"]
                for row in target_rows
                if row.get("elapsed_ms") is not None
            ),
            "successful_elapsed_ms": summarize(
                row["elapsed_ms"]
                for row in successful_rows
                if row.get("elapsed_ms") is not None
            ),
            "peak_pss_bytes": summarize(
                row["peak_pss_bytes"]
                for row in target_rows
                if row.get("peak_pss_bytes") is not None
            ),
            "successful_peak_pss_bytes": summarize(
                row["peak_pss_bytes"]
                for row in successful_rows
                if row.get("peak_pss_bytes") is not None
            ),
            "peak_rss_bytes": summarize(
                row["peak_rss_bytes"]
                for row in target_rows
                if row.get("peak_rss_bytes") is not None
            ),
            "successful_peak_rss_bytes": summarize(
                row["peak_rss_bytes"]
                for row in successful_rows
                if row.get("peak_rss_bytes") is not None
            ),
        }

    write_csv(suite_dir / "raw-runs.csv", rows)
    write_json(suite_dir / "runs.json", rows)
    write_json(suite_dir / "site-outcomes.json", site_outcomes)
    write_json(suite_dir / "summary.json", summary)
    return summary
