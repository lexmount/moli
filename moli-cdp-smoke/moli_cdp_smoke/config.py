from __future__ import annotations

import os
import socket
from pathlib import Path


PROXY_ENV_KEYS = (
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
)
TRACE_BACKGROUND_PROCESS_ENV = "MOLI_SMOKE_TRACE_BG"

REPO_ROOT = Path(__file__).resolve().parents[2]


def is_moli_browser() -> bool:
    # The worker sets this only after checking the actual discovery endpoint.
    return os.environ.get("MOLI_SMOKE_BROWSER", "moli") == "moli"


def clear_proxy_env(env: dict[str, str]) -> dict[str, str]:
    next_env = dict(env)
    for key in PROXY_ENV_KEYS:
        next_env.pop(key, None)
    next_env["NO_PROXY"] = "*"
    next_env["no_proxy"] = "*"
    return next_env


def clear_current_process_proxy_env() -> None:
    for key in PROXY_ENV_KEYS:
        os.environ.pop(key, None)
    os.environ["NO_PROXY"] = "*"
    os.environ["no_proxy"] = "*"


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def moli_binary() -> Path:
    if not is_moli_browser():
        raise RuntimeError("a Chromium smoke worker must not invoke the Moli CLI")
    override = os.environ.get("MOLI_BIN")
    if override:
        return Path(override).expanduser().resolve()
    candidates = [
        REPO_ROOT / "target" / "debug" / "moli",
        REPO_ROOT / "target" / "release" / "moli",
    ]
    existing = [candidate for candidate in candidates if candidate.exists()]
    if existing:
        return max(existing, key=lambda path: path.stat().st_mtime)
    raise RuntimeError("missing moli binary; run `cargo build -p moli` or set MOLI_BIN")
