"""Reproducible, local-only differential CDP multi-page state probes.

Uses a headed Chromium in an owned Xvfb display so tab visibility is meaningful.
All browsers, profiles, HTTP fixtures, sockets, and logs belong to this run.
"""
from __future__ import annotations

import argparse
import asyncio
import contextlib
import copy
from collections import Counter
import html
import json
import os
import random
import signal
import socket
import subprocess
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

import websockets

from .raw_cdp import RawCdpCommandError, RoutedRawCdpClient, connect_routed_raw_cdp


ROOT = Path(__file__).resolve().parents[2]
STATE = """(() => ({url: location.href, title: document.title,
  marker: window.marker ?? null, visibility: document.visibilityState,
  hidden: document.hidden, focus: document.hasFocus(), history: history.length,
  name: window.name, storage: (()=>{try{return sessionStorage.getItem('owner')}
    catch(e){return {error:e.name}}})()}))()"""


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def http_get(url: str) -> Any:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(url, timeout=3) as response:
        data = response.read().decode()
    try:
        return json.loads(data)
    except ValueError:
        return data


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class Fixture(BaseHTTPRequestHandler):
    hits: Counter[str] = Counter()
    hits_lock = threading.Lock()

    def log_message(self, *_args: Any) -> None:
        pass

    def do_GET(self) -> None:
        with self.hits_lock:
            self.hits[self.path] += 1
        parsed = urllib.parse.urlsplit(self.path)
        query = urllib.parse.parse_qs(parsed.query)
        if parsed.path == "/redirect":
            self.send_response(302)
            self.send_header("Location", query.get("to", ["/redirected"])[0])
            self.end_headers()
            return
        if parsed.path == "/slow":
            time.sleep(float(query.get("delay", ["0.25"])[0]))
        marker = parsed.path
        body = ("<!doctype html><meta charset=utf-8><title>" + html.escape(marker)
                + "</title><body><h1>" + html.escape(marker) + "</h1><script>"
                + "window.marker=" + json.dumps(marker) + ";window.stateEvents=[];"
                + "for(const e of ['focus','blur','pageshow','pagehide','visibilitychange'])"
                + "addEventListener(e,()=>stateEvents.push({event:e,url:location.href,"
                + "visibility:document.visibilityState,focus:document.hasFocus()}),true);"
                + "</script>")
        if parsed.path == "/with-frame":
            body += '<iframe src="/child-frame"></iframe>'
        encoded = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(encoded)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        with contextlib.suppress(BrokenPipeError, ConnectionResetError):
            self.wfile.write(encoded)


class Lab:
    def __init__(self, output: Path, moli: Path, chromium: str):
        self.output, self.moli, self.chromium = output, moli, chromium
        self.processes: list[subprocess.Popen] = []
        self.logs: list[Any] = []
        self.fixture = ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
        self.fixture.daemon_threads = True
        self.base = f"http://127.0.0.1:{self.fixture.server_port}"
        self.endpoints: dict[str, str] = {}

    def launch(self, args: list[str], name: str, **kwargs: Any) -> subprocess.Popen:
        log = (self.output / f"{name}.log").open("wb")
        self.logs.append(log)
        env = {k: v for k, v in os.environ.items() if k.lower() not in {
            "http_proxy", "https_proxy", "all_proxy", "no_proxy"}}
        env.update(kwargs.pop("env", {}))
        process = subprocess.Popen(args, stdout=log, stderr=log, env=env,
                                   start_new_session=True, **kwargs)
        self.processes.append(process)
        return process

    async def start(self) -> None:
        self.output.mkdir(parents=True, exist_ok=True)
        threading.Thread(target=self.fixture.serve_forever, daemon=True).start()
        read_fd, write_fd = os.pipe()
        try:
            self.launch(["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1280x900x24",
                         "-nolisten", "tcp"], "xvfb", pass_fds=(write_fd,))
            os.close(write_fd)
            write_fd = -1
            display = await asyncio.wait_for(asyncio.to_thread(os.read, read_fd, 32), 10)
        finally:
            os.close(read_fd)
            if write_fd != -1:
                os.close(write_fd)
        ports = {engine: free_port() for engine in ("chromium", "moli")}
        self.launch([self.chromium, "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage",
                     "--no-first-run", "--no-default-browser-check", "--disable-background-networking",
                     "--remote-allow-origins=devtools://devtools",
                     "--disable-component-update", "--disable-popup-blocking", "--no-proxy-server",
                     f"--user-data-dir={self.output / 'chromium-profile'}",
                     f"--remote-debugging-port={ports['chromium']}", "about:blank"],
                    "chromium", env={"DISPLAY": ":" + display.decode().strip()})
        self.launch([str(self.moli), "serve", "--host", "127.0.0.1", "--port", str(ports["moli"]),
                     "--layout", "--http-no-proxy", "*"], "moli")
        for engine, port in ports.items():
            endpoint = f"http://127.0.0.1:{port}"
            for _ in range(100):
                try:
                    version = await asyncio.to_thread(http_get, endpoint + "/json/version")
                    await asyncio.to_thread(http_get, endpoint + "/json/list")
                    write_json(self.output / f"{engine}-version.json", version)
                    self.endpoints[engine] = endpoint
                    break
                except (OSError, ValueError):
                    await asyncio.sleep(0.1)
            else:
                raise RuntimeError(f"{engine} did not start")

    def close(self) -> None:
        for process in reversed(self.processes):
            if process.poll() is None:
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    with contextlib.suppress(ProcessLookupError):
                        os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=3)
        self.fixture.shutdown()
        self.fixture.server_close()
        for log in self.logs:
            log.close()


class Case:
    def __init__(self, endpoint: str, base: str, output: Path):
        self.endpoint, self.base, self.output = endpoint, base, output
        self.clients: dict[str, RoutedRawCdpClient] = {}
        self.targets: dict[str, str] = {}
        self.sessions: dict[str, str] = {}
        self.closed: set[str] = set()
        self.commands: list[dict[str, Any]] = []
        self.observations: list[dict[str, Any]] = []
        self.extra_sessions: dict[str, dict[str, str]] = {}
        self.tab_targets: dict[str, str] = {}
        self.archive: dict[str, list[dict]] = {}
        self.contexts: dict[str, str] = {}
        self.screencasts: set[str] = set()
        self.screencast_acked: set[tuple[str, int]] = set()
        self.tab_sessions: dict[str, str] = {}

    async def connect(self, name: str = "main", ws: str | None = None) -> None:
        if ws:
            client = RoutedRawCdpClient(await websockets.connect(ws, proxy=None, max_size=None))
            client.start()
        else:
            client = await connect_routed_raw_cdp(self.endpoint)
        self.clients[name] = client

    async def command(self, method: str, params: dict | None = None,
                      sid: str | None = None, client: str = "main") -> dict:
        record: dict[str, Any] = {"client": client, "method": method, "params": params, "sessionId": sid}
        self.commands.append(record)
        try:
            result = await self.clients[client].command(method, params, session_id=sid, timeout=5)
            record["response"] = result.response
            return copy.deepcopy(result.response.get("result", {}))
        except Exception as error:
            record["error"] = getattr(error, "error", {"message": str(error), "type": type(error).__name__})
            return {"error": record["error"]}

    async def evaluate(self, expression: str, label: str, client: str = "main",
                       sid: str | None = None, *, user_gesture: bool = False) -> Any:
        params = {"expression": expression, "returnByValue": True}
        if user_gesture:
            params["userGesture"] = True
        result = await self.command("Runtime.evaluate", params,
                                    self.sessions.get(label) if sid is None else sid, client)
        if "exceptionDetails" in result:
            return {"exception": result["exceptionDetails"].get("text")}
        return result.get("result", {}).get("value", result)

    async def settle(self, label: str, url: str | None = None) -> None:
        for _ in range(60):
            state = await self.evaluate("({url:location.href,ready:document.readyState})", label)
            if isinstance(state, dict) and state.get("ready") == "complete" and (
                    url is None or state.get("url") == url):
                return
            await asyncio.sleep(0.05)

    async def snapshot(self) -> dict:
        await asyncio.sleep(0.12)
        result: dict[str, Any] = {"pages": {}, "targets": {}, "http": {}}
        for label in self.targets:
            if label not in self.closed:
                state = await self.evaluate(STATE, label)
                tree = await self.command("Page.getFrameTree", sid=self.sessions[label])
                frame = tree.get("frameTree", {}).get("frame", {})
                state["frameUrl"] = frame.get("url", "") + frame.get("urlFragment", "")
                state["childFrames"] = len(tree.get("frameTree", {}).get("childFrames", []))
                result["pages"][label] = state
        targets = await self.command("Target.getTargets")
        listing = await asyncio.to_thread(http_get, self.endpoint + "/json/list")
        for label, target_id in self.targets.items():
            for info in targets.get("targetInfos", []):
                if info["targetId"] == target_id:
                    result["targets"][label] = {k: info.get(k) for k in ("type", "url", "title", "attached")}
            for info in listing:
                if info["id"] == target_id:
                    result["http"][label] = {k: info.get(k) for k in ("type", "url", "title")}
        if self.screencasts:
            for record in self.clients["main"].recorded_messages():
                event = record["payload"]
                if event.get("method") == "Page.screencastFrame":
                    key = (event.get("sessionId", ""), event["params"]["sessionId"])
                    if key not in self.screencast_acked:
                        await self.command("Page.screencastFrameAck", {"sessionId": key[1]}, key[0])
                        self.screencast_acked.add(key)
            result["screencastVisible"] = {}
            for page in self.screencasts - self.closed:
                events = [r["payload"] for r in self.clients["main"].recorded_messages()
                          if r["payload"].get("method") == "Page.screencastVisibilityChanged"
                          and r["payload"].get("sessionId") == self.sessions[page]]
                result["screencastVisible"][page] = events[-1]["params"]["visible"] if events else None
        return result

    async def disconnect(self, client: str) -> None:
        instance = self.clients.pop(client)
        await instance.close()
        self.archive[f"{client}-{len(self.archive)}"] = instance.recorded_messages()

    async def attach_extra(self, client: str, label: str) -> str:
        attached = await self.command("Target.attachToTarget", {"targetId": self.targets[label], "flatten": True}, client=client)
        sid = attached["sessionId"]
        self.extra_sessions.setdefault(client, {})[label] = sid
        await self.command("Runtime.enable", sid=sid, client=client)
        await self.command("Page.enable", sid=sid, client=client)
        return sid

    async def action(self, action: dict) -> Any:
        op, label = action["op"], action.get("page", "A")
        sid = self.sessions.get(label)
        if op == "evaluate":
            return await self.evaluate(action["expression"], label,
                                       user_gesture=action.get("userGesture", False))
        elif op == "create":
            url = self.base + action.get("path", "/" + label)
            params = {"url": url, "background": action.get("background", False)}
            for key in ("newWindow", "hidden"):
                if key in action:
                    params[key] = action[key]
            if "context" in action:
                params["browserContextId"] = self.contexts[action["context"]]
            result = await self.command("Target.createTarget", params)
            if "targetId" not in result:
                return result
            self.targets[label] = result["targetId"]
            attach = await self.command("Target.attachToTarget", {"targetId": result["targetId"], "flatten": True})
            self.sessions[label] = attach["sessionId"]
            await self.command("Runtime.enable", sid=self.sessions[label])
            await self.command("Page.enable", sid=self.sessions[label])
            await self.settle(label, url)
        elif op == "activate":
            return await self.command("Target.activateTarget", {"targetId": self.targets[label]})
        elif op == "bring":
            return await self.command("Page.bringToFront", sid=sid)
        elif op == "http_activate":
            return await asyncio.to_thread(http_get, self.endpoint + "/json/activate/" + self.targets[label])
        elif op == "navigate":
            base = self.base.replace("127.0.0.1", "localhost") if action.get("cross") else self.base
            url = base + action["path"]
            result = await self.command("Page.navigate", {"url": url}, sid)
            expected = base + action["expected"] if "expected" in action else url
            await self.settle(label, expected)
            return {k: v for k, v in result.items() if k not in {"frameId", "loaderId", "isDownload"}}
        elif op == "reload":
            result = await self.command("Page.reload", {}, sid)
            await self.settle(label)
            return result
        elif op == "hash":
            await self.evaluate("location.hash=" + json.dumps(action["value"]), label)
            await self.settle(label)
        elif op == "title":
            return await self.evaluate("document.title=" + json.dumps(action["value"]), label)
        elif op == "storage":
            return await self.evaluate("sessionStorage.setItem('owner'," + json.dumps(label) + ");window.name=" + json.dumps(label), label)
        elif op in {"back", "forward"}:
            history = await self.command("Page.getNavigationHistory", sid=sid)
            index = history.get("currentIndex", 0)
            next_index = index + (-1 if op == "back" else 1)
            if 0 <= next_index < len(history.get("entries", [])):
                entry = history["entries"][next_index]
                result = await self.command("Page.navigateToHistoryEntry", {"entryId": entry["id"]}, sid)
                await self.settle(label, entry["url"])
                return result
        elif op == "close":
            result = await self.command("Target.closeTarget", {"targetId": self.targets[label]})
            if result.get("success"):
                self.closed.add(label)
            return result
        elif op == "detach_attach":
            await self.command("Target.detachFromTarget", {"sessionId": sid})
            attach = await self.command("Target.attachToTarget", {"targetId": self.targets[label], "flatten": True})
            self.sessions[label] = attach["sessionId"]
            await self.command("Runtime.enable", sid=self.sessions[label])
            await self.command("Page.enable", sid=self.sessions[label])
        elif op == "connect_browser":
            client = action["client"]
            await self.connect(client)
            await self.command("Target.setDiscoverTargets", {"discover": True}, client=client)
            await self.attach_extra(client, label)
        elif op == "connect_page":
            client = action["client"]
            listing = await asyncio.to_thread(http_get, self.endpoint + "/json/list")
            info = next(t for t in listing if t["id"] == self.targets[label])
            await self.connect(client, info["webSocketDebuggerUrl"])
            await self.command("Runtime.enable", client=client)
            await self.command("Page.enable", client=client)
        elif op == "side_state":
            client = action["client"]
            extra_sid = self.extra_sessions.get(client, {}).get(label)
            result = await self.command("Runtime.evaluate", {"expression": STATE, "returnByValue": True}, extra_sid, client)
            return result.get("result", {}).get("value", result)
        elif op == "side_navigate":
            client = action["client"]
            extra_sid = self.extra_sessions.get(client, {}).get(label)
            result = await self.command("Page.navigate", {"url": self.base + action["path"]}, extra_sid, client)
            await self.settle(label, self.base + action["path"])
            return {k: v for k, v in result.items() if k not in {"frameId", "loaderId", "isDownload"}}
        elif op == "side_bring":
            client = action["client"]
            return await self.command("Page.bringToFront", sid=self.extra_sessions.get(client, {}).get(label), client=client)
        elif op == "disconnect":
            await self.disconnect(action["client"])
        elif op == "reconnect":
            await self.disconnect("main")
            await self.connect()
            await self.command("Target.setDiscoverTargets", {"discover": True})
            for page in self.targets:
                if page not in self.closed:
                    self.sessions[page] = await self.attach_extra("main", page)
        elif op == "wrong_session":
            result = await self.command("Runtime.evaluate", {"expression": "location.href"}, sid, action["client"])
            return {"rejected": "error" in result}
        elif op == "attach_tab":
            targets = await self.command("Target.getTargets", {"filter": [{"type": "tab"}]})
            url = await self.evaluate("location.href", label)
            matching = [t for t in targets.get("targetInfos", []) if t["url"] == url]
            if not matching:
                return {"missingTab": True}
            self.tab_targets[label] = matching[0]["targetId"]
            attached = await self.command("Target.attachToTarget", {"targetId": matching[0]["targetId"], "flatten": True})
            tab_sid = attached["sessionId"]
            before = self.clients["main"].current_sequence
            result = await self.command("Target.setAutoAttach", {"autoAttach": True, "waitForDebuggerOnStart": False, "flatten": True}, tab_sid)
            await asyncio.sleep(0.1)
            children = [r.payload["params"] for r in self.clients["main"].messages_since(before)
                        if r.payload.get("method") == "Target.attachedToTarget" and r.payload.get("sessionId") == tab_sid]
            if children:
                self.extra_sessions.setdefault("tab", {})[label] = children[0]["sessionId"]
            return {"result": result, "childTypes": [c["targetInfo"]["type"] for c in children],
                    "correctChild": bool(children) and children[0]["targetInfo"]["targetId"] == self.targets[label]}
        elif op == "tab_activate":
            return await self.command("Target.activateTarget", {"targetId": self.tab_targets[label]})
        elif op == "tab_navigate":
            result = await self.command("Page.navigate", {"url": self.base + action["path"]}, self.extra_sessions["tab"][label])
            await self.settle(label, self.base + action["path"])
            return {k: v for k, v in result.items() if k not in {"frameId", "loaderId", "isDownload"}}
        elif op == "popup":
            path = action.get("path", "")
            url = action.get("url", self.base + path)
            hits_before = Fixture.hits[path]
            before = {t["targetId"] for t in (await self.command("Target.getTargets")).get("targetInfos", [])}
            arguments = [url, action.get("name", ""), action.get("features", "")]
            expression = "window.popupRef=window.open(" + ",".join(map(json.dumps, arguments)) + ");void 0"
            result = await self.command("Runtime.evaluate", {"expression": expression, "userGesture": action.get("userGesture", True)}, sid)
            await asyncio.sleep(0.2)
            targets = (await self.command("Target.getTargets")).get("targetInfos", [])
            new = [t for t in targets if t["targetId"] not in before and t["type"] == "page"]
            if new:
                child = action["child"]
                self.targets[child] = new[0]["targetId"]
                self.sessions[child] = await self.attach_extra("main", child)
                await self.settle(child, url or "about:blank")
            return {"newPages": len(new), "openerMatches": bool(new) and new[0].get("openerId") == self.targets[label],
                    "error": result.get("error"), "documentRequests": Fixture.hits[path] - hits_before}
        elif op == "target_opener":
            targets = (await self.command("Target.getTargets")).get("targetInfos", [])
            info = next(target for target in targets if target["targetId"] == self.targets[label])
            labels = {target_id: page for page, target_id in self.targets.items()}
            return labels.get(info.get("openerId"))
        elif op == "history_push":
            return await self.evaluate("history.pushState({},''," + json.dumps(action["path"]) + ");void 0", label)
        elif op == "concurrent_nav_activate":
            other = action["other"]
            results = await asyncio.gather(
                self.command("Page.navigate", {"url": self.base + "/slow?delay=0.3"}, sid),
                self.command("Target.activateTarget", {"targetId": self.targets[other]}))
            await self.settle(label, self.base + "/slow?delay=0.3")
            return [{k: v for k, v in r.items() if k not in {"frameId", "loaderId", "isDownload"}} for r in results]
        elif op == "clear_events":
            for page in self.targets:
                if page not in self.closed:
                    await self.evaluate("stateEvents=[];void 0", page)
        elif op == "busy_unrelated_context":
            witness = "/busy-context-witness"
            hits_before = Fixture.hits[witness]
            script = "(() => { const xhr = new XMLHttpRequest(); xhr.open('GET', " + json.dumps(witness) + ", false); xhr.send(); const until = Date.now() + 10000; while (Date.now() < until) {} return 'expired'; })()"
            busy = asyncio.create_task(self.evaluate(script, label))
            try:
                for _ in range(100):
                    if Fixture.hits[witness] > hits_before or busy.done():
                        break
                    await asyncio.sleep(0.02)
                entered = Fixture.hits[witness] > hits_before
                bring = await self.command("Page.bringToFront", sid=self.sessions[action["active"]])
                activate = await self.command("Target.activateTarget", {"targetId": self.targets[action["next"]]})
                created = await self.command("Target.createTarget", {"url": "about:blank"})
                closed = await self.command("Target.closeTarget", {"targetId": created["targetId"]}) if "targetId" in created else {"error": "creation failed"}
                return {"entered": entered, "completedWhileBusy": not busy.done(),
                        "commandsSucceeded": all("error" not in r for r in (bring, activate, created, closed))}
            finally:
                await self.command("Runtime.terminateExecution", sid=sid)
                await busy
        elif op == "event_log":
            return {page: await self.evaluate("stateEvents.map(x=>({event:x.event,visibility:x.visibility,focus:x.focus}))", page)
                    for page in self.targets if page not in self.closed}
        elif op == "focus_emulation":
            client = action.get("client", "main")
            extra_sid = self.sessions[label] if client == "main" else self.extra_sessions[client][label]
            return await self.command("Emulation.setFocusEmulationEnabled", {"enabled": action["enabled"]}, extra_sid, client)
        elif op == "new_context":
            result = await self.command("Target.createBrowserContext")
            self.contexts[action["context"]] = result["browserContextId"]
        elif op == "dispose_context":
            context = self.contexts[action["context"]]
            infos = (await self.command("Target.getTargets")).get("targetInfos", [])
            disposed = {t["targetId"] for t in infos if t.get("browserContextId") == context}
            result = await self.command("Target.disposeBrowserContext", {"browserContextId": context})
            self.closed.update(p for p, t in self.targets.items() if t in disposed)
            return result
        elif op == "auto_enable":
            client = action.get("client", "main")
            if client not in self.clients:
                await self.connect(client)
            params = {"autoAttach": True, "waitForDebuggerOnStart": action.get("wait", False), "flatten": True}
            if action.get("tabs"):
                params["filter"] = [{"type": "tab"}]
            return await self.command("Target.setAutoAttach", params, client=client)
        elif op == "auto_create_tab":
            pos = self.clients["main"].current_sequence
            created = await self.command("Target.createTarget", {"url": self.base + "/" + label})
            self.targets[label] = created["targetId"]
            attached = [r.payload["params"] for r in self.clients["main"].messages_since(pos)
                        if r.payload.get("method") == "Target.attachedToTarget"]
            tab = next(a for a in attached if a["targetInfo"]["type"] == "tab")
            self.tab_sessions[label] = tab["sessionId"]
            pos = self.clients["main"].current_sequence
            await self.command("Target.setAutoAttach", {"autoAttach": True, "waitForDebuggerOnStart": True, "flatten": True}, tab["sessionId"])
            children = [r.payload["params"] for r in self.clients["main"].messages_since(pos)
                        if r.payload.get("method") == "Target.attachedToTarget" and r.payload.get("sessionId") == tab["sessionId"]]
            child = children[0]
            self.sessions[label] = child["sessionId"]
            await self.command("Runtime.enable", sid=child["sessionId"])
            await self.command("Page.enable", sid=child["sessionId"])
            return {"tabWaiting": tab["waitingForDebugger"], "childWaiting": child["waitingForDebugger"], "childMatches": child["targetInfo"]["targetId"] == created["targetId"]}
        elif op == "resume_tab":
            result = await self.command("Runtime.runIfWaitingForDebugger", sid=self.tab_sessions[label])
            await self.settle(label, self.base + "/" + label)
            return result
        elif op == "auto_create":
            result = await self.command("Target.createTarget", {"url": self.base + "/" + label})
            self.targets[label] = result["targetId"]
            await asyncio.sleep(0.1)
            attachments = {}
            for client, instance in self.clients.items():
                records = [r["payload"] for r in instance.recorded_messages()]
                events = [r["params"] for r in records if r.get("method") == "Target.attachedToTarget"
                          and r["params"]["targetInfo"]["targetId"] == result["targetId"]]
                if events:
                    event = events[-1]
                    self.extra_sessions.setdefault(client, {})[label] = event["sessionId"]
                    if client == "main":
                        self.sessions[label] = event["sessionId"]
                    await self.command("Runtime.enable", sid=event["sessionId"], client=client)
                    await self.command("Page.enable", sid=event["sessionId"], client=client)
                    attachments[client] = {"count": len(events), "waiting": event["waitingForDebugger"]}
            return attachments
        elif op == "resume":
            client = action.get("client", "main")
            result = await self.command("Runtime.runIfWaitingForDebugger", sid=self.extra_sessions[client][label], client=client)
            if action.get("settle", True):
                await self.settle(label, self.base + "/" + label)
            return result
        elif op == "console_route":
            positions = {k: c.current_sequence for k, c in self.clients.items()}
            await self.evaluate("console.log('route-marker-' + window.marker);void 0", label)
            await asyncio.sleep(0.1)
            result = {}
            for client, instance in self.clients.items():
                owned = {s: p for p, s in self.sessions.items()} if client == "main" else {}
                owned.update({s: p for p, s in self.extra_sessions.get(client, {}).items()})
                events = [r.payload for r in instance.messages_since(positions[client])
                          if r.payload.get("method") == "Runtime.consoleAPICalled"]
                result[client] = [{"page": owned.get(e.get("sessionId"), "direct"),
                                   "args": [a.get("value") for a in e["params"]["args"]]} for e in events]
            return result
        elif op == "subscription_isolation":
            positions = {k: c.current_sequence for k, c in self.clients.items()}
            # Disable this observer only; the primary session must still see events.
            other_sid = self.extra_sessions["other"][label]
            await self.command("Runtime.disable", sid=other_sid, client="other")
            await self.evaluate("console.log('only-main');void 0", label)
            await asyncio.sleep(0.1)
            return {name: sum(r.payload.get("method") == "Runtime.consoleAPICalled"
                              for r in client.messages_since(positions[name])) for name, client in self.clients.items()}
        elif op == "stale_handles":
            obj = await self.command("Runtime.evaluate", {"expression": "({owner:window.marker})"}, sid)
            object_id = obj.get("result", {}).get("objectId")
            root = (await self.command("DOM.getDocument", sid=sid)).get("root", {})
            await self.command("Target.activateTarget", {"targetId": self.targets[action["other"]]})
            read = await self.command("Runtime.callFunctionOn", {"objectId": object_id, "functionDeclaration": "function(){return this.owner}", "returnByValue": True}, sid)
            dom = await self.command("DOM.describeNode", {"nodeId": root.get("nodeId")}, sid)
            await self.command("Page.navigate", {"url": self.base + "/handle-replaced"}, sid)
            await self.settle(label, self.base + "/handle-replaced")
            stale = await self.command("Runtime.callFunctionOn", {"objectId": object_id, "functionDeclaration": "function(){return this.owner}", "returnByValue": True}, sid)
            stale_dom = await self.command("DOM.describeNode", {"nodeId": root.get("nodeId")}, sid)
            return {"ownerBeforeNav": read.get("result", {}).get("value"), "documentBeforeNav": dom.get("node", {}).get("documentURL"),
                    "staleObjectRejected": "error" in stale, "staleNodeRejected": "error" in stale_dom}
        elif op == "window_map":
            windows = {page: await self.command("Browser.getWindowForTarget", {"targetId": target})
                       for page, target in self.targets.items() if page not in self.closed}
            return {a + "=" + b: wa.get("windowId") == wb.get("windowId")
                    for a, wa in windows.items() for b, wb in windows.items() if a < b}
        elif op == "screencast":
            self.screencasts.add(label)
            return await self.command("Page.startScreencast", {"format": "jpeg", "quality": 25, "maxWidth": 160, "maxHeight": 120}, sid)
        elif op == "location_navigate":
            # JS-initiated navigation takes a different scheduler path from Page.navigate.
            base = self.base.replace("127.0.0.1", "localhost") if action.get("cross") else self.base
            await self.evaluate("location.href=" + json.dumps(base + action["path"]), label)
            await self.settle(label, base + action["path"])
        elif op == "supersede":
            slow = asyncio.create_task(self.command("Page.navigate", {"url": self.base + "/slow?delay=0.8"}, sid))
            await asyncio.sleep(0.08)
            second = await self.command("Page.navigate", {"url": self.base + "/superseded-final"}, sid)
            first = await slow
            await self.settle(label, self.base + "/superseded-final")
            await asyncio.sleep(0.85)
            return {"firstAborted": bool(first.get("errorText") or first.get("error")), "secondFailed": bool(second.get("errorText") or second.get("error"))}
        elif op == "close_loading":
            slow = asyncio.create_task(self.command("Page.navigate", {"url": self.base + "/slow?delay=0.8"}, sid))
            await asyncio.sleep(0.08)
            closed = await self.command("Target.closeTarget", {"targetId": self.targets[label]})
            self.closed.add(label)
            navigation = await slow
            await asyncio.sleep(0.85)
            stale = await self.command("Runtime.evaluate", {"expression": "location.href"}, sid)
            return {"closed": closed.get("success"), "staleSessionRejected": "error" in stale,
                    "navigationTimedOut": navigation.get("error", {}).get("type") == "RawCdpTimeoutError"}
        elif op == "slow_other_target":
            slow = asyncio.create_task(self.command("Page.navigate", {"url": self.base + "/slow?delay=1.5"}, sid))
            await asyncio.sleep(0.08)
            started = time.monotonic()
            fast = await self.evaluate("window.marker", action["other"])
            elapsed = time.monotonic() - started
            # Keep measured timing in the trace, compare only a coarse blocking threshold.
            self.commands.append({"probe": "slow_other_target", "elapsed": elapsed})
            await slow
            await self.settle(label, self.base + "/slow?delay=1.5")
            return {"otherMarker": fast, "otherBlockedOverOneSecond": elapsed > 1}
        elif op == "popup_reuse":
            before = {t["targetId"] for t in (await self.command("Target.getTargets")).get("targetInfos", [])}
            base = self.base.replace("127.0.0.1", "localhost") if action.get("cross") else self.base
            url = action.get("url", base + action.get("path", ""))
            open_expression = "window.open(" + json.dumps(url) + "," + json.dumps(action["name"]) + ")"
            if "renameAfter" in action:
                expression = "(() => { const reused = " + open_expression + "; reused.name = " + json.dumps(action["renameAfter"]) + "; return reused === popupRef; })()"
            else:
                expression = open_expression + ";void 0"
            evaluated = await self.command("Runtime.evaluate", {"expression": expression, "userGesture": True}, sid)
            await asyncio.sleep(0.2)
            after = (await self.command("Target.getTargets")).get("targetInfos", [])
            new = [t for t in after if t["targetId"] not in before and t["type"] == "page"]
            for index, target in enumerate(new):
                child = f"unexpected{index}"
                self.targets[child] = target["targetId"]
                self.sessions[child] = await self.attach_extra("main", child)
            await self.settle(action["child"])
            result = {"newPages": len(new)}
            if "renameAfter" in action:
                result["identityRetained"] = evaluated.get("result", {}).get("value")
            return result
        elif op == "name":
            return await self.evaluate("window.name=" + json.dumps(action["value"]), label)
        elif op == "auto_disable":
            return await self.command("Target.setAutoAttach", {"autoAttach": False, "waitForDebuggerOnStart": False, "flatten": True})
        elif op == "discovery_refresh":
            await self.command("Target.setDiscoverTargets", {"discover": False})
            pos = self.clients["main"].current_sequence
            await self.command("Target.setDiscoverTargets", {"discover": True})
            events = [r.payload for r in self.clients["main"].messages_since(pos)
                      if r.payload.get("method") == "Target.targetCreated"]
            found = {e["params"]["targetInfo"]["targetId"]: e["params"]["targetInfo"] for e in events}
            return {p: {k: found[t].get(k) for k in ("url", "title", "attached")} if t in found else None
                    for p, t in self.targets.items() if p not in self.closed}
        elif op == "discover_samedoc_event":
            pos = self.clients["main"].current_sequence
            await self.evaluate("history.pushState({},'', '/same-document-new');void 0", label)
            await asyncio.sleep(0.2)
            events = [r.payload for r in self.clients["main"].messages_since(pos)
                      if r.payload.get("method") == "Target.targetInfoChanged"
                      and r.payload["params"]["targetInfo"]["targetId"] == self.targets[label]]
            return {"urlChangedEvent": any(e["params"]["targetInfo"]["url"].endswith("/same-document-new") for e in events)}
        elif op == "adopt_initial":
            # Discovery may lazily publish the standalone server's initial target.
            await asyncio.to_thread(http_get, self.endpoint + "/json/list")
            targets = (await self.command("Target.getTargets")).get("targetInfos", [])
            target = next((t for t in targets if t["type"] == "page" and t["url"] == "about:blank"), None)
            if target is None:
                raise RuntimeError("closed_default must run in a fresh browser; no initial about:blank target")
            self.targets[label] = target["targetId"]
            self.sessions[label] = await self.attach_extra("main", label)
        elif op == "closed_http_activate":
            try:
                await asyncio.to_thread(http_get, self.endpoint + "/json/activate/" + self.targets[label])
                return {"status": 200}
            except urllib.error.HTTPError as error:
                return {"status": error.code}
        elif op == "history_dump":
            result = await self.command("Page.getNavigationHistory", sid=sid)
            return {"index": result.get("currentIndex"), "entries": [{k: e.get(k) for k in ("url", "title", "transitionType")}
                    for e in result.get("entries", [])]}
        elif op == "popup_reference":
            return await self.evaluate("({closed:popupRef.closed,url:popupRef.location.href,name:popupRef.name,openerIsSelf:popupRef.opener===window,text:popupRef.document.querySelector('h1')?.textContent})", label)
        elif op == "opener_reference":
            return await self.evaluate("({hasOpener:!!opener,closed:opener?.closed,url:opener?.location.href,name:opener?.name,marker:opener?.marker})", label)
        elif op == "popup_ref_navigate":
            method = action.get("method", "href")
            if method not in {"href", "assign", "replace"}:
                raise ValueError(method)
            url = self.base + action["path"]
            request_path = action["path"].split("#", 1)[0]
            hits_before = Fixture.hits[request_path]
            expression = ("popupRef.location.href=" + json.dumps(url) if method == "href"
                          else f"popupRef.location.{method}(" + json.dumps(url) + ")")
            await self.evaluate(expression + ";void 0", label)
            await self.settle(action["child"], url)
            if action.get("countRequests"):
                return {"documentRequests": Fixture.hits[request_path] - hits_before}
        elif op == "popup_supersede":
            child = action["child"]
            method = action["method"]
            if method not in {"named", "replace"}:
                raise ValueError(method)
            child_sid = self.sessions[child]
            await self.command("Network.enable", sid=child_sid)
            slow_url = self.base + "/slow?delay=3&popup=" + method
            await self.evaluate("popupRef.location.href=" + json.dumps(slow_url) + ";void 0", label)
            for _ in range(100):
                started = any(r["payload"].get("method") == "Network.requestWillBeSent"
                              and r["payload"].get("sessionId") == child_sid
                              and r["payload"]["params"]["request"]["url"] == slow_url
                              for r in self.clients["main"].recorded_messages())
                if started:
                    break
                await asyncio.sleep(0.02)
            if not started:
                raise RuntimeError("popup's slow request did not start")
            read = asyncio.create_task(self.evaluate("document.URL", child))
            await asyncio.sleep(0.02)
            destination = self.base + "/popup-replaced-" + method
            expression = ("window.open(" + json.dumps(destination) + "," + json.dumps(action["name"]) + ")"
                          if method == "named" else "popupRef.location.replace(" + json.dumps(destination) + ")")
            began = time.monotonic()
            await self.evaluate(expression + ";void 0", label)
            await self.settle(child, destination)
            pending_read = await read
            elapsed = time.monotonic() - began
            self.commands.append({"probe": "popup_supersede", "method": method, "elapsed": elapsed})
            return {"replacementBlocked": elapsed > 1.5,
                    "pendingReadFailed": isinstance(pending_read, dict) and "error" in pending_read}
        elif op == "popup_ref_reload":
            child = action["child"]
            url = await self.evaluate("location.href", child)
            request_path = urllib.parse.urldefrag(url)[0].removeprefix(self.base)
            hits_before = Fixture.hits[request_path]
            await self.evaluate("window.marker='before-reference-reload';void 0", child)
            await self.evaluate("popupRef.location.reload();void 0", label)
            for _ in range(60):
                if await self.evaluate("window.marker", child) != "before-reference-reload":
                    break
                await asyncio.sleep(0.05)
            await self.settle(child)
            return {"reloaded": await self.evaluate("window.marker", child) != "before-reference-reload",
                    "documentRequests": Fixture.hits[request_path] - hits_before}
        elif op == "js_close":
            await self.evaluate("window.close();void 0", label)
            await asyncio.sleep(0.2)
            targets = (await self.command("Target.getTargets")).get("targetInfos", [])
            alive = any(t["targetId"] == self.targets[label] for t in targets)
            if not alive:
                self.closed.add(label)
            return {"stillAlive": alive}
        elif op == "popup_ref_close":
            await self.evaluate("popupRef.close();void 0", label)
            await asyncio.sleep(0.2)
            targets = (await self.command("Target.getTargets")).get("targetInfos", [])
            child = action["child"]
            alive = any(t["targetId"] == self.targets[child] for t in targets)
            if not alive:
                self.closed.add(child)
            return {"stillAlive": alive}
        elif op == "popup_ref_focus":
            await self.evaluate("popupRef.focus();void 0", label)
        elif op == "popup_message":
            child = action["child"]
            await self.evaluate("window.received=[];onmessage=e=>received.push({data:e.data,sourceIsOpener:e.source===opener});void 0", child)
            await self.evaluate("popupRef.postMessage('popup-message','*');void 0", label)
            await asyncio.sleep(0.2)
            return await self.evaluate("received", child)
        elif op == "delay":
            await asyncio.sleep(action.get("seconds", 1))
        elif op == "noop":
            pass
        else:
            raise ValueError(op)
        return {}

    async def close(self) -> None:
        for label, target in self.targets.items():
            if label not in self.closed:
                await self.command("Target.closeTarget", {"targetId": target})
        for client in self.clients.values():
            await client.close()
        write_json(self.output / "commands.json", self.commands)
        write_json(self.output / "observations.json", self.observations)
        write_json(self.output / "identities.json", {"targets": self.targets, "sessions": self.sessions})
        for name, client in self.clients.items():
            write_json(self.output / f"{name}-received.json", client.recorded_messages())
        for name, messages in self.archive.items():
            write_json(self.output / f"{name}-received.json", messages)


def deterministic() -> dict[str, list[dict]]:
    create = lambda p, **kw: {"op": "create", "page": p, **kw}
    return {
        "activation": [create("A"), create("B", background=True), {"op": "activate", "page": "B"},
                       {"op": "bring", "page": "A"}, {"op": "http_activate", "page": "B"},
                       create("C"), {"op": "close", "page": "C"}, {"op": "close", "page": "A"}],
        "background_navigation": [create("A"), create("B"), {"op": "storage", "page": "A"},
                                  {"op": "navigate", "page": "A", "path": "/A-next"},
                                  {"op": "navigate", "page": "A", "path": "/A-cross", "cross": True},
                                  {"op": "reload", "page": "A"},
                                  {"op": "hash", "page": "B", "value": "hash"},
                                  {"op": "back", "page": "A"}, {"op": "bring", "page": "A"}],
        "target_metadata": [create("A"), create("B"), {"op": "title", "page": "A", "value": "renamed-A"},
                            {"op": "navigate", "page": "A", "path": "/redirect?to=/A-final", "expected": "/A-final"},
                            {"op": "navigate", "page": "B", "path": "/with-frame"},
                            {"op": "detach_attach", "page": "A"}, {"op": "activate", "page": "A"}],
        "multi_client": [create("A"), create("B"),
                         {"op": "connect_browser", "page": "A", "client": "other"},
                         {"op": "connect_page", "page": "B", "client": "direct"},
                         {"op": "side_state", "page": "A", "client": "other"},
                         {"op": "side_state", "page": "B", "client": "direct"},
                         {"op": "wrong_session", "page": "A", "client": "other"},
                         {"op": "side_bring", "page": "A", "client": "other"},
                         {"op": "side_navigate", "page": "B", "client": "direct", "path": "/B-from-direct"},
                         {"op": "side_navigate", "page": "A", "client": "other", "path": "/A-from-other"},
                         {"op": "reconnect"}, {"op": "side_state", "page": "B", "client": "direct"},
                         {"op": "disconnect", "client": "other"}, {"op": "disconnect", "client": "direct"},
                         {"op": "bring", "page": "B"}],
        "tabs": [create("A"), create("B"), {"op": "attach_tab", "page": "A"},
                 {"op": "attach_tab", "page": "B"}, {"op": "tab_activate", "page": "A"},
                 {"op": "tab_navigate", "page": "B", "path": "/B-via-tab"},
                 {"op": "tab_activate", "page": "B"}, {"op": "close", "page": "A"}],
        "popups": [create("A"), create("B"), {"op": "activate", "page": "A"},
                   {"op": "popup", "page": "A", "path": "/popup", "child": "C", "name": "child"},
                   {"op": "activate", "page": "B"}, {"op": "navigate", "page": "C", "path": "/popup-next"},
                   {"op": "close", "page": "C"}],
        "same_document": [create("A"), create("B"), {"op": "hash", "page": "A", "value": "fragment"},
                          {"op": "history_push", "page": "B", "path": "/B-pushed"},
                          {"op": "back", "page": "A"}, {"op": "back", "page": "B"}],
        "concurrent_activation": [create("A"), create("B"),
                                  {"op": "concurrent_nav_activate", "page": "A", "other": "B"},
                                  {"op": "concurrent_nav_activate", "page": "B", "other": "A"}],
        "close_selection": [create("A"), create("B"), create("C"),
                            {"op": "activate", "page": "A"}, {"op": "close", "page": "A"}],
        "visibility_events": [create("A"), create("B"), {"op": "clear_events"},
                              {"op": "activate", "page": "A"}, {"op": "activate", "page": "B"}, {"op": "event_log"}],
        "focus_emulation": [create("A"), create("B"), {"op": "connect_browser", "page": "A", "client": "other"},
                            {"op": "focus_emulation", "page": "A", "enabled": True},
                            {"op": "disconnect", "client": "other"},
                            {"op": "navigate", "page": "A", "path": "/A-focus"},
                            {"op": "focus_emulation", "page": "A", "enabled": False},
                            {"op": "detach_attach", "page": "A"}],
        "contexts": [create("A"), {"op": "new_context", "context": "incognito"},
                     create("B", context="incognito"), create("C", context="incognito", background=True),
                     {"op": "activate", "page": "A"}, {"op": "bring", "page": "C"},
                     {"op": "dispose_context", "context": "incognito"}],
        "auto_pause": [{"op": "auto_enable", "wait": True}, {"op": "auto_enable", "client": "other", "wait": True},
                       {"op": "auto_create", "page": "A"}, {"op": "resume", "page": "A", "settle": False},
                       {"op": "resume", "page": "A", "client": "other"},
                       {"op": "auto_create", "page": "B"}, {"op": "disconnect", "client": "other"},
                       {"op": "resume", "page": "B"}],
        "event_isolation": [create("A"), create("B"), {"op": "connect_browser", "page": "A", "client": "other"},
                            {"op": "connect_page", "page": "B", "client": "direct"},
                            {"op": "console_route", "page": "A"}, {"op": "console_route", "page": "B"},
                            {"op": "subscription_isolation", "page": "A"},
                            {"op": "stale_handles", "page": "A", "other": "B"}],
        "new_window": [create("A"), create("B", newWindow=True), {"op": "window_map"},
                       create("C", background=True), {"op": "activate", "page": "A"},
                       {"op": "close", "page": "B"}, {"op": "window_map"}],
        "metadata_minimal": [create("A"), create("B"), {"op": "navigate", "page": "A", "path": "/A-updated"},
                             {"op": "delay", "seconds": 1}, {"op": "activate", "page": "A"}],
        "screencast_activation": [create("A"), create("B"), {"op": "screencast", "page": "A"},
                                  {"op": "screencast", "page": "B"}, {"op": "activate", "page": "A"},
                                  {"op": "navigate", "page": "B", "path": "/B-screencast"},
                                  {"op": "bring", "page": "B"}, {"op": "close", "page": "B"}],
        "javascript_navigation": [create("A"), create("B"), {"op": "location_navigate", "page": "A", "path": "/A-js"},
                                  {"op": "location_navigate", "page": "B", "path": "/B-js"},
                                  {"op": "discovery_refresh"}, {"op": "discover_samedoc_event", "page": "B"}],
        "navigation_races": [create("A"), create("B"), {"op": "supersede", "page": "A"},
                             {"op": "slow_other_target", "page": "A", "other": "B"},
                             {"op": "close_loading", "page": "A"}],
        "named_popup_reuse": [create("A"), {"op": "popup", "page": "A", "path": "/named", "child": "C", "name": "named-child"},
                              {"op": "popup_reuse", "page": "A", "path": "/reused", "child": "C", "name": "named-child"},
                              {"op": "name", "page": "C", "value": "renamed-child"},
                              {"op": "popup_reuse", "page": "A", "path": "/renamed", "child": "C", "name": "renamed-child"}],
        "tab_auto_pause": [{"op": "auto_enable", "wait": True, "tabs": True},
                           {"op": "auto_create_tab", "page": "A"}, {"op": "resume_tab", "page": "A"},
                           {"op": "auto_create_tab", "page": "B"}, {"op": "resume_tab", "page": "B"}],
        "popup_history": [create("A"), {"op": "popup", "page": "A", "path": "/popup-history", "child": "C"},
                          {"op": "history_dump", "page": "C"}, {"op": "back", "page": "C"}],
        "closed_default": [{"op": "adopt_initial", "page": "D"}, create("A"),
                           {"op": "close", "page": "D"}, {"op": "closed_http_activate", "page": "D"}],
        "popup_references": [create("A"), {"op": "name", "page": "A", "value": "opener-name"},
                             {"op": "popup", "page": "A", "path": "/popup-reference", "child": "C", "name": "child-name"},
                             {"op": "popup_reference", "page": "A"}, {"op": "opener_reference", "page": "C"},
                             {"op": "navigate", "page": "C", "path": "/child-navigated"},
                             {"op": "popup_reference", "page": "A"},
                             {"op": "popup_ref_navigate", "page": "A", "child": "C", "path": "/ref-navigated"},
                             {"op": "js_close", "page": "C"}, {"op": "popup_reference", "page": "A"}],
        "opener_lifetime": [create("A"), {"op": "popup", "page": "A", "path": "/opener-lifetime", "child": "C"},
                            {"op": "navigate", "page": "A", "path": "/opener-navigated"},
                            {"op": "opener_reference", "page": "C"}, {"op": "close", "page": "A"},
                            {"op": "opener_reference", "page": "C"}],
        "popup_actions": [create("A"), {"op": "popup", "page": "A", "path": "/popup-actions", "child": "C"},
                          create("B"), {"op": "popup_ref_focus", "page": "A"},
                          {"op": "popup_message", "page": "A", "child": "C"},
                          {"op": "popup_ref_close", "page": "A", "child": "C"}],
    }


def generated(seed: int, steps: int) -> list[dict]:
    rng = random.Random(seed)
    actions = [{"op": "create", "page": "A"}, {"op": "create", "page": "B", "background": True}]
    alive = ["A", "B"]
    serial = 2
    for index in range(steps):
        choices = ["activate", "bring", "http_activate", "navigate", "navigate", "reload", "hash", "title", "storage", "back", "detach_attach"]
        if len(alive) < 4:
            choices += ["create"]
        if len(alive) > 1:
            choices += ["close"]
        op, label = rng.choice(choices), rng.choice(alive)
        action: dict = {"op": op, "page": label}
        if op == "create":
            label = f"P{serial}"
            serial += 1
            alive.append(label)
            action.update(page=label, background=rng.choice([True, False]))
        elif op == "close":
            alive.remove(label)
        elif op == "navigate":
            action.update(path=f"/{label}-{seed}-{index}", cross=rng.choice([False, True]))
        elif op in {"hash", "title"}:
            action["value"] = f"v{seed}-{index}"
        actions.append(action)
    return actions


def generated_popup(seed: int, steps: int) -> list[dict]:
    rng = random.Random(seed)
    actions = [{"op": "create", "page": "A"},
               {"op": "popup", "page": "A", "path": "/popup-fuzz", "child": "C", "name": "fuzz-child"},
               {"op": "create", "page": "B"}]
    for index in range(steps):
        op = rng.choice(["navigate", "location_navigate", "activate", "bring", "back", "name",
                         "popup_reference", "opener_reference", "popup_message", "popup_ref_navigate"])
        action: dict[str, Any] = {"op": op, "page": "C"}
        if op in {"navigate", "location_navigate"}:
            action["path"] = f"/popup-{seed}-{index}"
        elif op in {"activate", "bring"}:
            action["page"] = rng.choice(["A", "B", "C"])
        elif op == "name":
            action["value"] = f"name-{seed}-{index}"
        elif op.startswith("popup_"):
            action.update(page="A", child="C", path=f"/ref-{seed}-{index}")
        actions.append(action)
    return actions


def canonical_activity_events(events: list[Any]) -> list[Any]:
    # Chromium/X11 exposes both orderings of focus and visibility notifications.
    # Accept only complete pairs with observed intermediate states. Preserve raw
    # observations and continue reporting missing events or other callback states.
    focused_hidden = {"event": "focus", "visibility": "hidden", "focus": True}
    shown_focused = {"event": "visibilitychange", "visibility": "visible", "focus": True}
    shown_unfocused = {"event": "visibilitychange", "visibility": "visible", "focus": False}
    focused_visible = {"event": "focus", "visibility": "visible", "focus": True}
    hidden_unfocused = {"event": "visibilitychange", "visibility": "hidden", "focus": False}
    blurred_hidden = {"event": "blur", "visibility": "hidden", "focus": False}
    blurred_visible = {"event": "blur", "visibility": "visible", "focus": False}
    result = []
    index = 0
    while index < len(events):
        if events[index:index + 2] == [focused_hidden, shown_focused]:
            result.extend([shown_unfocused, focused_visible])
            index += 2
        elif events[index:index + 2] == [hidden_unfocused, blurred_hidden]:
            result.extend([blurred_visible, hidden_unfocused])
            index += 2
        else:
            result.append(events[index])
            index += 1
    return result


def differences(a: Any, b: Any, path: str = "") -> list[dict]:
    if isinstance(a, dict) and isinstance(b, dict):
        result = []
        for key in sorted(a.keys() | b.keys()):
            result += differences(a.get(key, "<missing>"), b.get(key, "<missing>"), f"{path}.{key}")
        return result
    if isinstance(a, list) and isinstance(b, list):
        if canonical_activity_events(a) == canonical_activity_events(b):
            return []
    if a != b:
        return [{"path": path.lstrip("."), "chromium": a, "moli": b}]
    return []


async def run(args: argparse.Namespace) -> None:
    output = Path(args.output).resolve()
    lab = Lab(output, Path(args.moli).resolve(), args.chromium)
    cases = deterministic()
    # Closing the last normal window can terminate Chromium; keep this probe isolated.
    closed_default = cases.pop("closed_default")
    if args.only == "closed_default":
        cases = {"closed_default": closed_default}
    cases.update({f"seed-{seed}": generated(seed, args.steps) for seed in range(args.seed, args.seed + args.seeds)})
    cases.update({f"popup-seed-{seed}": generated_popup(seed, args.steps)
                  for seed in range(args.seed, args.seed + args.popup_seeds)})
    if args.actions:
        cases = {"replay": json.loads(Path(args.actions).read_text())}
    if args.case_file:
        cases = json.loads(Path(args.case_file).read_text())
    if args.only:
        cases = {key: value for key, value in cases.items() if key in args.only.split(",")}
    if not cases:
        raise ValueError("no cases selected")
    report = {}
    try:
        await lab.start()
        write_json(output / "run.json", {"head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT).decode().strip(),
                                        "fixture": lab.base, "endpoints": lab.endpoints, "arguments": vars(args)})
        for name, actions in cases.items():
            write_json(output / name / "actions.json", actions)
            observations = {}
            for engine, endpoint in lab.endpoints.items():
                case = Case(endpoint, lab.base, output / name / engine)
                try:
                    await case.connect()
                    await case.command("Target.setDiscoverTargets", {"discover": True})
                    for action in actions:
                        result = await case.action(action)
                        case.observations.append({"action": action, "result": result, "state": await case.snapshot()})
                    observations[engine] = case.observations
                finally:
                    await case.close()
            # Compare each step individually, preserving the shortest exposing prefix.
            diff = [
                {"step": i, "action": actions[i], "differences": fields}
                for i, (a, b) in enumerate(zip(observations["chromium"], observations["moli"]))
                if (fields := differences(a, b))
            ]
            report[name] = {"actions": len(actions), "different_steps": len(diff), "diffs": diff}
            write_json(output / "report.json", report)
            fields = sorted({d["path"] for step in diff for d in step["differences"]})
            print(json.dumps({"case": name, "actions": len(actions), "different_steps": len(diff), "fields": fields}), flush=True)
    finally:
        lab.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--moli", default=str(ROOT / "target/debug/moli"))
    parser.add_argument("--chromium", default="/usr/bin/chromium")
    parser.add_argument("--output", required=True)
    parser.add_argument("--seeds", type=int, default=5)
    parser.add_argument("--popup-seeds", type=int, default=0)
    parser.add_argument("--seed", type=int, default=20260929)
    parser.add_argument("--steps", type=int, default=25)
    parser.add_argument("--only")
    parser.add_argument("--actions", help="Replay an actions.json from an earlier run")
    parser.add_argument("--case-file", help="JSON object mapping case names to action lists")
    asyncio.run(run(parser.parse_args()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
