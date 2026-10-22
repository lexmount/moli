"""Measure identical successful layout work in release Moli and Chromium.

Run with: uv run --no-project --with websockets==15.0.1 python <this file> --help
The old-mode probe remains separate: stale geometry is not an equivalent task.
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
from itertools import pairwise
from pathlib import Path
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "moli-benchmark"))
from moli_benchmark.raw_cdp import connect_routed_raw_cdp
from moli_benchmark.realtime_workload import (
    CONTAINER_OBSERVATION,
    CONTAINERS,
    PHASES,
    fixture,
    validate_container,
)
from moli_benchmark.sampling import _process_tree, snapshot_resources
from moli_benchmark.target_serve import start_target_serve, stop_target_serve

ROOT = Path(__file__).resolve().parents[2]


def cpu_snapshot(root_pid):
    threads = {}
    unavailable = []
    for pid in _process_tree(root_pid):
        for task in Path(f"/proc/{pid}/task").glob("[0-9]*"):
            try:
                raw = (task / "stat").read_text()
                start = raw[raw.rfind(")") + 2 :].split()[19]
                threads[f"{pid}:{task.name}:{start}"] = int(
                    (task / "schedstat").read_text().split()[0]
                )
            except OSError:
                unavailable.append(f"{pid}:{task.name}")
    return {"threads": threads, "unavailable": unavailable}


def cpu_delta(before, after):
    old, new = before["threads"], after["threads"]
    return {
        "cpu_ms": sum(ns - old.get(key, 0) for key, ns in new.items()) / 1e6,
        "created_threads": sorted(new.keys() - old.keys()),
        "retired_threads": sorted(old.keys() - new.keys()),
        "unavailable_threads": before["unavailable"] + after["unavailable"],
    }


def pin_tree(root_pid, cpu):
    if cpu is None:
        return
    # Bind existing zygotes as well as the browser so later children inherit
    # the same affinity. Repeat after fixture setup and verify before timing.
    for pid in _process_tree(root_pid):
        for task in Path(f"/proc/{pid}/task").glob("[0-9]*"):
            try:
                os.sched_setaffinity(int(task.name), {cpu})
            except ProcessLookupError:
                pass


def verify_affinity(root_pid, cpu):
    if cpu is None:
        return None
    checked = 0
    for pid in _process_tree(root_pid):
        for task in Path(f"/proc/{pid}/task").glob("[0-9]*"):
            try:
                actual = os.sched_getaffinity(int(task.name))
            except ProcessLookupError:
                continue
            assert actual == {cpu}, (pid, task.name, actual)
            checked += 1
    return {"cpu": cpu, "checked_threads": checked}


async def one_run(binary, engine, url, cpu, nodes, idle_seconds, *, container="blocks"):
    record = {"engine": engine, "phases": {}, "started_unix": time.time()}
    handle = client = None
    try:
        target, flags = (
            ("moli", ("--realtime", "--font"))
            if engine == "Realtime"
            else (
                "chrome-cdp",
                ("--disable-background-networking", "--disable-component-update"),
            )
        )
        handle = start_target_serve(
            target, binary, 10, extra_args=flags, sample_interval_seconds=0.025
        )
        pid = handle.process.pid
        record.update(command=handle.command, pid=pid, startup_ms=handle.ready_ms)
        pin_tree(pid, cpu)
        client = await connect_routed_raw_cdp(handle.endpoint)

        async def command(method, params=None, session=None):
            result = await client.command(
                method, params, session_id=session, timeout=20
            )
            return result.response["result"]

        record["protocol_version"] = await command("Browser.getVersion")
        page = (await command("Target.createTarget", {"url": "about:blank"}))[
            "targetId"
        ]
        # A fresh Chromium server includes an initial blank tab. Keep one page
        # in both engines; don't charge that extra tab to the measured fixture.
        for info in (await command("Target.getTargets"))["targetInfos"]:
            if info["type"] == "page" and info["targetId"] != page:
                await command("Target.closeTarget", {"targetId": info["targetId"]})
        session = (
            await command("Target.attachToTarget", {"targetId": page, "flatten": True})
        )["sessionId"]
        await command("Runtime.enable", {}, session)
        await command("Page.enable", {}, session)
        await command(
            "Emulation.setDeviceMetricsOverride",
            {"width": 1280, "height": 720, "deviceScaleFactor": 1, "mobile": False},
            session,
        )
        await command("Runtime.addBinding", {"name": "reportProbeInput"}, session)

        async def evaluate(expression, await_promise=True):
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
        initial = await evaluate("""(async()=>{
          await document.fonts.load('16px LayoutProbe','Alpha Beta Gamma');
          await document.fonts.ready;
          window.probe={nodes:[...document.querySelectorAll('.item')],inputs:0,reported:0,pending:null};
          function report(){const p=probe.pending;
            if(!p||!p.checkpoint||scroller.scrollTop!==p.sequence*30||probe.reported===p.sequence)return;
            probe.reported=p.sequence;
            reportProbeInput(JSON.stringify({...p,scroll:scroller.scrollTop,completedAt:performance.now()}));}
          scroller.addEventListener('scroll',report);
          scroller.addEventListener('wheel',()=>{
            const sequence=++probe.inputs;
            probe.pending={sequence,handlerAt:performance.now(),checkpoint:false};
            probe.nodes[0].style.width=(500+sequence)+'px';
            queueMicrotask(()=>{
              probe.pending.width=probe.nodes[0].offsetWidth;
              probe.pending.checkpoint=true;probe.pending.checkpointAt=performance.now();report();
            });
          },{passive:false});
          const result={count:probe.nodes.length,width:probe.nodes[0].offsetWidth,
            fontReady:document.fonts.check('16px LayoutProbe'),
            fonts:[...document.fonts].map(face=>({family:face.family,status:face.status})),
            viewport:[innerWidth,innerHeight,devicePixelRatio]};
          await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
          return result;
        })()""")
        record["initial"] = initial
        assert initial["count"] == nodes and initial["width"] == 100, initial
        assert initial["viewport"] == [1280, 720, 1], initial
        assert initial["fontReady"] and any(
            face["family"].strip('"') == "LayoutProbe" and face["status"] == "loaded"
            for face in initial["fonts"]
        ), initial
        if container != "blocks":
            record["container"] = await evaluate(CONTAINER_OBSERVATION)
            validate_container(record["container"], container, nodes)
        pin_tree(pid, cpu)
        record["affinity"] = verify_affinity(pid, cpu)
        record["before"] = snapshot_resources(pid, include_lifetime_cpu=False)

        for name, body in PHASES.items():
            before = cpu_snapshot(pid)
            start = time.perf_counter()
            value = await evaluate(f"(()=>{{{body}}})()")
            elapsed = (time.perf_counter() - start) * 1000
            record["phases"][name] = {
                "wall_ms": elapsed,
                **cpu_delta(before, cpu_snapshot(pid)),
                "result": value,
            }
            if name == "clean":
                assert value["sum"] == 819200, value
            elif name == "batch":
                assert value["width"] == 333, value
            elif name == "alternating":
                assert value["values"] == [300 + i % 20 for i in range(64)], value
            elif name.startswith("raf_"):
                expected = (
                    [400 + i for i in range(12)] if name == "raf_write" else [303] * 12
                )
                assert value["widths"] == expected, value
                assert all(a <= b for a, b in pairwise(value["stamps"])), value

        record["input"] = []
        for sequence in range(1, 5):
            before = cpu_snapshot(pid)
            before_sequence = client.current_sequence
            start = time.perf_counter()
            reply = await client.command(
                "Input.dispatchMouseEvent",
                {"type": "mouseWheel", "x": 10, "y": 10, "deltaX": 0, "deltaY": 30},
                session_id=session,
                timeout=5,
            )
            acknowledgement = next(
                message
                for message in reply.messages
                if message.payload.get("id") == reply.message_id
            )
            event = await client.wait_for_event(
                "Runtime.bindingCalled",
                after_sequence=before_sequence,
                session_id=session,
                timeout=5,
                predicate=lambda message: (
                    message["params"]["name"] == "reportProbeInput"
                ),
            )
            value = json.loads(event.payload["params"]["payload"])
            record["input"].append(
                {
                    "ack_ms": (acknowledgement.received_monotonic - start) * 1000,
                    "completion_ms": (event.received_monotonic - start) * 1000,
                    "ack_message_sequence": acknowledgement.sequence,
                    "completion_message_sequence": event.sequence,
                    **cpu_delta(before, cpu_snapshot(pid)),
                    "result": value,
                }
            )
            assert value["sequence"] == sequence and value["width"] == 500 + sequence, (
                value
            )
            assert value["checkpoint"] and value["scroll"] == sequence * 30, value
        completions = [
            message
            for message in client.messages_since(0)
            if message.payload.get("method") == "Runtime.bindingCalled"
        ]
        assert len(completions) == 4, "duplicate input completion"

        before = cpu_snapshot(pid)
        start = time.perf_counter()
        await asyncio.sleep(idle_seconds)
        record["idle"] = {
            "wall_ms": (time.perf_counter() - start) * 1000,
            **cpu_delta(before, cpu_snapshot(pid)),
        }
        record["retained"] = snapshot_resources(pid, include_lifetime_cpu=False)
        record["affinity_after"] = verify_affinity(pid, cpu)
        record["ok"] = True
    except Exception as error:  # noqa: BLE001 -- retain failures; main fails the run
        record.update(ok=False, error=repr(error))
        if client is not None:
            record["failure_messages"] = client.recorded_messages()
    finally:
        if client is not None:
            await client.close()
        record["server"] = stop_target_serve(handle, include_resource_samples=True)
    return record


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--moli", type=Path, required=True)
    parser.add_argument("--moli-sha", required=True)
    parser.add_argument(
        "--chromium",
        type=Path,
        required=True,
        help="Actual Chromium executable, not a launcher script",
    )
    parser.add_argument(
        "--font", type=Path, required=True, help="Same TTF bytes served to both engines"
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--nodes", type=int, default=1024)
    parser.add_argument("--container", choices=CONTAINERS, default="blocks")
    parser.add_argument("--positioned-blocks", action="store_true")
    parser.add_argument("--cpu", type=int)
    parser.add_argument("--idle-seconds", type=float, default=1)
    args = parser.parse_args()
    if args.runs < 1 or args.nodes < 64 or args.idle_seconds <= 0:
        parser.error("runs >= 1, nodes >= 64, idle-seconds > 0 required")
    allowed_cpus = sorted(os.sched_getaffinity(0))
    if args.cpu is not None and args.cpu not in allowed_cpus:
        parser.error("CPU outside allowed affinity")
    if args.output.exists():
        parser.error("output already exists; preserve previous samples")
    for key in list(os.environ):
        if key.startswith("MOLI_"):
            del os.environ[key]
    html = fixture(
        args.nodes, args.positioned_blocks, web_font=True, container=args.container
    )
    font = args.font.read_bytes()
    requests = []

    class Fixture(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            path = urlsplit(self.path).path
            requests.append(path)
            body, content_type = (
                (font, "font/ttf")
                if path == "/probe.ttf"
                else (html, "text/html; charset=utf-8")
            )
            if path not in ("/probe.ttf", "/fixture"):
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    manifest = {
        "schema": 1,
        "moli_source": args.moli_sha,
        "binary_sha256": {
            engine: hashlib.sha256(path.read_bytes()).hexdigest()
            for engine, path in [("Realtime", args.moli), ("Chromium", args.chromium)]
        },
        "probe_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "workload_sha256": hashlib.sha256(
            (ROOT / "moli-benchmark/moli_benchmark/realtime_workload.py").read_bytes()
        ).hexdigest(),
        "fixture_sha256": hashlib.sha256(html).hexdigest(),
        "font": {"path": str(args.font), "sha256": hashlib.sha256(font).hexdigest()},
        "node_count": args.nodes,
        "positioned_blocks": args.positioned_blocks,
        "container": args.container,
        "viewport": [1280, 720, 1],
        "screenshots": 0,
        "resources": "local HTML and the same local TTF only; Moli --font",
        "cpu_method": "process-tree per-thread schedstat; exited threads make CPU a lower bound",
        "input_completion": "handler + microtask + fresh width + final scrollTop; excludes presentation",
        "runs": args.runs,
        "platform": platform.platform(),
        "allowed_cpus": allowed_cpus,
        "server_tree_cpu": args.cpu,
        "host_load_before": os.getloadavg(),
        "samples": [],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        for repetition in range(args.runs):
            engines = [("Realtime", args.moli), ("Chromium", args.chromium)]
            if repetition % 2:
                engines.reverse()
            for engine, binary in engines:
                request_start = len(requests)
                sample = await one_run(
                    binary.resolve(),
                    engine,
                    f"http://127.0.0.1:{server.server_port}/fixture",
                    args.cpu,
                    args.nodes,
                    args.idle_seconds,
                    container=args.container,
                )
                sample.update(
                    repetition=repetition, fixture_requests=requests[request_start:]
                )
                manifest["samples"].append(sample)
                args.output.write_text(json.dumps(manifest, indent=2) + "\n")
                print(
                    json.dumps(
                        {key: sample[key] for key in ("engine", "repetition", "ok")}
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
