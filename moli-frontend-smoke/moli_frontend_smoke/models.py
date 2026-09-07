from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any, Literal


FrameworkName = Literal["react", "vue", "angular"]
Complexity = Literal["simple", "medium", "complex"]
ObservationFailureKind = Literal["infrastructure", "fixture", "normalization"]
CaseStatus = Literal[
    "match",
    "reference_ok",
    "reference_error",
    "moli_error",
    "dom_mismatch",
    "diagnostic_mismatch",
    "infrastructure_error",
]


@dataclass(frozen=True)
class SmokeCase:
    id: str
    framework: FrameworkName
    family: str
    complexity: Complexity
    slug: str
    title: str
    variant: int
    seed: int
    size: int
    path: str

    @classmethod
    def from_json(cls, value: dict[str, Any]) -> "SmokeCase":
        return cls(
            id=str(value["id"]),
            framework=value["framework"],
            family=str(value["family"]),
            complexity=value["complexity"],
            slug=str(value["slug"]),
            title=str(value["title"]),
            variant=int(value["variant"]),
            seed=int(value["seed"]),
            size=int(value["size"]),
            path=str(value["path"]),
        )


@dataclass
class DomFrameObservation:
    index: int
    name: str
    token: str
    dom: dict[str, Any] | None = None
    dom_hash: str | None = None
    node_count: int | None = None

    def summary_json(self) -> dict[str, Any]:
        value = asdict(self)
        value.pop("dom", None)
        return value


@dataclass(frozen=True)
class ObservationFailure:
    attempt: int
    kind: ObservationFailureKind
    duration_ms: float
    error_type: str | None
    error: str | None


@dataclass
class EngineObservation:
    engine: str
    ok: bool
    duration_ms: float
    ready_state: dict[str, Any] | None = None
    dom: dict[str, Any] | None = None
    frames: list[DomFrameObservation] = field(default_factory=list)
    dom_hash: str | None = None
    node_count: int | None = None
    diagnostics: dict[str, Any] = field(default_factory=dict)
    error_type: str | None = None
    error: str | None = None
    failure_kind: ObservationFailureKind | None = None
    attempt_count: int = 1
    previous_failures: list[ObservationFailure] = field(default_factory=list)

    def summary_json(self) -> dict[str, Any]:
        value = asdict(self)
        value.pop("dom", None)
        value["frames"] = [frame.summary_json() for frame in self.frames]
        return value


@dataclass
class CaseResult:
    case: SmokeCase
    status: CaseStatus
    duration_ms: float
    chromium: EngineObservation
    moli: EngineObservation | None = None
    first_difference: str | None = None
    mismatched_frames: list[str] = field(default_factory=list)
    artifact: str | None = None

    def to_json(self) -> dict[str, Any]:
        return {
            "id": self.case.id,
            "framework": self.case.framework,
            "family": self.case.family,
            "complexity": self.case.complexity,
            "status": self.status,
            "durationMs": round(self.duration_ms, 3),
            "chromium": self.chromium.summary_json(),
            "moli": self.moli.summary_json() if self.moli else None,
            "firstDifference": self.first_difference,
            "mismatchedFrames": self.mismatched_frames,
            "artifact": self.artifact,
        }
