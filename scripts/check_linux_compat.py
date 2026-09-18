"""Check a Linux Moli binary against the Ubuntu 22.04 runtime baseline."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread

# Jammy's glibc and updated libstdc++6 (GCC 12). Checking the final ELF also
# covers prebuilt native archives, whose ABI is not controlled by Cargo.
MAX_VERSIONS = {"GLIBC": (2, 35), "GLIBCXX": (3, 4, 30), "CXXABI": (1, 3, 13)}
NAMED_VERSIONS = {"CXXABI_TM_1", "CXXABI_FLOAT128"}


def check_versions(output: str) -> dict[str, tuple[int, ...]]:
    # Ignore version definitions/symbol tables; only imported ABI requirements
    # constrain the runtime. LC_ALL=C below keeps readelf's section names stable.
    _, marker, needs = output.partition("Version needs section")
    if not marker:
        raise ValueError("ELF has no version requirements; expected a GNU/Linux binary")
    names = set(re.findall(r"\bName: ((?:GLIBC|GLIBCXX|CXXABI)_[\w.]+)", needs))
    if not any(name.startswith("GLIBC_") for name in names):
        raise ValueError("ELF has no GLIBC requirements")

    maxima: dict[str, tuple[int, ...]] = {}
    rejected = []
    for name in sorted(names):
        if name in NAMED_VERSIONS:
            continue
        family, _, raw_version = name.partition("_")
        if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)*", raw_version):
            # In particular, GLIBC_ABI_DT_RELR needs glibc 2.36 even when all
            # numeric GLIBC_* references happen to be older.
            rejected.append(name)
            continue
        version = tuple(int(part) for part in raw_version.split("."))
        maxima[family] = max(maxima.get(family, ()), version)
        if version > MAX_VERSIONS[family]:
            rejected.append(name)
    if rejected:
        raise ValueError("Ubuntu 22.04 does not provide: " + ", ".join(rejected))
    return maxima


def run_checked(command: list[str], *, env: dict[str, str] | None = None) -> str:
    print("+ " + " ".join(command), flush=True)
    result = subprocess.run(
        command, check=True, text=True, capture_output=True, timeout=60, env=env
    )
    return result.stdout


class SmokeHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path == "/answer":
            body = b'{"answer":42}'
            content_type = "application/json"
        else:
            body = (
                b"<!doctype html><title>jammy-smoke</title><p id=result>pending</p>"
                b"<script>document.getElementById('result').textContent='script-ran'</script>"
            )
            content_type = "text/html; charset=utf-8"
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        pass


def smoke_test(binary: Path) -> None:
    print(run_checked([str(binary), "--version"]).strip(), flush=True)
    # Exercise V8 and actual networking, not just the CLI version path. Keep the
    # test offline and explicitly bypass CI's proxy environment for loopback.
    with ThreadingHTTPServer(("127.0.0.1", 0), SmokeHandler) as server:
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            output = run_checked(
                [
                    str(binary),
                    "fetch",
                    "--log-level",
                    "error",
                    "--http-no-proxy",
                    "*",
                    "--wait-until",
                    "load",
                    "--eval",
                    (
                        "(async () => ({title: document.title, "
                        "text: document.getElementById('result').textContent, "
                        "answer: (await (await fetch('/answer')).json()).answer}))()"
                    ),
                    f"http://127.0.0.1:{server.server_port}/",
                ]
            )
            expected = {"title": "jammy-smoke", "text": "script-ran", "answer": 42}
            if json.loads(output) != expected:
                raise ValueError(
                    f"HTTP / JavaScript smoke returned unexpected output: {output}"
                )
        finally:
            server.shutdown()
            thread.join(timeout=5)
    print("HTTP / JavaScript smoke passed", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument(
        "--smoke", action="store_true", help="also run the native binary"
    )
    args = parser.parse_args()
    try:
        binary = args.binary.resolve(strict=True)
        env = dict(os.environ, LC_ALL="C")
        output = run_checked(
            ["readelf", "--version-info", "--wide", str(binary)], env=env
        )
        for family, version in sorted(check_versions(output).items()):
            print(f"Requires {family}_{'.'.join(map(str, version))}", flush=True)
        if args.smoke:
            # ldd exposes missing DT_NEEDED libraries as well as version errors.
            # Executing the binary below makes either condition a hard failure.
            print(run_checked(["ldd", str(binary)], env=env), end="", flush=True)
            smoke_test(binary)
        return 0
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stdout or "", end="", file=sys.stderr)
            print(error.stderr or "", end="", file=sys.stderr)
        print(f"Linux compatibility check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
