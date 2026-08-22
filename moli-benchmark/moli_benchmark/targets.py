from __future__ import annotations

import subprocess
from pathlib import Path
from typing import Any

from .config import chrome_binary, moli_binary, lightpanda_binary, obscura_binary
from .versions import MOLI_VERSION_ARGS, sha256_file


def _version(command: list[str]) -> str | None:
    try:
        completed = subprocess.run(command, capture_output=True, text=True, timeout=5, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return None
    output = (completed.stdout or completed.stderr).strip()
    return output.splitlines()[0] if output else None


def _binary_info(path: Path | None, version_args: tuple[str, ...]) -> dict[str, Any]:
    if path is None:
        return {"path": None, "available": False}
    stat = path.stat()
    return {
        "path": str(path),
        "available": True,
        "size_bytes": stat.st_size,
        "sha256": sha256_file(path),
        "version": _version([str(path), *version_args]),
    }


def collect_target_binaries(
    *,
    moli_override: str | None = None,
    lightpanda_override: str | None = None,
    chrome_override: str | None = None,
    obscura_override: str | None = None,
) -> dict[str, Any]:
    moli_path = moli_binary(moli_override)
    return {
        "moli": _binary_info(moli_path, MOLI_VERSION_ARGS),
        "lightpanda": _binary_info(lightpanda_binary(lightpanda_override), ("version",)),
        "chrome": _binary_info(chrome_binary(chrome_override), ("--version",)),
        "obscura": _binary_info(obscura_binary(obscura_override), ("--help",)),
    }
