"""Compare release layout policies through real CDP, without screenshots.

Use a built baseline and candidate, alternate their order, retain every sample
and validate each policy's actual contract. Legacy samples deliberately measure
legacy geometry; their lower costs aren't evidence of current-layout success.
This probe is one part of P7, not the complete release acceptance suite.

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
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "moli-benchmark"))
from moli_benchmark.raw_cdp import RawCdpError, connect_raw_cdp
from moli_benchmark.sampling import snapshot_resources
from moli_benchmark.target_serve import start_target_serve, stop_target_serve

PHASES = {
    "clean": """const values=[];let sum=0;for(let j=0;j<8;j++){
      const begin=performance.now();for(let i=0;i<1024;i++)sum+=probe.nodes[i%probe.nodes.length].offsetWidth;
      values.push(performance.now()-begin)}return {sum,timings:values};""",
    "cssom": """let sum=0;const styles=probe.nodes.slice(0,64).map(n=>getComputedStyle(n));
      for(let i=0;i<2048;i++)sum+=styles[i%styles.length].width.length+styles[i%styles.length].color.length;
      return {sum};""",
    "batch": """for(let i=0;i<1024;i++)probe.nodes[i%probe.nodes.length].style.width=(100+i%20)+'px';
      probe.nodes[0].style.width='333px';return {width:probe.nodes[0].offsetWidth};""",
    "alternating": """const values=[],timings=[];for(let i=0;i<64;i++){
      const begin=performance.now();probe.nodes[0].style.width=(300+i%20)+'px';
      values.push(probe.nodes[0].offsetWidth);timings.push(performance.now()-begin)}
      return {values,timings};""",
    "raf_clean": """return new Promise(resolve=>{const stamps=[],widths=[];function frame(t){
      stamps.push(t);widths.push(probe.nodes[0].offsetWidth);
      if(stamps.length===12)resolve({stamps,widths});else requestAnimationFrame(frame)}requestAnimationFrame(frame)});""",
    "raf_write": """return new Promise(resolve=>{const stamps=[],widths=[],timings=[];function frame(t){
      const start=performance.now();probe.nodes[0].style.width=(400+stamps.length)+'px';
      widths.push(probe.nodes[0].offsetWidth);timings.push(performance.now()-start);stamps.push(t);
      if(stamps.length===12)resolve({stamps,widths,timings});else requestAnimationFrame(frame)}requestAnimationFrame(frame)});""",
}


def fixture(nodes, positioned_blocks=False):
    return (
        "<!doctype html><meta charset=utf-8><style>body{margin:0}.item{display:block;"
        "width:100px;height:20px;padding:0;border:0;font:16px serif"
        + (";position:relative" if positioned_blocks else "")
        + "}</style>"
        + "<div class=item>Alpha Beta Gamma</div>" * nodes
        + '<div id=scroller style="position:fixed;left:0;top:0;width:200px;height:100px;overflow:auto">'
        '<div style="height:10000px"></div></div><script>window.fixtureReady=true</script>'
    ).encode()


def cpu_threads(pid):
    result = {}
    for thread in Path(f"/proc/{pid}/task").iterdir():
        try:
            result[thread.name] = int((thread / "schedstat").read_text().split()[0])
        except FileNotFoundError:
            pass
    return result


def cpu_delta(before, after):
    return {
        "cpu_ms": sum(ns - before.get(tid, 0) for tid, ns in after.items()) / 1e6,
        "retired_threads": sorted(before.keys() - after.keys()),
    }


async def one_run(binary, mode, url, cpu, nodes, idle_seconds):
    flags = {"Mock": (), "OnDemand": ("--layout",), "Realtime": ("--realtime",)}[mode]
    record = {"phases": {}, "mode": mode, "started_unix": time.time()}
    handle = None
    client = None
    try:
        handle = start_target_serve(
            "moli", binary, 10, extra_args=flags, sample_interval_seconds=0.025
        )
        pid = handle.process.pid
        record.update(command=handle.command, startup_ms=handle.ready_ms, pid=pid)
        if cpu is not None:
            for _ in range(2):
                for tid in Path(f"/proc/{pid}/task").iterdir():
                    try:
                        os.sched_setaffinity(int(tid.name), {cpu})
                    except ProcessLookupError:
                        pass
        client = await connect_raw_cdp(handle.endpoint)
        events = []

        async def command(method, params=None, session=None):
            ident = await client.send(method, params, session_id=session)
            response, seen = await client.recv_until_id(ident, timeout=20)
            events.extend(
                event
                for event in seen
                if event.get("method") == "Runtime.bindingCalled"
            )
            return response["result"]

        target = (await command("Target.createTarget", {"url": "about:blank"}))[
            "targetId"
        ]
        session = (
            await command(
                "Target.attachToTarget", {"targetId": target, "flatten": True}
            )
        )["sessionId"]
        await command("Runtime.enable", {}, session)
        await command("Page.enable", {}, session)
        await command(
            "Emulation.setDeviceMetricsOverride",
            {"width": 1280, "height": 720, "deviceScaleFactor": 1, "mobile": False},
            session,
        )
        await command("Runtime.addBinding", {"name": "reportProbeInput"}, session)

        async def evaluate(expression, await_promise=False):
            result = await command(
                "Runtime.evaluate",
                {
                    "expression": expression,
                    "returnByValue": True,
                    "awaitPromise": await_promise,
                },
                session,
            )
            if "exceptionDetails" in result:
                raise RuntimeError(f"JavaScript failed: {result}")
            return result["result"].get("value")

        await command("Page.navigate", {"url": url}, session)
        deadline = time.monotonic() + 15
        while not await evaluate("window.fixtureReady===true"):
            if time.monotonic() > deadline:
                raise RuntimeError("fixture not ready")
            await asyncio.sleep(0.02)
        initial = await evaluate("""(()=>{window.probe={nodes:[...document.querySelectorAll('.item')],inputs:0};
          scroller.addEventListener('wheel',()=>{probe.inputs++;probe.nodes[0].style.width=(500+probe.inputs)+'px';
            Promise.resolve().then(()=>reportProbeInput(JSON.stringify({sequence:probe.inputs,
              width:probe.nodes[0].offsetWidth,scroll:scroller.scrollTop})))});
          return {count:probe.nodes.length,width:probe.nodes[0].offsetWidth}})()""")
        assert initial == {"count": nodes, "width": 100}, initial
        # Let the initial rendering opportunity finish. Measured idle windows
        # below remain separate from this setup interval.
        await asyncio.sleep(0.15)
        record["before"] = snapshot_resources(pid, include_lifetime_cpu=False)

        for name, body in PHASES.items():
            before = cpu_threads(pid)
            started = time.perf_counter()
            value = await evaluate(f"(()=>{{{body}}})()", await_promise=True)
            phase = {
                "wall_ms": (time.perf_counter() - started) * 1000,
                **cpu_delta(before, cpu_threads(pid)),
                "result": value,
            }
            record["phases"][name] = phase
            if name == "clean":
                assert value["sum"] == 819200, value
            elif name == "batch":
                assert value["width"] == (333 if mode == "Realtime" else 100), value
            elif name == "alternating":
                assert value["values"] == (
                    [300 + i % 20 for i in range(64)]
                    if mode == "Realtime"
                    else [100] * 64
                ), value
            elif name.startswith("raf_"):
                expected = (
                    (
                        [400 + i for i in range(12)]
                        if name == "raf_write"
                        else [303] * 12
                    )
                    if mode == "Realtime"
                    else [100] * 12
                )
                assert value["widths"] == expected, value
                assert all(
                    a <= b for a, b in zip(value["stamps"], value["stamps"][1:])
                ), value

        record["input"] = []
        record["input_support"] = mode != "Mock"
        if mode == "Mock":
            try:
                await command(
                    "Input.dispatchMouseEvent",
                    {"type": "mouseWheel", "x": 10, "y": 10, "deltaX": 0, "deltaY": 30},
                    session,
                )
            except RawCdpError as error:
                assert (
                    "coordinate-based mouse input requires layout hit testing"
                    in str(error)
                ), error
                record["input_rejection"] = str(error)
            else:
                raise AssertionError("Mock unexpectedly accepted coordinate input")
        for sequence in range(1, 5) if mode != "Mock" else ():
            before = cpu_threads(pid)
            started = time.perf_counter()
            await command(
                "Input.dispatchMouseEvent",
                {"type": "mouseWheel", "x": 10, "y": 10, "deltaX": 0, "deltaY": 30},
                session,
            )
            ack_ms = (time.perf_counter() - started) * 1000
            deadline = time.monotonic() + 5
            while not events:
                event = await asyncio.wait_for(
                    client.recv(), max(0.01, deadline - time.monotonic())
                )
                if event.get("method") == "Runtime.bindingCalled":
                    events.append(event)
            event = events.pop(0)
            value = json.loads(event["params"]["payload"])
            sample = {
                "ack_ms": ack_ms,
                "completion_ms": (time.perf_counter() - started) * 1000,
                **cpu_delta(before, cpu_threads(pid)),
                "result": value,
            }
            record["input"].append(sample)
            assert value["sequence"] == sequence and value["width"] == (
                500 + sequence if mode == "Realtime" else 100
            ), value
            # Mock does not offer the same scroll geometry contract.
            if mode != "Mock":
                assert value["scroll"] == sequence * 30, value

        await asyncio.sleep(0.15)
        before = cpu_threads(pid)
        started = time.perf_counter()
        await asyncio.sleep(idle_seconds)
        record["idle"] = {
            "wall_ms": (time.perf_counter() - started) * 1000,
            **cpu_delta(before, cpu_threads(pid)),
        }
        record["retained"] = snapshot_resources(pid, include_lifetime_cpu=False)
        record["ok"] = True
    except Exception as error:  # noqa: BLE001 -- retain failures; main fails the run
        record.update(ok=False, error=repr(error))
    finally:
        if client is not None:
            await client.websocket.close()
        record["server"] = stop_target_serve(handle, include_resource_samples=True)
    return record


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline-sha", required=True)
    parser.add_argument("--candidate-sha", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--nodes", type=int, default=1024)
    parser.add_argument(
        "--positioned-blocks",
        action="store_true",
        help="Use position:relative blocks to exercise z-index:auto paint groups",
    )
    parser.add_argument("--idle-seconds", type=float, default=1)
    parser.add_argument("--cpu", type=int)
    args = parser.parse_args()
    if args.runs < 1 or args.nodes < 64 or args.idle_seconds <= 0:
        parser.error("runs >= 1, nodes >= 64, idle-seconds > 0 required")
    allowed_cpus = sorted(os.sched_getaffinity(0))
    if args.cpu is not None and args.cpu not in allowed_cpus:
        parser.error("CPU outside allowed affinity")
    for key in (
        "MOLI_LAYOUT",
        "MOLI_REALTIME",
        "MOLI_RESOURCE",
        "MOLI_SCROLLBARS",
        "MOLI_IMAGE",
        "MOLI_FONT",
        "MOLI_AUDIO",
        "MOLI_VIDEO",
        "MOLI_MEDIA",
        "MOLI_TEXT_TRACK",
        "MOLI_LOG_LEVEL",
        "MOLI_LOG_FILTER_SCOPES",
        "MOLI_CPU_PROFILE",
    ):
        os.environ.pop(key, None)
    html = fixture(args.nodes, args.positioned_blocks)

    class Fixture(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(html)))
            self.end_headers()
            self.wfile.write(html)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    manifest = {
        "schema": 1,
        "source": {"baseline": args.baseline_sha, "candidate": args.candidate_sha},
        "binary_sha256": {
            label: hashlib.sha256(binary.read_bytes()).hexdigest()
            for label, binary in (
                ("baseline", args.baseline),
                ("candidate", args.candidate),
            )
        },
        "probe_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "fixture_sha256": hashlib.sha256(html).hexdigest(),
        "node_count": args.nodes,
        "positioned_blocks": args.positioned_blocks,
        "viewport": [1280, 720, 1],
        "optional_resources": "none",
        "screenshots": 0,
        "runs": args.runs,
        "platform": platform.platform(),
        "allowed_cpus": allowed_cpus,
        "server_cpu": args.cpu,
        "host_load_before": os.getloadavg(),
        "samples": [],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        for repetition in range(args.runs):
            builds = [("baseline", args.baseline), ("candidate", args.candidate)]
            if repetition % 2:
                builds.reverse()
            cohorts = [
                (label, binary, mode)
                for mode in ("Mock", "OnDemand")
                for label, binary in builds
            ]
            realtime = ("candidate", args.candidate, "Realtime")
            cohorts.insert(0 if repetition % 2 else len(cohorts), realtime)
            for label, binary, mode in cohorts:
                sample = await one_run(
                    binary.resolve(),
                    mode,
                    f"http://127.0.0.1:{server.server_port}/fixture",
                    args.cpu,
                    args.nodes,
                    args.idle_seconds,
                )
                sample.update(build=label, repetition=repetition)
                manifest["samples"].append(sample)
                args.output.write_text(json.dumps(manifest, indent=2) + "\n")
                print(
                    json.dumps(
                        {k: sample[k] for k in ("build", "mode", "repetition", "ok")}
                    ),
                    flush=True,
                )
    finally:
        server.shutdown()
        server.server_close()
        manifest["host_load_after"] = os.getloadavg()
        args.output.write_text(json.dumps(manifest, indent=2) + "\n")
    if any(not sample["ok"] for sample in manifest["samples"]):
        raise SystemExit(
            "One or more workloads failed; every failure remains in the output"
        )


if __name__ == "__main__":
    asyncio.run(main())
