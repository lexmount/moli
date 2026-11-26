"""Record public-page responses, then compare engines with identical replay inputs."""

from __future__ import annotations

import argparse
import asyncio
import base64
import collections
import hashlib
import json
import os
import re
import signal
import socket
import subprocess
import time
import urllib.parse
import urllib.request
from pathlib import Path

import websockets

SITES = {
    "finance": "https://finance.baidu.com/index/ab-000001",
    "google": "https://www.google.com/?hl=en",
    "wikipedia": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
    "hh": "https://hh.ru/",
    "baidu": "https://www.baidu.com/",
    "bilibili": "https://www.bilibili.com/",
    "rust": "https://www.rust-lang.org/",
    "sina": "https://news.sina.com.cn/",
}


def request_key(request):
    """Use the exact URL and body; query rewriting can alias different resources."""
    return json.dumps(
        [
            request["method"],
            urllib.parse.urldefrag(request["url"])[0],
            hashlib.sha256(request.get("postData", "").encode()).hexdigest(),
        ],
        separators=(",", ":"),
    )


FREEZE_BOOTSTRAP = r"""(() => {
  const timeouts=new Set(), intervals=new Set(), frames=new Set();
  const st=window.setTimeout.bind(window), ct=window.clearTimeout.bind(window);
  const si=window.setInterval.bind(window), ci=window.clearInterval.bind(window);
  const ra=window.requestAnimationFrame.bind(window), ca=window.cancelAnimationFrame.bind(window);
  window.setTimeout=(callback, delay, ...args)=>{
    let id; id=st(typeof callback==='function'? (...inner)=>{timeouts.delete(id); return callback.apply(window,inner)}:callback,delay,...args);
    timeouts.add(id);return id;
  };
  window.setInterval=(...args)=>{const id=si(...args);intervals.add(id);return id};
  window.requestAnimationFrame=callback=>{let id;id=ra(t=>{frames.delete(id);callback.call(window,t)});frames.add(id);return id};
  window.__arenaFreeze=()=>{
    for(const id of timeouts)ct(id);for(const id of intervals)ci(id);for(const id of frames)ca(id);
    const style=document.createElement('style');style.textContent='*,*::before,*::after{animation:none!important;transition:none!important}';document.head.appendChild(style);
  };
})()"""

OBSERVE = r"""(() => {
  const nodes=Array.from(document.querySelectorAll('body *'));
  const boxes=nodes.filter(n=>n.getClientRects().length).slice(0,1000).map(n=>{
    const r=n.getBoundingClientRect();return [n.localName,n.id,r.x,r.y,r.width,r.height];
  });
  return {url:location.href,title:document.title,ready:document.readyState,nodes:nodes.length,
    text:document.body?.textContent?.length||0, boxes,
    navigation:performance.getEntriesByType('navigation').map(e=>e.toJSON())};
})()"""

PULSE = r"""(() => {
  const anchor=document.createElement('div');anchor.style.cssText='height:0;width:1px;display:block';document.body.prepend(anchor);
  const results=[];const geometry=[];
  for(let i=0;i<96;i++){
    const start=performance.now();
    if(i%2===0)anchor.append(document.createElement('div'));else anchor.firstChild.remove();
    const r=document.body.getBoundingClientRect();geometry.push([r.x,r.y,r.width,r.height]);results.push(performance.now()-start);
  }
  anchor.remove();return {samples:results,geometry,total:results.reduce((a,b)=>a+b,0)};
})()"""


def proc_resources(pid):
    base = Path("/proc") / str(pid)
    stat = (base / "stat").read_text().rsplit(")", 1)[1].split()
    status = (base / "status").read_text()
    mem = {
        k: int(v)
        for k, v in re.findall(
            r"^(Rss|Pss|Private_Dirty|Private_Clean):\s+(\d+)",
            (base / "smaps_rollup").read_text(),
            re.MULTILINE,
        )
    }
    return {
        "cpu_ticks": int(stat[11]) + int(stat[12]),
        "starttime": stat[19],
        "memory_kib": mem,
        "peak_rss_kib": int(re.search(r"^VmHWM:\s+(\d+)", status, re.MULTILINE)[1]),
        "monotonic": time.monotonic(),
        "unix_time": time.time(),
    }


class Client:
    def __init__(self, ws, folder):
        self.ws = ws
        self.folder = folder
        self.sequence = 0
        self.pending = {}
        self.sid = None
        self.events = []
        self.handler = None
        self.tasks = set()
        self.errors = []
        self.read_task = asyncio.create_task(self.read())

    async def read(self):
        async for raw in self.ws:
            message = json.loads(raw)
            if "id" in message:
                future = self.pending.get(message["id"])
                if future and not future.done():
                    future.set_result(message)
            else:
                message["_received_monotonic"] = time.monotonic()
                self.events.append(message)
                if message.get("method") == "Fetch.requestPaused" and self.handler:
                    task = asyncio.create_task(self.handler(message["params"]))
                    self.tasks.add(task)
                    task.add_done_callback(self.tasks.discard)
                    task.add_done_callback(self.task_finished)

    def task_finished(self, task):
        if not task.cancelled() and task.exception():
            self.errors.append(repr(task.exception()))

    async def command(self, method, params=None, root=False, timeout=60):
        self.sequence += 1
        ident = self.sequence
        message = {"id": ident, "method": method, "params": params or {}}
        if not root and self.sid:
            message["sessionId"] = self.sid
        future = asyncio.get_running_loop().create_future()
        self.pending[ident] = future
        try:
            await self.ws.send(json.dumps(message))
            response = await asyncio.wait_for(future, timeout)
            if "error" in response:
                raise RuntimeError(f"{method}: {response['error']}")
            return response["result"]
        finally:
            self.pending.pop(ident, None)

    async def evaluate(self, source):
        result = await self.command(
            "Runtime.evaluate",
            {"expression": source, "returnByValue": True, "awaitPromise": True},
            timeout=120,
        )
        if "exceptionDetails" in result:
            raise RuntimeError(result["exceptionDetails"])
        return result["result"].get("value")


async def one_run(args):
    folder = args.output.resolve()
    folder.mkdir(parents=True, exist_ok=False)
    archive = args.archive.resolve()
    if args.mode == "record":
        archive.mkdir(parents=True, exist_ok=False)
        (archive / "bodies").mkdir()
        entries = []
    else:
        entries = (
            json.loads((archive / "archive.json").read_text())["entries"]
            if args.mode in ["replay", "snapshot"]
            else []
        )
    recorded_observation = (
        json.loads((archive / "archive.json").read_text()).get("observation")
        if args.mode == "snapshot"
        else None
    )
    by_key = collections.defaultdict(list)
    for entry in entries:
        by_key[entry["key"]].append(entry)
    counters = collections.Counter()
    misses = []
    requests = []
    capture_warnings = []
    body_cache = {}
    snapshot = (
        json.loads((archive / "snapshot.json").read_text())
        if args.mode == "snapshot"
        else None
    )
    if snapshot and snapshot["openShadows"]:
        raise RuntimeError("Snapshot capture does not support shadow trees")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    command = [
        str(args.binary.resolve()),
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        str(port),
        "--realtime",
        "--http-no-proxy",
        "*",
        "--timeout",
        "150",
        "--log-level",
        "info",
    ]
    environment = dict(
        os.environ,
        TOKIO_WORKER_THREADS="2",
        MOLI_CPU_PROFILE="true",
        MOLI_LAYOUT="false",
        MOLI_REALTIME="false",
        MOLI_RESOURCE="false",
    )
    log = (folder / "engine.log").open("w")
    proc = subprocess.Popen(  # noqa: ASYNC220 - process stays live for asynchronous CDP driving
        command,
        cwd=folder,
        env=environment,
        stdout=log,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    if args.cpus:
        os.sched_setaffinity(proc.pid, {int(value) for value in args.cpus.split(",")})
    manifest = {
        "command": command,
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "site": args.site,
        "mode": args.mode,
        "url": SITES[args.site],
        "clock_ticks": os.sysconf("SC_CLK_TCK"),
        "cpus": args.cpus,
    }
    client = None
    try:
        version = None
        deadline = time.monotonic() + 30
        while version is None:
            try:
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                with opener.open(
                    f"http://127.0.0.1:{port}/json/version", timeout=0.5
                ) as response:
                    version = json.loads(response.read())
            except Exception:  # noqa: BLE001 - startup may fail before HTTP is available
                if proc.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("engine did not start")
                await asyncio.sleep(0.05)
        manifest["version"] = version
        ws = await websockets.connect(
            version["webSocketDebuggerUrl"], proxy=None, max_size=128 * 1024 * 1024
        )
        client = Client(ws, folder)
        target = await client.command(
            "Target.createTarget", {"url": "about:blank"}, root=True
        )
        attached = await client.command(
            "Target.attachToTarget",
            {"targetId": target["targetId"], "flatten": True},
            root=True,
        )
        client.sid = attached["sessionId"]
        for domain in ["Page", "Runtime", "Network"]:
            await client.command(domain + ".enable")
        await client.command("Network.setCacheDisabled", {"cacheDisabled": True})
        await client.command(
            "Emulation.setDeviceMetricsOverride",
            {"width": 1280, "height": 720, "deviceScaleFactor": 1, "mobile": False},
        )
        await client.command(
            "Page.addScriptToEvaluateOnNewDocument", {"source": FREEZE_BOOTSTRAP}
        )

        async def paused(params):
            ident = params["requestId"]
            request = params["request"]
            key = request_key(request)
            response_stage = (
                "responseStatusCode" in params or "responseErrorReason" in params
            )
            if not response_stage:
                requests.append(
                    {
                        "key": key,
                        "url": request["url"],
                        "type": params.get("resourceType"),
                    }
                )
            if not response_stage and urllib.parse.urlsplit(request["url"]).scheme in {
                "blob",
                "data",
                "about",
            }:
                await client.command("Fetch.continueRequest", {"requestId": ident})
                return
            if args.mode == "record":
                if not response_stage:
                    await client.command("Fetch.continueRequest", {"requestId": ident})
                    return
                entry = {
                    "key": key,
                    "url": request["url"],
                    "type": params.get("resourceType"),
                    "status": params.get("responseStatusCode"),
                    "headers": params.get("responseHeaders", []),
                    "error": params.get("responseErrorReason"),
                }
                try:
                    body = b""
                    if entry["status"] is not None and entry["status"] not in [
                        301,
                        302,
                        303,
                        304,
                        307,
                        308,
                        204,
                        205,
                    ]:
                        result = await client.command(
                            "Fetch.getResponseBody", {"requestId": ident}, timeout=30
                        )
                        body = (
                            base64.b64decode(result["body"])
                            if result["base64Encoded"]
                            else result["body"].encode()
                        )
                    digest = hashlib.sha256(body).hexdigest()
                    (archive / "bodies" / digest).write_bytes(body)
                    entry["body_sha256"] = digest
                    entries.append(entry)
                finally:
                    await client.command("Fetch.continueResponse", {"requestId": ident})
            elif args.mode in ["replay", "snapshot"]:
                options = by_key.get(key)
                if not options:
                    misses.append(
                        {
                            "key": key,
                            "url": request["url"],
                            "type": params.get("resourceType"),
                        }
                    )
                    await client.command(
                        "Fetch.failRequest",
                        {"requestId": ident, "errorReason": "Failed"},
                    )
                    return
                i = counters[key]
                counters[key] += 1
                entry = options[min(i, len(options) - 1)]
                if entry.get("error"):
                    await client.command(
                        "Fetch.failRequest",
                        {"requestId": ident, "errorReason": entry["error"]},
                    )
                    return
                digest = entry["body_sha256"]
                if digest not in body_cache:
                    body_cache[digest] = (archive / "bodies" / digest).read_bytes()
                body = body_cache[digest]
                headers = [
                    h
                    for h in entry["headers"]
                    if h["name"].lower()
                    not in ["content-encoding", "content-length", "transfer-encoding"]
                ]
                if (
                    snapshot
                    and request["url"].split("#")[0] == snapshot["url"].split("#")[0]
                    and params.get("resourceType") == "Document"
                ):
                    body = snapshot["html"].encode()
                    headers = [
                        h
                        for h in headers
                        if h["name"].lower()
                        not in [
                            "content-type",
                            "content-security-policy",
                            "content-security-policy-report-only",
                        ]
                    ]
                    headers.append(
                        {"name": "Content-Type", "value": "text/html; charset=utf-8"}
                    )
                elif snapshot and request["url"] in snapshot["css"]:
                    body = snapshot["css"][request["url"]].encode()
                    headers = [
                        h for h in headers if h["name"].lower() != "content-type"
                    ]
                    headers.append(
                        {"name": "Content-Type", "value": "text/css; charset=utf-8"}
                    )
                if (
                    snapshot
                    and params.get("resourceType") == "Document"
                    and request["url"].split("#")[0] != snapshot["url"].split("#")[0]
                    and body.strip()
                ):
                    await client.command(
                        "Fetch.failRequest",
                        {"requestId": ident, "errorReason": "Failed"},
                    )
                    raise RuntimeError(
                        "Snapshot replay does not capture nonempty child documents"
                    )
                await client.command(
                    "Fetch.fulfillRequest",
                    {
                        "requestId": ident,
                        "responseCode": entry["status"],
                        "responseHeaders": headers,
                        "body": base64.b64encode(body).decode(),
                    },
                )

        if args.mode != "live":
            client.handler = paused
            await client.command(
                "Fetch.enable",
                {
                    "patterns": [
                        {"urlPattern": "*", "requestStage": stage}
                        for stage in (
                            ["Request", "Response"]
                            if args.mode == "record"
                            else ["Request"]
                        )
                    ]
                },
            )
        before = proc_resources(proc.pid)
        started = time.monotonic()
        event_start = len(client.events)
        navigation = await client.command(
            "Page.navigate", {"url": snapshot["url"] if snapshot else SITES[args.site]}
        )
        await asyncio.sleep(args.seconds)
        if args.mode == "record":
            recorded = {entry["key"] for entry in entries}
            network_requests = {
                e["params"]["requestId"]: e["params"]["request"]
                for e in client.events
                if e.get("method") == "Network.requestWillBeSent"
            }
            responses = [
                e["params"]
                for e in client.events
                if e.get("method") == "Network.responseReceived"
            ]
            for received in responses:
                request = network_requests.get(received["requestId"])
                if not request or request_key(request) in recorded:
                    continue
                response = received["response"]
                try:
                    try:
                        result = await client.command(
                            "Network.getResponseBody",
                            {"requestId": received["requestId"]},
                        )
                        capture = "Network.getResponseBody"
                    except RuntimeError:
                        resource = await client.command(
                            "Page.getResourceContent",
                            {"frameId": navigation["frameId"], "url": request["url"]},
                        )
                        result = {
                            "body": resource["content"],
                            "base64Encoded": resource["base64Encoded"],
                        }
                        capture = "Page.getResourceContent"
                    body = (
                        base64.b64decode(result["body"])
                        if result["base64Encoded"]
                        else result["body"].encode()
                    )
                    digest = hashlib.sha256(body).hexdigest()
                    (archive / "bodies" / digest).write_bytes(body)
                    headers = [
                        {"name": k, "value": str(v)}
                        for k, v in response.get("headers", {}).items()
                    ]
                    entry = {
                        "key": request_key(request),
                        "url": request["url"],
                        "type": received.get("type"),
                        "status": int(response["status"]),
                        "headers": headers,
                        "error": None,
                        "body_sha256": digest,
                        "capture": capture,
                    }
                    entries.append(entry)
                    recorded.add(entry["key"])
                except Exception as error:  # noqa: BLE001 - preserve every capture failure in the result
                    capture_warnings.append(
                        f"Uncaptured response: {request['url']}: {error!r}"
                    )
        after_initialization = proc_resources(proc.pid)
        await client.evaluate("window.__arenaFreeze?.()")
        if snapshot:
            state = json.dumps(
                {
                    k: snapshot[k]
                    for k in ["canvases", "active", "scroll", "viewportScroll"]
                }
            )
            restore = """(async()=>{const s=STATE;const locate=p=>p?.reduce((n,i)=>n?.children[i],document.documentElement);
                const nodes=Array.from(document.querySelectorAll('canvas'));
                await Promise.all(s.canvases.map(async(data,i)=>{if(!data||!nodes[i])return;const image=new Image();
                    await new Promise((resolve,reject)=>{image.onload=resolve;image.onerror=reject;image.src=data});
                    nodes[i].getContext('2d').drawImage(image,0,0)}));
                const active=locate(s.active);if(active?.focus)active.focus({preventScroll:true});
                for(const value of s.scroll){const node=locate(value.path);if(node){node.scrollLeft=value.x;node.scrollTop=value.y}}
                scrollTo(...s.viewportScroll);return true})()""".replace("STATE", state)
            await client.evaluate(restore)
        if args.mode == "record":
            snap = await client.evaluate(
                (Path(__file__).resolve().parent / "snapshot_capture.js").read_text()
            )
            (archive / "snapshot.json").write_text(json.dumps(snap, ensure_ascii=False))
        await asyncio.sleep(0.1)
        observation = await client.evaluate(OBSERVE)
        before_pulse = proc_resources(proc.pid)
        pulse = await client.evaluate(PULSE)
        after_pulse = proc_resources(proc.pid)
        after_observation = await client.evaluate(OBSERVE)
        loads = [
            e["_received_monotonic"] - started
            for e in client.events[event_start:]
            if e.get("method") == "Page.loadEventFired"
        ]
        manifest.update(
            load_event_seconds=loads[0] if loads else None,
            after_observation=after_observation,
            navigation=navigation,
            elapsed=time.monotonic() - started,
            before=before,
            after_initialization=after_initialization,
            before_pulse=before_pulse,
            after_pulse=after_pulse,
            observation=observation,
            pulse=pulse,
            misses=misses,
            handler_errors=client.errors,
            requests=requests,
            replay_requests=sum(counters.values()),
            capture_warnings=capture_warnings,
            archive_sha256=hashlib.sha256(
                (archive / "archive.json").read_bytes()
            ).hexdigest()
            if (archive / "archive.json").exists()
            else None,
            snapshot_sha256=hashlib.sha256(
                (archive / "snapshot.json").read_bytes()
            ).hexdigest()
            if (archive / "snapshot.json").exists()
            else None,
        )
        (folder / "result.json").write_text(
            json.dumps(manifest, indent=2, ensure_ascii=False)
        )
        failures = []
        if args.mode in {"snapshot", "replay"}:
            if misses:
                failures.append(f"{len(misses)} missing resources")
            if client.errors:
                failures.append("CDP handler errors")
            if navigation.get("errorText"):
                failures.append("Navigation failed")
            if observation["ready"] != "complete" or observation["nodes"] < 20:
                failures.append("Page did not finish or is an error page")
            if recorded_observation:
                # CSSOM serialization changes hidden style text, not rendered geometry.
                for field in ["title", "nodes", "boxes"]:
                    if observation[field] != recorded_observation[field]:
                        failures.append(f"Snapshot changed original {field}")
            for field in ["title", "nodes", "text", "boxes"]:
                if observation[field] != after_observation[field]:
                    failures.append(f"Structural pulse changed final {field}")
        if failures:
            (folder / "validation-errors.json").write_text(
                json.dumps(failures, indent=2)
            )
            raise RuntimeError(failures)
        print(
            json.dumps(
                {
                    "site": args.site,
                    "mode": args.mode,
                    "title": observation["title"],
                    "nodes": observation["nodes"],
                    "misses": len(misses),
                    "errors": client.errors,
                    "pulse_ms": pulse["total"],
                },
                ensure_ascii=False,
            ),
            flush=True,
        )
    finally:
        if client:
            (folder / "events.jsonl").write_text(
                "".join(json.dumps(e) + "\n" for e in client.events)
            )
            for task in client.tasks:
                task.cancel()
            await asyncio.gather(*client.tasks, return_exceptions=True)
            await client.ws.close()
            client.read_task.cancel()
            await asyncio.gather(client.read_task, return_exceptions=True)
        if args.mode == "record":
            (archive / "archive.json").write_text(
                json.dumps(
                    {
                        "url": SITES[args.site],
                        "entries": entries,
                        "requests": requests,
                        "observation": manifest.get("observation"),
                        "handler_errors": client.errors if client else [],
                        "capture_warnings": capture_warnings,
                    },
                    indent=2,
                )
            )
        if proc.poll() is None:
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
        log.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--site", choices=SITES, required=True)
    parser.add_argument(
        "--mode", choices=["record", "replay", "snapshot", "live"], required=True
    )
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seconds", type=float, default=20)
    parser.add_argument("--cpus", default="")
    asyncio.run(one_run(parser.parse_args()))
