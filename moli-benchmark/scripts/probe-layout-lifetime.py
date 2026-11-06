"""Measure layout-policy retention across interaction, child and Page lifetimes.

Each cohort keeps one server alive throughout all phases. No screenshots or
optional resource downloads are requested. Natural retention is sampled before
the explicitly labelled end-of-phase GC experiment; no GC is forced per cycle.
Legacy samples validate their own snapshot contract, not fresh-layout success.
PSS/RSS are process measurements, not a census of live layout allocations.

Run with: uv run --no-project --with websockets==15.0.1 python <this file> --help
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import http.server
import json
import os
import platform
import statistics
import sys
import tempfile
import threading
import time
import traceback
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from moli_benchmark.raw_cdp import connect_raw_cdp
from moli_benchmark.sampling import (
    affinity_snapshot,
    clock_ticks_evidence,
    cpu_snapshot,
    error_evidence,
    instrumentation_manifest,
    snapshot_resources,
)
from moli_benchmark.sampling import (
    cpu_delta as observed_cpu_delta,
)
from moli_benchmark.target_serve import (
    finish_target_serve_run,
    start_target_serve,
    stop_fixture_server,
)

MODES = {"Mock": (), "OnDemand": ("--layout",), "Realtime": ("--realtime",)}
SCRIPT = """
window.probe = {
  nodes: [...document.querySelectorAll('.item')],
  frames: () => new Promise(resolve => requestAnimationFrame(() =>
    requestAnimationFrame(() => setTimeout(resolve, 0))))
};
probe.run = async function(seed) {
  const values = [], timings = [];
  for (let step=0; step<4; step++) {
    const begin = performance.now();
    this.nodes[0].style.width = (200 + (seed*4+step)%80)+'px';
    for (let i=1; i<=16; i++)
      this.nodes[i].textContent = 'row '+i+' generation '+seed+' step '+step;
    values.push(this.nodes[0].offsetWidth);
    timings.push(performance.now()-begin);
  }
  await this.frames();
  return {values, timings, nodes:document.querySelectorAll('.item').length};
};
probe.child = async function(seed) {
  const frame = document.createElement('iframe');
  frame.style.cssText='width:300px;height:100px;border:0';
  const loaded = new Promise(resolve => { frame.onload=resolve; });
  frame.srcdoc = '<!doctype html><style>p{width:123px;margin:0;font:16px serif}</style>'+
    Array.from({length:64},(_,i)=>'<p>child '+seed+' row '+i+'</p>').join('');
  document.body.append(frame);
  await loaded;
  const width = frame.contentDocument.querySelector('p').offsetWidth;
  const count = frame.contentDocument.querySelectorAll('p').length;
  await this.frames();
  frame.remove();
  await this.frames();
  return {width,count,remaining:document.querySelectorAll('iframe').length};
};
window.fixtureInstance=Math.random().toString(36).slice(2);
window.fixtureReady=true;
"""


def fixture(nodes):
    return (
        '<!doctype html><meta charset="utf-8"><style>body{margin:0}'
        ".item{display:block;width:100px;height:20px;font:16px serif}</style>"
        + "<div class=item>Alpha Beta Gamma</div>" * nodes
        + "<script>"
        + SCRIPT
        + "</script>"
    ).encode()


def resource_trend(checkpoints):
    result = {}
    for key in ("pss_bytes", "rss_bytes", "thread_count", "fd_count"):
        values = [point["resources"][key] for point in checkpoints]
        if not values or any(value is None for value in values):
            result[key] = {"complete": False}
            continue
        # Describe the second half, separately from startup/allocator warming.
        warm = values[len(values) // 2 :]
        n = len(warm)
        mean_x, mean_y = (n - 1) / 2, statistics.mean(warm)
        denominator = sum((i - mean_x) ** 2 for i in range(n))
        result[key] = {
            "complete": True,
            "first": values[0],
            "last": values[-1],
            "peak_checkpoint": max(values),
            "second_half_min": min(warm),
            "second_half_max": max(warm),
            "second_half_slope_per_cycle": (
                sum((i - mean_x) * (value - mean_y) for i, value in enumerate(warm))
                / denominator
                if denominator
                else None
            ),
        }
    return result


async def one_run(binary, mode, url, args, record, save):
    handle = client = None
    page = session = None
    errors = []
    record.update(
        mode=mode, phases={}, started_unix=time.time(), ok=False, runtime_errors=errors
    )
    started = time.monotonic()
    try:
        handle = start_target_serve(
            "moli", binary, 10, extra_args=MODES[mode], sample_interval_seconds=0.05
        )
        pid = handle.process.pid
        record.update(command=handle.command, pid=pid, startup_ms=handle.ready_ms)
        expected_root = (handle.root_identity or {}).get("identity")
        record["clock_ticks"] = clock_ticks_evidence()
        record["pin_attempts"] = []
        cpu_observations = []
        resource_observations = []

        def cpu_delta(before, after):
            evidence = observed_cpu_delta(
                before, after, record["clock_ticks"], expected_root
            )
            cpu_observations.append(evidence)
            return evidence

        if args.cpu is not None:
            for task in Path(f"/proc/{pid}/task").glob("[0-9]*"):
                pin = {
                    "tid": int(task.name),
                    "requested_cpu": args.cpu,
                    "started_monotonic_ns": time.monotonic_ns(),
                }
                record["pin_attempts"].append(pin)
                try:
                    os.sched_setaffinity(int(task.name), {args.cpu})
                except ProcessLookupError as error:
                    pin["error"] = error_evidence(error)
                finally:
                    pin["finished_monotonic_ns"] = time.monotonic_ns()
        record["affinity_before"] = affinity_snapshot(pid, args.cpu, expected_root)
        client = await connect_raw_cdp(handle.endpoint)

        async def command(method, params=None, target_session=None):
            ident = None
            try:
                ident = await client.send(method, params, session_id=target_session)
                response, seen = await client.recv_until_id(
                    ident, timeout=30, method=method
                )
            except BaseException as error:
                observed = getattr(error, "cdp_receive_evidence", None)
                record.setdefault("cdp_command_errors", []).append(
                    {
                        "method": method,
                        "message_id": ident,
                        "session_id": target_session,
                        "error": error_evidence(error),
                        "receive": observed,
                    }
                )
                seen = observed["messages"] if observed is not None else []
                errors.extend(
                    e for e in seen if e.get("method") == "Runtime.exceptionThrown"
                )
                raise
            errors.extend(
                e for e in seen if e.get("method") == "Runtime.exceptionThrown"
            )
            return response["result"]

        async def evaluate(expression):
            value = await command(
                "Runtime.evaluate",
                {"expression": expression, "returnByValue": True, "awaitPromise": True},
                session,
            )
            if "exceptionDetails" in value:
                raise RuntimeError(f"JavaScript failed: {value}")
            return value["result"].get("value")

        async def targets():
            return sorted(
                info["targetId"]
                for info in (await command("Target.getTargets"))["targetInfos"]
                if info["type"] == "page"
            )

        async def create():
            nonlocal page, session
            page = (await command("Target.createTarget", {"url": "about:blank"}))[
                "targetId"
            ]
            session = (
                await command(
                    "Target.attachToTarget", {"targetId": page, "flatten": True}
                )
            )["sessionId"]
            await command("Runtime.enable", {}, session)
            await command("Page.enable", {}, session)
            await command(
                "Emulation.setDeviceMetricsOverride",
                {"width": 1280, "height": 720, "deviceScaleFactor": 1, "mobile": False},
                session,
            )

        async def navigate():
            # Reuse the exact URL so an unbounded HTTP cache key set cannot
            # masquerade as a document/layout retention problem.
            previous = await evaluate("globalThis.fixtureInstance??null")
            await command("Page.navigate", {"url": url}, session)
            deadline = time.monotonic() + 15
            ready = (
                "window.fixtureReady===true && window.fixtureInstance!=="
                + json.dumps(previous)
            )
            while not await evaluate(ready):
                if time.monotonic() >= deadline:
                    raise RuntimeError("fixture not ready")
                await asyncio.sleep(0.01)
            value = await evaluate("probe.nodes[0].offsetWidth")
            assert value == 100, value
            await evaluate("probe.frames()")

        async def close():
            nonlocal page, session
            closed = {"target": page}
            record.setdefault("target_closes", []).append(closed)
            result = await command("Target.closeTarget", {"targetId": page})
            closed["reply"] = result
            assert result["success"], result
            page = session = None
            remaining = await targets()
            closed["remaining"] = remaining
            assert remaining == record["initial_targets"], remaining

        async def checkpoint():
            result = {
                "elapsed_seconds": time.monotonic() - started,
                "resources": snapshot_resources(pid, include_lifetime_cpu=False),
            }
            resource_observations.append(result["resources"])
            if session is not None:
                result["heap"] = await command("Runtime.getHeapUsage", {}, session)
            return result

        async def idle(label):
            before = cpu_snapshot(pid)
            begin = time.monotonic()
            # This is the measured idle window, not a readiness workaround.
            await asyncio.sleep(args.idle_seconds)
            return {
                "label": label,
                "wall_seconds": time.monotonic() - begin,
                **cpu_delta(before, cpu_snapshot(pid)),
                **await checkpoint(),
            }

        record["version"] = await command("Browser.getVersion")
        record["initial_targets"] = await targets()
        record["server_idle"] = await idle("server_before_pages")
        await create()
        await navigate()
        record["page_initial"] = await checkpoint()
        for name in ("interaction", "child_churn", "navigation", "page_close"):
            phase = {
                "cycles": [],
                "started_elapsed_seconds": time.monotonic() - started,
            }
            record["phases"][name] = phase
            for index in range(args.cycles):
                if name == "page_close":
                    if page is not None:
                        await close()
                    await create()
                    await navigate()
                elif name == "navigation":
                    await navigate()
                before = cpu_snapshot(pid)
                begin = time.monotonic()
                value = await evaluate(
                    f"probe.{'child' if name == 'child_churn' else 'run'}({index})"
                )
                measured = {
                    "index": index,
                    "operation_ms": (time.monotonic() - begin) * 1000,
                    **cpu_delta(before, cpu_snapshot(pid)),
                    "value": value,
                }
                # Keep the returned result even when its contract fails;
                # the finally block persists the entire partial cohort.
                phase["cycles"].append(measured)
                if name == "child_churn":
                    assert value["count"] == 64 and value["remaining"] == 0, value
                    if mode == "Realtime":
                        assert value["width"] == 123, value
                else:
                    expected = (
                        [200 + (index * 4 + step) % 80 for step in range(4)]
                        if mode == "Realtime"
                        else [100] * 4
                    )
                    assert value["values"] == expected, value
                    assert value["nodes"] == args.nodes, value
                if name == "page_close":
                    await close()
                measured.update(await checkpoint())
            phase["trend"] = resource_trend(phase["cycles"])
            phase["natural_idle"] = await idle(name + "_natural")
            if session is not None:
                phase["diagnostics_before_gc"] = await command(
                    "HeapProfiler.moliDiagnostics", {}, session
                )
                await command("HeapProfiler.collectGarbage", {}, session)
                phase["explicit_gc_idle"] = await idle(name + "_after_explicit_gc")
            save()
            print(
                json.dumps({"mode": mode, "phase": name, "cycles": args.cycles}),
                flush=True,
            )
        record["final_targets"] = await targets()
        record["final_idle"] = await idle("server_after_pages")
        # moliDiagnostics is a Page-session command. After closing every Page
        # use target enumeration and procfs; don't create a Page just to sample.
        record["runtime_errors"] = errors
        assert not errors, errors
        record["affinity_after"] = affinity_snapshot(pid, args.cpu, expected_root)
        record["checkpoint_reads_complete"] = bool(resource_observations) and all(
            point.get("selected_process_reads_complete") is True
            for point in resource_observations
        )
        record["cpu_evidence_complete"] = bool(cpu_observations) and all(
            item["process_cpu"]["cpu_ms"] is not None
            and item["thread_cpu_observed"]["boundary_read_complete"]
            and not item["thread_cpu_observed"]["counter_regressions"]
            for item in cpu_observations
        )
        record["affinity_evidence_complete"] = all(
            record[key]["boundary_complete"]
            and (args.cpu is None or record[key]["all_observed_match"])
            for key in ("affinity_before", "affinity_after")
        )
        record["ok"] = True
    except asyncio.CancelledError as error:
        record["error"] = f"{type(error).__name__}: {error}"
        record["traceback"] = traceback.format_exc()
        if getattr(error, "evidence", None) is not None:
            record["stopped"] = error.evidence
        raise
    except Exception as error:  # noqa: BLE001 -- Preserve every failed cohort; main exits nonzero.
        record["error"] = f"{type(error).__name__}: {error}"
        record["traceback"] = traceback.format_exc()
        if getattr(error, "evidence", None) is not None:
            record["stopped"] = error.evidence
    finally:
        try:
            await finish_target_serve_run(client, handle, record, key="stopped")
        finally:
            record["elapsed_seconds"] = time.monotonic() - started
            resources = record["stopped"].get("resources", {})
            if resources.get("observer_error") or resources.get(
                "thread_alive_after_stop"
            ):
                record["ok"] = False
                record["sampling_error"] = resources
            save()


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--baseline-sha")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--cycles", type=int, default=64)
    parser.add_argument("--nodes", type=int, default=1024)
    parser.add_argument("--idle-seconds", type=float, default=3)
    parser.add_argument("--cpu", type=int)
    args = parser.parse_args()
    if args.cycles < 2 or args.runs < 1 or args.nodes < 17 or args.idle_seconds <= 0:
        parser.error("cycles >= 2, runs >= 1, nodes >= 17, idle-seconds > 0 required")
    if bool(args.baseline) != bool(args.baseline_sha):
        parser.error("baseline and baseline-sha must be supplied together")
    if args.cpu is not None and args.cpu not in os.sched_getaffinity(0):
        parser.error("CPU outside allowed affinity")
    # Refuse to erase any preceding failed/partial evidence.
    with args.output.open("x") as stream:
        stream.write("{}\n")
    removed_env = sorted(key for key in os.environ if key.startswith("MOLI_"))
    for key in removed_env:
        del os.environ[key]
    html = fixture(args.nodes)

    class Fixture(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(html)))
            self.end_headers()
            self.wfile.write(html)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    binaries = {"candidate": args.candidate.resolve()}
    if args.baseline:
        binaries["baseline"] = args.baseline.resolve()
    manifest = {
        "schema": 1,
        "instrumentation": instrumentation_manifest(
            Path(__file__),
            Path(sys.modules["moli_benchmark.raw_cdp"].__file__),
        ),
        "parameters": {
            key: str(value) if isinstance(value, Path) else value
            for key, value in vars(args).items()
        },
        "binary_sha256": {
            key: hashlib.sha256(path.read_bytes()).hexdigest()
            for key, path in binaries.items()
        },
        "probe_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "fixture_sha256": hashlib.sha256(html).hexdigest(),
        "platform": platform.platform(),
        "host_load_before": os.getloadavg(),
        "removed_environment_keys": removed_env,
        "samples": [],
    }

    def save():
        # Publish only whole JSON documents. Save at phase/failure boundaries,
        # so growing history doesn't impose O(cycles²) serialization work.
        with tempfile.NamedTemporaryFile(
            mode="w",
            dir=args.output.parent,
            prefix=args.output.name + ".",
            delete=False,
        ) as stream:
            json.dump(manifest, stream, indent=2)
            stream.write("\n")
            temporary = Path(stream.name)
        temporary.replace(args.output)

    save()
    try:
        for run in range(args.runs):
            cohorts = [
                (build, mode) for mode in ("Mock", "OnDemand") for build in binaries
            ]
            if run % 2:
                cohorts.reverse()
            cohorts.insert(
                0 if run % 2 == 0 else len(cohorts), ("candidate", "Realtime")
            )
            for build, mode in cohorts:
                record = {"build": build, "repetition": run}
                manifest["samples"].append(record)
                await one_run(
                    binaries[build],
                    mode,
                    f"http://127.0.0.1:{server.server_port}/fixture",
                    args,
                    record,
                    save,
                )
                print(
                    json.dumps(
                        {"build": build, "mode": mode, "run": run, "ok": record["ok"]}
                    ),
                    flush=True,
                )
                if not record["safe_to_continue"]:
                    raise RuntimeError(
                        "target cleanup failed; stopping subsequent cohorts"
                    )
    finally:
        manifest["fixture_server_cleanup"] = stop_fixture_server(server, thread)
        manifest["host_load_after"] = os.getloadavg()
        save()
    if not manifest["fixture_server_cleanup"]["complete"] or any(
        not record["ok"] for record in manifest["samples"]
    ):
        raise SystemExit(
            "One or more cohorts failed; all partial samples remain in the output"
        )


if __name__ == "__main__":
    asyncio.run(main())
