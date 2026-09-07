from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import os
import platform
import subprocess
import sys
import time
import traceback
from collections import Counter, deque
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Iterable

from .browsers import (
    BrowserProcess,
    start_chromium,
    start_moli,
    wait_for_cdp_endpoint,
)
from .cdp import RawCdpClient, connect, observe_case
from .config import (
    PROJECT_ROOT,
    REPO_ROOT,
    chromium_binary,
    moli_binary,
    sha256_file,
    sha256_fixture_tree,
)
from .dom import dom_hash, first_difference, iter_nodes, normalize_dom_node, unified_dom_diff
from .fixture_server import FixtureServer
from .manifest import SmokeManifest, load_manifest, select_cases
from .models import CaseResult, EngineObservation, ObservationFailure, SmokeCase


def _split_values(values: Iterable[str]) -> set[str]:
    return {
        item.strip()
        for value in values
        for item in value.split(",")
        if item.strip()
    }


def _command_text(command: list[str]) -> str:
    try:
        completed = subprocess.run(
            command,
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError) as error:
        return f"unavailable: {error}"
    output = (completed.stdout or completed.stderr).strip()
    if output:
        return output
    return "" if completed.returncode == 0 else f"exit {completed.returncode}"


def _git_metadata() -> dict[str, Any]:
    commit = _command_text(["git", "rev-parse", "HEAD"])
    status = _command_text(["git", "status", "--porcelain"])
    return {
        "commit": commit,
        "dirty": bool(status) and not status.startswith("unavailable:"),
        "status": status.splitlines()[:100] if status else [],
    }


def _runtime_metadata() -> dict[str, str]:
    return {
        "python": platform.python_version(),
        "node": _command_text(["node", "--version"]),
        "uv": _command_text(["uv", "--version"]),
    }


def _process_log_tail(process: BrowserProcess | None) -> list[str] | None:
    logs = getattr(process, "logs", None)
    return list(logs)[-100:] if logs is not None else None


def _process_diagnostics(process: BrowserProcess | None) -> dict[str, Any]:
    logs = getattr(process, "logs", None)
    if logs is None:
        return {"available": False, "unexpectedErrors": []}
    unexpected_errors = [
        line
        for line in logs
        if ":stderr: " in line
        and (" ERROR " in line or '"level":"ERROR"' in line)
    ]
    return {
        "available": True,
        "unexpectedErrors": unexpected_errors,
    }


def _manifest_hash(manifest: SmokeManifest) -> str:
    return hashlib.sha256(manifest.path.read_bytes()).hexdigest()


def _normalize_observation(observation: EngineObservation) -> None:
    for frame in observation.frames:
        if frame.dom is None:
            continue
        normalized_frame = normalize_dom_node(frame.dom)
        frame.dom = normalized_frame
        frame.dom_hash = dom_hash(normalized_frame)
        frame.node_count = sum(1 for _ in iter_nodes(normalized_frame))
    if observation.dom is not None:
        normalized = normalize_dom_node(observation.dom)
        observation.dom = normalized
        observation.dom_hash = dom_hash(normalized)
        observation.node_count = sum(1 for _ in iter_nodes(normalized))
    if (
        observation.ok
        and observation.frames
        and observation.frames[-1].dom_hash != observation.dom_hash
    ):
        raise RuntimeError("settled frame does not match final DOM")


def _observation_artifact_json(observation: EngineObservation | None) -> dict[str, Any] | None:
    return observation.summary_json() if observation else None


def _frame_file_stem(index: int, name: str) -> str:
    safe_name = "".join(character if character.isalnum() or character in "-_" else "-" for character in name)
    return f"{index:02d}-{safe_name}"


def _timeline_difference(
    chromium: EngineObservation,
    moli: EngineObservation,
) -> tuple[str | None, list[str]]:
    if not chromium.frames and not moli.frames:
        return first_difference(chromium.dom, moli.dom), []
    mismatched: list[str] = []
    first: str | None = None
    if len(chromium.frames) != len(moli.frames):
        first = "$.frames.length"
        mismatched.append("frame-count")
    for index, (reference_frame, candidate_frame) in enumerate(
        zip(chromium.frames, moli.frames)
    ):
        if reference_frame.name != candidate_frame.name:
            if first is None:
                first = f"$.frames[{index}].name"
            mismatched.append(f"{reference_frame.name}!={candidate_frame.name}")
            continue
        difference = first_difference(reference_frame.dom, candidate_frame.dom)
        if difference:
            if first is None:
                suffix = difference[1:] if difference.startswith("$") else f".{difference}"
                first = f"$.frames[{index}].dom{suffix}"
            mismatched.append(reference_frame.name)
    return first, mismatched


def _expected_diagnostic_difference(
    chromium: EngineObservation,
    moli: EngineObservation,
) -> str | None:
    reference = chromium.diagnostics.get("expectedNetworkFailures") or []
    candidate = moli.diagnostics.get("expectedNetworkFailures") or []
    return None if reference == candidate else "$.diagnostics.expectedNetworkFailures"


def _write_failure_artifact(
    output_dir: Path,
    result: CaseResult,
) -> str:
    case_dir = output_dir / "cases" / Path(*result.case.id.split("/"))
    case_dir.mkdir(parents=True, exist_ok=True)
    if result.chromium.dom is not None:
        (case_dir / "chromium.dom.json").write_text(
            json.dumps(result.chromium.dom, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    if result.moli and result.moli.dom is not None:
        (case_dir / "moli.dom.json").write_text(
            json.dumps(result.moli.dom, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    if (
        result.status == "dom_mismatch"
        and result.chromium.dom is not None
        and result.moli
        and result.moli.dom is not None
    ):
        (case_dir / "diff.txt").write_text(
            unified_dom_diff(result.chromium.dom, result.moli.dom) + "\n",
            encoding="utf-8",
        )
    frames_dir = case_dir / "frames"
    frame_metadata: dict[str, Any] = {
        "chromium": [],
        "moli": [],
        "mismatchedFrames": result.mismatched_frames,
    }
    for engine, observation in (
        ("chromium", result.chromium),
        ("moli", result.moli),
    ):
        if observation is None:
            continue
        for frame in observation.frames:
            frame_metadata[engine].append(frame.summary_json())
            if frame.dom is None:
                continue
            frames_dir.mkdir(parents=True, exist_ok=True)
            stem = _frame_file_stem(frame.index, frame.name)
            (frames_dir / f"{stem}.{engine}.dom.json").write_text(
                json.dumps(frame.dom, indent=2, ensure_ascii=False) + "\n",
                encoding="utf-8",
            )
    if result.moli:
        for reference_frame, candidate_frame in zip(
            result.chromium.frames, result.moli.frames
        ):
            if (
                reference_frame.name == candidate_frame.name
                and reference_frame.dom is not None
                and candidate_frame.dom is not None
                and first_difference(reference_frame.dom, candidate_frame.dom)
            ):
                frames_dir.mkdir(parents=True, exist_ok=True)
                stem = _frame_file_stem(reference_frame.index, reference_frame.name)
                (frames_dir / f"{stem}.diff.txt").write_text(
                    unified_dom_diff(reference_frame.dom, candidate_frame.dom) + "\n",
                    encoding="utf-8",
                )
    if frame_metadata["chromium"] or frame_metadata["moli"]:
        (case_dir / "timeline.json").write_text(
            json.dumps(frame_metadata, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
    diagnostics = {
        "id": result.case.id,
        "status": result.status,
        "firstDifference": result.first_difference,
        "mismatchedFrames": result.mismatched_frames,
        "chromium": _observation_artifact_json(result.chromium),
        "moli": _observation_artifact_json(result.moli),
    }
    (case_dir / "diagnostics.json").write_text(
        json.dumps(diagnostics, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    return str(case_dir.relative_to(output_dir))


async def _close_cdp_client(client: RawCdpClient | None) -> None:
    if client is None:
        return
    try:
        await client.websocket.close()
    except Exception:
        pass


def _attempt_failure(attempt: int, observation: EngineObservation) -> ObservationFailure:
    return ObservationFailure(
        attempt=attempt,
        kind=observation.failure_kind or "infrastructure",
        duration_ms=observation.duration_ms,
        error_type=observation.error_type,
        error=observation.error,
    )


async def _observe_phase_case(
    *,
    engine: str,
    endpoint: str,
    case: SmokeCase,
    fixture_url: str,
    timeout_ms: int,
    client: RawCdpClient | None,
    infrastructure_retries: int,
    progress: dict[str, int],
    total: int,
) -> tuple[EngineObservation, RawCdpClient | None]:
    url = fixture_url.rstrip("/") + case.path
    previous_failures: list[ObservationFailure] = []
    attempt = 0
    while True:
        attempt += 1
        if client is None:
            connect_started = time.perf_counter()
            try:
                client = await connect(endpoint)
            except Exception as error:
                observation = EngineObservation(
                    engine=engine,
                    ok=False,
                    duration_ms=(time.perf_counter() - connect_started) * 1000,
                    error_type=type(error).__name__,
                    error=f"CDP connection failed: {error}",
                    failure_kind="infrastructure",
                )
            else:
                observation = await observe_case(
                    engine=engine,
                    endpoint=endpoint,
                    case=case,
                    url=url,
                    timeout_ms=timeout_ms,
                    client=client,
                )
        else:
            observation = await observe_case(
                engine=engine,
                endpoint=endpoint,
                case=case,
                url=url,
                timeout_ms=timeout_ms,
                client=client,
            )
        try:
            _normalize_observation(observation)
        except Exception as error:
            observation.ok = False
            observation.error_type = type(error).__name__
            observation.error = f"DOM normalization failed: {error}"
            observation.failure_kind = "normalization"

        if observation.failure_kind == "infrastructure":
            await _close_cdp_client(client)
            client = None

        should_retry = (
            engine == "chromium"
            and observation.failure_kind == "infrastructure"
            and attempt <= infrastructure_retries
        )
        if not should_retry:
            break
        previous_failures.append(_attempt_failure(attempt, observation))
        error = (observation.error or observation.error_type or "unknown").replace(
            "\n", " "
        )
        print(
            f"[{engine} retry {attempt}/{infrastructure_retries}] "
            f"{case.id}: {error[:240]}",
            file=sys.stderr,
            flush=True,
        )

    observation.duration_ms += sum(
        failure.duration_ms for failure in previous_failures
    )
    observation.attempt_count = attempt
    observation.previous_failures = previous_failures
    progress["completed"] += 1
    state = "ok" if observation.ok else "error"
    print(
        f"[{engine} {progress['completed']:>3}/{total}] {state:<5} {case.id}",
        file=sys.stderr,
        flush=True,
    )
    return observation, client


async def _observe_phase(
    *,
    engine: str,
    endpoint: str,
    cases: tuple[SmokeCase, ...],
    fixture_url: str,
    timeout_ms: int,
    jobs: int,
    infrastructure_retries: int = 0,
) -> dict[str, EngineObservation]:
    if not cases:
        return {}
    queue: asyncio.Queue[SmokeCase] = asyncio.Queue()
    for case in cases:
        queue.put_nowait(case)
    progress = {"completed": 0}
    observations: dict[str, EngineObservation] = {}

    async def worker() -> None:
        client: RawCdpClient | None = None
        try:
            while True:
                try:
                    case = queue.get_nowait()
                except asyncio.QueueEmpty:
                    return
                observation, client = await _observe_phase_case(
                    engine=engine,
                    endpoint=endpoint,
                    case=case,
                    fixture_url=fixture_url,
                    timeout_ms=timeout_ms,
                    client=client,
                    infrastructure_retries=infrastructure_retries,
                    progress=progress,
                    total=len(cases),
                )
                observations[case.id] = observation
        finally:
            await _close_cdp_client(client)

    await asyncio.gather(
        *(asyncio.create_task(worker()) for _ in range(min(jobs, len(cases))))
    )
    return {case.id: observations[case.id] for case in cases}


def _case_result(
    *,
    case: SmokeCase,
    chromium: EngineObservation,
    moli: EngineObservation | None,
    reference_only: bool,
    output_dir: Path,
) -> CaseResult:
    difference = None
    mismatched_frames: list[str] = []
    if not chromium.ok:
        status = "reference_error"
    elif reference_only:
        status = "reference_ok"
    elif moli is None:
        status = "infrastructure_error"
    elif not moli.ok:
        status = "moli_error"
    else:
        difference = _expected_diagnostic_difference(chromium, moli)
        if difference:
            status = "diagnostic_mismatch"
        else:
            difference, mismatched_frames = _timeline_difference(chromium, moli)
            status = "dom_mismatch" if difference else "match"
    result = CaseResult(
        case=case,
        status=status,
        duration_ms=chromium.duration_ms + (moli.duration_ms if moli else 0),
        chromium=chromium,
        moli=moli,
        first_difference=difference,
        mismatched_frames=mismatched_frames,
    )
    if status not in {"match", "reference_ok"}:
        result.artifact = _write_failure_artifact(output_dir, result)
    return result


def _validate_complete_manifest(manifest: SmokeManifest) -> None:
    if not manifest.complete:
        raise RuntimeError(
            "fixture manifest is a focused/partial build; run `npm run build` "
            "or pass --allow-partial-manifest for a focused probe"
        )
    if len(manifest.cases) < 1020:
        raise RuntimeError(
            f"complete manifest must contain at least 1020 cases, got {len(manifest.cases)}"
        )
    frameworks = Counter(case.framework for case in manifest.cases)
    for framework in ("react", "vue", "angular"):
        if frameworks[framework] < 340:
            raise RuntimeError(
                f"complete manifest must contain at least 340 {framework} cases, "
                f"got {frameworks[framework]}"
            )
        complexities = Counter(
            case.complexity for case in manifest.cases if case.framework == framework
        )
        expected_complexities = {"simple": 40, "medium": 40, "complex": 260}
        if dict(complexities) != expected_complexities:
            raise RuntimeError(
                f"complete manifest has invalid {framework} complexity quota: "
                f"expected {expected_complexities}, got {dict(complexities)}"
            )
        families = Counter(case.family for case in manifest.cases if case.framework == framework)
        if len(families) * 10 != frameworks[framework] or set(families.values()) != {10}:
            raise RuntimeError(
                f"complete manifest has invalid {framework} family quota: {dict(families)}"
            )


def _default_output_dir() -> Path:
    stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    return PROJECT_ROOT / "artifacts" / stamp


async def run(args: argparse.Namespace) -> tuple[int, dict[str, Any]]:
    reference_infrastructure_retries = getattr(
        args, "reference_infrastructure_retries", 0
    )
    manifest_path = Path(args.manifest).expanduser().resolve()
    manifest = load_manifest(manifest_path)
    if not args.allow_partial_manifest:
        _validate_complete_manifest(manifest)

    cases = select_cases(
        manifest,
        frameworks=_split_values(args.framework),
        families=_split_values(args.family),
        complexities=_split_values(args.complexity),
        patterns=args.case,
    )
    output_dir = Path(args.output).expanduser().resolve() if args.output else _default_output_dir()
    output_dir.mkdir(parents=True, exist_ok=True)
    fixture_root = Path(args.dist).expanduser().resolve()
    fixture_tree_hash = sha256_fixture_tree(fixture_root)
    if manifest.fixtures_sha256 is None:
        raise RuntimeError("fixture manifest does not include fixturesSha256; rebuild fixtures")
    if fixture_tree_hash != manifest.fixtures_sha256:
        raise RuntimeError(
            "fixture dist content does not match manifest: "
            f"expected {manifest.fixtures_sha256}, got {fixture_tree_hash}; "
            "run `npm run build` and do not rebuild during a smoke run"
        )
    fixture = FixtureServer(fixture_root)
    chromium_process: BrowserProcess | None = None
    moli_process: BrowserProcess | None = None
    started = time.perf_counter()
    started_at = datetime.now(UTC).isoformat()
    fixture.start()
    try:
        if args.chromium_endpoint:
            chromium_endpoint = args.chromium_endpoint.rstrip("/")
            chromium_version = await wait_for_cdp_endpoint(
                chromium_endpoint, None, deque(maxlen=1)
            )
            chromium_path = None
        else:
            chromium_path = chromium_binary(args.chromium_bin)
            chromium_process = await start_chromium(chromium_path)
            chromium_endpoint = chromium_process.endpoint
            chromium_version = chromium_process.version

        chromium_observations = await _observe_phase(
            engine="chromium",
            endpoint=chromium_endpoint,
            cases=cases,
            fixture_url=fixture.url,
            timeout_ms=args.timeout_ms,
            jobs=args.jobs,
            infrastructure_retries=reference_infrastructure_retries,
        )
        reference_gate_ok = all(
            observation.ok for observation in chromium_observations.values()
        )

        moli_endpoint: str | None = None
        moli_path: Path | None = None
        moli_version: dict[str, Any] | None = None
        moli_observations: dict[str, EngineObservation] = {}
        if not args.reference_only and reference_gate_ok:
            if args.moli_endpoint:
                moli_endpoint = args.moli_endpoint.rstrip("/")
                moli_version = await wait_for_cdp_endpoint(
                    moli_endpoint, None, deque(maxlen=1)
                )
            else:
                moli_path = moli_binary(args.moli_bin)
                moli_process = await start_moli(moli_path)
                moli_endpoint = moli_process.endpoint
                moli_version = moli_process.version
            moli_observations = await _observe_phase(
                engine="moli",
                endpoint=moli_endpoint,
                cases=cases,
                fixture_url=fixture.url,
                timeout_ms=args.timeout_ms,
                jobs=args.jobs,
                infrastructure_retries=0,
            )

        effective_reference_only = args.reference_only or not reference_gate_ok
        results = [
            _case_result(
                case=case,
                chromium=chromium_observations[case.id],
                moli=moli_observations.get(case.id),
                reference_only=effective_reference_only,
                output_dir=output_dir,
            )
            for case in cases
        ]
        if moli_process:
            await moli_process.stop()
        if chromium_process:
            await chromium_process.stop()
        counts = Counter(result.status for result in results)
        ok_statuses = {"reference_ok"} if args.reference_only else {"match"}
        chromium_process_diagnostics = _process_diagnostics(chromium_process)
        moli_process_diagnostics = _process_diagnostics(moli_process)
        ok = (
            reference_gate_ok
            and all(result.status in ok_statuses for result in results)
            and not moli_process_diagnostics["unexpectedErrors"]
        )
        metadata = {
            "schemaVersion": 1,
            "runId": output_dir.name,
            "ok": ok,
            "mode": "reference" if args.reference_only else "differential",
            "startedAt": started_at,
            "durationMs": round((time.perf_counter() - started) * 1000, 3),
            "git": _git_metadata(),
            "runtimes": _runtime_metadata(),
            "manifest": {
                "path": str(manifest.path),
                "sha256": _manifest_hash(manifest),
                "complete": manifest.complete,
                "catalogCases": manifest.total_catalog_cases,
                "selectedCases": len(cases),
                "fixturesSha256": manifest.fixtures_sha256,
                "verifiedFixturesSha256": fixture_tree_hash,
                "tools": manifest.tools,
            },
            "referenceGate": {
                "ok": reference_gate_ok,
                "cases": len(chromium_observations),
                "errors": sum(
                    1 for observation in chromium_observations.values() if not observation.ok
                ),
                "moliPhaseStarted": bool(moli_observations),
                "infrastructureRetryLimit": reference_infrastructure_retries,
                "retriedCases": sum(
                    observation.attempt_count > 1
                    for observation in chromium_observations.values()
                ),
                "recoveredCases": sum(
                    observation.ok and bool(observation.previous_failures)
                    for observation in chromium_observations.values()
                ),
            },
            "fixture": fixture.url,
            "engines": {
                "chromium": {
                    "endpoint": chromium_endpoint,
                    "executable": str(chromium_path) if chromium_path else None,
                    "version": chromium_version,
                    "processLogTail": _process_log_tail(chromium_process),
                    "processDiagnostics": chromium_process_diagnostics,
                },
                "moli": (
                    {
                        "endpoint": moli_endpoint,
                        "executable": str(moli_path) if moli_path else None,
                        "sha256": (
                            sha256_file(moli_path)
                            if moli_path and moli_path.is_file()
                            else None
                        ),
                        "version": moli_version,
                        "processLogTail": _process_log_tail(moli_process),
                        "processDiagnostics": moli_process_diagnostics,
                    }
                    if moli_endpoint
                    else None
                ),
            },
            "counts": dict(sorted(counts.items())),
            "timeline": {
                "chromiumFrames": sum(
                    len(observation.frames) for observation in chromium_observations.values()
                ),
                "moliFrames": sum(
                    len(observation.frames) for observation in moli_observations.values()
                ),
                "mismatchedFrames": sum(len(result.mismatched_frames) for result in results),
            },
            "results": [result.to_json() for result in results],
        }
        (output_dir / "summary.json").write_text(
            json.dumps(metadata, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        metadata["output"] = str(output_dir)
        return (0 if ok else 1), metadata
    finally:
        if moli_process:
            await moli_process.stop()
        if chromium_process:
            await chromium_process.stop()
        fixture.stop()


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare React, Vue, and Angular fixture DOM trees in Chromium and Moli."
    )
    parser.add_argument(
        "--manifest",
        default=str(PROJECT_ROOT / "dist" / "manifest.json"),
        help="Built flat fixture manifest.",
    )
    parser.add_argument(
        "--dist",
        default=str(PROJECT_ROOT / "dist"),
        help="Built fixture directory to serve.",
    )
    parser.add_argument("--framework", action="append", default=[], help="Framework filter.")
    parser.add_argument("--family", action="append", default=[], help="Family filter.")
    parser.add_argument("--complexity", action="append", default=[], help="Complexity filter.")
    parser.add_argument(
        "--case",
        action="append",
        default=[],
        help="Case id glob. May be repeated.",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=1,
        help=(
            "Concurrent case workers per engine. The default is serial because current "
            "Moli accepts one browser frontend; values above 1 also probe that boundary."
        ),
    )
    parser.add_argument(
        "--timeout-ms",
        type=int,
        default=15_000,
        help="Per-page fixture ready timeout.",
    )
    parser.add_argument(
        "--reference-infrastructure-retries",
        type=int,
        default=0,
        help=(
            "Retry only Chromium CDP/infrastructure observation failures; fixture, "
            "normalization, and Moli failures remain single-attempt gates."
        ),
    )
    parser.add_argument("--output", help="Artifact output directory.")
    parser.add_argument("--chromium-bin", help="Chromium executable.")
    parser.add_argument("--moli-bin", help="Moli executable.")
    parser.add_argument("--chromium-endpoint", help="Existing Chromium CDP HTTP endpoint.")
    parser.add_argument("--moli-endpoint", help="Existing Moli CDP HTTP endpoint.")
    parser.add_argument(
        "--reference-only",
        action="store_true",
        help="Run Chromium fixture readiness and DOM capture without Moli.",
    )
    parser.add_argument(
        "--allow-partial-manifest",
        action="store_true",
        help="Permit a focused build manifest with fewer than 300 cases.",
    )
    parser.add_argument("--list", action="store_true", help="Print selected flat case list and exit.")
    args = parser.parse_args(argv)
    if args.jobs < 1 or args.jobs > 32:
        parser.error("--jobs must be between 1 and 32")
    if args.timeout_ms < 100 or args.timeout_ms > 120_000:
        parser.error("--timeout-ms must be between 100 and 120000")
    if not 0 <= args.reference_infrastructure_retries <= 3:
        parser.error("--reference-infrastructure-retries must be between 0 and 3")
    if args.reference_only and args.moli_endpoint:
        parser.error("--reference-only cannot be combined with --moli-endpoint")
    return args


def _list_cases(args: argparse.Namespace) -> int:
    manifest = load_manifest(Path(args.manifest).expanduser().resolve())
    cases = select_cases(
        manifest,
        frameworks=_split_values(args.framework),
        families=_split_values(args.family),
        complexities=_split_values(args.complexity),
        patterns=args.case,
    )
    print(
        json.dumps(
            {
                "count": len(cases),
                "complete": manifest.complete,
                "cases": [
                    {
                        "id": case.id,
                        "framework": case.framework,
                        "family": case.family,
                        "complexity": case.complexity,
                        "path": case.path,
                    }
                    for case in cases
                ],
            },
            indent=2,
            ensure_ascii=False,
        )
    )
    return 0


async def async_main(args: argparse.Namespace) -> int:
    try:
        code, summary = await run(args)
        print(json.dumps(summary, indent=2, ensure_ascii=False))
        return code
    except Exception as error:
        print(
            json.dumps(
                {
                    "ok": False,
                    "errorType": type(error).__name__,
                    "error": str(error),
                    "traceback": "".join(traceback.format_exception(error)),
                },
                indent=2,
                ensure_ascii=False,
            ),
            file=sys.stderr,
        )
        return 2


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)
    if args.list:
        raise SystemExit(_list_cases(args))
    raise SystemExit(asyncio.run(async_main(args)))
