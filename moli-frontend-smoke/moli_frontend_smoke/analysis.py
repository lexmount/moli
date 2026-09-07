from __future__ import annotations

import argparse
import json
import sys
import traceback
from collections import Counter
from pathlib import Path
from typing import Any, Iterable

from .dom import (
    EDGE_FIELDS,
    dom_hash,
    first_difference,
    iter_nodes,
    unified_dom_diff,
)


_EXTENDED_EDGES = ("shadowRoots", "pseudoElements", "templateContent", "contentDocument")
_VERSION_FIELDS = (
    "Browser",
    "Protocol-Version",
    "User-Agent",
    "V8-Version",
    "WebKit-Version",
)
_ORDINARY_FRAME_NAMES = ("document", "mounted", "ready", "settled")
_GALLERY_FRAME_NAMES = (
    "document",
    "mounted",
    "interaction-1",
    "ready",
    "settled",
)
_ANIMATION_FRAME_NAMES = (
    "document",
    "mounted",
    "animation-frame-1",
    "animation-frame-2",
    "animation-frame-3",
    "ready",
    "settled",
)
_BOUNDARY_FRAME_NAMES = (
    "document",
    "mounted",
    "boundary-1",
    "boundary-2",
    "ready",
    "settled",
)
_PLATFORM_FRAME_NAMES = (
    "document",
    "mounted",
    "platform-1",
    "platform-2",
    "ready",
    "settled",
)


def _summary_path(value: str | Path) -> Path:
    path = Path(value).expanduser().resolve()
    return path / "summary.json" if path.is_dir() else path


def _load_summary(value: str | Path) -> tuple[Path, dict[str, Any]]:
    path = _summary_path(value)
    try:
        summary = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise RuntimeError(f"missing smoke summary: {path}") from error
    if not isinstance(summary, dict) or not isinstance(summary.get("results"), list):
        raise RuntimeError(f"invalid smoke summary: {path}")
    return path, summary


def _project_dom(
    node: dict[str, Any],
    *,
    drop_whitespace: bool,
    drop_edges: frozenset[str],
) -> dict[str, Any]:
    projected: dict[str, Any] = {}
    node_type = node.get("nodeType")
    for key, value in node.items():
        if node_type == 10 and key == "nodeName" and isinstance(value, str):
            projected[key] = value.lower()
        elif key in drop_edges:
            continue
        elif key in EDGE_FIELDS and isinstance(value, list):
            children = [
                _project_dom(
                    child,
                    drop_whitespace=drop_whitespace,
                    drop_edges=drop_edges,
                )
                for child in value
                if isinstance(child, dict)
                and not (
                    drop_whitespace
                    and child.get("nodeType") == 3
                    and not str(child.get("nodeValue", "")).strip()
                )
            ]
            if children:
                projected[key] = children
        elif key in EDGE_FIELDS and isinstance(value, dict):
            projected[key] = _project_dom(
                value,
                drop_whitespace=drop_whitespace,
                drop_edges=drop_edges,
            )
        else:
            projected[key] = value
    return projected


def _difference_kind(path: str) -> str:
    for field in (
        "pseudoElements",
        "shadowRoots",
        "templateContent",
        "contentDocument",
        "attributes",
        "nodeName",
        "nodeValue",
        "namespaceURI",
        "localName",
    ):
        if f".{field}" in path:
            return field
    return "children" if ".children" in path else path


def _frame_stem(frame: dict[str, Any]) -> str:
    index = int(frame["index"])
    name = str(frame["name"])
    safe_name = "".join(
        character if character.isalnum() or character in "-_" else "-"
        for character in name
    )
    return f"{index:02d}-{safe_name}"


def _artifact_relative_path(result: dict[str, Any]) -> tuple[Path | None, str | None]:
    artifact = result.get("artifact")
    if not artifact:
        return None, None
    path = Path(str(artifact))
    expected = Path("cases", *str(result.get("id", "")).split("/"))
    if path.is_absolute() or ".." in path.parts or path != expected:
        return None, (
            f"{result.get('id')}: invalid artifact path {artifact!r}; "
            f"expected {expected}"
        )
    return path, None


def _expected_artifact_files(
    summary: dict[str, Any],
) -> tuple[set[Path], list[str]]:
    expected: set[Path] = set()
    errors: list[str] = []
    for result in summary["results"]:
        case_dir, path_error = _artifact_relative_path(result)
        if path_error:
            errors.append(path_error)
            continue
        if case_dir is None:
            if result.get("status") not in {"match", "reference_ok"}:
                errors.append(f"{result.get('id')}: failed result has no artifact")
            continue
        expected.add(case_dir / "diagnostics.json")
        chromium = result.get("chromium") or {}
        moli = result.get("moli") or {}
        if chromium.get("dom_hash"):
            expected.add(case_dir / "chromium.dom.json")
        if moli.get("dom_hash"):
            expected.add(case_dir / "moli.dom.json")
        if (
            result.get("status") == "dom_mismatch"
            and chromium.get("dom_hash")
            and moli.get("dom_hash")
        ):
            expected.add(case_dir / "diff.txt")
        chromium_frames = chromium.get("frames") or []
        moli_frames = moli.get("frames") or []
        if chromium_frames or moli_frames:
            expected.add(case_dir / "timeline.json")
        for engine, frames in (
            ("chromium", chromium_frames),
            ("moli", moli_frames),
        ):
            for frame in frames:
                if frame.get("dom_hash"):
                    expected.add(
                        case_dir
                        / "frames"
                        / f"{_frame_stem(frame)}.{engine}.dom.json"
                    )
        for reference_frame, candidate_frame in zip(
            chromium_frames, moli_frames
        ):
            if (
                reference_frame.get("name") == candidate_frame.get("name")
                and reference_frame.get("dom_hash")
                and candidate_frame.get("dom_hash")
                and reference_frame.get("dom_hash")
                != candidate_frame.get("dom_hash")
            ):
                expected.add(
                    case_dir
                    / "frames"
                    / f"{_frame_stem(reference_frame)}.diff.txt"
                )
    return expected, errors


def _load_json_object(path: Path, errors: list[str]) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        errors.append(f"{path}: cannot read JSON: {error}")
        return None
    if not isinstance(value, dict):
        errors.append(f"{path}: expected a JSON object")
        return None
    return value


def _validate_dom_artifact(
    path: Path,
    metadata: dict[str, Any],
    errors: list[str],
) -> dict[str, Any] | None:
    node = _load_json_object(path, errors)
    if node is None:
        return None
    actual_hash = dom_hash(node)
    actual_count = sum(1 for _ in iter_nodes(node))
    if actual_hash != metadata.get("dom_hash"):
        errors.append(
            f"{path}: hash mismatch; expected {metadata.get('dom_hash')}, "
            f"got {actual_hash}"
        )
    if actual_count != metadata.get("node_count"):
        errors.append(
            f"{path}: node-count mismatch; expected {metadata.get('node_count')}, "
            f"got {actual_count}"
        )
    return node


def _validate_case_artifacts(
    output_dir: Path,
    result: dict[str, Any],
    errors: list[str],
) -> None:
    relative, path_error = _artifact_relative_path(result)
    if path_error:
        return
    if relative is None:
        return
    case_dir = output_dir / relative
    diagnostics = _load_json_object(case_dir / "diagnostics.json", errors)
    expected_diagnostics = {
        "id": result.get("id"),
        "status": result.get("status"),
        "firstDifference": result.get("firstDifference"),
        "mismatchedFrames": result.get("mismatchedFrames") or [],
        "chromium": result.get("chromium"),
        "moli": result.get("moli"),
    }
    if diagnostics is not None and diagnostics != expected_diagnostics:
        errors.append(f"{case_dir / 'diagnostics.json'}: content differs from summary")

    loaded_frames: dict[tuple[str, int], dict[str, Any]] = {}
    observations = {
        "chromium": result.get("chromium") or {},
        "moli": result.get("moli") or {},
    }
    for engine, observation in observations.items():
        if observation.get("dom_hash"):
            _validate_dom_artifact(
                case_dir / f"{engine}.dom.json",
                observation,
                errors,
            )
        for frame in observation.get("frames") or []:
            if not frame.get("dom_hash"):
                continue
            path = case_dir / "frames" / f"{_frame_stem(frame)}.{engine}.dom.json"
            node = _validate_dom_artifact(path, frame, errors)
            if node is not None:
                loaded_frames[(engine, int(frame["index"]))] = node

    chromium_frames = observations["chromium"].get("frames") or []
    moli_frames = observations["moli"].get("frames") or []
    if chromium_frames or moli_frames:
        timeline = _load_json_object(case_dir / "timeline.json", errors)
        expected_timeline = {
            "chromium": chromium_frames,
            "moli": moli_frames,
            "mismatchedFrames": result.get("mismatchedFrames") or [],
        }
        if timeline is not None and timeline != expected_timeline:
            errors.append(f"{case_dir / 'timeline.json'}: content differs from summary")

    reference_final = None
    candidate_final = None
    if observations["chromium"].get("dom_hash"):
        reference_final = _load_json_object(case_dir / "chromium.dom.json", errors)
    if observations["moli"].get("dom_hash"):
        candidate_final = _load_json_object(case_dir / "moli.dom.json", errors)
    if (
        result.get("status") == "dom_mismatch"
        and reference_final is not None
        and candidate_final is not None
    ):
        expected_diff = unified_dom_diff(reference_final, candidate_final) + "\n"
        diff_path = case_dir / "diff.txt"
        try:
            actual_diff = diff_path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{diff_path}: cannot read diff: {error}")
        else:
            if actual_diff != expected_diff:
                errors.append(f"{diff_path}: content does not match DOM artifacts")

    for reference_frame, candidate_frame in zip(
        chromium_frames, moli_frames
    ):
        if (
            reference_frame.get("name") != candidate_frame.get("name")
            or not reference_frame.get("dom_hash")
            or not candidate_frame.get("dom_hash")
            or reference_frame.get("dom_hash") == candidate_frame.get("dom_hash")
        ):
            continue
        index = int(reference_frame["index"])
        reference = loaded_frames.get(("chromium", index))
        candidate = loaded_frames.get(("moli", int(candidate_frame["index"])))
        if reference is None or candidate is None:
            continue
        diff_path = case_dir / "frames" / f"{_frame_stem(reference_frame)}.diff.txt"
        expected_diff = unified_dom_diff(reference, candidate) + "\n"
        try:
            actual_diff = diff_path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{diff_path}: cannot read diff: {error}")
        else:
            if actual_diff != expected_diff:
                errors.append(f"{diff_path}: content does not match DOM artifacts")


def _validate_artifacts(
    output_dir: Path,
    summary: dict[str, Any],
) -> dict[str, Any]:
    expected, errors = _expected_artifact_files(summary)
    missing_paths = {
        path for path in expected if not (output_dir / path).is_file()
    }
    missing = sorted(str(path) for path in missing_paths)
    cases_root = output_dir / "cases"
    actual_paths = (
        {
            path.relative_to(output_dir)
            for path in cases_root.rglob("*")
            if path.is_file()
        }
        if cases_root.is_dir()
        else set()
    )
    unexpected = sorted(str(path) for path in actual_paths - expected)
    if not missing:
        for result in summary["results"]:
            _validate_case_artifacts(output_dir, result, errors)
    return {
        "ok": not errors and not missing and not unexpected,
        "expectedFiles": len(expected),
        "actualCaseFiles": len(actual_paths),
        "missing": missing[:100],
        "unexpected": unexpected[:100],
        "errors": errors[:100],
    }


def _iter_frame_pairs(
    output_dir: Path,
    summary: dict[str, Any],
) -> Iterable[
    tuple[dict[str, Any], str, dict[str, Any], dict[str, Any]]
]:
    for result in summary["results"]:
        artifact, path_error = _artifact_relative_path(result)
        chromium = result.get("chromium") or {}
        moli = result.get("moli") or {}
        if artifact is None or path_error or not moli:
            continue
        frames_dir = output_dir / artifact / "frames"
        for reference_frame, candidate_frame in zip(
            chromium.get("frames") or [],
            moli.get("frames") or [],
        ):
            if (
                reference_frame.get("name") != candidate_frame.get("name")
                or not reference_frame.get("dom_hash")
                or not candidate_frame.get("dom_hash")
            ):
                continue
            stem = _frame_stem(reference_frame)
            reference = json.loads(
                (frames_dir / f"{stem}.chromium.dom.json").read_text(
                    encoding="utf-8"
                )
            )
            candidate = json.loads(
                (frames_dir / f"{stem}.moli.dom.json").read_text(
                    encoding="utf-8"
                )
            )
            yield result, str(reference_frame["name"]), reference, candidate


def _projection_summary(
    records: list[
        tuple[dict[str, Any], str, dict[str, Any], dict[str, Any]]
    ],
    *,
    name: str,
    drop_whitespace: bool,
    drop_edges: frozenset[str],
    known_matches: list[tuple[dict[str, Any], str]] | None = None,
) -> dict[str, Any]:
    known_matches = known_matches or []
    mismatched_cases: set[str] = set()
    framework_totals: Counter[str] = Counter()
    framework_mismatches: Counter[str] = Counter()
    frame_totals: Counter[str] = Counter()
    frame_mismatches: Counter[str] = Counter()
    kinds: Counter[str] = Counter()
    mismatch_details: dict[str, dict[str, Any]] = {}
    mismatch_frames = 0
    for result, frame_name in known_matches:
        framework = str(result["framework"])
        framework_totals[framework] += 1
        frame_totals[frame_name] += 1
    for result, frame_name, reference, candidate in records:
        framework = str(result["framework"])
        framework_totals[framework] += 1
        frame_totals[frame_name] += 1
        difference = first_difference(
            _project_dom(
                reference,
                drop_whitespace=drop_whitespace,
                drop_edges=drop_edges,
            ),
            _project_dom(
                candidate,
                drop_whitespace=drop_whitespace,
                drop_edges=drop_edges,
            ),
        )
        if difference is None:
            continue
        mismatch_frames += 1
        mismatched_cases.add(str(result["id"]))
        framework_mismatches[framework] += 1
        frame_mismatches[frame_name] += 1
        kinds[_difference_kind(difference)] += 1
        detail = mismatch_details.setdefault(
            str(result["id"]),
            {
                "id": str(result["id"]),
                "framework": framework,
                "frames": [],
            },
        )
        detail["frames"].append(
            {
                "name": frame_name,
                "firstDifference": difference,
            }
        )
    case_ids = {
        str(result["id"]) for result, *_rest in records
    } | {
        str(result["id"]) for result, _frame_name in known_matches
    }
    total_frames = len(records) + len(known_matches)
    return {
        "name": name,
        "gating": False,
        "canonicalizedDoctypeCase": True,
        "droppedWhitespaceOnlyText": drop_whitespace,
        "droppedEdges": sorted(drop_edges),
        "frames": {
            "total": total_frames,
            "match": total_frames - mismatch_frames,
            "mismatch": mismatch_frames,
        },
        "cases": {
            "total": len(case_ids),
            "allFramesMatch": len(case_ids) - len(mismatched_cases),
            "anyMismatch": len(mismatched_cases),
        },
        "byFramework": {
            framework: {
                "totalFrames": framework_totals[framework],
                "mismatchFrames": framework_mismatches[framework],
            }
            for framework in sorted(framework_totals)
        },
        "byFrameName": {
            frame_name: {
                "total": frame_totals[frame_name],
                "mismatch": frame_mismatches[frame_name],
            }
            for frame_name in sorted(frame_totals)
        },
        "firstDifferenceKinds": dict(kinds.most_common()),
        "mismatchCases": [
            mismatch_details[case_id] for case_id in sorted(mismatch_details)
        ],
    }


def _edge_counts(node: dict[str, Any]) -> Counter[str]:
    counts: Counter[str] = Counter()
    for edge in EDGE_FIELDS:
        value = node.get(edge)
        if isinstance(value, list):
            roots = [item for item in value if isinstance(item, dict)]
        elif isinstance(value, dict):
            roots = [value]
        else:
            roots = []
        if edge in _EXTENDED_EDGES:
            counts[edge] += len(roots)
        for root in roots:
            counts.update(_edge_counts(root))
    return counts


def _edge_inventory(
    records: list[
        tuple[dict[str, Any], str, dict[str, Any], dict[str, Any]]
    ],
) -> dict[str, Any]:
    totals = {"chromium": Counter(), "moli": Counter()}
    frames_with = {"chromium": Counter(), "moli": Counter()}
    for _result, _name, chromium, moli in records:
        for engine, dom in (("chromium", chromium), ("moli", moli)):
            counts = _edge_counts(dom)
            totals[engine].update(counts)
            for edge in _EXTENDED_EDGES:
                if counts[edge]:
                    frames_with[engine][edge] += 1
    return {
        engine: {
            "edgeRootsAcrossFrames": {
                edge: totals[engine][edge] for edge in _EXTENDED_EDGES
            },
            "framesContainingEdge": {
                edge: frames_with[engine][edge] for edge in _EXTENDED_EDGES
            },
        }
        for engine in ("chromium", "moli")
    }


def _known_strict_match_frames(
    summary: dict[str, Any],
) -> list[tuple[dict[str, Any], str]]:
    matches = []
    for result in summary["results"]:
        if result.get("status") != "match":
            continue
        chromium_frames = (result.get("chromium") or {}).get("frames") or []
        moli_frames = (result.get("moli") or {}).get("frames") or []
        for reference_frame, candidate_frame in zip(
            chromium_frames, moli_frames
        ):
            if reference_frame.get("name") == candidate_frame.get("name"):
                matches.append((result, str(reference_frame["name"])))
    return matches


def _diagnostics_summary(summary: dict[str, Any]) -> dict[str, Any]:
    report: dict[str, Any] = {}
    for engine in ("chromium", "moli"):
        kinds: Counter[str] = Counter()
        cases = 0
        expected_cases = 0
        for result in summary["results"]:
            observation = result.get(engine)
            if not isinstance(observation, dict):
                continue
            diagnostics = observation.get("diagnostics") or {}
            case_count = 0
            for key in (
                "exceptions",
                "consoleErrors",
                "networkFailures",
                "httpErrors",
            ):
                count = len(diagnostics.get(key) or [])
                kinds[key] += count
                case_count += count
            expected_count = len(diagnostics.get("expectedNetworkFailures") or [])
            kinds["expectedNetworkFailures"] += expected_count
            expected_cases += int(expected_count > 0)
            cases += int(case_count > 0)
        process_diagnostics = (
            ((summary.get("engines") or {}).get(engine) or {}).get(
                "processDiagnostics"
            )
            or {}
        )
        report[engine] = {
            "casesWithDiagnostics": cases,
            "casesWithExpectedDiagnostics": expected_cases,
            **{key: kinds[key] for key in (
                "exceptions",
                "consoleErrors",
                "networkFailures",
                "httpErrors",
                "expectedNetworkFailures",
            )},
            "processErrors": len(process_diagnostics.get("unexpectedErrors") or []),
        }
    return report


def _transition_summary(summary: dict[str, Any]) -> dict[str, Any]:
    report: dict[str, Any] = {}
    for engine in ("chromium", "moli"):
        ordinary = []
        gallery = []
        animation = []
        boundary = []
        platform = []
        for result in summary["results"]:
            observation = result.get(engine)
            if not isinstance(observation, dict) or not observation.get("ok"):
                continue
            frames = observation.get("frames") or []
            frame_names = tuple(frame.get("name") for frame in frames)
            if frame_names == _ORDINARY_FRAME_NAMES:
                ordinary.append(frames)
            elif frame_names == _GALLERY_FRAME_NAMES:
                gallery.append(frames)
            elif frame_names == _ANIMATION_FRAME_NAMES:
                animation.append(frames)
            elif frame_names == _BOUNDARY_FRAME_NAMES:
                boundary.append(frames)
            elif frame_names == _PLATFORM_FRAME_NAMES:
                platform.append(frames)
        report[engine] = {
            "ordinaryCases": len(ordinary),
            "documentToMountedChanged": sum(
                frames[0].get("dom_hash") != frames[1].get("dom_hash")
                for frames in ordinary
            ),
            "mountedToReadyChanged": sum(
                frames[1].get("dom_hash") != frames[2].get("dom_hash")
                for frames in ordinary
            ),
            "readyToSettledChanged": sum(
                frames[2].get("dom_hash") != frames[3].get("dom_hash")
                for frames in ordinary
            ),
            "galleryCases": len(gallery),
            "galleryDocumentToMountedChanged": sum(
                frames[0].get("dom_hash") != frames[1].get("dom_hash")
                for frames in gallery
            ),
            "galleryMountedToInteractionChanged": sum(
                frames[1].get("dom_hash") != frames[2].get("dom_hash")
                for frames in gallery
            ),
            "galleryInteractionToReadyChanged": sum(
                frames[2].get("dom_hash") != frames[3].get("dom_hash")
                for frames in gallery
            ),
            "galleryReadyToSettledChanged": sum(
                frames[3].get("dom_hash") != frames[4].get("dom_hash")
                for frames in gallery
            ),
            "animationCases": len(animation),
            "firstFourAnimationTransitionsChanged": sum(
                all(
                    frames[index].get("dom_hash")
                    != frames[index + 1].get("dom_hash")
                    for index in range(4)
                )
                for frames in animation
            ),
            "animationFrame3ToReadyStable": sum(
                frames[4].get("dom_hash") == frames[5].get("dom_hash")
                for frames in animation
            ),
            "animationReadyToSettledChanged": sum(
                frames[5].get("dom_hash") != frames[6].get("dom_hash")
                for frames in animation
            ),
            "boundaryCases": len(boundary),
            "boundaryDocumentToMountedChanged": sum(
                frames[0].get("dom_hash") != frames[1].get("dom_hash")
                for frames in boundary
            ),
            "boundaryMountedToFirstChanged": sum(
                frames[1].get("dom_hash") != frames[2].get("dom_hash")
                for frames in boundary
            ),
            "boundaryFirstToSecondChanged": sum(
                frames[2].get("dom_hash") != frames[3].get("dom_hash")
                for frames in boundary
            ),
            "boundarySecondToReadyChanged": sum(
                frames[3].get("dom_hash") != frames[4].get("dom_hash")
                for frames in boundary
            ),
            "boundaryReadyToSettledChanged": sum(
                frames[4].get("dom_hash") != frames[5].get("dom_hash")
                for frames in boundary
            ),
            "platformCases": len(platform),
            "platformDocumentToMountedChanged": sum(
                frames[0].get("dom_hash") != frames[1].get("dom_hash")
                for frames in platform
            ),
            "platformMountedToFirstChanged": sum(
                frames[1].get("dom_hash") != frames[2].get("dom_hash")
                for frames in platform
            ),
            "platformFirstToSecondChanged": sum(
                frames[2].get("dom_hash") != frames[3].get("dom_hash")
                for frames in platform
            ),
            "platformSecondToReadyChanged": sum(
                frames[3].get("dom_hash") != frames[4].get("dom_hash")
                for frames in platform
            ),
            "platformReadyToSettledChanged": sum(
                frames[4].get("dom_hash") != frames[5].get("dom_hash")
                for frames in platform
            ),
        }
    return report


def _without_harness_state(node: dict[str, Any]) -> dict[str, Any]:
    projected: dict[str, Any] = {}
    for key, value in node.items():
        if key == "attributes" and isinstance(value, list):
            attributes = [
                attribute
                for attribute in value
                if not (
                    isinstance(attribute, list)
                    and attribute
                    and attribute[0]
                    in {"data-frontend-smoke", "data-frontend-smoke-id"}
                )
            ]
            if attributes:
                projected[key] = attributes
        elif key in EDGE_FIELDS and isinstance(value, list):
            projected[key] = [
                _without_harness_state(child)
                for child in value
                if isinstance(child, dict)
            ]
        elif key in EDGE_FIELDS and isinstance(value, dict):
            projected[key] = _without_harness_state(value)
        else:
            projected[key] = value
    return projected


def _transition_metrics(
    timelines: list[list[tuple[str, str]]],
) -> dict[str, Any]:
    ordinary = [
        frames
        for frames in timelines
        if tuple(name for name, _hash in frames) == _ORDINARY_FRAME_NAMES
    ]
    gallery = [
        frames
        for frames in timelines
        if tuple(name for name, _hash in frames) == _GALLERY_FRAME_NAMES
    ]
    animation = [
        frames
        for frames in timelines
        if tuple(name for name, _hash in frames) == _ANIMATION_FRAME_NAMES
    ]
    boundary = [
        frames
        for frames in timelines
        if tuple(name for name, _hash in frames) == _BOUNDARY_FRAME_NAMES
    ]
    platform = [
        frames
        for frames in timelines
        if tuple(name for name, _hash in frames) == _PLATFORM_FRAME_NAMES
    ]
    return {
        "artifactCases": len(timelines),
        "ordinaryCases": len(ordinary),
        "documentToMountedChanged": sum(
            frames[0][1] != frames[1][1] for frames in ordinary
        ),
        "mountedToReadyChanged": sum(
            frames[1][1] != frames[2][1] for frames in ordinary
        ),
        "readyToSettledChanged": sum(
            frames[2][1] != frames[3][1] for frames in ordinary
        ),
        "galleryCases": len(gallery),
        "galleryDocumentToMountedChanged": sum(
            frames[0][1] != frames[1][1] for frames in gallery
        ),
        "galleryMountedToInteractionChanged": sum(
            frames[1][1] != frames[2][1] for frames in gallery
        ),
        "galleryInteractionToReadyChanged": sum(
            frames[2][1] != frames[3][1] for frames in gallery
        ),
        "galleryReadyToSettledChanged": sum(
            frames[3][1] != frames[4][1] for frames in gallery
        ),
        "animationCases": len(animation),
        "firstFourAnimationTransitionsChanged": sum(
            all(frames[index][1] != frames[index + 1][1] for index in range(4))
            for frames in animation
        ),
        "animationFrame3ToReadyStable": sum(
            frames[4][1] == frames[5][1] for frames in animation
        ),
        "animationReadyToSettledChanged": sum(
            frames[5][1] != frames[6][1] for frames in animation
        ),
        "boundaryCases": len(boundary),
        "boundaryDocumentToMountedChanged": sum(
            frames[0][1] != frames[1][1] for frames in boundary
        ),
        "boundaryMountedToFirstChanged": sum(
            frames[1][1] != frames[2][1] for frames in boundary
        ),
        "boundaryFirstToSecondChanged": sum(
            frames[2][1] != frames[3][1] for frames in boundary
        ),
        "boundarySecondToReadyChanged": sum(
            frames[3][1] != frames[4][1] for frames in boundary
        ),
        "boundaryReadyToSettledChanged": sum(
            frames[4][1] != frames[5][1] for frames in boundary
        ),
        "platformCases": len(platform),
        "platformDocumentToMountedChanged": sum(
            frames[0][1] != frames[1][1] for frames in platform
        ),
        "platformMountedToFirstChanged": sum(
            frames[1][1] != frames[2][1] for frames in platform
        ),
        "platformFirstToSecondChanged": sum(
            frames[2][1] != frames[3][1] for frames in platform
        ),
        "platformSecondToReadyChanged": sum(
            frames[3][1] != frames[4][1] for frames in platform
        ),
        "platformReadyToSettledChanged": sum(
            frames[4][1] != frames[5][1] for frames in platform
        ),
    }


def _content_transition_summary(
    records: list[
        tuple[dict[str, Any], str, dict[str, Any], dict[str, Any]]
    ],
) -> dict[str, Any]:
    grouped: dict[str, dict[str, list[tuple[str, str]]]] = {
        "chromium": {},
        "moli": {},
    }
    for result, frame_name, chromium, moli in records:
        case_id = str(result["id"])
        for engine, node in (
            ("chromium", chromium),
            ("moli", moli),
        ):
            grouped[engine].setdefault(case_id, []).append(
                (frame_name, dom_hash(_without_harness_state(node)))
            )
    return {
        engine: _transition_metrics(list(grouped[engine].values()))
        for engine in ("chromium", "moli")
    }


def analyze_results(value: str | Path) -> dict[str, Any]:
    summary_path, summary = _load_summary(value)
    output_dir = summary_path.parent
    artifacts = _validate_artifacts(output_dir, summary)
    records = list(_iter_frame_pairs(output_dir, summary)) if artifacts["ok"] else []
    known_matches = _known_strict_match_frames(summary)
    projections = (
        [
            _projection_summary(
                records,
                name="doctype-case-only",
                drop_whitespace=False,
                drop_edges=frozenset(),
                known_matches=known_matches,
            ),
            _projection_summary(
                records,
                name="author-dom-without-ua-extended-edges",
                drop_whitespace=False,
                drop_edges=frozenset({"pseudoElements", "shadowRoots"}),
                known_matches=known_matches,
            ),
        ]
        if records or known_matches
        else []
    )
    return {
        "analysisOk": artifacts["ok"],
        "runOk": bool(summary.get("ok")),
        "summary": str(summary_path),
        "commit": (summary.get("git") or {}).get("commit"),
        "dirty": (summary.get("git") or {}).get("dirty"),
        "manifest": summary.get("manifest"),
        "strict": {
            "counts": summary.get("counts"),
            "timeline": summary.get("timeline"),
            "diagnostics": _diagnostics_summary(summary),
            "instrumentedHashTransitions": _transition_summary(summary),
            "contentTransitionsExcludingHarnessAttributes": (
                _content_transition_summary(records) if records else {}
            ),
        },
        "artifacts": artifacts,
        "diagnosticProjections": projections,
        "extendedEdgeInventory": _edge_inventory(records) if records else {},
    }


def _observation_signature(observation: dict[str, Any]) -> dict[str, Any]:
    return {
        "ok": observation.get("ok"),
        "dom_hash": observation.get("dom_hash"),
        "node_count": observation.get("node_count"),
        "frames": [
            {
                key: frame.get(key)
                for key in ("index", "name", "token", "dom_hash", "node_count")
            }
            for frame in observation.get("frames") or []
        ],
        "expectedNetworkFailures": (
            (observation.get("diagnostics") or {}).get("expectedNetworkFailures")
            or []
        ),
    }


def _diagnostic_count(observation: dict[str, Any]) -> int:
    diagnostics = observation.get("diagnostics") or {}
    return sum(
        len(diagnostics.get(key) or [])
        for key in (
            "exceptions",
            "consoleErrors",
            "networkFailures",
            "httpErrors",
        )
    )


def _is_nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value)


def _is_node_count(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def _validate_observation_contract(
    *,
    case_id: str,
    observation: Any,
    prefix: str,
    engine: str,
) -> tuple[list[str], int]:
    errors: list[str] = []
    if not isinstance(observation, dict) or observation.get("ok") is not True:
        return [f"{prefix}: {engine} observation is not successful"], 0
    attempt_count = observation.get("attempt_count", 1)
    previous_failures = observation.get("previous_failures", [])
    if (
        not isinstance(attempt_count, int)
        or isinstance(attempt_count, bool)
        or attempt_count < 1
    ):
        errors.append(f"{prefix}: attempt count is invalid")
    if not isinstance(previous_failures, list):
        errors.append(f"{prefix}: previous failures is not a list")
    elif isinstance(attempt_count, int) and not isinstance(attempt_count, bool):
        if attempt_count != len(previous_failures) + 1:
            errors.append(f"{prefix}: attempt count does not match previous failures")
        for index, failure in enumerate(previous_failures, start=1):
            if (
                not isinstance(failure, dict)
                or failure.get("attempt") != index
                or failure.get("kind") != "infrastructure"
            ):
                errors.append(
                    f"{prefix}: previous failure[{index - 1}] is invalid"
                )
    if not _is_nonempty_string(observation.get("dom_hash")):
        errors.append(f"{prefix}: final DOM hash is missing")
    if not _is_node_count(observation.get("node_count")):
        errors.append(f"{prefix}: final node count is invalid")
    frames = observation.get("frames")
    if not isinstance(frames, list):
        return [*errors, f"{prefix}: frames is not a list"], 0
    frame_names = tuple(
        frame.get("name") if isinstance(frame, dict) else None for frame in frames
    )
    if frame_names not in {
        _ORDINARY_FRAME_NAMES,
        _GALLERY_FRAME_NAMES,
        _ANIMATION_FRAME_NAMES,
        _BOUNDARY_FRAME_NAMES,
        _PLATFORM_FRAME_NAMES,
    }:
        errors.append(f"{prefix}: frame-name sequence violates the timeline contract")
    for index, frame in enumerate(frames):
        frame_prefix = f"{prefix}: frame[{index}]"
        if not isinstance(frame, dict):
            errors.append(f"{frame_prefix}: frame is not an object")
            continue
        name = frame.get("name")
        expected_token = (
            f"{case_id}:settled"
            if name == "settled"
            else f"{case_id}:{index}:{name}"
        )
        if frame.get("index") != index:
            errors.append(f"{frame_prefix}: index does not match position")
        if not _is_nonempty_string(name):
            errors.append(f"{frame_prefix}: name is missing")
        if frame.get("token") != expected_token:
            errors.append(f"{frame_prefix}: token violates the handshake contract")
        if not _is_nonempty_string(frame.get("dom_hash")):
            errors.append(f"{frame_prefix}: DOM hash is missing")
        if not _is_node_count(frame.get("node_count")):
            errors.append(f"{frame_prefix}: node count is invalid")
    if frames:
        last_frame = frames[-1]
        if isinstance(last_frame, dict) and (
            last_frame.get("dom_hash") != observation.get("dom_hash")
            or last_frame.get("node_count") != observation.get("node_count")
        ):
            errors.append(f"{prefix}: settled frame does not equal final DOM")

    diagnostics = observation.get("diagnostics")
    if not isinstance(diagnostics, dict):
        errors.append(f"{prefix}: diagnostics is not an object")
    else:
        for key in (
            "exceptions",
            "consoleErrors",
            "networkFailures",
            "httpErrors",
        ):
            if diagnostics.get(key) != []:
                errors.append(f"{prefix}: {key} is not empty")
        expected_failures = diagnostics.get("expectedNetworkFailures", [])
        if not isinstance(expected_failures, list):
            errors.append(f"{prefix}: expectedNetworkFailures is not a list")
        missing_failures = diagnostics.get("missingExpectedNetworkFailures", [])
        if missing_failures != []:
            errors.append(f"{prefix}: missingExpectedNetworkFailures is not empty")
    return errors, len(frames)


def _validate_engine_metadata(
    summary: dict[str, Any],
    *,
    engine: str,
    label: str,
    require_binary_hash: bool,
) -> list[str]:
    errors: list[str] = []
    metadata = ((summary.get("engines") or {}).get(engine) or {})
    version = metadata.get("version") or {}
    for field in _VERSION_FIELDS:
        if not _is_nonempty_string(version.get(field)):
            errors.append(f"{label}: {engine} version field {field} is missing")
    if require_binary_hash and not _is_nonempty_string(metadata.get("sha256")):
        errors.append(f"{label}: {engine} binary hash is missing")
    return errors


def _validate_reference_summary(
    summary: dict[str, Any],
    *,
    label: str,
) -> list[str]:
    errors: list[str] = []
    results = summary.get("results")
    if not isinstance(results, list):
        return [f"{label}: results is not a list"]
    mode = summary.get("mode")
    if mode not in {"reference", "differential"}:
        errors.append(f"{label}: mode has no Chromium reference phase")
    gate = summary.get("referenceGate") or {}
    if gate.get("ok") is not True:
        errors.append(f"{label}: reference gate is not green")
    if gate.get("cases") != len(results):
        errors.append(
            f"{label}: reference gate case count {gate.get('cases')} "
            f"does not match {len(results)} results"
        )
    if gate.get("errors") != 0:
        errors.append(f"{label}: reference gate reports errors")
    expected_moli_phase = mode == "differential"
    if gate.get("moliPhaseStarted") is not expected_moli_phase:
        errors.append(
            f"{label}: Moli phase marker does not match {mode} mode"
        )

    manifest = summary.get("manifest") or {}
    if manifest.get("selectedCases") != len(results):
        errors.append(
            f"{label}: selected case count {manifest.get('selectedCases')} "
            f"does not match {len(results)} results"
        )
    if not _is_nonempty_string(manifest.get("sha256")):
        errors.append(f"{label}: manifest hash is missing")
    fixtures_hash = manifest.get("fixturesSha256")
    if not _is_nonempty_string(fixtures_hash):
        errors.append(f"{label}: fixture hash is missing")
    if manifest.get("verifiedFixturesSha256") != fixtures_hash:
        errors.append(f"{label}: verified fixture hash does not match manifest")

    ids: set[str] = set()
    frame_total = 0
    retried_cases = 0
    recovered_cases = 0
    for position, result in enumerate(results):
        case_id = result.get("id")
        prefix = f"{label}: result[{position}]"
        if not _is_nonempty_string(case_id):
            errors.append(f"{prefix}: case id is missing")
            continue
        if case_id in ids:
            errors.append(f"{prefix}: duplicate case id {case_id}")
        ids.add(case_id)
        status = result.get("status")
        if mode == "reference" and status != "reference_ok":
            errors.append(f"{prefix}: status is not reference_ok")
        if mode == "differential" and status in {
            "reference_error",
            "infrastructure_error",
            None,
        }:
            errors.append(f"{prefix}: status invalidates the Chromium reference phase")
        chromium_observation = result.get("chromium")
        if isinstance(chromium_observation, dict):
            attempt_count = chromium_observation.get("attempt_count", 1)
            if (
                isinstance(attempt_count, int)
                and not isinstance(attempt_count, bool)
                and attempt_count > 1
            ):
                retried_cases += 1
            previous_failures = chromium_observation.get("previous_failures", [])
            if (
                chromium_observation.get("ok") is True
                and isinstance(previous_failures, list)
                and previous_failures
            ):
                recovered_cases += 1
        observation_errors, observation_frames = _validate_observation_contract(
            case_id=case_id,
            observation=chromium_observation,
            prefix=prefix,
            engine="Chromium",
        )
        errors.extend(observation_errors)
        frame_total += observation_frames

    if "retriedCases" in gate and gate.get("retriedCases") != retried_cases:
        errors.append(f"{label}: retried case count does not match results")
    if "recoveredCases" in gate and gate.get("recoveredCases") != recovered_cases:
        errors.append(f"{label}: recovered case count does not match results")

    counts = summary.get("counts")
    if mode == "reference" and counts != {"reference_ok": len(results)}:
        errors.append(f"{label}: strict counts do not match reference results")
    if mode == "differential" and (
        not isinstance(counts, dict)
        or sum(
            count
            for count in counts.values()
            if isinstance(count, int) and not isinstance(count, bool)
        )
        != len(results)
        or counts.get("reference_error", 0)
        or counts.get("infrastructure_error", 0)
    ):
        errors.append(f"{label}: differential counts invalidate the reference phase")
    timeline = summary.get("timeline") or {}
    if timeline.get("chromiumFrames") != frame_total:
        errors.append(f"{label}: Chromium frame total does not match results")
    if mode == "reference" and timeline.get("moliFrames") != 0:
        errors.append(f"{label}: reference run contains Moli frames")
    if mode == "reference" and timeline.get("mismatchedFrames") != 0:
        errors.append(f"{label}: reference run contains mismatched frames")
    errors.extend(
        _validate_engine_metadata(
            summary,
            engine="chromium",
            label=label,
            require_binary_hash=False,
        )
    )
    return errors


def _validate_moli_summary(
    summary: dict[str, Any],
    *,
    label: str,
) -> list[str]:
    errors = _validate_reference_summary(summary, label=label)
    results = summary.get("results")
    if not isinstance(results, list):
        return errors
    if summary.get("mode") != "differential":
        errors.append(f"{label}: mode has no Moli candidate phase")
    frame_total = 0
    for position, result in enumerate(results):
        case_id = result.get("id")
        prefix = f"{label}: result[{position}]"
        if not _is_nonempty_string(case_id):
            continue
        if result.get("status") not in {
            "match",
            "dom_mismatch",
            "diagnostic_mismatch",
        }:
            errors.append(f"{prefix}: status invalidates the Moli phase")
        observation_errors, observation_frames = _validate_observation_contract(
            case_id=case_id,
            observation=result.get("moli"),
            prefix=prefix,
            engine="Moli",
        )
        errors.extend(observation_errors)
        frame_total += observation_frames
    timeline = summary.get("timeline") or {}
    if timeline.get("moliFrames") != frame_total:
        errors.append(f"{label}: Moli frame total does not match results")
    counts = summary.get("counts")
    if (
        not isinstance(counts, dict)
        or sum(
            count
            for count in counts.values()
            if isinstance(count, int) and not isinstance(count, bool)
        )
        != len(results)
        or any(
            counts.get(status, 0)
            for status in (
                "reference_error",
                "moli_error",
                "infrastructure_error",
            )
        )
    ):
        errors.append(f"{label}: strict counts invalidate the Moli phase")
    errors.extend(
        _validate_engine_metadata(
            summary,
            engine="moli",
            label=label,
            require_binary_hash=True,
        )
    )
    return errors


def _compare_engine_data(
    first: dict[str, Any],
    second: dict[str, Any],
    *,
    engine: str,
) -> dict[str, Any]:
    first_results = first["results"]
    second_results = second["results"]
    first_ids = [str(result.get("id")) for result in first_results]
    second_ids = [str(result.get("id")) for result in second_results]
    case_ids_equal = first_ids == second_ids
    different_cases: list[dict[str, Any]] = []
    if case_ids_equal:
        for first_result, second_result in zip(
            first_results, second_results, strict=True
        ):
            first_observation = first_result.get(engine) or {}
            second_observation = second_result.get(engine) or {}
            if _observation_signature(first_observation) != _observation_signature(
                second_observation
            ):
                different_cases.append(
                    {
                        "id": first_result.get("id"),
                        "firstFrames": len(first_observation.get("frames") or []),
                        "secondFrames": len(second_observation.get("frames") or []),
                    }
                )
    else:
        different_cases.append(
            {
                "id": "case-list",
                "firstCases": len(first_ids),
                "secondCases": len(second_ids),
            }
        )
    first_manifest = first.get("manifest") or {}
    second_manifest = second.get("manifest") or {}
    manifest_signature_keys = (
        "sha256",
        "fixturesSha256",
        "verifiedFixturesSha256",
        "selectedCases",
    )
    manifests_equal = all(
        first_manifest.get(key) == second_manifest.get(key)
        for key in manifest_signature_keys
    )
    first_engine = (first.get("engines") or {}).get(engine) or {}
    second_engine = (second.get("engines") or {}).get(engine) or {}
    first_version = first_engine.get("version") or {}
    second_version = second_engine.get("version") or {}
    versions_equal = all(
        first_version.get(key) == second_version.get(key)
        for key in _VERSION_FIELDS
    )
    binary_hashes_equal = (
        engine != "moli"
        or (
            _is_nonempty_string(first_engine.get("sha256"))
            and first_engine.get("sha256") == second_engine.get("sha256")
        )
    )
    first_diagnostics = sum(
        _diagnostic_count(result.get(engine) or {})
        for result in first_results
    )
    second_diagnostics = sum(
        _diagnostic_count(result.get(engine) or {})
        for result in second_results
    )
    gates_ok = bool((first.get("referenceGate") or {}).get("ok")) and bool(
        (second.get("referenceGate") or {}).get("ok")
    )
    validator = (
        _validate_reference_summary
        if engine == "chromium"
        else _validate_moli_summary
    )
    first_validation_errors = validator(first, label="first")
    second_validation_errors = validator(second, label="second")
    ok = (
        gates_ok
        and manifests_equal
        and versions_equal
        and binary_hashes_equal
        and case_ids_equal
        and not different_cases
        and first_diagnostics == 0
        and second_diagnostics == 0
        and not first_validation_errors
        and not second_validation_errors
    )
    return {
        "ok": ok,
        "engine": engine,
        "referenceGatesOk": gates_ok,
        "manifestsEqual": manifests_equal,
        "versionsEqual": versions_equal,
        "binaryHashesEqual": binary_hashes_equal,
        "caseIdsEqual": case_ids_equal,
        "firstCases": len(first_results),
        "secondCases": len(second_results),
        "firstFrames": sum(
            len((result.get(engine) or {}).get("frames") or [])
            for result in first_results
        ),
        "secondFrames": sum(
            len((result.get(engine) or {}).get("frames") or [])
            for result in second_results
        ),
        "firstDiagnostics": first_diagnostics,
        "secondDiagnostics": second_diagnostics,
        "firstValidationErrors": first_validation_errors[:100],
        "secondValidationErrors": second_validation_errors[:100],
        "differentCases": different_cases[:100],
    }


def compare_reference_data(
    first: dict[str, Any],
    second: dict[str, Any],
) -> dict[str, Any]:
    return _compare_engine_data(first, second, engine="chromium")


def compare_engine_data(
    first: dict[str, Any],
    second: dict[str, Any],
    *,
    engine: str,
) -> dict[str, Any]:
    result = _compare_engine_data(first, second, engine=engine)
    if engine == "moli":
        reference = compare_reference_data(first, second)
        result["chromiumReferenceStable"] = reference["ok"]
        result["chromiumReferenceDifferentCases"] = reference["differentCases"]
        result["chromiumReferenceValidationErrors"] = {
            "first": reference["firstValidationErrors"],
            "second": reference["secondValidationErrors"],
        }
        result["ok"] = bool(result["ok"] and reference["ok"])
    return result


def compare_references(
    first_value: str | Path,
    second_value: str | Path,
) -> dict[str, Any]:
    first_path, first = _load_summary(first_value)
    second_path, second = _load_summary(second_value)
    result = compare_reference_data(first, second)
    result["first"] = str(first_path)
    result["second"] = str(second_path)
    return result


def compare_engine_runs(
    engine: str,
    first_value: str | Path,
    second_value: str | Path,
) -> dict[str, Any]:
    first_path, first = _load_summary(first_value)
    second_path, second = _load_summary(second_value)
    result = compare_engine_data(first, second, engine=engine)
    result["first"] = str(first_path)
    result["second"] = str(second_path)
    return result


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Validate frontend-smoke artifacts, produce non-gating diagnostic "
            "DOM projections, or compare repeated engine timelines."
        )
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    results = subparsers.add_parser(
        "results",
        help="Validate and analyze one differential summary/artifact directory.",
    )
    results.add_argument("summary", help="summary.json or its output directory.")
    stability = subparsers.add_parser(
        "reference-stability",
        help="Require two Chromium reference summaries to have identical frame signatures.",
    )
    stability.add_argument("first", help="First summary.json or output directory.")
    stability.add_argument("second", help="Second summary.json or output directory.")
    engine_stability = subparsers.add_parser(
        "engine-stability",
        help=(
            "Require two runs to have identical Chromium or Moli frame "
            "signatures; Moli comparison also requires a stable Chromium oracle."
        ),
    )
    engine_stability.add_argument(
        "engine",
        choices=("chromium", "moli"),
        help="Engine timeline to compare.",
    )
    engine_stability.add_argument(
        "first",
        help="First summary.json or output directory.",
    )
    engine_stability.add_argument(
        "second",
        help="Second summary.json or output directory.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> None:
    args = parse_args(argv)
    try:
        if args.command == "results":
            result = analyze_results(args.summary)
            code = 0 if result["analysisOk"] else 1
        elif args.command == "reference-stability":
            result = compare_references(args.first, args.second)
            code = 0 if result["ok"] else 1
        else:
            result = compare_engine_runs(args.engine, args.first, args.second)
            code = 0 if result["ok"] else 1
        print(json.dumps(result, indent=2, ensure_ascii=False))
        raise SystemExit(code)
    except SystemExit:
        raise
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
        raise SystemExit(2) from error
