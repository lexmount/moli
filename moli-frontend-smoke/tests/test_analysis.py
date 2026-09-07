from __future__ import annotations

import json
from copy import deepcopy

from moli_frontend_smoke.analysis import (
    _content_transition_summary,
    _diagnostics_summary,
    _project_dom,
    _projection_summary,
    _transition_summary,
    _validate_artifacts,
    compare_engine_data,
    compare_reference_data,
)
from moli_frontend_smoke.dom import dom_hash, unified_dom_diff


def _reference_summary() -> dict[str, object]:
    return {
        "mode": "reference",
        "referenceGate": {
            "ok": True,
            "cases": 1,
            "errors": 0,
            "moliPhaseStarted": False,
        },
        "manifest": {
            "sha256": "manifest",
            "fixturesSha256": "fixtures",
            "verifiedFixturesSha256": "fixtures",
            "selectedCases": 1,
        },
        "counts": {"reference_ok": 1},
        "timeline": {
            "chromiumFrames": 4,
            "moliFrames": 0,
            "mismatchedFrames": 0,
        },
        "engines": {
            "chromium": {
                "version": {
                    "Browser": "Chrome/145",
                    "Protocol-Version": "1.3",
                    "User-Agent": "Chromium",
                    "V8-Version": "14.5",
                    "WebKit-Version": "537.36",
                    "webSocketDebuggerUrl": "ws://random/first",
                }
            }
        },
        "results": [
            {
                "id": "react/family/case",
                "status": "reference_ok",
                "chromium": {
                    "ok": True,
                    "dom_hash": "settled",
                    "node_count": 3,
                    "diagnostics": {
                        "exceptions": [],
                        "consoleErrors": [],
                        "networkFailures": [],
                        "httpErrors": [],
                    },
                    "frames": [
                        {
                            "index": 0,
                            "name": "document",
                            "token": "react/family/case:0:document",
                            "dom_hash": "document",
                            "node_count": 2,
                        },
                        {
                            "index": 1,
                            "name": "mounted",
                            "token": "react/family/case:1:mounted",
                            "dom_hash": "mounted",
                            "node_count": 3,
                        },
                        {
                            "index": 2,
                            "name": "ready",
                            "token": "react/family/case:2:ready",
                            "dom_hash": "ready",
                            "node_count": 3,
                        },
                        {
                            "index": 3,
                            "name": "settled",
                            "token": "react/family/case:settled",
                            "dom_hash": "settled",
                            "node_count": 3,
                        },
                    ],
                },
            }
        ],
    }


def test_diagnostics_summary_includes_process_errors() -> None:
    summary = _reference_summary()
    summary["engines"]["chromium"]["processDiagnostics"] = {  # type: ignore[index]
        "unexpectedErrors": []
    }
    summary["engines"]["moli"] = {  # type: ignore[index]
        "processDiagnostics": {
            "unexpectedErrors": ["first process error", "second process error"]
        }
    }

    diagnostics = _diagnostics_summary(summary)  # type: ignore[arg-type]

    assert diagnostics["chromium"]["processErrors"] == 0
    assert diagnostics["moli"]["processErrors"] == 2


def test_reference_stability_ignores_endpoint_but_requires_every_frame_hash() -> None:
    first = _reference_summary()
    second = deepcopy(first)
    second["engines"]["chromium"]["version"]["webSocketDebuggerUrl"] = (  # type: ignore[index]
        "ws://random/second"
    )

    assert compare_reference_data(first, second)["ok"]  # type: ignore[arg-type]

    first["results"][0]["chromium"]["frames"][0]["dom_hash"] = None  # type: ignore[index]
    second["results"][0]["chromium"]["frames"][0]["dom_hash"] = None  # type: ignore[index]
    result = compare_reference_data(first, second)  # type: ignore[arg-type]
    assert not result["ok"]
    assert result["differentCases"] == []
    assert result["firstValidationErrors"] == [
        "first: result[0]: frame[0]: DOM hash is missing"
    ]
    assert result["secondValidationErrors"] == [
        "second: result[0]: frame[0]: DOM hash is missing"
    ]


def test_reference_contract_preserves_recovered_infrastructure_failures() -> None:
    first = _reference_summary()
    first["referenceGate"].update(  # type: ignore[union-attr]
        {
            "infrastructureRetryLimit": 1,
            "retriedCases": 1,
            "recoveredCases": 1,
        }
    )
    first["results"][0]["chromium"].update(  # type: ignore[index,union-attr]
        {
            "attempt_count": 2,
            "previous_failures": [
                {
                    "attempt": 1,
                    "kind": "infrastructure",
                    "duration_ms": 1.0,
                    "error_type": "CdpCommandError",
                    "error": "Target.createBrowserContext failed: unknown",
                }
            ],
        }
    )
    second = deepcopy(first)

    assert compare_reference_data(first, second)["ok"]  # type: ignore[arg-type]

    second["referenceGate"]["recoveredCases"] = 0  # type: ignore[index]
    result = compare_reference_data(first, second)  # type: ignore[arg-type]
    assert not result["ok"]
    assert result["secondValidationErrors"] == [
        "second: recovered case count does not match results"
    ]


def test_projection_is_explicitly_non_gating_and_can_isolate_ua_edges() -> None:
    case = {"id": "react/family/case", "framework": "react"}
    chromium = {
        "nodeType": 9,
        "nodeName": "#document",
        "nodeValue": "",
        "children": [
            {"nodeType": 10, "nodeName": "html", "nodeValue": ""},
            {
                "nodeType": 1,
                "nodeName": "HTML",
                "nodeValue": "",
                "children": [
                    {"nodeType": 3, "nodeName": "#text", "nodeValue": "\n  "},
                    {
                        "nodeType": 1,
                        "nodeName": "OL",
                        "nodeValue": "",
                        "pseudoElements": [
                            {
                                "nodeType": 1,
                                "nodeName": "::marker",
                                "nodeValue": "",
                            }
                        ],
                    },
                ],
            },
        ],
    }
    moli = {
        "nodeType": 9,
        "nodeName": "#document",
        "nodeValue": "",
        "children": [
            {"nodeType": 10, "nodeName": "HTML", "nodeValue": ""},
            {
                "nodeType": 1,
                "nodeName": "HTML",
                "nodeValue": "",
                "children": [
                    {
                        "nodeType": 1,
                        "nodeName": "OL",
                        "nodeValue": "",
                    }
                ],
            },
        ],
    }
    records = [(case, "mounted", chromium, moli)]

    extended = _projection_summary(
        records,
        name="extended",
        drop_whitespace=True,
        drop_edges=frozenset(),
    )
    author = _projection_summary(
        records,
        name="author",
        drop_whitespace=True,
        drop_edges=frozenset({"pseudoElements", "shadowRoots"}),
    )

    assert extended["gating"] is False
    assert extended["frames"]["mismatch"] == 1
    assert extended["firstDifferenceKinds"] == {"pseudoElements": 1}
    assert extended["mismatchCases"] == [
        {
            "id": "react/family/case",
            "framework": "react",
            "frames": [
                {
                    "name": "mounted",
                    "firstDifference": "$.children[1].children[0].pseudoElements",
                }
            ],
        }
    ]
    assert author["frames"] == {"total": 1, "match": 1, "mismatch": 0}
    assert author["mismatchCases"] == []


def test_reference_stability_accepts_a_clean_differential_reference_phase() -> None:
    reference = _reference_summary()
    differential = deepcopy(reference)
    differential["mode"] = "differential"
    differential["referenceGate"]["moliPhaseStarted"] = True  # type: ignore[index]
    differential["counts"] = {"dom_mismatch": 1}
    differential["timeline"]["moliFrames"] = 4  # type: ignore[index]
    differential["timeline"]["mismatchedFrames"] = 4  # type: ignore[index]
    differential["results"][0]["status"] = "dom_mismatch"  # type: ignore[index]

    result = compare_reference_data(reference, differential)  # type: ignore[arg-type]

    assert result["ok"]
    assert result["firstValidationErrors"] == []
    assert result["secondValidationErrors"] == []


def test_content_transitions_exclude_harness_phase_attributes() -> None:
    case = {"id": "react/family/case", "framework": "react"}

    def dom(phase: str, content: str) -> dict[str, object]:
        return {
            "nodeType": 9,
            "nodeName": "#document",
            "nodeValue": "",
            "children": [
                {
                    "nodeType": 1,
                    "nodeName": "HTML",
                    "nodeValue": "",
                    "attributes": [
                        ["data-frontend-smoke", phase],
                        ["data-frontend-smoke-id", "react/family/case"],
                    ],
                    "children": [
                        {
                            "nodeType": 3,
                            "nodeName": "#text",
                            "nodeValue": content,
                        }
                    ],
                }
            ],
        }

    records = [
        (case, "document", dom("checkpoint", "document"), dom("checkpoint", "document")),
        (case, "mounted", dom("checkpoint", "mounted"), dom("checkpoint", "mounted")),
        (case, "ready", dom("checkpoint", "ready"), dom("checkpoint", "ready")),
        (case, "settled", dom("ready", "ready"), dom("ready", "ready")),
    ]

    transitions = _content_transition_summary(records)  # type: ignore[arg-type]

    assert transitions["chromium"]["documentToMountedChanged"] == 1
    assert transitions["chromium"]["mountedToReadyChanged"] == 1
    assert transitions["chromium"]["readyToSettledChanged"] == 0
    assert transitions["moli"] == transitions["chromium"]


def test_gallery_transition_summary_tracks_the_intermediate_interaction() -> None:
    frames = [
        {"name": "document", "dom_hash": "document"},
        {"name": "mounted", "dom_hash": "mounted"},
        {"name": "interaction-1", "dom_hash": "interaction"},
        {"name": "ready", "dom_hash": "ready"},
        {"name": "settled", "dom_hash": "settled"},
    ]
    summary = {
        "results": [
            {
                "chromium": {"ok": True, "frames": frames},
                "moli": {"ok": True, "frames": deepcopy(frames)},
            }
        ]
    }

    transitions = _transition_summary(summary)  # type: ignore[arg-type]

    for engine in ("chromium", "moli"):
        assert transitions[engine]["galleryCases"] == 1
        assert transitions[engine]["galleryDocumentToMountedChanged"] == 1
        assert transitions[engine]["galleryMountedToInteractionChanged"] == 1
        assert transitions[engine]["galleryInteractionToReadyChanged"] == 1
        assert transitions[engine]["galleryReadyToSettledChanged"] == 1


def test_boundary_transition_summary_tracks_both_browser_boundaries() -> None:
    frames = [
        {"name": "document", "dom_hash": "document"},
        {"name": "mounted", "dom_hash": "mounted"},
        {"name": "boundary-1", "dom_hash": "boundary-1"},
        {"name": "boundary-2", "dom_hash": "boundary-2"},
        {"name": "ready", "dom_hash": "ready"},
        {"name": "settled", "dom_hash": "settled"},
    ]
    summary = {
        "results": [
            {
                "chromium": {"ok": True, "frames": frames},
                "moli": {"ok": True, "frames": deepcopy(frames)},
            }
        ]
    }

    transitions = _transition_summary(summary)  # type: ignore[arg-type]

    for engine in ("chromium", "moli"):
        assert transitions[engine]["boundaryCases"] == 1
        assert transitions[engine]["boundaryDocumentToMountedChanged"] == 1
        assert transitions[engine]["boundaryMountedToFirstChanged"] == 1
        assert transitions[engine]["boundaryFirstToSecondChanged"] == 1
        assert transitions[engine]["boundarySecondToReadyChanged"] == 1
        assert transitions[engine]["boundaryReadyToSettledChanged"] == 1


def test_platform_transition_summary_tracks_network_and_storage_boundaries() -> None:
    frames = [
        {"name": "document", "dom_hash": "document"},
        {"name": "mounted", "dom_hash": "mounted"},
        {"name": "platform-1", "dom_hash": "platform-1"},
        {"name": "platform-2", "dom_hash": "platform-2"},
        {"name": "ready", "dom_hash": "ready"},
        {"name": "settled", "dom_hash": "settled"},
    ]
    summary = {
        "results": [
            {
                "chromium": {"ok": True, "frames": frames},
                "moli": {"ok": True, "frames": deepcopy(frames)},
            }
        ]
    }

    transitions = _transition_summary(summary)  # type: ignore[arg-type]

    for engine in ("chromium", "moli"):
        assert transitions[engine]["platformCases"] == 1
        assert transitions[engine]["platformDocumentToMountedChanged"] == 1
        assert transitions[engine]["platformMountedToFirstChanged"] == 1
        assert transitions[engine]["platformFirstToSecondChanged"] == 1
        assert transitions[engine]["platformSecondToReadyChanged"] == 1
        assert transitions[engine]["platformReadyToSettledChanged"] == 1


def test_moli_stability_also_requires_a_stable_chromium_oracle() -> None:
    first = _reference_summary()
    first["mode"] = "differential"
    first["referenceGate"]["moliPhaseStarted"] = True  # type: ignore[index]
    first["counts"] = {"dom_mismatch": 1}
    first["timeline"]["moliFrames"] = 4  # type: ignore[index]
    first["timeline"]["mismatchedFrames"] = 4  # type: ignore[index]
    first["engines"]["moli"] = {  # type: ignore[index]
        "sha256": "moli-binary",
        "version": deepcopy(first["engines"]["chromium"]["version"]),  # type: ignore[index]
    }
    first["results"][0]["status"] = "dom_mismatch"  # type: ignore[index]
    first["results"][0]["moli"] = deepcopy(  # type: ignore[index]
        first["results"][0]["chromium"]  # type: ignore[index]
    )
    second = deepcopy(first)

    assert compare_engine_data(  # type: ignore[arg-type]
        first,
        second,
        engine="moli",
    )["ok"]

    second["results"][0]["chromium"]["frames"][1]["dom_hash"] = "oracle-drift"  # type: ignore[index]
    result = compare_engine_data(first, second, engine="moli")  # type: ignore[arg-type]

    assert not result["ok"]
    assert not result["chromiumReferenceStable"]
    assert result["chromiumReferenceDifferentCases"] == [
        {
            "id": "react/family/case",
            "firstFrames": 4,
            "secondFrames": 4,
        }
    ]


def test_dom_projection_canonicalizes_doctype_and_removes_only_whitespace_text() -> None:
    projected = _project_dom(
        {
            "nodeType": 9,
            "nodeName": "#document",
            "nodeValue": "",
            "children": [
                {"nodeType": 10, "nodeName": "HTML", "nodeValue": ""},
                {"nodeType": 3, "nodeName": "#text", "nodeValue": " \n "},
                {"nodeType": 3, "nodeName": "#text", "nodeValue": "kept"},
            ],
        },
        drop_whitespace=True,
        drop_edges=frozenset(),
    )

    assert projected["children"] == [
        {"nodeType": 10, "nodeName": "html", "nodeValue": ""},
        {"nodeType": 3, "nodeName": "#text", "nodeValue": "kept"},
    ]


def test_artifact_validation_reports_every_missing_required_file(tmp_path) -> None:
    summary = {
        "results": [
            {
                "id": "react/family/case",
                "status": "dom_mismatch",
                "artifact": "cases/react/family/case",
                "mismatchedFrames": ["document"],
                "chromium": {
                    "dom_hash": "reference-final",
                    "frames": [
                        {
                            "index": 0,
                            "name": "document",
                            "dom_hash": "reference-frame",
                        }
                    ],
                },
                "moli": {
                    "dom_hash": "candidate-final",
                    "frames": [
                        {
                            "index": 0,
                            "name": "document",
                            "dom_hash": "candidate-frame",
                        }
                    ],
                },
            }
        ]
    }

    result = _validate_artifacts(tmp_path, summary)

    assert not result["ok"]
    assert result["expectedFiles"] == 8
    assert len(result["missing"]) == 8


def test_artifact_validation_recomputes_dom_hash_count_and_diff(tmp_path) -> None:
    chromium_dom = {
        "nodeType": 9,
        "nodeName": "#document",
        "nodeValue": "",
        "children": [{"nodeType": 10, "nodeName": "html", "nodeValue": ""}],
    }
    moli_dom = {
        "nodeType": 9,
        "nodeName": "#document",
        "nodeValue": "",
        "children": [{"nodeType": 10, "nodeName": "HTML", "nodeValue": ""}],
    }
    chromium_frame = {
        "index": 0,
        "name": "document",
        "token": "react/family/case:0:document",
        "dom_hash": dom_hash(chromium_dom),
        "node_count": 2,
    }
    moli_frame = {
        **chromium_frame,
        "dom_hash": dom_hash(moli_dom),
    }
    result = {
        "id": "react/family/case",
        "status": "dom_mismatch",
        "artifact": "cases/react/family/case",
        "firstDifference": "$.frames[0].dom.children[0].nodeName",
        "mismatchedFrames": ["document"],
        "chromium": {
            "ok": True,
            "dom_hash": dom_hash(chromium_dom),
            "node_count": 2,
            "frames": [chromium_frame],
        },
        "moli": {
            "ok": True,
            "dom_hash": dom_hash(moli_dom),
            "node_count": 2,
            "frames": [moli_frame],
        },
    }
    summary = {"results": [result]}
    case_dir = tmp_path / "cases/react/family/case"
    frames_dir = case_dir / "frames"
    frames_dir.mkdir(parents=True)

    def write_json(path, value) -> None:
        path.write_text(
            json.dumps(value, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )

    write_json(case_dir / "chromium.dom.json", chromium_dom)
    write_json(case_dir / "moli.dom.json", moli_dom)
    write_json(frames_dir / "00-document.chromium.dom.json", chromium_dom)
    write_json(frames_dir / "00-document.moli.dom.json", moli_dom)
    write_json(
        case_dir / "timeline.json",
        {
            "chromium": [chromium_frame],
            "moli": [moli_frame],
            "mismatchedFrames": ["document"],
        },
    )
    write_json(
        case_dir / "diagnostics.json",
        {
            "id": result["id"],
            "status": result["status"],
            "firstDifference": result["firstDifference"],
            "mismatchedFrames": result["mismatchedFrames"],
            "chromium": result["chromium"],
            "moli": result["moli"],
        },
    )
    expected_diff = unified_dom_diff(chromium_dom, moli_dom) + "\n"
    (case_dir / "diff.txt").write_text(expected_diff, encoding="utf-8")
    (frames_dir / "00-document.diff.txt").write_text(
        expected_diff,
        encoding="utf-8",
    )

    assert _validate_artifacts(tmp_path, summary)["ok"]

    tampered = deepcopy(moli_dom)
    tampered["children"].append(
        {"nodeType": 8, "nodeName": "#comment", "nodeValue": "tampered"}
    )
    write_json(frames_dir / "00-document.moli.dom.json", tampered)
    validation = _validate_artifacts(tmp_path, summary)

    assert not validation["ok"]
    assert any(
        "00-document.moli.dom.json: hash mismatch" in error
        for error in validation["errors"]
    )
