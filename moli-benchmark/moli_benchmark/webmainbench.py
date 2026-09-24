"""Replay the pinned WebMainBench corpus as a Linux fetch stability check."""

from __future__ import annotations

import argparse
import hashlib
import json
import logging
import os
import platform
import re
import shutil
import ssl
import subprocess
import tempfile
import threading
import time
from collections import Counter
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.error import URLError
from urllib.parse import urlsplit
from urllib.request import urlopen
from uuid import UUID

from .artifacts import write_json, write_text
from .process import ProcessResult, run_process

LOGGER = logging.getLogger(__name__)

DATASET_REVISION = "5da0972e9b58d0c7891ae75053ced97c268f52e3"
DATASET_SHA256 = "0efaa4b49a45e320a27fe6e5a0b6aad5b57259fc3321ac3448519cacc74c537e"
DATASET_URL = (
    "https://huggingface.co/datasets/opendatalab/WebMainBench/resolve/"
    f"{DATASET_REVISION}/WebMainBench_545.jsonl?download=true"
)
CASE_COUNT = 545
TIMEOUT_MS = 45_000
PROCESS_GRACE_SECONDS = 10

# This page redirects off the fixture origin. Only its DNS failure is allowed;
# a panic, timeout, signal, or any other error on this page still fails the job.
REDIRECT_CASE_ID = "ccb6033c-0a12-4f9c-8c68-794f26129841"
REDIRECT_ERROR = re.compile(
    rb"(?m)^Reason: failed to resolve langrensha\.163\.com:443: "
    rb"failed to lookup address information: [^\r\n]+\r?\n?\Z"
)
PANIC = re.compile(rb"(?m)^thread [^\r\n]*panicked(?: at\b|:)")
FETCH_TIMEOUT = re.compile(
    rb"(?mi)^Reason:[^\r\n]*(?:timed out|timeout|deadline exceeded)"
)
PASS_STATUSES = {"success", "expected_failure"}


@dataclass(frozen=True)
class Case:
    track_id: str
    url: str
    html: str


def file_sha256(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def verify_dataset(path: Path) -> None:
    actual = file_sha256(path)
    if actual != DATASET_SHA256:
        raise ValueError(
            f"Dataset SHA-256 mismatch: expected {DATASET_SHA256}, got {actual}"
        )


def download_dataset(path: Path) -> None:
    if path.exists():
        verify_dataset(path)
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    # A failed download must never become the cached dataset.
    with tempfile.TemporaryDirectory(dir=path.parent) as temporary:
        pending = Path(temporary) / "dataset.jsonl"
        for attempt in range(3):
            try:
                with (
                    urlopen(DATASET_URL, timeout=60) as source,
                    pending.open("wb") as output,
                ):
                    shutil.copyfileobj(source, output)
                verify_dataset(pending)
                pending.replace(path)
                return
            except (URLError, TimeoutError, ConnectionError):
                if attempt == 2:
                    raise
                time.sleep(attempt + 1)


def load_cases(path: Path) -> list[Case]:
    verify_dataset(path)
    cases = []
    with path.open(encoding="utf-8") as source:
        for line in source:
            row = json.loads(line)
            track_id = row["track_id"]
            if str(UUID(track_id)) != track_id:
                raise ValueError(f"Invalid case ID: {track_id!r}")
            if not isinstance(row["html"], str) or not row["html"]:
                raise ValueError(f"Empty or invalid HTML: {track_id}")
            if urlsplit(row["url"]).scheme not in {"http", "https"}:
                raise ValueError(f"Invalid source URL: {track_id}")
            cases.append(Case(track_id, row["url"], row["html"]))
    if len(cases) != CASE_COUNT or len({case.track_id for case in cases}) != CASE_COUNT:
        raise ValueError(f"Expected {CASE_COUNT} unique cases, got {len(cases)} rows")
    return cases


def require_loopback_namespace() -> None:
    links = json.loads(subprocess.check_output(["ip", "-j", "link", "show"]))
    if [link["ifname"] for link in links] != ["lo"]:
        raise RuntimeError(
            "Run inside a fresh network namespace with only loopback (see README)"
        )
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)


@contextmanager
def fixture_server(cases: list[Case]) -> Iterator[tuple[str, Path]]:
    bodies = {
        f"/case/{case.track_id}.html": case.html.encode("utf-8") for case in cases
    }

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args: object) -> None:
            pass

        def do_GET(self) -> None:
            body = bodies.get(urlsplit(self.path).path)
            self.send_response(200 if body is not None else 404)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            body = body if body is not None else b""
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

    # Keep the throwaway TLS private key outside uploaded artifacts.
    with tempfile.TemporaryDirectory(prefix="moli-webmainbench-tls-") as temporary:
        key = Path(temporary) / "key.pem"
        cert = Path(temporary) / "cert.pem"
        subprocess.run(
            [
                "openssl",
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                str(key),
                "-out",
                str(cert),
                "-days",
                "1",
                "-subj",
                "/CN=127.0.0.1",
                "-addext",
                "subjectAltName=IP:127.0.0.1",
                "-addext",
                "keyUsage=critical,digitalSignature,keyEncipherment,keyCertSign",
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, key)
        with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
            server.socket = context.wrap_socket(server.socket, server_side=True)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                yield f"https://127.0.0.1:{server.server_port}", cert
            finally:
                server.shutdown()
                thread.join()


def classify(case: Case, result: ProcessResult) -> str:
    if PANIC.search(result.stderr):
        return "panic"
    if result.timed_out or (
        result.returncode != 0 and FETCH_TIMEOUT.search(result.stderr)
    ):
        return "timeout"
    if result.returncode is not None and result.returncode < 0:
        return "crash"
    if result.returncode == 0:
        return "success" if result.stdout.strip() else "empty_output"
    if (
        case.track_id == REDIRECT_CASE_ID
        and result.returncode == 1
        and not result.stdout
        and REDIRECT_ERROR.search(result.stderr)
    ):
        return "expected_failure"
    return "failure"


def health_issues(cases: list[Case], results: list[dict]) -> list[str]:
    expected = Counter(case.track_id for case in cases)
    actual = Counter(result["track_id"] for result in results)
    issues = []
    if len(cases) != CASE_COUNT or len(expected) != CASE_COUNT or actual != expected:
        issues.append(f"Expected exactly one result for each of {CASE_COUNT} cases")
    issues.extend(
        f"{result['track_id']}: {result['status']}"
        for result in results
        if result["status"] not in PASS_STATUSES
    )
    return issues


def render_summary(report: dict) -> str:
    counts = report["counts"]
    lines = [
        "## WebMainBench · 545 pages",
        "",
        f"Revision: `{report['revision']}`",
        "",
        (
            f"**{'PASS' if report['passed'] else 'FAIL'}** — "
            f"{report['completed']}/{CASE_COUNT} completed; "
            f"{counts.get('success', 0)} successful; "
            f"{counts.get('expected_failure', 0)} expected DNS failures."
        ),
        "",
        (
            f"Panics: {counts.get('panic', 0)} · crashes: {counts.get('crash', 0)} · "
            f"timeouts: {counts.get('timeout', 0)}."
        ),
        "",
        "Frozen HTML, `--wait done`, 45 seconds per page, loopback-only HTTPS, no retries.",
        "This is a fetch stability check; it does not calculate content quality scores.",
        "",
    ]
    if report["issues"]:
        lines.extend(["Failures:", ""])
        lines.extend(f"- {issue}" for issue in report["issues"])
        lines.append("")
    lines.extend(
        [
            (
                f"Known exception: `{REDIRECT_CASE_ID}` may fail resolving `langrensha.163.com:443` "
                "after its external redirect. Other errors on that page fail the check."
            ),
            "",
            (
                "The `webmainbench-results` artifact contains every page's Markdown, stderr, "
                "HTML/output hashes, exit status, and elapsed time."
            ),
            "",
        ]
    )
    return "\n".join(lines)


def replay(binary: Path, dataset: Path, output: Path, revision: str) -> bool:
    cases = load_cases(dataset)
    binary = binary.resolve(strict=True)
    require_loopback_namespace()
    output.mkdir(parents=True, exist_ok=False)
    metadata = {
        "schema_version": 1,
        "revision": revision,
        "binary_sha256": file_sha256(binary),
        "dataset_revision": DATASET_REVISION,
        "dataset_sha256": DATASET_SHA256,
        "dataset_url": DATASET_URL,
        "platform": platform.platform(),
        "wait": "done",
        "timeout_ms": TIMEOUT_MS,
        "process_grace_seconds": PROCESS_GRACE_SECONDS,
        "network": "loopback-only network namespace; HTTPS fixture; other paths return 404",
        "concurrency": 1,
        "retries": 0,
        "expected_cases": CASE_COUNT,
    }
    write_json(output / "metadata.json", metadata)
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.lower().endswith("_proxy")
    }
    results = []
    infrastructure_error = None
    try:
        with (
            fixture_server(cases) as (origin, cert),
            (output / "results.jsonl").open("w") as log,
        ):
            for index, case in enumerate(cases, 1):
                result = run_process(
                    [
                        str(binary),
                        "fetch",
                        "--dump",
                        "markdown",
                        "--wait",
                        "done",
                        "--timeout",
                        str(TIMEOUT_MS),
                        "--ca-cert",
                        str(cert),
                        f"{origin}/case/{case.track_id}.html",
                    ],
                    cwd=output,
                    timeout_seconds=TIMEOUT_MS / 1000 + PROCESS_GRACE_SECONDS,
                    env=env,
                    sample_resources=False,
                )
                (output / f"{case.track_id}.md").write_bytes(result.stdout)
                (output / f"{case.track_id}.stderr").write_bytes(result.stderr)
                record = {
                    "track_id": case.track_id,
                    "url": case.url,
                    "status": classify(case, result),
                    "returncode": result.returncode,
                    "process_timed_out": result.timed_out,
                    "elapsed_ms": result.elapsed_ms,
                    "html_sha256": hashlib.sha256(
                        case.html.encode("utf-8")
                    ).hexdigest(),
                    "markdown_sha256": hashlib.sha256(result.stdout).hexdigest(),
                    "markdown_bytes": len(result.stdout),
                }
                results.append(record)
                log.write(json.dumps(record) + "\n")
                log.flush()
                if (
                    index % 10 == 0
                    or index == len(cases)
                    or record["status"] != "success"
                ):
                    print(
                        f"{index}/{len(cases)} {case.track_id}: {record['status']}",
                        flush=True,
                    )
    except Exception as error:
        LOGGER.exception("WebMainBench replay failed")
        infrastructure_error = f"{type(error).__name__}: {error}"
    finally:
        issues = health_issues(cases, results)
        if infrastructure_error:
            issues.append(f"Infrastructure error: {infrastructure_error}")
        report = {
            **metadata,
            "completed": len(results),
            "counts": dict(Counter(result["status"] for result in results)),
            "passed": not issues,
            "issues": issues,
        }
        write_json(output / "summary.json", report)
        write_text(output / "summary.md", render_summary(report))
    print(render_summary(report), flush=True)
    return report["passed"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    download = commands.add_parser(
        "download", help="Download and verify the frozen corpus"
    )
    download.add_argument("--output", type=Path, required=True)
    run = commands.add_parser(
        "run", help="Replay all 545 pages inside a network namespace"
    )
    run.add_argument("--moli-bin", type=Path, required=True)
    run.add_argument("--dataset", type=Path, required=True)
    run.add_argument("--output", type=Path, required=True)
    run.add_argument("--revision", required=True)
    args = parser.parse_args()
    if args.command == "download":
        download_dataset(args.output)
        print(f"Verified {DATASET_SHA256}  {args.output}")
        return 0
    return (
        0
        if replay(args.moli_bin, args.dataset, args.output.resolve(), args.revision)
        else 1
    )


if __name__ == "__main__":
    raise SystemExit(main())
