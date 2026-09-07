from __future__ import annotations

import asyncio
import json
from collections import deque
from dataclasses import replace
from types import SimpleNamespace

import moli_frontend_smoke.runner as runner
from moli_frontend_smoke.models import (
    DomFrameObservation,
    EngineObservation,
    SmokeCase,
)
from moli_frontend_smoke.runner import (
    _case_result,
    _normalize_observation,
    _observe_phase,
    parse_args,
)


def test_command_text_keeps_successful_empty_output_empty(monkeypatch) -> None:
    monkeypatch.setattr(
        runner.subprocess,
        "run",
        lambda *_args, **_kwargs: SimpleNamespace(
            stdout="",
            stderr="",
            returncode=0,
        ),
    )
    assert runner._command_text(["git", "status", "--porcelain"]) == ""


def test_process_diagnostics_flags_structured_stderr_errors() -> None:
    process = SimpleNamespace(
        logs=deque(
            [
                "moli:stderr: 2026-08-01T00:00:00Z  INFO ready",
                "moli:stdout: 2026-08-01T00:00:01Z ERROR not stderr",
                "moli:stderr: 2026-08-01T00:00:02Z ERROR unexpected",
            ]
        )
    )

    assert runner._process_diagnostics(process) == {
        "available": True,
        "unexpectedErrors": [
            "moli:stderr: 2026-08-01T00:00:02Z ERROR unexpected"
        ],
    }
    assert runner._process_diagnostics(None) == {
        "available": False,
        "unexpectedErrors": [],
    }


def _case() -> SmokeCase:
    return SmokeCase(
        id="react/family/case",
        framework="react",
        family="family",
        complexity="simple",
        slug="case",
        title="Case",
        variant=0,
        seed=1,
        size=4,
        path="/cases/react/family/case/index.html",
    )


def _observation(engine: str, dom: dict[str, object]) -> EngineObservation:
    return EngineObservation(engine=engine, ok=True, duration_ms=1.0, dom=dom)


def test_default_differential_is_serial() -> None:
    assert parse_args([]).jobs == 1


def test_reference_infrastructure_retry_is_opt_in() -> None:
    assert parse_args([]).reference_infrastructure_retries == 0
    assert (
        parse_args(["--reference-infrastructure-retries", "1"])
        .reference_infrastructure_retries
        == 1
    )


class _FakeWebSocket:
    def __init__(self) -> None:
        self.close_count = 0

    async def close(self) -> None:
        self.close_count += 1


class _FakeClient:
    def __init__(self) -> None:
        self.websocket = _FakeWebSocket()
        self.events: list[dict[str, object]] = []


def test_observation_worker_reuses_one_cdp_connection(monkeypatch) -> None:
    clients: list[_FakeClient] = []
    observed_clients: list[_FakeClient] = []

    async def fake_connect(_endpoint):
        client = _FakeClient()
        clients.append(client)
        return client

    async def fake_observe_case(*, engine, client, **_kwargs):
        observed_clients.append(client)
        return _observation(engine, {"nodeType": 9, "nodeName": "#document"})

    monkeypatch.setattr(runner, "connect", fake_connect)
    monkeypatch.setattr(runner, "observe_case", fake_observe_case)
    first = _case()
    second = replace(first, id="react/family/second", slug="second")

    observations = asyncio.run(
        _observe_phase(
            engine="chromium",
            endpoint="http://127.0.0.1:9222",
            cases=(first, second),
            fixture_url="http://127.0.0.1:3000",
            timeout_ms=1000,
            jobs=1,
        )
    )

    assert list(observations) == [first.id, second.id]
    assert len(clients) == 1
    assert observed_clients == [clients[0], clients[0]]
    assert clients[0].websocket.close_count == 1


def test_chromium_retries_only_infrastructure_failures(monkeypatch) -> None:
    clients: list[_FakeClient] = []
    attempts = 0

    async def fake_connect(_endpoint):
        client = _FakeClient()
        clients.append(client)
        return client

    async def fake_observe_case(*, engine, **_kwargs):
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            return EngineObservation(
                engine=engine,
                ok=False,
                duration_ms=2.0,
                error_type="CdpCommandError",
                error="Target.createBrowserContext failed: unknown",
                failure_kind="infrastructure",
            )
        return _observation(engine, {"nodeType": 9, "nodeName": "#document"})

    monkeypatch.setattr(runner, "connect", fake_connect)
    monkeypatch.setattr(runner, "observe_case", fake_observe_case)

    observation = asyncio.run(
        _observe_phase(
            engine="chromium",
            endpoint="http://127.0.0.1:9222",
            cases=(_case(),),
            fixture_url="http://127.0.0.1:3000",
            timeout_ms=1000,
            jobs=1,
            infrastructure_retries=1,
        )
    )[_case().id]

    assert observation.ok
    assert observation.attempt_count == 2
    assert observation.duration_ms == 3.0
    assert [failure.error for failure in observation.previous_failures] == [
        "Target.createBrowserContext failed: unknown"
    ]
    assert len(clients) == 2
    assert all(client.websocket.close_count == 1 for client in clients)


def test_fixture_failure_is_never_retried(monkeypatch) -> None:
    attempts = 0

    async def fake_connect(_endpoint):
        return _FakeClient()

    async def fake_observe_case(*, engine, **_kwargs):
        nonlocal attempts
        attempts += 1
        return EngineObservation(
            engine=engine,
            ok=False,
            duration_ms=1.0,
            error_type="FixtureObservationError",
            error="fixture assertion failed",
            failure_kind="fixture",
        )

    monkeypatch.setattr(runner, "connect", fake_connect)
    monkeypatch.setattr(runner, "observe_case", fake_observe_case)

    observation = asyncio.run(
        _observe_phase(
            engine="chromium",
            endpoint="http://127.0.0.1:9222",
            cases=(_case(),),
            fixture_url="http://127.0.0.1:3000",
            timeout_ms=1000,
            jobs=1,
            infrastructure_retries=1,
        )
    )[_case().id]

    assert not observation.ok
    assert observation.failure_kind == "fixture"
    assert observation.attempt_count == 1
    assert observation.previous_failures == []
    assert attempts == 1


def test_moli_failure_is_never_retried(monkeypatch) -> None:
    attempts = 0

    async def fake_connect(_endpoint):
        return _FakeClient()

    async def fake_observe_case(*, engine, **_kwargs):
        nonlocal attempts
        attempts += 1
        return EngineObservation(
            engine=engine,
            ok=False,
            duration_ms=1.0,
            error_type="CdpCommandError",
            error="candidate connection failed",
            failure_kind="infrastructure",
        )

    monkeypatch.setattr(runner, "connect", fake_connect)
    monkeypatch.setattr(runner, "observe_case", fake_observe_case)

    observation = asyncio.run(
        _observe_phase(
            engine="moli",
            endpoint="http://127.0.0.1:9222",
            cases=(_case(),),
            fixture_url="http://127.0.0.1:3000",
            timeout_ms=1000,
            jobs=1,
            infrastructure_retries=1,
        )
    )[_case().id]

    assert not observation.ok
    assert observation.attempt_count == 1
    assert observation.previous_failures == []
    assert attempts == 1


def test_any_moli_tree_difference_is_a_failure(tmp_path) -> None:
    chromium = _observation("chromium", {"nodeType": 3, "nodeValue": "reference"})
    moli = _observation("moli", {"nodeType": 3, "nodeValue": "different"})
    result = _case_result(
        case=_case(),
        chromium=chromium,
        moli=moli,
        reference_only=False,
        output_dir=tmp_path,
    )
    assert result.status == "dom_mismatch"
    assert result.first_difference == "$.nodeValue"
    assert result.artifact is not None


def test_expected_network_failure_projection_must_match_chromium(tmp_path) -> None:
    dom = {"nodeType": 3, "nodeValue": "same"}
    chromium = _observation("chromium", dom)
    chromium.diagnostics = {
        "expectedNetworkFailures": [
            {
                "label": "expected-abort",
                "errorText": "net::ERR_ABORTED",
                "type": "Fetch",
                "canceled": True,
                "blockedReason": None,
            }
        ]
    }
    moli = _observation("moli", dom)
    moli.diagnostics = {
        "expectedNetworkFailures": [
            {
                "label": "expected-abort",
                "errorText": "operation aborted",
                "type": "Fetch",
                "canceled": True,
                "blockedReason": None,
            }
        ]
    }

    result = _case_result(
        case=_case(),
        chromium=chromium,
        moli=moli,
        reference_only=False,
        output_dir=tmp_path,
    )

    assert result.status == "diagnostic_mismatch"
    assert result.first_difference == "$.diagnostics.expectedNetworkFailures"
    assert result.artifact is not None


def test_any_intermediate_frame_difference_is_a_failure(tmp_path) -> None:
    reference_final = {"nodeType": 3, "nodeValue": "settled"}
    candidate_final = {"nodeType": 3, "nodeValue": "settled"}
    chromium = EngineObservation(
        engine="chromium",
        ok=True,
        duration_ms=1.0,
        dom=reference_final,
        frames=[
            DomFrameObservation(
                index=0,
                name="document",
                token="case:0:document",
                dom={"nodeType": 3, "nodeValue": "before mount"},
            ),
            DomFrameObservation(
                index=1,
                name="mounted",
                token="case:1:mounted",
                dom={"nodeType": 3, "nodeValue": "reference mounted"},
            ),
            DomFrameObservation(
                index=2,
                name="settled",
                token="case:settled",
                dom=reference_final,
            ),
        ],
    )
    moli = EngineObservation(
        engine="moli",
        ok=True,
        duration_ms=1.0,
        dom=candidate_final,
        frames=[
            DomFrameObservation(
                index=0,
                name="document",
                token="case:0:document",
                dom={"nodeType": 3, "nodeValue": "before mount"},
            ),
            DomFrameObservation(
                index=1,
                name="mounted",
                token="case:1:mounted",
                dom={"nodeType": 3, "nodeValue": "candidate mounted"},
            ),
            DomFrameObservation(
                index=2,
                name="settled",
                token="case:settled",
                dom=candidate_final,
            ),
        ],
    )

    result = _case_result(
        case=_case(),
        chromium=chromium,
        moli=moli,
        reference_only=False,
        output_dir=tmp_path,
    )

    assert result.status == "dom_mismatch"
    assert result.first_difference == "$.frames[1].dom.nodeValue"
    assert result.mismatched_frames == ["mounted"]
    assert result.artifact is not None
    case_dir = tmp_path / result.artifact
    assert (case_dir / "timeline.json").is_file()
    assert (case_dir / "frames" / "01-mounted.diff.txt").is_file()


def test_partial_error_frames_are_normalized_for_diagnostics() -> None:
    observation = EngineObservation(
        engine="chromium",
        ok=False,
        duration_ms=1.0,
        frames=[
            DomFrameObservation(
                index=0,
                name="document",
                token="case:0:document",
                dom={
                    "nodeId": 42,
                    "backendNodeId": 84,
                    "nodeType": 3,
                    "nodeName": "#text",
                    "nodeValue": "partial",
                },
            )
        ],
        error_type="CdpError",
        error="later checkpoint failed",
    )

    _normalize_observation(observation)

    assert observation.frames[0].dom == {
        "nodeType": 3,
        "nodeName": "#text",
        "nodeValue": "partial",
    }
    assert observation.frames[0].dom_hash is not None
    assert observation.frames[0].node_count == 1


def test_moli_error_after_valid_reference_is_a_failure(tmp_path) -> None:
    chromium = _observation("chromium", {"nodeType": 9, "nodeName": "#document"})
    moli = EngineObservation(
        engine="moli",
        ok=False,
        duration_ms=1.0,
        error_type="CdpError",
        error="fixture did not become ready",
    )
    result = _case_result(
        case=_case(),
        chromium=chromium,
        moli=moli,
        reference_only=False,
        output_dir=tmp_path,
    )
    assert result.status == "moli_error"


def test_reference_error_never_becomes_a_moli_bug(tmp_path) -> None:
    chromium = EngineObservation(
        engine="chromium",
        ok=False,
        duration_ms=1.0,
        error_type="CdpError",
        error="reference fixture failed",
    )
    result = _case_result(
        case=_case(),
        chromium=chromium,
        moli=None,
        reference_only=True,
        output_dir=tmp_path,
    )
    assert result.status == "reference_error"


def test_failed_reference_gate_never_starts_moli(monkeypatch, tmp_path) -> None:
    manifest = {
        "schemaVersion": 1,
        "complete": False,
        "totalCatalogCases": 300,
        "fixturesSha256": (
            "e3b0c44298fc1c149afbf4c8996fb924"
            "27ae41e4649b934ca495991b7852b855"
        ),
        "tools": {},
        "cases": [
            {
                "id": "react/family/case",
                "framework": "react",
                "family": "family",
                "complexity": "simple",
                "slug": "case",
                "title": "Case",
                "variant": 0,
                "seed": 1,
                "size": 4,
                "path": "/cases/react/family/case/index.html",
            }
        ],
    }
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(json.dumps(manifest))
    dist = tmp_path / "dist"
    dist.mkdir()

    class FakeFixture:
        url = "http://127.0.0.1:3000"

        def __init__(self, root):
            assert root == dist

        def start(self):
            pass

        def stop(self):
            pass

    class FakeChromium:
        endpoint = "http://127.0.0.1:9223"
        version = {"Browser": "Fake Chromium"}

        async def stop(self):
            pass

    phases = []

    async def fake_start_chromium(_binary):
        return FakeChromium()

    async def fake_observe_phase(*, engine, cases, **_kwargs):
        phases.append(engine)
        assert engine == "chromium"
        return {
            cases[0].id: EngineObservation(
                engine="chromium",
                ok=False,
                duration_ms=1.0,
                error_type="CdpError",
                error="reference failed",
            )
        }

    monkeypatch.setattr(runner, "FixtureServer", FakeFixture)
    monkeypatch.setattr(runner, "chromium_binary", lambda _override: tmp_path / "chromium")
    monkeypatch.setattr(runner, "start_chromium", fake_start_chromium)
    monkeypatch.setattr(runner, "_observe_phase", fake_observe_phase)
    monkeypatch.setattr(
        runner,
        "moli_binary",
        lambda _override: (_ for _ in ()).throw(AssertionError("Moli must not start")),
    )

    args = SimpleNamespace(
        manifest=str(manifest_path),
        allow_partial_manifest=True,
        framework=[],
        family=[],
        complexity=[],
        case=[],
        output=str(tmp_path / "artifacts"),
        dist=str(dist),
        chromium_endpoint=None,
        chromium_bin=None,
        reference_only=False,
        moli_endpoint=None,
        moli_bin=None,
        jobs=1,
        timeout_ms=1000,
    )
    code, summary = asyncio.run(runner.run(args))
    assert code == 1
    assert phases == ["chromium"]
    assert summary["referenceGate"] == {
        "ok": False,
        "cases": 1,
        "errors": 1,
        "moliPhaseStarted": False,
        "infrastructureRetryLimit": 0,
        "retriedCases": 0,
        "recoveredCases": 0,
    }
