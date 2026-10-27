"""Compare structural list updates, observers and coordinate input without screenshots.

The shared fixture also runs against cold, warm and evicted work in PageVm tests.
This is a finite functional comparison, not a performance or lifetime benchmark.
"""

import argparse
import asyncio
import hashlib
import http.server
import json
import os
import sys
import threading
import time
from pathlib import Path

repo = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--moli", type=Path, required=True)
parser.add_argument("--moli-sha", required=True)
parser.add_argument("--chromium", type=Path, default=Path("/usr/lib/chromium/chromium"))
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
if len(args.moli_sha) != 40 or any(
    char not in "0123456789abcdef" for char in args.moli_sha
):
    parser.error("moli-sha must be the full embedded source SHA")
for binary in [args.moli, args.chromium]:
    if not binary.is_file():
        parser.error(f"binary does not exist: {binary}")
args.output.parent.mkdir(parents=True, exist_ok=True)
removed_environment = sorted(key for key in os.environ if key.startswith("MOLI_"))
for key in removed_environment:
    del os.environ[key]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


sys.path.insert(0, str(repo / "moli-benchmark"))
from moli_benchmark.raw_cdp import connect_routed_raw_cdp
from moli_benchmark.target_serve import (
    start_target_serve,
    stop_fixture_server,
    stop_target_serve,
)

fixture_path = repo / "moli-renderer-v8/tests/fixtures/realtime-working-set-churn.js"
fixture = fixture_path.read_text()
font = (repo / "moli-layout/tests/fixtures/moli-ahem.ttf").read_bytes()
output = args.output.resolve()
with output.open("x") as stream:
    json.dump({"status": "reserved"}, stream)
requests = []


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        requests.append(self.path)
        if self.path == "/probe.ttf":
            data, kind = font, "font/ttf"
        elif self.path == "/":
            data, kind = (
                b"<!doctype html><meta charset=utf-8><style>@font-face{font-family:MoliAhem;src:url(/probe.ttf)}</style><body></body>",
                "text/html; charset=utf-8",
            )
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
raw = {
    "scope": "Functional two-round dynamic structure/callback/input comparison. Not a formal cost or lifetime experiment.",
    "fixture_sha256": hashlib.sha256(fixture.encode()).hexdigest(),
    "driver_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    "font_sha256": hashlib.sha256(font).hexdigest(),
    "screenshots": 0,
    "viewport": [1280, 720, 1],
    "samples": [],
    "removed_environment": removed_environment,
}


def save():
    output.write_text(json.dumps(raw, indent=2) + "\n")


async def one_run(engine, binary):
    record = {
        "engine": engine,
        "binary": str(binary),
        "snapshots": [],
        "started_unix": time.time(),
        "ok": False,
    }
    raw["samples"].append(record)
    save()
    handle = client = None
    try:
        target, flags = (
            ("moli", ("--realtime", "--font"))
            if engine == "Moli"
            else (
                "chrome-cdp",
                (
                    "--disable-background-networking",
                    "--disable-component-update",
                    "--hide-scrollbars",
                ),
            )
        )
        handle = start_target_serve(target, binary, 15, extra_args=flags)
        record["command"] = handle.command
        record["binary_sha256"] = digest(binary)
        client = await connect_routed_raw_cdp(handle.endpoint)

        async def command(method, params=None, session=None):
            reply = await client.command(method, params, session_id=session, timeout=15)
            return reply.response["result"]

        record["version"] = await command("Browser.getVersion")
        if engine == "Moli":
            assert record["version"]["revision"] == "@" + args.moli_sha
        page = (await command("Target.createTarget", {"url": "about:blank"}))[
            "targetId"
        ]
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

        async def evaluate(script):
            reply = await command(
                "Runtime.evaluate",
                {"expression": script, "returnByValue": True, "awaitPromise": True},
                session,
            )
            if "exceptionDetails" in reply:
                raise RuntimeError(reply)
            return reply["result"].get("value")

        await command(
            "Page.navigate", {"url": f"http://127.0.0.1:{server.server_port}/"}, session
        )
        deadline = time.monotonic() + 15
        while not await evaluate('document.readyState === "complete"'):
            assert time.monotonic() < deadline
            await asyncio.sleep(0.01)
        record["fonts"] = await evaluate(
            "(async()=>{await document.fonts.load('16px MoliAhem','ABCD');await document.fonts.ready;return [...document.fonts].map(face=>({family:face.family,status:face.status}))})()"
        )
        assert any(
            face["family"].strip('"') == "MoliAhem" and face["status"] == "loaded"
            for face in record["fonts"]
        )
        await evaluate(fixture)
        await evaluate(
            "globalThis.churnScrollEvents=0;churn.feed.addEventListener('scroll',()=>churnScrollEvents++)"
        )

        async def settle(label, count, scroll, scroll_events):
            # Poll only callback and DOM state. A timer does not request a frame;
            # no geometry/hit query is made before the page's own work completes.
            expected_io = count * 24 - scroll <= 96
            await evaluate(f"""new Promise((resolve,reject)=>{{
                const deadline=performance.now()+4000;
                function check(){{
                  if(churn.state.raf.at(-1)==={json.dumps(label)} &&
                    churn.state.microtasks.at(-1)==={json.dumps(label)} &&
                    churn.state.ro.at(-1)==={count * 24} &&
                    churn.state.io.at(-1)==={str(expected_io).lower()} &&
                    churnScrollEvents>={scroll_events}){{resolve(true);return;}}
                  if(performance.now()>deadline){{reject(new Error('churn did not settle: '+{json.dumps(label)}));return;}}
                  setTimeout(check,1);
                }}setTimeout(check,0);
            }})""")
            snapshot = await evaluate("churn.snapshot()")
            assert len(snapshot["rows"]) == count
            assert snapshot["list"][3] == count * 24
            assert snapshot["feed"][9] == (count + 1) * 24
            assert snapshot["scrollTop"] == scroll
            assert snapshot["state"]["errors"] == []
            assert snapshot["state"]["raf"] == snapshot["state"]["microtasks"]
            assert all(row["identity"] for row in snapshot["rows"])
            assert snapshot["hit"] == snapshot["rows"][scroll // 24]["id"]
            record["snapshots"].append(snapshot)
            save()
            return snapshot

        count, scroll, scroll_events = 16, 0, 0
        await settle("initial", count, scroll, scroll_events)
        operations = await evaluate("churn.operations")
        for round_number in range(2):
            for operation in operations:
                previous_scroll = scroll
                if operation in ["append", "prepend"]:
                    count += 2
                elif operation == "insert-middle":
                    count += 1
                elif operation in ["remove-head", "remove-middle", "remove-tail"]:
                    count -= 1
                elif operation == "shrink":
                    count = 3
                elif operation == "grow":
                    count += 11
                if operation == "wheel":
                    scroll += 48
                elif operation == "bottom":
                    scroll = (count + 1) * 24 - 96
                elif operation == "top":
                    scroll = 0
                scroll = min(scroll, max(0, (count + 1) * 24 - 96))
                if scroll != previous_scroll:
                    scroll_events += 1
                await evaluate(f"churn.apply({json.dumps(operation)},{round_number})")
                if operation == "wheel":
                    await command(
                        "Input.dispatchMouseEvent",
                        {
                            "type": "mouseWheel",
                            "x": 12,
                            "y": 12,
                            "deltaX": 0,
                            "deltaY": 48,
                        },
                        session,
                    )
                label = str(round_number) + ":" + operation
                snapshot = await settle(label, count, scroll, scroll_events)
                for event in ["mousePressed", "mouseReleased"]:
                    await command(
                        "Input.dispatchMouseEvent",
                        {
                            "type": event,
                            "x": 12,
                            "y": 12,
                            "button": "left",
                            "buttons": 1 if event == "mousePressed" else 0,
                            "clickCount": 1,
                        },
                        session,
                    )
                clicked = await settle("click:" + label, count, scroll, scroll_events)
                assert clicked["state"]["clicks"][-1] == {
                    "id": snapshot["hit"],
                    "connected": True,
                    "identity": True,
                }
        final = record["snapshots"][-1]
        assert len(record["snapshots"]) == 65
        assert len(final["state"]["clicks"]) == 32
        assert final["state"]["wheels"] == [48, 48]
        assert final["state"]["io"] == [
            False,
            True,
            False,
            True,
            False,
            True,
            False,
            True,
            False,
        ]
        assert final["state"]["ro"] == [
            384,
            432,
            480,
            504,
            480,
            456,
            432,
            72,
            336,
            384,
            432,
            456,
            432,
            408,
            384,
            72,
            336,
        ]
        record["ok"] = True
    except Exception as error:  # noqa: BLE001 -- retain failed/partial evidence, then fail the run
        record["error"] = repr(error)
        if client is not None:
            record["messages"] = client.recorded_messages()
    finally:
        if client is not None:
            await client.close()
        if handle is not None:
            record["server"] = stop_target_serve(handle, include_resource_samples=True)
        record["finished_unix"] = time.time()
        save()
    assert record["ok"], record.get("error")
    assert (
        record["server"]["cleanup_complete"]
        and record["server"]["stop_protocol_verified"]
    )
    assert digest(binary) == record["binary_sha256"]
    print(
        json.dumps(
            {
                "engine": engine,
                "snapshots": len(record["snapshots"]),
                "ok": record["ok"],
            }
        ),
        flush=True,
    )


async def main():
    try:
        await one_run("Moli", args.moli.resolve())
        await one_run("Chromium", args.chromium.resolve())
        a, b = raw["samples"]
        raw["equal_snapshots"] = a["snapshots"] == b["snapshots"]
        raw["snapshot_difference_indices"] = [
            index
            for index, (x, y) in enumerate(zip(a["snapshots"], b["snapshots"]))
            if x != y
        ]
        assert raw["equal_snapshots"], raw["snapshot_difference_indices"]
        assert (
            hashlib.sha256(fixture_path.read_bytes()).hexdigest()
            == raw["fixture_sha256"]
        )
        raw["ok"] = True
    finally:
        raw["requests"] = requests
        raw["fixture_cleanup"] = stop_fixture_server(server, thread)
        save()
    print(
        json.dumps(
            {
                "engines": len(raw["samples"]),
                "snapshots_equal": raw["equal_snapshots"],
                "formal_cost_sample": False,
            }
        )
    )


asyncio.run(main())
