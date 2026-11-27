#!/usr/bin/env python3
"""Measure the RSS peak of repeated Realtime screen layouts on Linux.

Run with uv run --project moli-benchmark --locked python <script> --help.
For the ablation and overlap audit, apply realtime-screen-baseline-ablation.patch
in a scratch checkout of 528552da7df and build with cargo build --locked -p moli.
With an ordinary build, use the default mode to measure that binary's behavior.
Pass --candidate to interleave samples from a second ordinary build.
Builds that publish styles without a retained baseline report zero table entries.

Each sample covers the full RSS high-water window after two warm-up layouts.
The overlap audit separately counts table entry storage estimates and the
shallow size of computed values with a single remaining strong reference.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path

from moli_benchmark.config import clear_proxy_env, reserve_port
from moli_benchmark.raw_cdp import connect_raw_cdp
from moli_benchmark.serve import probe_url


WIDTHS = (1000, 1024)
MODES = ("keep", "release-old", "no-baseline")
ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")
BASELINE_LINE = re.compile(
    r'stage="screen_style_baseline" element_count=(\d+) entry_storage_bytes=(\d+)'
)
OVERLAP_LINE = re.compile(
    r'stage="screen_style_overlap" old_entries=(\d+) old_entry_storage_bytes=(\d+) '
    r"new_entry_storage_bytes=(\d+) old_unique_style_bytes=(\d+)"
)


def fixture(elements: int, distinct: bool) -> str:
    return f"""(() => {{
        const styles = document.createElement('style');
        styles.textContent = `
            :root {{ --shift: 0px; }}
            @media (min-width: 1012px) {{ :root {{ --shift: 8px; }} }}
            .tile {{
                width: calc(64px + var(--offset, 0px) + var(--shift));
                height: 4px;
            }}
            .tile::before {{ display: block; content: ""; width: inherit; height: 1px; }}
        `;
        document.head.append(styles);
        document.body.innerHTML = Array.from({{length: {elements}}}, (_, index) => {{
            const offset = {json.dumps(distinct)} ? ' style="--offset:' + (index / 50) + 'px"' : '';
            return '<div class="tile"' + offset + '></div>';
        }}).join('');
        return document.body.childElementCount;
    }})()"""


@contextmanager
def server(binary: Path, mode: str, audit_overlap: bool, timeout: float):
    env = clear_proxy_env(os.environ)
    env["MOLI_CPU_PROFILE"] = "1"
    env["MOLI_PROBE_SCREEN_BASELINE"] = mode
    env.pop("MOLI_PROBE_SCREEN_BASELINE_OVERLAP", None)
    if audit_overlap:
        env["MOLI_PROBE_SCREEN_BASELINE_OVERLAP"] = "1"
    with reserve_port() as port, tempfile.TemporaryDirectory() as temp:
        endpoint = f"http://127.0.0.1:{port.port}"
        log = Path(temp) / "moli.log"
        port.release_socket()
        with log.open("wb") as stderr, subprocess.Popen(
            [
                str(binary), "serve", "--realtime", "--host", "127.0.0.1",
                "--port", str(port.port), "--log-level", "info",
            ],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=stderr,
        ) as process:
            try:
                deadline = time.monotonic() + timeout
                while process.poll() is None and time.monotonic() < deadline:
                    if probe_url(endpoint + "/json/version"):
                        break
                    time.sleep(0.025)
                else:
                    raise RuntimeError("Moli CDP did not become ready")
                yield process.pid, endpoint, log
            except Exception:
                print(log.read_text(errors="replace")[-6000:], file=sys.stderr)
                raise
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        process.kill()


def proc_bytes(path: Path, *fields: str) -> dict[str, int]:
    values = {}
    for line in path.read_text().splitlines():
        name, _, value = line.partition(":")
        if name in fields:
            kib, unit = value.split()
            if unit != "kB":
                raise RuntimeError(f"unexpected unit for {name} in {path}")
            values[name] = int(kib) * 1024
    if values.keys() != set(fields):
        raise RuntimeError(f"missing memory fields in {path}: {set(fields) - values.keys()}")
    return values


async def run_sample(args, binary: Path, mode: str, elements: int, style: str) -> dict:
    distinct = style == "distinct"
    with server(binary, mode, args.audit_overlap, args.timeout) as (pid, endpoint, log):
        client = await connect_raw_cdp(endpoint)
        session = None

        async def call(method, params=None):
            message_id = await client.send(method, params, session_id=session)
            response, _ = await client.recv_until_id(message_id, timeout=args.timeout)
            return response["result"]

        async def evaluate(expression):
            result = await call("Runtime.evaluate", {"expression": expression, "returnByValue": True})
            if result.get("exceptionDetails"):
                raise RuntimeError(result["exceptionDetails"])
            return result["result"]["value"]

        async def resize(width):
            await call("Emulation.setDeviceMetricsOverride", {
                "width": width, "height": 640, "deviceScaleFactor": 1, "mobile": False,
            })
            measured = await evaluate("document.body.lastElementChild.getBoundingClientRect().width")
            expected = 64 + (8 if width == WIDTHS[1] else 0) + ((elements - 1) / 50 if distinct else 0)
            if abs(measured - expected) > 0.05:
                raise RuntimeError(f"viewport {width} produced width {measured}, expected {expected}")

        try:
            version = await call("Browser.getVersion")
            target = await call("Target.createTarget", {"url": "about:blank"})
            attachment = await call("Target.attachToTarget", {
                "targetId": target["targetId"], "flatten": True,
            })
            session = attachment["sessionId"]
            if await evaluate(fixture(elements, distinct)) != elements:
                raise RuntimeError("fixture size differs from the requested element count")
            for width in WIDTHS[::-1]:
                await resize(width)
            before_trace = ANSI_ESCAPE.sub("", log.read_text())
            proc = Path(f"/proc/{pid}")
            before = proc_bytes(proc / "smaps_rollup", "Rss", "Pss", "Pss_Anon")
            (proc / "clear_refs").write_text("5\n")
            started = time.monotonic()
            for index in range(args.passes):
                await resize(WIDTHS[(index + 1) % len(WIDTHS)])
            elapsed_ms = (time.monotonic() - started) * 1000
            peak = proc_bytes(proc / "status", "VmHWM")["VmHWM"]
            after = proc_bytes(proc / "smaps_rollup", "Rss", "Pss", "Pss_Anon")
            trace = ANSI_ESCAPE.sub("", log.read_text())[len(before_trace):]
            layouts = trace.count('stage="layout_pass"')
            publications = [tuple(map(int, row)) for row in BASELINE_LINE.findall(trace)]
            publication_count = len(publications) + trace.count('stage="screen_style_publication"')
            if layouts != args.passes or publication_count != args.passes:
                raise RuntimeError(
                    f"expected {args.passes} full screen passes, observed {layouts} layouts "
                    f"and {publication_count} screen publications"
                )
            if any(
                count != 0 if mode == "no-baseline" else count < elements
                for count, _ in publications
            ):
                raise RuntimeError(f"unexpected baseline entries for {mode}: {publications}")
            overlap = [tuple(map(int, row)) for row in OVERLAP_LINE.findall(trace)]
            if args.audit_overlap:
                if len(overlap) != args.passes:
                    raise RuntimeError("the binary did not report overlap at each publication")
                if any(
                    old_entries < elements if mode == "keep" else old_entries != 0
                    for old_entries, *_ in overlap
                ):
                    raise RuntimeError(f"unexpected old baseline entries for {mode}: {overlap}")
            return {
                "style": style, "elements": elements, "revision": version["revision"],
                "layout_passes": layouts, "elapsed_ms": elapsed_ms, "peak_rss_bytes": peak,
                "before": before, "after": after,
                "baseline_entries": max((entries for entries, _ in publications), default=0),
                "baseline_entry_estimate_bytes": max((size for _, size in publications), default=0),
                "overlap": [
                    {
                        "old_entries": old_entries, "old_entry_estimate_bytes": old_bytes,
                        "new_entry_estimate_bytes": new_bytes,
                        "old_unique_style_inline_bytes": unique_bytes,
                    }
                    for old_entries, old_bytes, new_bytes, unique_bytes in overlap
                ],
            }
        finally:
            await client.websocket.close()


async def main(args):
    variants = [(mode, args.moli, mode) for mode in args.modes]
    if args.candidate:
        variants.append(("candidate", args.candidate, "keep"))
    samples = []
    medians = []
    for elements in args.elements:
        for style in args.styles:
            group = {label: [] for label, _, _ in variants}
            for run in range(args.runs):
                start = run % len(variants)
                for label, binary, mode in variants[start:] + variants[:start]:
                    sample = await run_sample(args, binary, mode, elements, style)
                    sample["mode"] = label
                    samples.append(sample)
                    group[label].append(sample["peak_rss_bytes"])
                    print(
                        f"{elements} {style} {label} {run + 1}/{args.runs} "
                        f"{sample['peak_rss_bytes'] / 1048576:.2f} MiB peak RSS",
                        file=sys.stderr, flush=True,
                    )
            medians.extend({
                "elements": elements, "style": style, "mode": mode,
                "peak_rss_bytes": statistics.median(peaks),
            } for mode, peaks in group.items())
    print(json.dumps({"samples": samples, "medians": medians}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--moli", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, help="second Moli binary for comparison")
    parser.add_argument("--elements", type=int, nargs="+", default=[5000, 20000])
    parser.add_argument("--styles", choices=("shared", "distinct"), nargs="+", default=["shared", "distinct"])
    parser.add_argument("--modes", choices=MODES, nargs="+", default=["keep"])
    parser.add_argument("--passes", type=int, default=6)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--audit-overlap", action="store_true")
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("this probe requires Linux procfs")
    if min(args.runs, args.passes, *args.elements) <= 0 or args.timeout <= 0:
        parser.error("counts and timeout must be positive")
    if args.candidate and args.audit_overlap:
        parser.error("run the overlap audit on the instrumented binary alone")
    args.moli = args.moli.resolve()
    if args.candidate:
        args.candidate = args.candidate.resolve()
    asyncio.run(main(args))
