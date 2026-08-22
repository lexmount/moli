from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path
from typing import Any

from . import __version__
from .config import REPO_ROOT


MOLI_VERSION_ARGS = ("--version",)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _run(command: list[str], timeout: float = 5.0) -> str | None:
    try:
        completed = subprocess.run(command, cwd=REPO_ROOT, capture_output=True, text=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    output = (completed.stdout or completed.stderr).strip()
    return output if output else None


def collect_versions(moli_bin: Path | None = None, targets: dict[str, Any] | None = None) -> dict[str, Any]:
    moli: dict[str, Any] | None = None
    if moli_bin is not None:
        moli = {
            "path": str(moli_bin),
            "size_bytes": moli_bin.stat().st_size,
            "sha256": sha256_file(moli_bin),
            "version": _run([str(moli_bin), *MOLI_VERSION_ARGS]),
        }
    elif targets and isinstance(targets.get("moli"), dict):
        moli = targets["moli"]

    return {
        "benchmark": {"version": __version__},
        "git": {
            "commit": _run(["git", "rev-parse", "HEAD"]),
            "branch": _run(["git", "branch", "--show-current"]),
            "dirty": _run(["git", "status", "--porcelain"]),
        },
        "tools": {
            "rustc": _run(["rustc", "--version"]),
            "cargo": _run(["cargo", "--version"]),
            "python": _run(["python", "--version"]),
            "python3": _run(["python3", "--version"]),
            "node": _run(["node", "--version"]),
            "go": _run(["go", "version"]),
            "smem": _run(["smem", "--version"]),
        },
        "moli": moli,
        "targets": targets,
    }
