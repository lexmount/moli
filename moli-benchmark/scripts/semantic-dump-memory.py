#!/usr/bin/env python3
"""Compare semantic dump time/RSS and verify output bytes on a generated list.

Build both revisions with `cargo build --release -p moli`, then run:
    python3 moli-benchmark/scripts/semantic-dump-memory.py \
        --before /tmp/moli-before --after target/release/moli \
        --output-dir /tmp/semantic-dump-results

Requires Unix wait4; RSS is reported in bytes on both Linux and macOS.
Both executables are copied into the output directory so executable mappings
have the same filesystem backing when their RSS is compared.
"""

import argparse
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading
import time


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


def positive_int(value):
    result = int(value)
    if result <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return result


def measure(binary, label, fmt, repeat, rows, url, output_dir, env):
    output = output_dir / f"{label}-{fmt}.out"
    errors = output_dir / f"{label}-{fmt}.stderr"
    command = [
        str(binary), "fetch", "--timeout", "120000", "--dump", fmt, url
    ]
    started = time.monotonic()
    with output.open("wb") as out, errors.open("wb") as err:
        with subprocess.Popen(command, stdout=out, stderr=err, env=env) as process:
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
    wall_seconds = time.monotonic() - started
    with output.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    record = {
        "binary": label,
        "executable": str(binary),
        "rows": rows,
        "format": fmt,
        "repeat": repeat,
        "returncode": process.returncode,
        "maxrss_bytes": usage.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
        "wall_seconds": wall_seconds,
        "output_bytes": output.stat().st_size,
        "sha256": digest,
    }
    with (output_dir / "results.jsonl").open("a") as results:
        results.write(json.dumps(record) + "\n")
    print(json.dumps(record), flush=True)
    if process.returncode:
        raise RuntimeError(f"{label} {fmt} failed; see {errors}")
    return record["output_bytes"], digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True, type=Path)
    parser.add_argument("--after", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--rows", type=positive_int, default=40000)
    parser.add_argument("--runs", type=positive_int, default=3)
    args = parser.parse_args()
    binaries = [("before", args.before.resolve()), ("after", args.after.resolve())]
    for label, binary in binaries:
        if not os.access(binary, os.X_OK):
            parser.error(f"{label} binary is not executable: {binary}")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    staged_binaries = []
    manifest = {"rows": args.rows, "runs": args.runs, "binaries": {}}
    for label, binary in binaries:
        staged = (args.output_dir / f"moli-{label}").resolve()
        if binary != staged:
            shutil.copy2(binary, staged)
        with staged.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        manifest["binaries"][label] = {
            "source": str(binary), "staged": str(staged), "sha256": digest
        }
        staged_binaries.append((label, staged))
    binaries = staged_binaries
    (args.output_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    fixture = args.output_dir / "large-list.html"
    with fixture.open("w") as page:
        page.write("<!doctype html><html><head><title>Large list</title></head><body><main>\n")
        for index in range(args.rows):
            page.write(
                f'<div class="row"><a href="/item/{index}">Item {index}</a>'
                f'<span> description {index}</span></div>\n'
            )
        page.write("</main></body></html>\n")
    handler = functools.partial(QuietHandler, directory=str(args.output_dir.resolve()))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    url = f"http://127.0.0.1:{server.server_port}/{fixture.name}"
    env = {
        key: value for key, value in os.environ.items()
        if key.lower() not in ("http_proxy", "https_proxy", "all_proxy")
    }
    try:
        for fmt in ("semantic_tree_text", "semantic_tree", "markdown"):
            expected = None
            for repeat in range(args.runs):
                # Alternate the order to reduce warm-cache/order bias.
                order = binaries if repeat % 2 == 0 else list(reversed(binaries))
                for label, binary in order:
                    actual = measure(
                        binary, label, fmt, repeat, args.rows, url, args.output_dir, env
                    )
                    if expected is None:
                        expected = actual
                    elif actual != expected:
                        raise RuntimeError(f"{fmt} output differs: {label}, run {repeat}")
        print("All dump outputs are byte-for-byte identical.", flush=True)
    finally:
        server.shutdown()
        thread.join()
        server.server_close()


if __name__ == "__main__":
    main()
