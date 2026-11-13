from __future__ import annotations

import argparse
import datetime as dt
import re
import sys
from pathlib import Path
from typing import Any

from .amiibo_crawler import (
    AMIIBO_CONCURRENCY_MATRIX,
    AMIIBO_MODES,
    AMIIBO_PROFILES,
    AMIIBO_SMOKE_CONCURRENCY_MATRIX,
    AMIIBO_SMOKE_LIMIT,
    DEFAULT_AMIIBO_RUNS,
    run_amiibo_crawler_suite,
)
from .agent_episode import (
    AGENT_EPISODE_TARGETS,
    DEFAULT_MANIFEST_PATH,
    run_agent_episode_suite,
)
from .artifacts import ensure_dir, write_csv, write_json, write_text
from .cdp_smoke import CDP_SMOKE_PROFILES, run_cdp_smoke_suite
from .config import FORMAL_RESULTS_ROOT, RESULTS_ROOT, moli_binary
from .crawler import run_crawler_suite
from .environment import collect_environment
from .html_report import write_benchmark_html
from .publish_readiness import build_publish_readiness
from .report_diff import build_report_diff, load_baseline_summary
from .startup import (
    FORMAL_STARTUP_IDLE_SECONDS,
    FORMAL_STARTUP_RUNS,
    FORMAL_STARTUP_WARM_PAGES,
    STARTUP_PROFILES,
    run_startup_suite,
)
from .synthetic import SYNTHETIC_CASES, run_synthetic_suite
from .synthetic_compare import CDP_TARGETS, FETCH_TARGETS, TARGETS, WEBFETCH_TARGETS, normalize_cdp_target, run_synthetic_compare_suite
from .synthetic_matrix import (
    DEFAULT_STABILITY_THRESHOLD_PERCENT,
    DEFAULT_SYNTHETIC_CONCURRENCY_MATRIX,
    FORMAL_SYNTHETIC_REPEATS,
    FORMAL_SYNTHETIC_RUNS,
    SYNTHETIC_MATRIX_PROFILES,
    run_synthetic_matrix_suite,
)
from .targets import collect_target_binaries
from .versions import collect_versions
from .render_compare import (
    DEFAULT_RENDER_COMPARE_BASELINE,
    DEFAULT_RENDER_COMPARE_KEY_HIT_THRESHOLD,
    DEFAULT_RENDER_COMPARE_MATCH_THRESHOLD,
    DEFAULT_RENDER_COMPARE_MIN_BASELINE_TEXT_CHARS,
    DEFAULT_RENDER_COMPARE_NGRAM_SIZE,
    DEFAULT_RENDER_COMPARE_PARTIAL_KEY_HIT_THRESHOLD,
    DEFAULT_RENDER_COMPARE_PARTIAL_THRESHOLD,
    run_render_compare_suite,
)
from .top_sites import (
    COMPOSITE_TOP_SITES_SOURCES,
    DEFAULT_TOP_SITES_SOURCE,
    DEFAULT_TOP_SITES_MIN_BODY_BYTES,
    DEFAULT_TOP_SITES_PARALLELISM,
    DEFAULT_TOP_SITES_PROFILE,
    TOP_SITES_PROFILES,
    TOP_SITES_SOURCES,
    run_top_sites_suite,
)
from .wild_web import WILD_WEB_SEEDS, run_wild_web_suite
from .wpt import run_wpt_suite


DEFAULT_RUNS = 5
DEFAULT_SYNTHETIC_MATRIX_REPEATS = 1
RUN_PROFILES = ("smoke", "horizontal")
RUN_PROFILE_SUITES = {
    "smoke": ("startup", "synthetic"),
    "horizontal": ("synthetic-compare", "cdp-session"),
}
RUN_PROFILE_DEFAULT_RUNS = {
    "horizontal": 10,
}


def _startup_exit_code(summary: dict[str, Any]) -> int:
    failure_key = "gate_failures" if summary.get("profile") == "formal" else "total_failures"
    return 1 if int(summary.get(failure_key, 0) or 0) else 0


def _default_output_dir() -> Path:
    timestamp = dt.datetime.now(dt.UTC).strftime("%Y-%m-%dT%H%M%SZ")
    return RESULTS_ROOT / timestamp


def _report_date_output_dir(value: str) -> Path:
    if re.fullmatch(r"\d{4}-\d{2}-\d{2}", value) is None:
        raise RuntimeError(f"invalid report date `{value}`; expected YYYY-MM-DD")
    try:
        report_date = dt.date.fromisoformat(value)
    except ValueError as error:
        raise RuntimeError(f"invalid report date `{value}`; expected YYYY-MM-DD") from error
    return FORMAL_RESULTS_ROOT / report_date.isoformat()


def _report_output_dir(args: argparse.Namespace) -> Path:
    report_date = getattr(args, "report_date", None)
    if report_date is not None:
        return _report_date_output_dir(str(report_date))
    return args.output_dir


def _add_output_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--output-dir", type=Path, default=_default_output_dir())
    parser.add_argument(
        "--report-date",
        type=str,
        default=None,
        help="write a formal report under benchmarks/results/YYYY-MM-DD",
    )


def _add_common_run_args(parser: argparse.ArgumentParser) -> None:
    _add_output_args(parser)
    parser.add_argument("--baseline-report", type=Path, default=None, help="compare this report with a previous summary.json or report directory")
    parser.add_argument("--moli-bin", type=str, default=None)
    parser.add_argument("--lightpanda-bin", type=str, default=None)
    parser.add_argument("--chrome-bin", type=str, default=None)
    parser.add_argument("--obscura-bin", type=str, default=None)
    parser.add_argument("--runs", type=int, default=None)
    parser.add_argument("--timeout", type=float, default=30.0)


def _write_metadata(
    output_dir: Path,
    moli_bin: Path | None,
    *,
    moli_override: str | None = None,
    lightpanda_override: str | None = None,
    chrome_override: str | None = None,
    obscura_override: str | None = None,
) -> None:
    target_matrix = collect_target_binaries(
        moli_override=str(moli_bin) if moli_bin is not None else moli_override,
        lightpanda_override=lightpanda_override,
        chrome_override=chrome_override,
        obscura_override=obscura_override,
    )
    write_json(output_dir / "environment.json", collect_environment())
    write_json(output_dir / "versions.json", collect_versions(moli_bin, target_matrix))


def _target_matrix(
    moli_bin: Path | None,
    *,
    moli_override: str | None = None,
    lightpanda_override: str | None = None,
    chrome_override: str | None = None,
    obscura_override: str | None = None,
) -> dict[str, Any]:
    return collect_target_binaries(
        moli_override=str(moli_bin) if moli_bin is not None else moli_override,
        lightpanda_override=lightpanda_override,
        chrome_override=chrome_override,
        obscura_override=obscura_override,
    )


def _summary_markdown(output_dir: Path, summaries: list[dict[str, Any]], report_diff: dict[str, Any] | None = None) -> str:
    lines = [
        "# moli benchmark summary",
        "",
        f"Output: `{output_dir}`",
        "",
        "| Suite | Failures | Cases |",
        "| --- | ---: | --- |",
    ]
    for summary in summaries:
        cases_value = summary.get("cases", {})
        if isinstance(cases_value, dict):
            cases = ", ".join(cases_value.keys())
        else:
            cases = str(cases_value)
        lines.append(f"| {summary.get('suite')} | {summary.get('total_failures', 0)} | {cases} |")
    lines.append("")
    lines.append("Detailed raw data is written beside this file as JSON and CSV.")
    if report_diff is not None:
        diff_summary = report_diff.get("summary", {})
        lines.extend(
            [
                "",
                "Previous report diff:",
                "",
                f"- Baseline: `{report_diff.get('baseline')}`",
                f"- added suites: `{diff_summary.get('added', 0)}`",
                f"- removed suites: `{diff_summary.get('removed', 0)}`",
                f"- changed suites: `{diff_summary.get('changed', 0)}`",
                f"- gate failures delta: `{diff_summary.get('gate_failures_delta', 0)}`",
                f"- total failures delta: `{diff_summary.get('total_failures_delta', 0)}`",
            ]
        )
    lines.append("")
    return "\n".join(lines)


def _finish_report(
    *,
    output_dir: Path,
    moli_bin: Path | None,
    target_matrix: dict[str, Any],
    summaries: list[dict[str, Any]],
    baseline_report: Path | None = None,
) -> None:
    versions = collect_versions(moli_bin, target_matrix)
    report_diff = None
    if baseline_report is not None:
        report_diff = build_report_diff(
            current_summaries=summaries,
            baseline_summary=load_baseline_summary(baseline_report),
            baseline_path=baseline_report,
        )
        write_json(output_dir / "report-diff.json", report_diff)
        write_csv(output_dir / "report-diff.csv", report_diff["suites"])
    write_json(
        output_dir / "summary.json",
        {
            "suites": summaries,
            "total_failures": sum(int(summary.get("total_failures", 0) or 0) for summary in summaries),
            "gate_failures": sum(int(summary.get("gate_failures", summary.get("total_failures", 0)) or 0) for summary in summaries),
        },
    )
    write_text(output_dir / "summary.md", _summary_markdown(output_dir, summaries, report_diff))
    write_json(output_dir / "publish-readiness.json", {"status": "pending", "checks": []})
    write_json(output_dir / "report-data.json", {"status": "pending"})
    write_text(output_dir / "index.html", "<!doctype html>\n<title>Moli Benchmark Report</title>\n")
    publish_readiness = build_publish_readiness(output_dir=output_dir, versions=versions, summaries=summaries)
    write_json(output_dir / "publish-readiness.json", publish_readiness)
    write_benchmark_html(
        output_dir=output_dir,
        versions=versions,
        summaries=summaries,
        publish_readiness=publish_readiness,
        report_diff=report_diff,
    )


def cmd_collect_env(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = None
    if args.moli_bin:
        moli_bin = moli_binary(args.moli_bin)
    _write_metadata(
        output_dir,
        moli_bin,
        moli_override=args.moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    print(output_dir)
    return 0


def cmd_startup(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_startup_suite(
        moli_bin=moli_bin,
        output_dir=output_dir,
        profile=_startup_profile(args),
        runs=_startup_runs(args),
        timeout_seconds=args.timeout,
        include_cdp_first_page=_startup_include_cdp_first_page(args),
        include_cdp_warm_pages=_startup_include_cdp_warm_pages(args),
        cdp_warm_pages=_startup_warm_pages(args),
        idle_seconds=_startup_idle_seconds(args),
        drop_os_cache=args.drop_os_cache,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return _startup_exit_code(summary)


def cmd_synthetic(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    cases = tuple(args.case or SYNTHETIC_CASES)
    summary = run_synthetic_suite(
        moli_bin=moli_bin,
        output_dir=output_dir,
        runs=_runs(args),
        timeout_seconds=args.timeout,
        cases=cases,
        concurrency=args.concurrency,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("total_failures") else 0


def _synthetic_matrix_concurrency(args: argparse.Namespace) -> tuple[int, ...]:
    values = args.matrix_concurrency or DEFAULT_SYNTHETIC_CONCURRENCY_MATRIX
    return tuple(dict.fromkeys(int(value) for value in values))


def _runs(args: argparse.Namespace) -> int:
    return DEFAULT_RUNS if args.runs is None else int(args.runs)


def _run_profile(args: argparse.Namespace) -> str:
    return str(getattr(args, "profile", "smoke"))


def _run_suites(args: argparse.Namespace) -> tuple[str, ...]:
    if args.suite is not None:
        return tuple(args.suite)
    return RUN_PROFILE_SUITES[_run_profile(args)]


def _run_runs(args: argparse.Namespace) -> int:
    if args.runs is None:
        return RUN_PROFILE_DEFAULT_RUNS.get(_run_profile(args), DEFAULT_RUNS)
    return int(args.runs)


def _selected_fetch_targets(args: argparse.Namespace) -> tuple[str, ...]:
    if args.target is None:
        return FETCH_TARGETS
    targets = tuple(dict.fromkeys(target for target in args.target if target in FETCH_TARGETS))
    if not targets:
        raise RuntimeError("no fetch targets selected; use moli, moli-full, lightpanda, chrome, or obscura")
    return targets


def _selected_webfetch_targets(args: argparse.Namespace) -> tuple[str, ...]:
    if args.target is None:
        return WEBFETCH_TARGETS
    targets = tuple(dict.fromkeys(target for target in args.target if target in WEBFETCH_TARGETS))
    if not targets:
        raise RuntimeError(
            "no webfetch targets selected; use moli, moli-cdp, moli-full, "
            "moli-full-cdp, lightpanda, lightpanda-cdp, chrome, obscura, or obscura-cdp"
        )
    return targets


def _selected_cdp_targets(args: argparse.Namespace) -> tuple[str, ...]:
    if args.target is None:
        return CDP_TARGETS
    return tuple(dict.fromkeys(normalize_cdp_target(target) for target in args.target))


def _selected_agent_episode_targets(args: argparse.Namespace) -> tuple[str, ...]:
    if args.target is None:
        return AGENT_EPISODE_TARGETS
    return tuple(dict.fromkeys(str(target) for target in args.target))


def _startup_profile(args: argparse.Namespace) -> str:
    return str(getattr(args, "startup_profile", None) or getattr(args, "profile", "smoke"))


def _startup_runs(args: argparse.Namespace) -> int:
    if _startup_profile(args) == "formal" and args.runs is None:
        return FORMAL_STARTUP_RUNS
    return _runs(args)


def _startup_include_cdp_first_page(args: argparse.Namespace) -> bool:
    return _startup_profile(args) == "formal" or bool(args.include_cdp_first_page)


def _startup_include_cdp_warm_pages(args: argparse.Namespace) -> bool:
    return _startup_profile(args) == "formal" or bool(args.include_cdp_warm_pages)


def _startup_warm_pages(args: argparse.Namespace) -> int:
    if _startup_profile(args) == "formal" and args.cdp_warm_pages == FORMAL_STARTUP_WARM_PAGES:
        return FORMAL_STARTUP_WARM_PAGES
    return int(args.cdp_warm_pages)


def _startup_idle_seconds(args: argparse.Namespace) -> tuple[float, ...]:
    if args.idle_seconds is None:
        return FORMAL_STARTUP_IDLE_SECONDS if _startup_profile(args) == "formal" else ()
    return tuple(dict.fromkeys(float(value) for value in args.idle_seconds))


def _synthetic_matrix_profile(args: argparse.Namespace) -> str:
    return str(getattr(args, "synthetic_matrix_profile", None) or getattr(args, "profile", "smoke"))


def _synthetic_matrix_runs(args: argparse.Namespace) -> int:
    profile = _synthetic_matrix_profile(args)
    if profile == "formal" and args.runs is None:
        return FORMAL_SYNTHETIC_RUNS
    return _runs(args)


def _synthetic_matrix_repeats(args: argparse.Namespace) -> int:
    profile = _synthetic_matrix_profile(args)
    if profile == "formal" and args.matrix_repeats is None:
        return FORMAL_SYNTHETIC_REPEATS
    return DEFAULT_SYNTHETIC_MATRIX_REPEATS if args.matrix_repeats is None else int(args.matrix_repeats)


def _amiibo_pools(args: argparse.Namespace) -> tuple[int, ...]:
    values = args.pool
    if values is None:
        values = AMIIBO_CONCURRENCY_MATRIX if _amiibo_profile(args) == "formal" else AMIIBO_SMOKE_CONCURRENCY_MATRIX
    return tuple(dict.fromkeys(int(value) for value in values))


def _amiibo_profile(args: argparse.Namespace) -> str:
    return str(getattr(args, "amiibo_profile", None) or "smoke")


def _amiibo_modes(args: argparse.Namespace) -> tuple[str, ...]:
    values = args.amiibo_mode
    if values is None:
        values = AMIIBO_MODES if _amiibo_profile(args) == "formal" else ("session",)
    return tuple(dict.fromkeys(str(value) for value in values))


def _amiibo_limit(args: argparse.Namespace) -> int:
    value = args.limit
    if value is None:
        return 0 if _amiibo_profile(args) == "formal" else AMIIBO_SMOKE_LIMIT
    return int(value)


def _amiibo_runs(args: argparse.Namespace) -> int:
    return DEFAULT_AMIIBO_RUNS if args.runs is None else int(args.runs)


def cmd_run(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)

    suites = _run_suites(args)
    summaries: list[dict[str, Any]] = []
    exit_code = 0
    if "startup" in suites:
        summary = run_startup_suite(
            moli_bin=moli_bin,
            output_dir=output_dir,
            profile=_startup_profile(args),
            runs=_startup_runs(args),
            timeout_seconds=args.timeout,
            include_cdp_first_page=_startup_include_cdp_first_page(args),
            include_cdp_warm_pages=_startup_include_cdp_warm_pages(args),
            cdp_warm_pages=_startup_warm_pages(args),
            idle_seconds=_startup_idle_seconds(args),
            drop_os_cache=args.drop_os_cache,
        )
        summaries.append(summary)
        exit_code = exit_code or _startup_exit_code(summary)
    if "synthetic" in suites:
        summary = run_synthetic_suite(
            moli_bin=moli_bin,
            output_dir=output_dir,
            runs=_run_runs(args),
            timeout_seconds=args.timeout,
            cases=tuple(args.case or SYNTHETIC_CASES),
            concurrency=args.concurrency,
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("total_failures") else 0)
    if "synthetic-matrix" in suites:
        summary = run_synthetic_matrix_suite(
            moli_bin=moli_bin,
            output_dir=output_dir,
            profile=_synthetic_matrix_profile(args),
            runs=_synthetic_matrix_runs(args),
            timeout_seconds=args.timeout,
            cases=tuple(args.case or SYNTHETIC_CASES),
            concurrency_levels=_synthetic_matrix_concurrency(args),
            repeats=_synthetic_matrix_repeats(args),
            stability_threshold_percent=args.stability_threshold_percent,
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("gate_failures") else 0)
    if "cdp-smoke" in suites:
        summary = run_cdp_smoke_suite(
            output_dir=output_dir,
            moli_bin=moli_bin,
            timeout_seconds=args.timeout,
            groups=tuple(args.cdp_group or ()),
            profile=args.cdp_profile,
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("gate_failures") else 0)
    if "wpt" in suites:
        summary = run_wpt_suite(
            output_dir=output_dir,
            timeout_seconds=args.timeout,
            runner=args.wpt_runner,
            compat=args.wpt_compat,
            case_filter=args.wpt_case,
            tag_filter=args.wpt_tag,
            no_run=args.wpt_no_run,
            baseline=args.wpt_baseline,
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("total_failures") else 0)
    if "cdp-session" in suites:
        from .cdp_session import run_cdp_session_suite

        summary = run_cdp_session_suite(
            output_dir=output_dir,
            target_matrix=target_matrix,
            targets=_selected_cdp_targets(args),
            cases=tuple(args.case or SYNTHETIC_CASES),
            runs=_run_runs(args),
            timeout_seconds=args.timeout,
            gate_target=normalize_cdp_target(args.gate_target),
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("gate_failures") else 0)
    if "synthetic-compare" in suites:
        summary = run_synthetic_compare_suite(
            output_dir=output_dir,
            target_matrix=target_matrix,
            targets=_selected_fetch_targets(args),
            runs=_run_runs(args),
            timeout_seconds=args.timeout,
            cases=tuple(args.case or SYNTHETIC_CASES),
            concurrency=args.concurrency,
            gate_target=args.gate_target,
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("gate_failures") else 0)
    if "crawler" in suites:
        summary = run_crawler_suite(
            output_dir=output_dir,
            target_matrix=target_matrix,
            targets=_selected_fetch_targets(args),
            pages=args.pages,
            runs=_run_runs(args),
            timeout_seconds=args.timeout,
            gate_target=args.gate_target,
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("gate_failures") else 0)
    if "amiibo-crawler" in suites:
        summary = run_amiibo_crawler_suite(
            output_dir=output_dir,
            target_matrix=target_matrix,
            profile=_amiibo_profile(args),
            targets=_selected_cdp_targets(args),
            pools=_amiibo_pools(args),
            modes=_amiibo_modes(args),
            runs=_amiibo_runs(args),
            limit=_amiibo_limit(args),
            timeout_seconds=args.timeout,
            gate_target=normalize_cdp_target(args.gate_target),
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("gate_failures") else 0)
    if "wild-web" in suites:
        summary = run_wild_web_suite(
            output_dir=output_dir,
            target_matrix=target_matrix,
            targets=_selected_webfetch_targets(args),
            seeds=tuple(args.seed or ()),
            runs=_run_runs(args),
            timeout_seconds=args.timeout,
            gate_target=args.gate_target,
            capture_replay=args.wild_web_capture_replay,
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("gate_failures") else 0)
    if "top-sites" in suites:
        summary = run_top_sites_suite(
            output_dir=output_dir,
            target_matrix=target_matrix,
            targets=_selected_webfetch_targets(args),
            profile=args.top_sites_profile,
            source=args.top_sites_source,
            list_path=args.top_sites_list_path,
            runs=_run_runs(args),
            timeout_seconds=args.timeout,
            gate_target=args.gate_target,
            parallelism=args.top_sites_parallelism,
            chrome_parallelism=args.top_sites_chrome_parallelism,
            min_body_bytes=args.top_sites_min_body_bytes,
            limit_override=args.top_sites_limit,
        )
        summaries.append(summary)
        exit_code = exit_code or (1 if summary.get("gate_failures") else 0)

    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=summaries, baseline_report=args.baseline_report)
    print(output_dir)
    return exit_code


def cmd_synthetic_compare(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_synthetic_compare_suite(
        output_dir=output_dir,
        target_matrix=target_matrix,
        targets=_selected_fetch_targets(args),
        runs=_runs(args),
        timeout_seconds=args.timeout,
        cases=tuple(args.case or SYNTHETIC_CASES),
        concurrency=args.concurrency,
        gate_target=args.gate_target,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_synthetic_matrix(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_synthetic_matrix_suite(
        moli_bin=moli_bin,
        output_dir=output_dir,
        profile=_synthetic_matrix_profile(args),
        runs=_synthetic_matrix_runs(args),
        timeout_seconds=args.timeout,
        cases=tuple(args.case or SYNTHETIC_CASES),
        concurrency_levels=_synthetic_matrix_concurrency(args),
        repeats=_synthetic_matrix_repeats(args),
        stability_threshold_percent=args.stability_threshold_percent,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_cdp_session(args: argparse.Namespace) -> int:
    from .cdp_session import run_cdp_session_suite

    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_cdp_session_suite(
        output_dir=output_dir,
        target_matrix=target_matrix,
        targets=_selected_cdp_targets(args),
        cases=tuple(args.case or SYNTHETIC_CASES),
        runs=_runs(args),
        timeout_seconds=args.timeout,
        gate_target=normalize_cdp_target(args.gate_target),
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_agent_episode(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    targets = _selected_agent_episode_targets(args)
    moli_bin = (
        moli_binary(args.moli_bin)
        if "moli-cdp" in targets or args.moli_bin
        else None
    )
    target_matrix = _target_matrix(
        moli_bin,
        chrome_override=args.chrome_bin,
    )
    _write_metadata(
        output_dir,
        moli_bin,
        chrome_override=args.chrome_bin,
    )
    summary = run_agent_episode_suite(
        output_dir=output_dir,
        target_matrix=target_matrix,
        targets=targets,
        runs=1 if args.runs is None else int(args.runs),
        workers=args.workers,
        parallelism=args.parallelism,
        step_dwell_ms=args.step_dwell_ms,
        sample_interval_ms=args.sample_interval_ms,
        timeout_seconds=args.timeout,
        manifest_path=args.manifest,
    )
    _finish_report(
        output_dir=output_dir,
        moli_bin=moli_bin,
        target_matrix=target_matrix,
        summaries=[summary],
        baseline_report=args.baseline_report,
    )
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_crawler(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_crawler_suite(
        output_dir=output_dir,
        target_matrix=target_matrix,
        targets=_selected_fetch_targets(args),
        pages=args.pages,
        runs=_runs(args),
        timeout_seconds=args.timeout,
        gate_target=args.gate_target,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_amiibo_crawler(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_amiibo_crawler_suite(
        output_dir=output_dir,
        target_matrix=target_matrix,
        profile=_amiibo_profile(args),
        targets=_selected_cdp_targets(args),
        pools=_amiibo_pools(args),
        modes=_amiibo_modes(args),
        runs=_amiibo_runs(args),
        limit=_amiibo_limit(args),
        timeout_seconds=args.timeout,
        gate_target=normalize_cdp_target(args.gate_target),
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_wild_web(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_wild_web_suite(
        output_dir=output_dir,
        target_matrix=target_matrix,
        targets=_selected_webfetch_targets(args),
        seeds=tuple(args.seed or ()),
        runs=_runs(args),
        timeout_seconds=args.timeout,
        gate_target=args.gate_target,
        capture_replay=args.capture_replay,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_top_sites(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_top_sites_suite(
        output_dir=output_dir,
        target_matrix=target_matrix,
        targets=_selected_webfetch_targets(args),
        profile=args.profile,
        source=args.source,
        list_path=args.list_path,
        runs=_runs(args),
        timeout_seconds=args.timeout,
        gate_target=args.gate_target,
        parallelism=args.parallelism,
        chrome_parallelism=args.chrome_parallelism,
        min_body_bytes=args.min_body_bytes,
        limit_override=args.limit,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_render_compare(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_render_compare_suite(
        output_dir=output_dir,
        target_matrix=target_matrix,
        targets=_selected_webfetch_targets(args) if args.target is not None else ("moli", "moli-cdp", "lightpanda", "lightpanda-cdp"),
        baseline_target=args.baseline_target,
        profile=args.profile,
        source=args.source,
        list_path=args.list_path,
        timeout_seconds=args.timeout,
        gate_target=args.gate_target,
        parallelism=args.parallelism,
        min_body_bytes=args.min_body_bytes,
        limit_override=args.limit,
        ngram_size=args.ngram_size,
        match_threshold=args.match_threshold,
        partial_threshold=args.partial_threshold,
        key_hit_threshold=args.key_hit_threshold,
        partial_key_hit_threshold=args.partial_key_hit_threshold,
        min_baseline_text_chars=args.min_baseline_text_chars,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_cdp_smoke(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(output_dir, moli_bin, lightpanda_override=args.lightpanda_bin, chrome_override=args.chrome_bin, obscura_override=args.obscura_bin)
    summary = run_cdp_smoke_suite(
        output_dir=output_dir,
        moli_bin=moli_bin,
        timeout_seconds=args.timeout,
        groups=tuple(args.group or ()),
        profile=args.profile,
        command=tuple(args.command) if args.command else None,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("gate_failures") else 0


def cmd_wpt(args: argparse.Namespace) -> int:
    output_dir = ensure_dir(_report_output_dir(args))
    moli_bin = None
    if args.moli_bin:
        moli_bin = moli_binary(args.moli_bin)
    target_matrix = _target_matrix(
        moli_bin,
        moli_override=args.moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    _write_metadata(
        output_dir,
        moli_bin,
        moli_override=args.moli_bin,
        lightpanda_override=args.lightpanda_bin,
        chrome_override=args.chrome_bin,
        obscura_override=args.obscura_bin,
    )
    summary = run_wpt_suite(
        output_dir=output_dir,
        timeout_seconds=args.timeout,
        runner=args.runner,
        compat=args.compat,
        case_filter=args.case,
        tag_filter=args.tag,
        no_run=args.no_run,
        baseline=args.baseline,
    )
    _finish_report(output_dir=output_dir, moli_bin=moli_bin, target_matrix=target_matrix, summaries=[summary], baseline_report=args.baseline_report)
    print(output_dir)
    return 1 if summary.get("total_failures") else 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="moli-benchmark")
    subparsers = parser.add_subparsers(dest="command", required=True)

    from .fingerprint.cli import add_arguments as add_fingerprint_arguments, command as fingerprint_command

    fingerprint = subparsers.add_parser("fingerprint", help="observe twelve public fingerprint sites over native CDP")
    add_fingerprint_arguments(fingerprint)
    fingerprint.set_defaults(func=fingerprint_command)

    collect_env = subparsers.add_parser("collect-env", help="write environment.json and versions.json")
    _add_output_args(collect_env)
    collect_env.add_argument("--moli-bin", type=str, default=None)
    collect_env.add_argument("--lightpanda-bin", type=str, default=None)
    collect_env.add_argument("--chrome-bin", type=str, default=None)
    collect_env.add_argument("--obscura-bin", type=str, default=None)
    collect_env.set_defaults(func=cmd_collect_env)

    startup = subparsers.add_parser("startup", help="run startup/deploy-size benchmark subset")
    _add_common_run_args(startup)
    startup.add_argument("--profile", choices=STARTUP_PROFILES, default="smoke")
    startup.add_argument("--include-cdp-first-page", action="store_true")
    startup.add_argument("--include-cdp-warm-pages", action="store_true")
    startup.add_argument("--cdp-warm-pages", type=int, default=10)
    startup.add_argument("--idle-seconds", action="append", type=float)
    startup.add_argument("--drop-os-cache", action="store_true")
    startup.set_defaults(func=cmd_startup)

    synthetic = subparsers.add_parser("synthetic", help="run local synthetic fixture benchmark subset")
    _add_common_run_args(synthetic)
    synthetic.add_argument("--case", action="append", choices=SYNTHETIC_CASES)
    synthetic.add_argument("--concurrency", type=int, default=1)
    synthetic.set_defaults(func=cmd_synthetic)

    synthetic_matrix = subparsers.add_parser("synthetic-matrix", help="run synthetic fixtures across a concurrency and repeat matrix")
    _add_common_run_args(synthetic_matrix)
    synthetic_matrix.add_argument("--case", action="append", choices=SYNTHETIC_CASES)
    synthetic_matrix.add_argument("--profile", choices=SYNTHETIC_MATRIX_PROFILES, default="smoke")
    synthetic_matrix.add_argument("--matrix-concurrency", action="append", type=int)
    synthetic_matrix.add_argument("--matrix-repeats", type=int, default=None)
    synthetic_matrix.add_argument("--stability-threshold-percent", type=float, default=DEFAULT_STABILITY_THRESHOLD_PERCENT)
    synthetic_matrix.set_defaults(func=cmd_synthetic_matrix)

    synthetic_compare = subparsers.add_parser(
        "synthetic-compare",
        help="run synthetic fixtures across moli variants, lightpanda, chrome, and obscura",
    )
    _add_common_run_args(synthetic_compare)
    synthetic_compare.add_argument("--case", action="append", choices=SYNTHETIC_CASES)
    synthetic_compare.add_argument("--target", action="append", choices=FETCH_TARGETS)
    synthetic_compare.add_argument("--concurrency", type=int, default=1)
    synthetic_compare.add_argument("--gate-target", choices=FETCH_TARGETS, default="moli")
    synthetic_compare.set_defaults(func=cmd_synthetic_compare)

    cdp_session = subparsers.add_parser("cdp-session", help="run long-lived CDP session navigation benchmark")
    _add_common_run_args(cdp_session)
    cdp_session.add_argument("--case", action="append", choices=SYNTHETIC_CASES)
    cdp_session.add_argument("--target", action="append", choices=TARGETS)
    cdp_session.add_argument("--gate-target", choices=TARGETS, default="moli")
    cdp_session.set_defaults(func=cmd_cdp_session)

    agent_episode = subparsers.add_parser(
        "agent-episode",
        help="run deterministic RL-shaped CDP episodes against Moli and Chromium",
    )
    _add_output_args(agent_episode)
    agent_episode.add_argument("--baseline-report", type=Path, default=None)
    agent_episode.add_argument("--moli-bin", type=str, default=None)
    agent_episode.add_argument("--chrome-bin", type=str, default=None)
    agent_episode.add_argument("--runs", type=int, default=1)
    agent_episode.add_argument("--timeout", type=float, default=30.0)
    agent_episode.add_argument(
        "--target",
        action="append",
        choices=AGENT_EPISODE_TARGETS,
        help="repeat to select targets; defaults to Moli and Chromium",
    )
    agent_episode.add_argument("--workers", type=int, default=1)
    agent_episode.add_argument("--parallelism", type=int, default=1)
    agent_episode.add_argument("--step-dwell-ms", type=int, default=14_000)
    agent_episode.add_argument("--sample-interval-ms", type=int, default=500)
    agent_episode.add_argument(
        "--manifest",
        type=Path,
        default=DEFAULT_MANIFEST_PATH,
        help="versioned local episode manifest",
    )
    agent_episode.set_defaults(func=cmd_agent_episode)

    crawler = subparsers.add_parser("crawler", help="run local multi-page crawler benchmark")
    _add_common_run_args(crawler)
    crawler.add_argument("--target", action="append", choices=FETCH_TARGETS)
    crawler.add_argument("--pages", type=int, default=50)
    crawler.add_argument("--gate-target", choices=FETCH_TARGETS, default="moli")
    crawler.set_defaults(func=cmd_crawler)

    amiibo_crawler = subparsers.add_parser("amiibo-crawler", help="run the Python raw-CDP Amiibo crawler benchmark")
    _add_common_run_args(amiibo_crawler)
    amiibo_crawler.add_argument("--target", action="append", choices=TARGETS)
    amiibo_crawler.add_argument("--amiibo-profile", choices=AMIIBO_PROFILES, default="smoke")
    amiibo_crawler.add_argument("--pool", action="append", type=int)
    amiibo_crawler.add_argument("--amiibo-mode", action="append", choices=AMIIBO_MODES)
    amiibo_crawler.add_argument("--limit", type=int, default=None)
    amiibo_crawler.add_argument("--gate-target", choices=TARGETS, default="moli")
    amiibo_crawler.set_defaults(func=cmd_amiibo_crawler)

    wild_web = subparsers.add_parser("wild-web", help="run real-site seed classification benchmark")
    _add_common_run_args(wild_web)
    wild_web.add_argument("--target", action="append", choices=WEBFETCH_TARGETS)
    wild_web.add_argument("--seed", action="append", choices=tuple(WILD_WEB_SEEDS.keys()))
    wild_web.add_argument("--gate-target", choices=WEBFETCH_TARGETS, default="moli")
    wild_web.add_argument("--capture-replay", action="store_true", help="capture successful wild-web HTML snapshots under wild-web/replay")
    wild_web.set_defaults(func=cmd_wild_web)

    top_sites = subparsers.add_parser(
        "top-sites",
        help="run the Chinese community top-100 public-web benchmark across CLI fetch and CDP-DCL targets",
    )
    _add_common_run_args(top_sites)
    top_sites.add_argument("--target", action="append", choices=WEBFETCH_TARGETS)
    top_sites.add_argument("--profile", choices=tuple(TOP_SITES_PROFILES.keys()), default=DEFAULT_TOP_SITES_PROFILE)
    top_sites.add_argument(
        "--source",
        choices=tuple(TOP_SITES_SOURCES.keys()) + COMPOSITE_TOP_SITES_SOURCES,
        default=DEFAULT_TOP_SITES_SOURCE,
        help="seed list source: chinese-community (default), global (Tranco), mixed, webfetch-longtail, webfetch-mix, render-quality, or legacy-encoding",
    )
    top_sites.add_argument("--list-path", type=Path, default=None, help="override seed list path (advanced)")
    top_sites.add_argument("--limit", type=int, default=None, help="override the top-N count picked by --profile")
    top_sites.add_argument("--gate-target", choices=WEBFETCH_TARGETS, default="moli")
    top_sites.add_argument("--parallelism", type=int, default=DEFAULT_TOP_SITES_PARALLELISM)
    top_sites.add_argument("--chrome-parallelism", type=int, default=1, help="concurrency for the chrome CDP-DCL target only")
    top_sites.add_argument("--min-body-bytes", type=int, default=DEFAULT_TOP_SITES_MIN_BODY_BYTES)
    top_sites.set_defaults(func=cmd_top_sites)

    render_compare = subparsers.add_parser(
        "render-compare",
        help="compare rendered fetch output against a baseline target using visible-text similarity",
    )
    _add_common_run_args(render_compare)
    render_compare.add_argument("--target", action="append", choices=WEBFETCH_TARGETS)
    render_compare.add_argument("--baseline-target", choices=WEBFETCH_TARGETS, default=DEFAULT_RENDER_COMPARE_BASELINE)
    render_compare.add_argument("--profile", choices=tuple(TOP_SITES_PROFILES.keys()), default=DEFAULT_TOP_SITES_PROFILE)
    render_compare.add_argument(
        "--source",
        choices=tuple(TOP_SITES_SOURCES.keys()) + COMPOSITE_TOP_SITES_SOURCES,
        default=DEFAULT_TOP_SITES_SOURCE,
        help="seed list source: chinese-community (default), global (Tranco), mixed, webfetch-longtail, webfetch-mix, render-quality, or legacy-encoding",
    )
    render_compare.add_argument("--list-path", type=Path, default=None, help="override seed list path (advanced)")
    render_compare.add_argument("--limit", type=int, default=None, help="override the top-N count picked by --profile")
    render_compare.add_argument("--gate-target", choices=WEBFETCH_TARGETS, default="moli")
    render_compare.add_argument("--parallelism", type=int, default=DEFAULT_TOP_SITES_PARALLELISM)
    render_compare.add_argument("--min-body-bytes", type=int, default=DEFAULT_TOP_SITES_MIN_BODY_BYTES)
    render_compare.add_argument("--ngram-size", type=int, default=DEFAULT_RENDER_COMPARE_NGRAM_SIZE)
    render_compare.add_argument("--match-threshold", type=float, default=DEFAULT_RENDER_COMPARE_MATCH_THRESHOLD)
    render_compare.add_argument("--partial-threshold", type=float, default=DEFAULT_RENDER_COMPARE_PARTIAL_THRESHOLD)
    render_compare.add_argument("--key-hit-threshold", type=float, default=DEFAULT_RENDER_COMPARE_KEY_HIT_THRESHOLD)
    render_compare.add_argument("--partial-key-hit-threshold", type=float, default=DEFAULT_RENDER_COMPARE_PARTIAL_KEY_HIT_THRESHOLD)
    render_compare.add_argument("--min-baseline-text-chars", type=int, default=DEFAULT_RENDER_COMPARE_MIN_BASELINE_TEXT_CHARS)
    render_compare.set_defaults(func=cmd_render_compare)

    cdp_smoke = subparsers.add_parser("cdp-smoke", help="run and archive moli-cdp-smoke")
    _add_output_args(cdp_smoke)
    cdp_smoke.add_argument("--baseline-report", type=Path, default=None, help="compare this report with a previous summary.json or report directory")
    cdp_smoke.add_argument("--moli-bin", type=str, default=None)
    cdp_smoke.add_argument("--lightpanda-bin", type=str, default=None)
    cdp_smoke.add_argument("--chrome-bin", type=str, default=None)
    cdp_smoke.add_argument("--obscura-bin", type=str, default=None)
    cdp_smoke.add_argument("--timeout", type=float, default=30.0)
    cdp_smoke.add_argument("--profile", choices=CDP_SMOKE_PROFILES, default="smoke")
    cdp_smoke.add_argument("--group", action="append")
    cdp_smoke.add_argument("--command", nargs=argparse.REMAINDER)
    cdp_smoke.set_defaults(func=cmd_cdp_smoke)

    wpt = subparsers.add_parser("wpt", help="run or collect WPT compat reports")
    _add_output_args(wpt)
    wpt.add_argument("--baseline-report", type=Path, default=None, help="compare this report with a previous summary.json or report directory")
    wpt.add_argument("--moli-bin", type=str, default=None)
    wpt.add_argument("--lightpanda-bin", type=str, default=None)
    wpt.add_argument("--chrome-bin", type=str, default=None)
    wpt.add_argument("--obscura-bin", type=str, default=None)
    wpt.add_argument("--timeout", type=float, default=60.0)
    wpt.add_argument("--runner", choices=("nextest",), default="nextest", help="WPT compat always runs through cargo nextest --release")
    wpt.add_argument("--compat", choices=("smoke", "broad", "experimental", "all"), default=None)
    wpt.add_argument("--case")
    wpt.add_argument("--tag")
    wpt.add_argument("--no-run", action="store_true")
    wpt.add_argument("--baseline", type=Path, default=None)
    wpt.set_defaults(func=cmd_wpt)

    run = subparsers.add_parser("run", help="run selected benchmark suites")
    _add_common_run_args(run)
    run.add_argument(
        "--profile",
        choices=RUN_PROFILES,
        default="smoke",
        help="preset suite selection; `horizontal` runs synthetic-compare plus cdp-session with 10 runs by default",
    )
    run.add_argument("--suite", action="append", choices=("startup", "synthetic", "synthetic-matrix", "synthetic-compare", "cdp-session", "crawler", "amiibo-crawler", "wild-web", "top-sites", "cdp-smoke", "wpt"))
    run.add_argument("--case", action="append", choices=SYNTHETIC_CASES)
    run.add_argument("--target", action="append", choices=TARGETS)
    run.add_argument("--concurrency", type=int, default=1)
    run.add_argument("--startup-profile", choices=STARTUP_PROFILES, default="smoke")
    run.add_argument("--synthetic-matrix-profile", choices=SYNTHETIC_MATRIX_PROFILES, default="smoke")
    run.add_argument("--matrix-concurrency", action="append", type=int)
    run.add_argument("--matrix-repeats", type=int, default=None)
    run.add_argument("--stability-threshold-percent", type=float, default=DEFAULT_STABILITY_THRESHOLD_PERCENT)
    run.add_argument("--include-cdp-first-page", action="store_true")
    run.add_argument("--include-cdp-warm-pages", action="store_true")
    run.add_argument("--cdp-warm-pages", type=int, default=10)
    run.add_argument("--drop-os-cache", action="store_true")
    run.add_argument("--idle-seconds", action="append", type=float)
    run.add_argument("--cdp-profile", choices=CDP_SMOKE_PROFILES, default="smoke")
    run.add_argument("--gate-target", choices=TARGETS, default="moli")
    run.add_argument("--pages", type=int, default=50)
    run.add_argument("--amiibo-profile", choices=AMIIBO_PROFILES, default="smoke")
    run.add_argument("--pool", action="append", type=int)
    run.add_argument("--amiibo-mode", action="append", choices=AMIIBO_MODES)
    run.add_argument("--limit", type=int, default=None)
    run.add_argument("--seed", action="append", choices=tuple(WILD_WEB_SEEDS.keys()))
    run.add_argument("--wild-web-capture-replay", action="store_true", help="capture successful wild-web HTML snapshots under wild-web/replay")
    run.add_argument("--top-sites-profile", choices=tuple(TOP_SITES_PROFILES.keys()), default=DEFAULT_TOP_SITES_PROFILE)
    run.add_argument(
        "--top-sites-source",
        choices=tuple(TOP_SITES_SOURCES.keys()) + COMPOSITE_TOP_SITES_SOURCES,
        default=DEFAULT_TOP_SITES_SOURCE,
    )
    run.add_argument("--top-sites-list-path", type=Path, default=None)
    run.add_argument("--top-sites-limit", type=int, default=None, help="override the top-N count picked by --top-sites-profile")
    run.add_argument("--top-sites-parallelism", type=int, default=DEFAULT_TOP_SITES_PARALLELISM)
    run.add_argument("--top-sites-chrome-parallelism", type=int, default=1, help="concurrency for the top-sites chrome CDP-DCL target only")
    run.add_argument("--top-sites-min-body-bytes", type=int, default=DEFAULT_TOP_SITES_MIN_BODY_BYTES)
    run.add_argument("--cdp-group", action="append")
    run.add_argument("--wpt-runner", choices=("nextest",), default="nextest", help="WPT compat always runs through cargo nextest --release")
    run.add_argument("--wpt-compat", choices=("smoke", "broad", "experimental", "all"), default=None)
    run.add_argument("--wpt-case")
    run.add_argument("--wpt-tag")
    run.add_argument("--wpt-no-run", action="store_true")
    run.add_argument("--wpt-baseline", type=Path, default=None)
    run.set_defaults(func=cmd_run)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return int(args.func(args))
    except KeyboardInterrupt:
        return 130
    except Exception as error:
        print(f"moli-benchmark: {error}", file=sys.stderr)
        return 1
