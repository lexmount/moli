"""CLI entry point for cross-engine WPT runs.

Usage:

    python -m moli_benchmark.wpt_cross \\
        --wpt-root ../wpt \\
        --engine moli --engine lightpanda --engine obscura \\
        --output-dir /tmp/moli-wpt-cross \\
        --limit 20

The runner:

1. Enumerates either the default semantic baseline or an explicit layout
   profile from ``--wpt-root``.
2. Starts a single fixture server (loopback + optional global IPv6 for Obscura).
3. For each engine, launches it via :class:`EngineDriver`, runs every case
   through the testharness bridge or the CDP screenshot reftest path, and
   writes per-engine JSON results.
4. Emits ``matrix.json`` with the cross-engine pass/fail/timeout/crash table.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any

from .case_set import (
    ANY_JS_GLOBAL_CHOICES,
    LAYOUT_PROFILE_DIR_PREFIXES,
    WptCase,
    enumerate_cases,
    enumerate_reftest_cases,
    explicit_case,
    explicit_reftest_case,
)
from .cli_runner import run_engine_on_cases_cli
from .engine import ENGINES, build_driver
from .audit import audit_matrix, load_known_failure_manifest
from .runner import (
    LAYOUT_VIEWPORT,
    MAX_RECORDED_FAILURES,
    ReftestReferenceRun,
    ReftestRun,
    engine_result_to_dict,
    run_engine_on_cases,
)
from .scheduler import build_run_schedule, write_run_schedule
from ..config import clear_current_proxy_env

REPO_CASE_LIST_DIR = Path(__file__).resolve().parents[2] / "wpt-cross-current"
WPT_CROSS_CASE_TIMEOUT_SECONDS = 120.0
WPT_CROSS_PARALLELISM = 100
WPT_CROSS_PROFILES = (
    "default",
    "layout-testharness",
    "layout-reftest",
    "layout",
    "all",
)
CASE_LIST_FILES = {
    "pass": "passed-cases.txt",
    "fail": "failed-cases.txt",
    "timeout": "timeout-cases.txt",
    "crash": "crash-cases.txt",
    "harness-stalled": "harness-stalled-cases.txt",
    "error": "error-cases.txt",
    "missing": "missing-cases.txt",
    "other": "other-cases.txt",
}
NON_TRUSTWORTHY_ORIGIN_CASES = frozenset(
    {
        "audio-output/secure-context.html",
        "credential-management/require_securecontext.html",
        "pointerevents/pointerevent_constructor.html",
    }
)


def _case_requires_trustworthy_origin(case_path: str) -> bool:
    file_part = case_path.split("?", 1)[0].split("#", 1)[0]
    return ".https." in file_part or file_part.endswith(".https.html")


def _case_requires_non_trustworthy_origin(case_path: str) -> bool:
    file_part = case_path.split("?", 1)[0].split("#", 1)[0]
    return (
        ".http." in file_part
        or file_part.endswith("_insecure_context.html")
        or file_part.startswith("secure-contexts/")
        or file_part in NON_TRUSTWORTHY_ORIGIN_CASES
    )


def _url_for_case_origin(server: Any, case_path: str, *, external: bool) -> str:
    if external:
        return server.url_for_case(case_path, external=True)
    if _case_requires_trustworthy_origin(case_path):
        return server.url_for_case(case_path, external=False)
    if _case_requires_non_trustworthy_origin(case_path) and server.external_base_url:
        return server.url_for_case(case_path, external=True)
    return server.url_for_case(case_path, external=False)


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="python -m moli_benchmark.wpt_cross",
        description="Run a curated WPT case set across multiple headless engines.",
    )
    parser.add_argument(
        "--wpt-root",
        type=Path,
        required=True,
        help="Path to the upstream WPT checkout (e.g. ../wpt).",
    )
    parser.add_argument(
        "--engine",
        action="append",
        required=True,
        choices=sorted(ENGINES),
        help="Engine to run (repeatable). Each engine is launched in turn.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        required=True,
        help="Directory to write per-engine results and matrix.json.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=None,
        help="Max number of cases to run (after filter). Useful for smoke runs.",
    )
    parser.add_argument(
        "--profile",
        choices=WPT_CROSS_PROFILES,
        default="default",
        help=(
            "Case profile. 'default' keeps the broad semantic baseline; "
            "'layout-testharness' runs deterministic layout testharness cases; "
            "'layout-reftest' runs manifest-backed screenshot reftests; "
            "'layout' combines both layout profiles; 'all' combines the "
            "default and layout baselines in one matrix. Layout and all "
            "profiles use a fixed 800x600 viewport at DPR 1 and CDP mode."
        ),
    )
    parser.add_argument(
        "--dir-prefix",
        action="append",
        default=None,
        help=(
            "Restrict enumeration to a WPT directory prefix (repeatable). "
            "When omitted, the default profile scans the whole WPT tree with "
            "its rendering/layout blacklist, while layout profiles use their "
            "stable CSS directory list."
        ),
    )
    parser.add_argument(
        "--include-tentative",
        action="store_true",
        help=(
            "Include .tentative.* cases during --dir-prefix enumeration. "
            "Default directory enumeration keeps tentative cases out of the "
            "stable broad baseline."
        ),
    )
    parser.add_argument(
        "--any-js-global",
        choices=ANY_JS_GLOBAL_CHOICES,
        default="none",
        help=(
            "Include .any.js cases during --dir-prefix enumeration for the "
            "selected global: window, dedicatedworker, both, or none "
            "(default). The broad baseline includes both supported globals "
            "for Streams automatically. Explicit --case path.any.js still "
            "runs the window variant by default."
        ),
    )
    parser.add_argument(
        "--case",
        action="append",
        default=None,
        help=(
            "Run an explicit WPT case path (repeatable). This bypasses the "
            "curated directory filters and is useful for tentative or "
            "testdriver-backed investigations."
        ),
    )
    parser.add_argument(
        "--launch-timeout",
        type=float,
        default=30.0,
        help="Per-engine launch (CDP ready) timeout in seconds.",
    )
    parser.add_argument(
        "--moli-bin",
        type=str,
        default=None,
        help="Override MOLI_BIN.",
    )
    parser.add_argument(
        "--lightpanda-bin",
        type=str,
        default=None,
        help="Override LIGHTPANDA_BIN.",
    )
    parser.add_argument(
        "--obscura-bin",
        type=str,
        default=None,
        help="Override OBSCURA_BIN.",
    )
    parser.add_argument(
        "--chrome-bin",
        type=str,
        default=None,
        help="Override CHROME_BIN.",
    )
    parser.add_argument(
        "--mode",
        choices=("auto", "cli", "cdp"),
        default="auto",
        help=(
            "Driver mode: 'cli' uses each engine's `fetch` CLI plus an HTTP "
            "callback bridge (no CDP); 'cdp' uses the CDP runner; 'auto' "
            "(default) picks CLI when the engine's cli_fetch_command is set "
            "(currently moli, lightpanda) and falls back to CDP "
            "otherwise (obscura, chrome)."
        ),
    )
    parser.add_argument(
        "--known-failures",
        type=Path,
        default=None,
        help=(
            "Optional known-failure manifest to audit after writing matrix.json. "
            "This does not rewrite case statuses; it only reports unexpected "
            "failures, changed known-failure shapes, missing expected failures, "
            "and resolved known failures. Any non-empty audit bucket except "
            "known_failures makes the run fail."
        ),
    )
    parser.add_argument(
        "--known-failures-engine",
        choices=sorted(ENGINES),
        default=None,
        help=(
            "Engine to audit with --known-failures. Default: manifest engine "
            "field when present, otherwise the first --engine value."
        ),
    )
    parser.add_argument(
        "--allow-missing-known-failures",
        action="store_true",
        help=(
            "Allow --known-failures rules that are absent from this matrix. "
            "Use only with focused --case / small-slice investigations; full "
            "baseline audits should keep the default strict behavior."
        ),
    )
    return parser


def _binary_override_for(engine: str, args: argparse.Namespace) -> str | None:
    return getattr(args, f"{engine}_bin", None)


def _build_matrix(
    cases: list[WptCase],
    engine_results: dict[str, list[dict[str, Any]]],
) -> list[dict[str, Any]]:
    by_engine_case: dict[str, dict[str, dict[str, Any]]] = {
        engine: {case_dict["case_path"]: case_dict for case_dict in engine_cases}
        for engine, engine_cases in engine_results.items()
    }
    matrix = []
    for case in cases:
        row: dict[str, Any] = {
            "case_path": case.case_path,
            "test_type": case.test_type,
            "references": [
                {
                    "reference_path": reference.reference_path,
                    "relation": reference.relation,
                    "fuzzy": (
                        reference.fuzzy.to_dict()
                        if reference.fuzzy is not None
                        else None
                    ),
                }
                for reference in case.references
            ],
            "results": {},
        }
        for engine in engine_results:
            r = by_engine_case[engine].get(case.case_path)
            if r is None:
                row["results"][engine] = {"status": "missing"}
            else:
                row["results"][engine] = {
                    "status": r["status"],
                    "duration_ms": r["duration_ms"],
                    "subtests": r["subtests"],
                    "failures": r.get("failures", []),
                    "failure_names": r.get("failure_names", []),
                    "harness_status_name": r.get("harness_status_name"),
                    "harness_message": r.get("harness_message"),
                    "error": r.get("error"),
                    "test_type": r.get("test_type", case.test_type),
                    "reftest_comparisons": r.get("reftest_comparisons", []),
                    "artifacts": r.get("artifacts", {}),
                }
        matrix.append(row)
    return matrix


def _summarize(matrix: list[dict[str, Any]], engines: list[str]) -> dict[str, Any]:
    summary: dict[str, Any] = {"total": len(matrix), "engines": {}}
    for engine in engines:
        counter: Counter[str] = Counter()
        for row in matrix:
            counter[row["results"][engine]["status"]] += 1
        summary["engines"][engine] = dict(counter)
    return summary


def _primary_case_list_engine(engines: list[str]) -> str:
    return "moli" if "moli" in engines else engines[0]


def _write_case_list(path: Path, cases: list[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    content = "".join(f"{case}\n" for case in cases)
    temp_path = path.with_name(f".{path.name}.tmp")
    temp_path.write_text(content, encoding="utf-8")
    temp_path.replace(path)


def _write_repo_case_lists(
    matrix: list[dict[str, Any]],
    engines: list[str],
    *,
    case_list_dir: Path | None = None,
) -> None:
    if case_list_dir is None:
        case_list_dir = REPO_CASE_LIST_DIR
    engine = _primary_case_list_engine(engines)
    cases_by_status: dict[str, list[str]] = {
        status: [] for status in CASE_LIST_FILES
    }
    for row in matrix:
        case_path = str(row.get("case_path", ""))
        status = (
            row.get("results", {})
            .get(engine, {})
            .get("status", "missing")
        )
        bucket = status if status in CASE_LIST_FILES else "other"
        cases_by_status[bucket].append(case_path)

    for status, file_name in CASE_LIST_FILES.items():
        cases = cases_by_status[status]
        cases.sort()
        _write_case_list(case_list_dir / file_name, cases)


def _is_full_case_list_run(args: argparse.Namespace) -> bool:
    return (
        args.profile in {"default", "all"}
        and args.case is None
        and args.dir_prefix is None
        and args.limit is None
        and not args.include_tentative
        and args.any_js_global == "none"
    )
def _recorded_failure_names(result: dict[str, Any]) -> dict[str, str]:
    names: dict[str, str] = {}
    for index, failure in enumerate(result.get("failures", [])):
        if not isinstance(failure, dict):
            continue
        name = failure.get("name")
        if not isinstance(name, str) or not name:
            status = failure.get("status_name") or failure.get("status") or "failure"
            name = f"<unnamed {status} #{index + 1}>"
        message = failure.get("message")
        names[name] = message if isinstance(message, str) else ""
    return names


def _failure_name_set(result: dict[str, Any]) -> set[str]:
    names = result.get("failure_names")
    if isinstance(names, list):
        return {name for name in names if isinstance(name, str)}
    return set(_recorded_failure_names(result))


def _recorded_failure_drift(
    matrix: list[dict[str, Any]],
    engines: list[str],
    *,
    primary: str = "moli",
    max_examples: int = 8,
) -> dict[str, Any]:
    if primary not in engines:
        primary = engines[0] if engines else "moli"
    comparisons = []
    for row in matrix:
        results = row.get("results", {})
        primary_result = results.get(primary, {})
        if not isinstance(primary_result, dict):
            continue
        primary_failure_names = _failure_name_set(primary_result)
        primary_failures = _recorded_failure_names(primary_result)
        for peer in engines:
            if peer == primary:
                continue
            peer_result = results.get(peer, {})
            if not isinstance(peer_result, dict):
                continue
            peer_failure_names = _failure_name_set(peer_result)
            peer_failures = _recorded_failure_names(peer_result)
            primary_only = sorted(primary_failure_names - peer_failure_names)
            peer_only = sorted(peer_failure_names - primary_failure_names)
            message_diffs = sorted(
                name
                for name in set(primary_failures) & set(peer_failures)
                if primary_failures[name] != peer_failures[name]
            )
            if not primary_only and not peer_only and not message_diffs:
                continue
            comparisons.append(
                {
                    "case_path": row.get("case_path", ""),
                    "primary": primary,
                    "peer": peer,
                    "primary_only_count": len(primary_only),
                    "peer_only_count": len(peer_only),
                    "message_diff_count": len(message_diffs),
                    "primary_only_examples": primary_only[:max_examples],
                    "peer_only_examples": peer_only[:max_examples],
                    "message_diff_examples": message_diffs[:max_examples],
                }
            )
    primary_only_count = sum(1 for row in comparisons if row["primary_only_count"] > 0)
    peer_only_count = sum(1 for row in comparisons if row["peer_only_count"] > 0)
    message_diff_count = sum(1 for row in comparisons if row["message_diff_count"] > 0)
    return {
        "primary": primary,
        "failure_name_source": "full_failure_names",
        "recorded_failure_limit_per_engine": MAX_RECORDED_FAILURES,
        "comparison_count": len(comparisons),
        "primary_only_comparison_count": primary_only_count,
        "peer_only_comparison_count": peer_only_count,
        "message_diff_comparison_count": message_diff_count,
        "comparisons": comparisons,
    }


def _case_timeout(base_seconds: float, case: WptCase) -> float:
    if case.test_type == "reftest":
        return base_seconds * case.timeout_multiplier
    return base_seconds


STEP_TIMEOUT_SENSITIVE_CASES = frozenset(
    {
        "html/semantics/scripting-1/the-script-element/module/dynamic-import/delay-load-event.html",
    }
)


def _harness_timeout_multiplier(
    case_timeout_seconds: float,
    default_harness_timeout_seconds: float,
    case_path: str | None = None,
) -> float:
    case_path = case_path.split("?", 1)[0].split("#", 1)[0] if case_path else None
    if case_path in STEP_TIMEOUT_SENSITIVE_CASES:
        return 1.0
    return max(1.0, case_timeout_seconds / default_harness_timeout_seconds)


def _is_layout_profile(profile: str) -> bool:
    return profile in {"layout-testharness", "layout-reftest", "layout", "all"}


def _deduplicate_cases(cases: list[WptCase]) -> list[WptCase]:
    by_path: dict[str, WptCase] = {}
    for case in cases:
        current = by_path.get(case.case_path)
        if current is None or case.test_type == "reftest":
            by_path[case.case_path] = case
    return sorted(by_path.values(), key=lambda case: case.case_path)


def _select_cases(args: argparse.Namespace) -> list[WptCase]:
    if args.case:
        if args.profile == "layout-reftest":
            cases = [
                explicit_reftest_case(args.wpt_root, case_path)
                for case_path in args.case
            ]
        elif args.profile in {"layout", "all"}:
            cases = []
            for case_path in args.case:
                try:
                    case = explicit_reftest_case(args.wpt_root, case_path)
                except RuntimeError:
                    case = explicit_case(args.wpt_root, case_path)
                cases.append(case)
        else:
            cases = [explicit_case(args.wpt_root, case_path) for case_path in args.case]
        return cases[: args.limit] if args.limit is not None else cases

    requested_prefixes = tuple(args.dir_prefix) if args.dir_prefix else None
    if args.profile == "default":
        return enumerate_cases(
            args.wpt_root,
            dir_prefixes=requested_prefixes,
            include_tentative=args.include_tentative,
            any_js_global=args.any_js_global,
            limit=args.limit,
        )

    cases: list[WptCase] = []
    if args.profile == "all":
        cases.extend(
            enumerate_cases(
                args.wpt_root,
                dir_prefixes=requested_prefixes,
                include_tentative=args.include_tentative,
                any_js_global=args.any_js_global,
            )
        )
    layout_prefixes = requested_prefixes or LAYOUT_PROFILE_DIR_PREFIXES
    if args.profile in {"layout-testharness", "layout", "all"}:
        cases.extend(
            enumerate_cases(
                args.wpt_root,
                dir_prefixes=layout_prefixes,
                include_tentative=args.include_tentative,
                any_js_global=args.any_js_global,
                layout_static_only=True,
            )
        )
    if args.profile in {"layout-reftest", "layout", "all"}:
        cases.extend(
            enumerate_reftest_cases(
                args.wpt_root,
                dir_prefixes=layout_prefixes,
                include_tentative=args.include_tentative,
            )
        )
    cases = _deduplicate_cases(cases)
    if args.limit is not None:
        cases = cases[: args.limit]
    return cases


def _cdp_case_run(
    server: Any,
    case: WptCase,
    *,
    external: bool,
    timeout_seconds: float,
) -> tuple[str, str, float] | ReftestRun:
    url = _url_for_case_origin(server, case.case_path, external=external)
    if case.test_type != "reftest":
        return case.case_path, url, timeout_seconds
    return ReftestRun(
        case_path=case.case_path,
        url=url,
        timeout_seconds=timeout_seconds,
        references=tuple(
            ReftestReferenceRun(
                reference_path=reference.reference_path,
                url=_url_for_case_origin(
                    server,
                    reference.reference_path,
                    external=external,
                ),
                relation=reference.relation,
                fuzzy=reference.fuzzy,
            )
            for reference in case.references
        ),
    )


def _run_case_path(case: tuple[str, str, float] | ReftestRun) -> str:
    return case.case_path if isinstance(case, ReftestRun) else case[0]


def main(argv: list[str] | None = None) -> int:
    clear_current_proxy_env()

    parser = _build_parser()
    args = parser.parse_args(argv)
    output_dir: Path = args.output_dir
    output_dir.mkdir(parents=True, exist_ok=True)

    known_failure_rules: list[dict[str, Any]] | None = None
    known_failure_categories: dict[str, Any] | None = None
    known_failure_engine: str | None = None
    if args.known_failures is not None:
        try:
            known_failure_manifest = load_known_failure_manifest(args.known_failures)
        except Exception as exc:
            print(f"error: {exc}", file=sys.stderr)
            return 2
        known_failure_rules = known_failure_manifest["rules"]
        known_failure_categories = known_failure_manifest.get("categories")
        manifest_engine = known_failure_manifest.get("engine")
        if args.known_failures_engine is not None:
            known_failure_engine = args.known_failures_engine
        elif isinstance(manifest_engine, str) and manifest_engine:
            known_failure_engine = manifest_engine
        else:
            known_failure_engine = args.engine[0]
        if known_failure_engine not in args.engine:
            print(
                "error: --known-failures targets engine "
                f"{known_failure_engine!r}, but selected engines are {args.engine!r}",
                file=sys.stderr,
            )
            return 2

    try:
        cases = _select_cases(args)
    except RuntimeError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    if not cases:
        print(
            "error: no WPT cases selected; check --wpt-root, --profile, and --dir-prefix",
            file=sys.stderr,
        )
        return 2

    has_reftests = any(case.test_type == "reftest" for case in cases)
    fixed_layout_viewport = _is_layout_profile(args.profile) or has_reftests
    if fixed_layout_viewport and args.mode == "cli":
        print(
            "error: layout profiles and reftests require CDP mode for fixed viewport screenshots",
            file=sys.stderr,
        )
        return 4

    case_list_path = output_dir / "cases.txt"
    case_list_path.write_text("\n".join(c.case_path for c in cases) + "\n", encoding="utf-8")
    scheduled_cases, schedule_metadata = build_run_schedule(
        cases,
        case_path=lambda case: case.case_path,
    )
    write_run_schedule(
        output_dir,
        metadata=schedule_metadata,
        scheduled_case_paths=[case.case_path for case in scheduled_cases],
    )

    # Fixture server is started lazily (we need it before ANY engine runs).
    from .server import DEFAULT_TESTHARNESS_TIMEOUT_SECONDS, WptFixtureServer

    started_at = time.perf_counter()
    engine_results: dict[str, list[dict[str, Any]]] = {}
    engine_metadata: dict[str, dict[str, Any]] = {}

    with WptFixtureServer(args.wpt_root) as server:
        meta_path = output_dir / "fixture-server.json"
        meta_path.write_text(
            json.dumps(
                {
                    "wpt_root": str(server.wpt_root),
                    "loopback_base_url": server.base_url,
                    "loopback_alternate_base_url": server.alternate_base_url,
                    "external_base_url": server.external_base_url,
                    "external_alternate_base_url": server.external_alternate_base_url,
                    "external_remote_base_url": server.external_remote_base_url,
                    "external_host": server.external_host,
                },
                indent=2,
                sort_keys=True,
            ),
            encoding="utf-8",
        )

        for engine in args.engine:
            driver = build_driver(engine)
            external = engine == "obscura"
            mode_for_check = "cdp" if fixed_layout_viewport else args.mode
            if mode_for_check == "auto":
                mode_for_check = "cli" if driver.cli_fetch_command is not None else "cdp"
            if external and mode_for_check == "cdp" and server.external_base_url is None:
                print(
                    f"error: engine {engine} requires global IPv6 fixture host but none was detected",
                    file=sys.stderr,
                )
                return 3
            engine_started = time.perf_counter()
            engine_case_timeout = WPT_CROSS_CASE_TIMEOUT_SECONDS
            server.set_harness_timeout_multipliers(
                {
                    case.case_path: _harness_timeout_multiplier(
                        _case_timeout(engine_case_timeout, case),
                        DEFAULT_TESTHARNESS_TIMEOUT_SECONDS,
                        case.case_path,
                    )
                    for case in cases
                },
                default_multiplier=_harness_timeout_multiplier(
                    engine_case_timeout,
                    DEFAULT_TESTHARNESS_TIMEOUT_SECONDS,
                ),
            )
            cases_for_engine = [
                _cdp_case_run(
                    server,
                    case,
                    external=external,
                    timeout_seconds=_case_timeout(engine_case_timeout, case),
                )
                for case in cases
            ]
            scheduled_cases_for_engine = [
                _cdp_case_run(
                    server,
                    case,
                    external=external,
                    timeout_seconds=_case_timeout(engine_case_timeout, case),
                )
                for case in scheduled_cases
            ]

            mode = "cdp" if fixed_layout_viewport else args.mode
            if mode == "auto":
                mode = "cli" if driver.cli_fetch_command is not None else "cdp"
            elif mode == "cli" and driver.cli_fetch_command is None:
                print(
                    f"error: engine {engine} has no cli_fetch_command; cannot use --mode cli",
                    file=sys.stderr,
                )
                return 4

            if mode == "cli":
                # CLI HTTP-callback runner: bridge POSTs results via fetch().
                # Keep secure-context negative cases off localhost, which
                # browsers treat as potentially trustworthy. Other cases stay
                # on loopback because many WPTs build cross-origin URLs by
                # string-editing location.host, which breaks on IPv6 literals.
                cases_for_engine_cli = [
                    (
                        case.case_path,
                        _url_for_case_origin(server, case.case_path, external=external),
                        _case_timeout(engine_case_timeout, case),
                        _harness_timeout_multiplier(
                            _case_timeout(engine_case_timeout, case),
                            DEFAULT_TESTHARNESS_TIMEOUT_SECONDS,
                            case.case_path,
                        ),
                    )
                    for case in cases
                ]
                scheduled_cases_for_engine_cli = [
                    (
                        case.case_path,
                        _url_for_case_origin(server, case.case_path, external=external),
                        _case_timeout(engine_case_timeout, case),
                        _harness_timeout_multiplier(
                            _case_timeout(engine_case_timeout, case),
                            DEFAULT_TESTHARNESS_TIMEOUT_SECONDS,
                            case.case_path,
                        ),
                    )
                    for case in scheduled_cases
                ]
                print(
                    f"[wpt-cross] running {engine} (CLI mode) on {len(cases_for_engine_cli)} cases via fixture origins",
                    file=sys.stderr,
                )
                result = run_engine_on_cases_cli(
                    driver=driver,
                    fixture_server=server,
                    cases=cases_for_engine_cli,
                    execution_cases=scheduled_cases_for_engine_cli,
                    binary_override=_binary_override_for(engine, args),
                    case_timeout_seconds=engine_case_timeout,
                    parallelism=WPT_CROSS_PARALLELISM,
                )
            else:
                print(
                    f"[wpt-cross] running {engine} (CDP mode) on {len(cases_for_engine)} cases via fixture origins",
                    file=sys.stderr,
                )
                cdp_par = WPT_CROSS_PARALLELISM
                if cdp_par == 1 or len(cases_for_engine) <= 1:
                    result = run_engine_on_cases(
                        driver=driver,
                        cases=scheduled_cases_for_engine,
                        binary_override=_binary_override_for(engine, args),
                        case_timeout_seconds=engine_case_timeout,
                        launch_timeout_seconds=args.launch_timeout,
                        viewport=LAYOUT_VIEWPORT if fixed_layout_viewport else None,
                        artifact_output_dir=output_dir if has_reftests else None,
                    )
                    by_path = {case.case_path: case for case in result.cases}
                    result.cases = [
                        by_path[_run_case_path(case)]
                        for case in cases_for_engine
                        if _run_case_path(case) in by_path
                    ]
                else:
                    print(
                        f"[wpt-cross] {engine}: launching {cdp_par} parallel CDP workers",
                        file=sys.stderr,
                    )
                    chunks = [[] for _ in range(cdp_par)]
                    for i, case in enumerate(scheduled_cases_for_engine):
                        chunks[i % cdp_par].append(case)
                    chunks = [c for c in chunks if c]
                    from concurrent.futures import ThreadPoolExecutor as _TPE
                    from .runner import EngineRunResult as _EngineRunResult
                    with _TPE(max_workers=len(chunks)) as pool:
                        futures = [
                            pool.submit(
                                run_engine_on_cases,
                                driver=driver,
                                cases=chunk,
                                binary_override=_binary_override_for(engine, args),
                                case_timeout_seconds=engine_case_timeout,
                                launch_timeout_seconds=args.launch_timeout,
                                viewport=LAYOUT_VIEWPORT if fixed_layout_viewport else None,
                                artifact_output_dir=output_dir if has_reftests else None,
                            )
                            for chunk in chunks
                        ]
                        partials = [f.result() for f in futures]
                    base = partials[0]
                    merged_cases = list(base.cases)
                    setup_errors = [p.setup_error for p in partials if p.setup_error]
                    shutdown_infos = [p.shutdown_info for p in partials]
                    for p in partials[1:]:
                        merged_cases.extend(p.cases)
                    by_path = {c.case_path: c for c in merged_cases}
                    merged_cases = [
                        by_path[_run_case_path(case)]
                        for case in cases_for_engine
                        if _run_case_path(case) in by_path
                    ]
                    result = _EngineRunResult(
                        engine=base.engine,
                        binary=base.binary,
                        binary_sha256=base.binary_sha256,
                        binary_version=base.binary_version,
                        endpoint=base.endpoint,
                        ready_ms=base.ready_ms,
                        setup_error="; ".join(setup_errors) if setup_errors else None,
                        cases=merged_cases,
                        shutdown_info={"workers": shutdown_infos, "parallelism": len(chunks)},
                    )
            engine_elapsed = time.perf_counter() - engine_started

            if not hasattr(result, "shutdown_info") or result.shutdown_info is None:
                result.shutdown_info = {}
            result.shutdown_info["run_schedule"] = schedule_metadata
            result_dict = engine_result_to_dict(result)
            (output_dir / f"engine-{engine}.json").write_text(
                json.dumps(result_dict, indent=2, sort_keys=True), encoding="utf-8"
            )
            engine_results[engine] = result_dict["cases"]
            engine_metadata[engine] = {
                "binary": result_dict.get("binary"),
                "binary_sha256": result_dict.get("binary_sha256"),
                "binary_version": result_dict.get("binary_version"),
                "endpoint": result_dict.get("endpoint"),
                "ready_ms": result_dict.get("ready_ms"),
                "setup_error": result_dict.get("setup_error"),
                "elapsed_seconds": engine_elapsed,
                "run_schedule": schedule_metadata,
            }
            if result.setup_error:
                print(
                    f"[wpt-cross] {engine} setup error: {result.setup_error}",
                    file=sys.stderr,
                )

    matrix = _build_matrix(cases, engine_results)
    summary = _summarize(matrix, list(args.engine))
    summary["profile"] = args.profile
    summary["viewport"] = (
        {
            "width": LAYOUT_VIEWPORT.width,
            "height": LAYOUT_VIEWPORT.height,
            "device_scale_factor": LAYOUT_VIEWPORT.device_scale_factor,
        }
        if fixed_layout_viewport
        else None
    )
    summary["recorded_failure_drift"] = _recorded_failure_drift(matrix, list(args.engine))
    summary["total_elapsed_seconds"] = time.perf_counter() - started_at
    summary["engine_metadata"] = engine_metadata
    summary["run_schedule"] = schedule_metadata
    if _is_full_case_list_run(args):
        _write_repo_case_lists(matrix, list(args.engine))
        summary["repo_case_list_dir"] = str(REPO_CASE_LIST_DIR)
        print(f"[wpt-cross] refreshed repo case lists: {REPO_CASE_LIST_DIR}")
    else:
        print("[wpt-cross] skipped repo case list refresh for non-full run")

    known_failure_audit: dict[str, Any] | None = None
    if known_failure_rules is not None and known_failure_engine is not None:
        known_failure_audit = audit_matrix(
            matrix,
            known_failure_engine,
            known_failure_rules,
            categories=known_failure_categories,
            allow_missing_known_failures=args.allow_missing_known_failures,
        )
        audit_name = f"known-failure-audit-{known_failure_engine}.json"
        (output_dir / audit_name).write_text(
            json.dumps(known_failure_audit, indent=2, sort_keys=True),
            encoding="utf-8",
        )
        summary["known_failure_audits"] = {
            known_failure_engine: {
                "manifest": str(args.known_failures),
                "output": audit_name,
                "ok": known_failure_audit["ok"],
                "allow_missing_known_failures": args.allow_missing_known_failures,
                "categories": known_failure_audit["categories"],
                "counts": known_failure_audit["counts"],
                "category_counts": known_failure_audit["category_counts"],
            }
        }

    (output_dir / "matrix.json").write_text(
        json.dumps(matrix, indent=2, sort_keys=True), encoding="utf-8"
    )
    (output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8"
    )

    try:
        from .render_html import render_html
        html_path = render_html(output_dir)
        print(f"[wpt-cross] wrote html report: {html_path}")
    except Exception as exc:
        print(f"[wpt-cross] html render failed: {exc}", file=sys.stderr)

    # Compact text summary on stdout.
    print(f"\n=== WPT cross-engine summary ({len(cases)} cases) ===")
    for engine in args.engine:
        counts = summary["engines"][engine]
        ordered = ", ".join(f"{k}={v}" for k, v in sorted(counts.items()))
        print(f"  {engine}: {ordered}")
    if known_failure_audit is not None and known_failure_engine is not None:
        counts = known_failure_audit["counts"]
        print(
            f"  {known_failure_engine} known-failure audit: "
            "known={known_failures}, resolved={resolved_known_failures}, "
            "mismatched={mismatched_known_failures}, "
            "missing={missing_expected_failures}, "
            "skipped={skipped_known_failures}, "
            "unexpected={unexpected_failures}".format(**counts)
        )
    print(f"output: {output_dir}")
    if known_failure_audit is not None and not known_failure_audit["ok"]:
        print(
            "[wpt-cross] known-failure audit found unexpected, changed, missing, or resolved failures",
            file=sys.stderr,
        )
        return 5
    return 0


if __name__ == "__main__":
    sys.exit(main())
