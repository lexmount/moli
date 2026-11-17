"""Worker-owned loading: the same JS, HTTP barriers and CDP assertions on both browsers."""

from __future__ import annotations

import asyncio
import base64
import json
import threading
from collections import Counter
from contextlib import AbstractContextManager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import urlsplit

from ..assertions import SmokeError, assert_equal, wait_until
from ..diagnostics import new_cdp_session
from ..state import SmokeState

SERVICE_WORKER = r"""
addEventListener('install', event => event.waitUntil(skipWaiting()));
addEventListener('activate', event => event.waitUntil(clients.claim()));
addEventListener('fetch', event => {
  const path = new URL(event.request.url).pathname;
  if (path === '/sw-controlled') event.respondWith(new Response('controlled'));
  if (path === '/sw-synthetic-start') event.respondWith(Response.redirect(new URL('/sw-final',location.href).href,302));
  if (path === '/sw-final') event.respondWith(new Response('SW final'));
});
"""

SCRIPT = r"""
let xhr, controller, innerWorker;
function receive(data, reply) {
  if (data.kind === 'overlap') {
    fetch(data.base + '/overlap-fetch').then(r => r.text()).then(text => reply({fetch:text}), e => reply({fetchError:e.name}));
    xhr = new XMLHttpRequest();
    xhr.open('GET', data.base + '/overlap-xhr');
    xhr.onabort = () => reply({xhr:'aborted'});
    xhr.onload = () => reply({xhr:xhr.responseText});
    xhr.onerror = () => reply({xhr:'error'});
    xhr.send();
  } else if (data.kind === 'abort-xhr') xhr.abort();
  else if (data.kind === 'stream') {
    controller = new AbortController();
    fetch(data.url, {signal:controller.signal}).then(async r => {
      reply({phase:'headers',status:r.status});
      try { reply({phase:'body',text:await r.text()}); }
      catch (e) { reply({phase:'body',sameReason:e === controller.signal.reason,reason:e}); }
    }, e => reply({phase:'fetch-error',error:e.name}));
  } else if (data.kind === 'read-large') {
    fetch(data.url).then(async response => {
      importScripts(data.barrier);
      const reader=response.body.getReader();
      let total=0,maxChunk=0,reads=0;
      for (;;) {
        const {done,value}=await reader.read();
        if(done)break;
        total+=value.byteLength;maxChunk=Math.max(maxChunk,value.byteLength);reads++;
      }
      reply({large:total,maxChunk,reads});
    }).catch(error=>reply({largeError:String(error)}));
  } else if (data.kind === 'abort-body') controller.abort('native-body-abort');
  else if (data.kind === 'fetch') {
    fetch(data.url, {method:data.method || 'GET',body:data.body,credentials:data.credentials || 'same-origin',redirect:data.redirect || 'follow',cache:data.cache || 'default'}).then(async r => {
      const clone = r.clone();
      reply({status:r.status,type:r.type,bodyNull:r.body === null,body:await r.text(),clone:await clone.text()});
    }, e => reply({error:e.name}));
  } else if (data.kind === 'nested') {
    innerWorker = new Worker(data.worker);
    innerWorker.onmessage = event => {
      if (event.data.ready) reply({nestedReady:true});
      else reply({nested:event.data});
    };
  } else if (data.kind === 'nested-fetch') {
    innerWorker.postMessage({kind:'fetch',url:data.url});
  } else if (data.kind === 'post') {
    fetch(data.url, {method:'POST',body:'worker-upload',headers:{'Content-Type':'text/plain'}})
      .then(r=>r.text()).then(body=>reply({post:body}),e=>reply({error:e.name}));
  } else if (data.kind === 'close') close();
}
onmessage = event => receive(event.data, value => postMessage(value));
onconnect = event => {
  const port = event.ports[0];
  port.onmessage = event => receive(event.data, value => port.postMessage(value));
  port.start(); port.postMessage({ready:true});
};
if (typeof SharedWorkerGlobalScope === 'undefined' || !(self instanceof SharedWorkerGlobalScope)) postMessage({ready:true});
"""


class _Fixture(AbstractContextManager):
    def __init__(self) -> None:
        self.hits: Counter[str] = Counter()
        self.requests: list[dict[str, Any]] = []
        self.gates: dict[str, threading.Event] = {}
        self.header_gates: set[str] = set()
        self.sent_headers: set[str] = set()
        self.lock = threading.Lock()
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:
                self.do_GET()

            def do_GET(self) -> None:
                path = urlsplit(self.path).path
                with fixture.lock:
                    fixture.hits[path] += 1
                    body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                    fixture.requests.append(
                        {
                            "path": path,
                            "method": self.command,
                            "headers": dict(self.headers),
                            "body": body.decode(),
                        }
                    )
                    gate = fixture.gates.get(path)
                if path in fixture.header_gates and gate is not None:
                    gate.wait()
                if path == "/missing-nested-worker.js":
                    self.send_response(404)
                    self.send_header("Content-Type", "text/javascript")
                    self.send_header("Content-Length", "7")
                    self.end_headers()
                    self.wfile.write(b"missing")
                    return
                if path.startswith("/native-probe/real-redirect-"):
                    suffix = path.removeprefix("/native-probe/real-redirect-")
                    self.send_response(int(suffix.split("-")[0]))
                    self.send_header("Location", "/original-" + suffix)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if path.startswith("/original-"):
                    body, kind = b"original body", "text/plain"
                elif path.startswith("/edited-"):
                    body, kind = b"edited body", "text/plain"
                elif path.startswith("/worker-cache-"):
                    self.protocol_version = "HTTP/1.1"
                    if self.headers.get("If-None-Match") == '"worker-cache-probe"':
                        self.send_response(304)
                        self.send_header("ETag", '"worker-cache-probe"')
                        self.send_header("Cache-Control", "max-age=0, must-revalidate")
                        self.send_header("X-Validated", "wire-304")
                        self.send_header("Connection", "close")
                        self.end_headers()
                        return
                    self.send_response(200)
                    self.send_header("Content-Type", "text/plain")
                    self.send_header("Content-Length", "11")
                    self.send_header("ETag", '"worker-cache-probe"')
                    self.send_header("Cache-Control", "max-age=0, must-revalidate")
                    self.send_header("X-Cached", "body-200")
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.wfile.write(b"cached body")
                    return
                elif path.startswith("/native-probe/basic-auth-"):
                    if self.headers.get("Authorization") != "Basic dXNlcjpwYXNz":
                        self.send_response(401)
                        self.send_header(
                            "WWW-Authenticate", 'Basic realm="worker-extra-info"'
                        )
                        self.send_header("Content-Length", "0")
                        self.end_headers()
                        return
                    body, kind = b"authenticated", "text/plain"
                elif path == "/worker.js":
                    body, kind = SCRIPT.encode(), "text/javascript"
                elif path == "/reader-barrier.js":
                    body, kind = b"void 0;", "text/javascript"
                elif path == "/native-probe/read-large":
                    body, kind = b"x" * (4 * 1024 * 1024), "application/octet-stream"
                elif path == "/sw.js":
                    body, kind = SERVICE_WORKER.encode(), "text/javascript"
                elif path == "/sw-network-start":
                    self.send_response(302)
                    self.send_header("Location", "/sw-final")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                elif path == "/post-redirect":
                    self.send_response(303)
                    self.send_header("Location", "/post-final")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                elif path == "/sw-final":
                    body, kind = b"HTTP final", "text/plain"
                elif path == "/page":
                    body, kind = (
                        b"<!doctype html><title>native Worker loading</title>",
                        "text/html",
                    )
                else:
                    body, kind = b"native-worker-body", "text/plain"
                self.send_response(
                    302 if "/http-empty-" in path or "/http-missing-" in path else 200
                )
                if "/http-empty-" in path or "/http-missing-" in path:
                    self.send_header("Access-Control-Allow-Origin", "*")
                if "/http-empty-" in path:
                    self.send_header("Location", "")
                self.send_header("Content-Type", kind)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.flush()
                fixture.sent_headers.add(path)
                if gate is not None:
                    gate.wait()
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass  # Expected when the tested request cancels its body.

            def log_message(self, *_: Any) -> None:
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.origin = f"http://127.0.0.1:{self.server.server_port}"

    def hold(self, path: str, *, before_headers: bool = False) -> None:
        with self.lock:
            self.sent_headers.discard(path)
            self.gates[path] = threading.Event()
            if before_headers:
                self.header_gates.add(path)
            else:
                self.header_gates.discard(path)

    def release(self, path: str) -> None:
        self.gates[path].set()

    def __exit__(self, *_: object) -> None:
        for gate in self.gates.values():
            gate.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()


class _Child:
    def __init__(self, parent: Any, session: str, events: list[dict[str, Any]]) -> None:
        self.parent, self.session, self.events = parent, session, events
        self.sequence = 0
        self.pending: dict[int, asyncio.Future[Any]] = {}
        self.listeners: dict[str, list[Any]] = {}
        parent.on("Target.receivedMessageFromTarget", self.receive)

    def receive(self, outer: dict[str, Any]) -> None:
        if outer.get("sessionId") != self.session:
            return
        message = json.loads(outer["message"])
        if "id" in message:
            future = self.pending.pop(message["id"], None)
            if future is not None and not future.done():
                if "error" in message:
                    future.set_exception(SmokeError(str(message["error"])))
                else:
                    future.set_result(message.get("result", {}))
        else:
            self.events.append({**message, "sessionId": self.session})
            for callback in self.listeners.get(message["method"], []):
                callback(message.get("params", {}))

    def on(self, method: str, callback: Any) -> None:
        self.listeners.setdefault(method, []).append(callback)

    async def send(self, method: str, params: dict[str, Any] | None = None) -> Any:
        self.sequence += 1
        future = asyncio.get_running_loop().create_future()
        self.pending[self.sequence] = future
        await self.parent.send(
            "Target.sendMessageToTarget",
            {
                "sessionId": self.session,
                "message": json.dumps(
                    {"id": self.sequence, "method": method, "params": params or {}}
                ),
            },
        )
        return await asyncio.wait_for(future, 10)


class _Targets:
    def __init__(self, page_cdp: Any, browser_cdp: Any) -> None:
        self.page_cdp, self.browser_cdp = page_cdp, browser_cdp
        self.events: list[dict[str, Any]] = []
        self.children: dict[str, _Child] = {}
        self.targets: dict[str, dict[str, Any]] = {}
        self.tasks: list[asyncio.Task[Any]] = []
        self.fetch = False
        self.attaching: set[str] = set()
        for parent in (page_cdp, browser_cdp):
            parent.on(
                "Target.attachedToTarget",
                lambda event, parent=parent: self.attach(parent, event),
            )
            parent.on(
                "Target.detachedFromTarget",
                lambda params: self.events.append(
                    {"method": "Target.detachedFromTarget", "params": params}
                ),
            )
        for method in (
            "Fetch.requestPaused",
            "Fetch.authRequired",
            "Network.requestWillBeSent",
            "Network.requestWillBeSentExtraInfo",
            "Network.responseReceivedExtraInfo",
            "Network.loadingFinished",
            "Network.loadingFailed",
        ):
            page_cdp.on(
                method,
                lambda params, method=method: self.events.append(
                    {"method": method, "params": params, "sessionId": "page"}
                ),
            )

    def attach(self, parent: Any, event: dict[str, Any]) -> None:
        info = event["targetInfo"]
        if info["type"] not in ("worker", "shared_worker"):
            return
        if parent is self.browser_cdp:
            if info["targetId"] not in self.attaching:
                self.attaching.add(info["targetId"])
                self.tasks.append(asyncio.create_task(self.attach_shared(info)))
            return
        session = event["sessionId"]
        child = _Child(parent, session, self.events)
        self.children[session] = child
        self.targets[session] = info
        self.tasks.append(asyncio.create_task(self.enable(child, info["type"])))

    async def attach_shared(self, info: dict[str, Any]) -> None:
        # Browser-level auto-attach requires flatten. A separate explicit
        # legacy attachment lets Playwright's public CDPSession carry commands.
        attached = await self.browser_cdp.send(
            "Target.attachToTarget", {"targetId": info["targetId"], "flatten": False}
        )
        session = attached["sessionId"]
        child = _Child(self.browser_cdp, session, self.events)
        self.children[session] = child
        self.targets[session] = info
        await self.enable(child, "shared_worker")

    async def enable(self, child: _Child, kind: str) -> None:
        child.on("Target.attachedToTarget", lambda event: self.attach(child, event))
        child.on(
            "Target.detachedFromTarget",
            lambda params: self.events.append(
                {"method": "Target.detachedFromTarget", "params": params}
            ),
        )
        await child.send(
            "Target.setAutoAttach",
            {
                "autoAttach": True,
                "waitForDebuggerOnStart": True,
                "flatten": False,
                "filter": [{"type": "worker", "exclude": False}, {"exclude": True}],
            },
        )
        await child.send("Network.enable")
        if kind == "shared_worker" and self.fetch:
            await child.send(
                "Fetch.enable",
                {
                    "patterns": [
                        {"urlPattern": "*native-probe*", "resourceType": kind}
                        for kind in ("Fetch", "XHR")
                    ]
                },
            )
        await child.send("Runtime.runIfWaitingForDebugger")

    def check(self) -> None:
        for task in self.tasks:
            if task.done():
                task.result()

    async def start(self, intercept: bool = False) -> None:
        self.fetch = intercept
        await self.page_cdp.send("Network.enable")
        if intercept:
            await self.page_cdp.send(
                "Fetch.enable",
                {
                    "patterns": [
                        {"urlPattern": "*native-probe*", "resourceType": kind}
                        for kind in ("Fetch", "XHR")
                    ]
                },
            )
        for parent, kind in (
            (self.page_cdp, "worker"),
            (self.browser_cdp, "shared_worker"),
        ):
            await parent.send(
                "Target.setAutoAttach",
                {
                    "autoAttach": True,
                    "waitForDebuggerOnStart": True,
                    "flatten": parent is self.browser_cdp,
                    "filter": [{"type": kind, "exclude": False}, {"exclude": True}],
                },
            )

    async def worker(self, page: Any, url: str, shared: bool) -> _Child:
        before = set(self.children)
        await page.evaluate(
            """([url,shared]) => {
          globalThis.nativeMessages=[];
          globalThis.nativeWorker=shared ? new SharedWorker(url) : new Worker(url);
          globalThis.nativePort=shared ? nativeWorker.port : nativeWorker;
          globalThis.nativeReady=new Promise(resolve=>{
            nativePort.onmessage=e=>{nativeMessages.push(e.data);if(e.data.ready)resolve(true);};
          });
          if(shared)nativePort.start();
        }""",
            [url, shared],
        )

        def attached() -> bool:
            self.check()
            return any(
                self.targets[session]["url"] == url
                for session in set(self.children) - before
            )

        await wait_until(attached, "native Worker target attachment")
        child = next(
            self.children[session]
            for session in set(self.children) - before
            if self.targets[session]["url"] == url
        )
        await page.evaluate("nativeReady")
        return child

    async def event(self, method: str, **fields: Any) -> dict[str, Any]:
        found: dict[str, Any] | None = None

        def ready() -> bool:
            nonlocal found
            self.check()
            found = next(
                (
                    event
                    for event in self.events
                    if event["method"] == method
                    and all(event["params"].get(k) == v for k, v in fields.items())
                ),
                None,
            )
            return found is not None

        await wait_until(ready, f"{method} {fields}")
        assert found is not None
        return found

    async def request(self, url: str, child: _Child) -> str:
        await wait_until(
            lambda: any(
                e["method"] == "Network.requestWillBeSent"
                and e["params"]["request"]["url"] == url
                and e.get("sessionId") == child.session
                for e in self.events
            ),
            f"Worker Network request {url}",
        )
        event = next(
            e
            for e in self.events
            if e["method"] == "Network.requestWillBeSent"
            and e["params"]["request"]["url"] == url
            and e.get("sessionId") == child.session
        )
        assert_equal(event["params"].get("frameId"), None, "Worker frameId")
        assert_equal(event["params"]["loaderId"], "", "Worker loaderId")
        return event["params"]["requestId"]

    async def pause(
        self, url: str, child: _Child, shared: bool, *, response: bool = False
    ) -> dict[str, Any]:
        await wait_until(
            lambda: any(
                e["method"] == "Fetch.requestPaused"
                and e["params"]["request"]["url"] == url
                and ("responseStatusCode" in e["params"]) == response
                for e in self.events
            ),
            "native Worker Fetch pause",
        )
        event = next(
            e
            for e in self.events
            if e["method"] == "Fetch.requestPaused"
            and e["params"]["request"]["url"] == url
            and ("responseStatusCode" in e["params"]) == response
        )
        assert_equal(
            event["sessionId"],
            child.session if shared else "page",
            "Fetch control target",
        )
        assert_equal(
            event["params"]["networkId"],
            await self.request(url, child),
            "Fetch / Worker Network identity",
        )
        return event["params"]

    async def cleanup(self) -> None:
        for parent in (self.page_cdp, self.browser_cdp):
            await parent.send(
                "Target.setAutoAttach",
                {
                    "autoAttach": False,
                    "waitForDebuggerOnStart": False,
                    "flatten": parent is self.browser_cdp,
                },
            )
        if self.tasks:
            await asyncio.gather(*self.tasks)


async def _message(page: Any, field: str, value: Any) -> dict[str, Any]:
    await wait_until(
        lambda: page.evaluate(
            "([field,value])=>nativeMessages.some(m=>m[field]===value)", [field, value]
        ),
        f"Worker message {field}={value}",
    )
    return await page.evaluate(
        "([field,value])=>nativeMessages.find(m=>m[field]===value)", [field, value]
    )


async def _check_redirect_responses(
    state: SmokeState, fixture: _Fixture, page: Any, targets: _Targets, child: _Child
) -> None:
    for decision in ("request-fulfill", "response-fulfill", "continue-response"):
        for location_kind in ("missing", "empty", "target"):
            for mode in ("error", "follow", "manual"):
                if location_kind == "target" and mode != "follow":
                    continue
                label = f"{decision}-{location_kind}-{mode}"
                next_path = "/next-" + label
                path = "/native-probe/synthetic-" + label
                url = fixture.origin + path
                await page.evaluate(
                    '([url,redirect])=>{nativeMessages=[];nativePort.postMessage({kind:"fetch",url,redirect});}',
                    [url, mode],
                )
                pause = await targets.pause(url, child, False)
                if decision != "request-fulfill":
                    await targets.page_cdp.send(
                        "Fetch.continueRequest",
                        {"requestId": pause["requestId"], "interceptResponse": True},
                    )
                    pause = await targets.pause(url, child, False, response=True)
                    extra = await targets.event(
                        "Network.responseReceivedExtraInfo",
                        requestId=pause["networkId"],
                    )
                    assert_equal(
                        extra.get("sessionId"), "page", "wire response ExtraInfo owner"
                    )
                    assert_equal(
                        extra["params"]["statusCode"],
                        200,
                        "wire status before debugger override",
                    )
                    wire_headers = {
                        k.lower(): v for k, v in extra["params"]["headers"].items()
                    }
                    assert_equal(
                        wire_headers["content-type"], "text/plain", "wire Content-Type"
                    )
                    assert_equal(
                        wire_headers["content-length"],
                        str(len("native-worker-body")),
                        "wire Content-Length",
                    )
                params: dict[str, Any] = {
                    "requestId": pause["requestId"],
                    "responseCode": 302,
                    "responsePhrase": "Found",
                    "responseHeaders": [
                        {"name": "Content-Type", "value": "text/plain"}
                    ],
                }
                if location_kind != "missing":
                    params["responseHeaders"].append(
                        {
                            "name": "Location",
                            "value": next_path if location_kind == "target" else "",
                        }
                    )
                body = (
                    "native-worker-body"
                    if decision == "continue-response"
                    else "synthetic"
                )
                if decision != "continue-response":
                    params["body"] = base64.b64encode(body.encode()).decode()
                await targets.page_cdp.send(
                    "Fetch.continueResponse"
                    if decision == "continue-response"
                    else "Fetch.fulfillRequest",
                    params,
                )
                await wait_until(
                    lambda: page.evaluate("nativeMessages.length>0"),
                    "rewritten Worker response",
                )
                result = await page.evaluate("nativeMessages[0]")
                expected = (
                    {
                        "status": 0,
                        "type": "opaqueredirect",
                        "bodyNull": True,
                        "body": "",
                        "clone": "",
                    }
                    if mode == "manual"
                    else {
                        "status": 302,
                        "type": "basic",
                        "bodyNull": False,
                        "body": body,
                        "clone": body,
                    }
                )
                follows = (
                    location_kind == "target"
                    and decision != "continue-response"
                    and mode == "follow"
                )
                if follows:
                    expected = {
                        "status": 200,
                        "type": "basic",
                        "bodyNull": False,
                        "body": "native-worker-body",
                        "clone": "native-worker-body",
                    }
                assert_equal(result, expected, "rewritten redirect " + label)
                await targets.event(
                    "Network.loadingFinished",
                    requestId=pause["networkId"],
                )
                assert_equal(
                    fixture.hits[next_path],
                    int(follows),
                    "actual next-hop requests " + label,
                )
                assert_equal(
                    fixture.hits[path],
                    int(decision != "request-fulfill"),
                    "no next hop " + label,
                )
                state.record("native_worker_synthetic_" + label, {"response": result})


async def _check_continued_real_redirects(
    state: SmokeState, fixture: _Fixture, page: Any, targets: _Targets, child: _Child
) -> None:
    for original_status, status in ((302, 302), (302, 200), (307, 302), (307, 200)):
        suffix = f"{original_status}-{status}"
        path = "/native-probe/real-redirect-" + suffix
        url = fixture.origin + path
        await page.evaluate(
            'url=>{nativeMessages=[];nativePort.postMessage({kind:"fetch",url,method:"POST",body:"upload"});}',
            url,
        )
        pause = await targets.pause(url, child, False)
        await targets.page_cdp.send(
            "Fetch.continueRequest",
            {"requestId": pause["requestId"], "interceptResponse": True},
        )
        pause = await targets.pause(url, child, False, response=True)
        await targets.page_cdp.send(
            "Fetch.continueResponse",
            {
                "requestId": pause["requestId"],
                "responseCode": status,
                "responseHeaders": [
                    {"name": "Location", "value": "/edited-" + suffix},
                    {"name": "Content-Type", "value": "text/plain"},
                ],
            },
        )
        await wait_until(
            lambda: page.evaluate("nativeMessages.length>0"),
            "continued original redirect",
        )
        result = await page.evaluate("nativeMessages[0]")
        assert_equal(
            result,
            {
                "status": 200,
                "type": "basic",
                "bodyNull": False,
                "body": "original body",
                "clone": "original body",
            },
            "continueResponse follows the original transport target",
        )
        await targets.event("Network.loadingFinished", requestId=pause["networkId"])
        assert_equal(
            fixture.hits["/original-" + suffix], 1, "original target actually fetched"
        )
        assert_equal(
            fixture.hits["/edited-" + suffix],
            0,
            "edited Location is only a response header",
        )
        redirects = [
            e
            for e in targets.events
            if e["method"] == "Network.requestWillBeSent"
            and e["params"]["requestId"] == pause["networkId"]
            and "redirectResponse" in e["params"]
        ]
        assert_equal(len(redirects), 1, "one reported redirect")
        assert_equal(
            redirects[0]["params"]["redirectResponse"]["status"],
            status,
            "edited redirect response status",
        )
        headers = {
            k.lower(): v
            for k, v in redirects[0]["params"]["redirectResponse"]["headers"].items()
        }
        assert_equal(
            headers.get("location"),
            "/edited-" + suffix,
            "edited response header projection",
        )
        received = next(
            r for r in fixture.requests if r["path"] == "/original-" + suffix
        )
        assert_equal(
            received["method"],
            "GET" if original_status == 302 else "POST",
            "method follows original redirect status",
        )
        assert_equal(
            received["body"],
            "" if original_status == 302 else "upload",
            "upload follows original redirect status",
        )
        state.record(
            "native_worker_continued_real_redirect_" + suffix,
            {"response": result, "redirects": redirects},
        )


async def _check_cache_revalidation(
    state: SmokeState, fixture: _Fixture, page: Any, targets: _Targets, child: _Child
) -> None:
    path = "/worker-cache-revalidate"
    url = fixture.origin + path
    for index, wire_status in enumerate((200, 304)):
        before = len(targets.events)
        await page.evaluate(
            'url=>{nativeMessages=[];nativePort.postMessage({kind:"fetch",url,cache:"no-cache",credentials:"omit"});}',
            url,
        )
        await wait_until(
            lambda: page.evaluate("nativeMessages.length>0"),
            "Worker cache revalidation",
        )
        result = await page.evaluate("nativeMessages[0]")
        assert_equal(result["status"], 200, "cache-revalidated effective status")
        assert_equal(result["body"], "cached body", "cache-revalidated body")
        fresh = targets.events[before:]
        request = next(
            e
            for e in fresh
            if e["method"] == "Network.requestWillBeSent"
            and e["params"]["request"]["url"] == url
            and e.get("sessionId") == child.session
        )
        request_id = request["params"]["requestId"]
        extra = await targets.event(
            "Network.responseReceivedExtraInfo", requestId=request_id
        )
        assert_equal(
            extra["params"]["statusCode"], wire_status, "raw revalidation status"
        )
        if index:
            headers = {k.lower(): v for k, v in extra["params"]["headers"].items()}
            assert_equal(
                headers.get("x-validated"), "wire-304", "raw validation headers"
            )
            assert_equal(
                headers.get("x-cached"), None, "cached headers are absent from raw 304"
            )
        await targets.event("Network.loadingFinished", requestId=request_id)
        response = await targets.event("Network.responseReceived", requestId=request_id)
        assert_equal(
            response["params"]["response"]["status"],
            200,
            "effective Network response after validation",
        )
        state.record(
            "native_worker_cache_revalidation_" + str(index),
            {"response": result, "extra": extra},
        )
    assert_equal(
        fixture.hits[path], 2, "both cache validation requests reached the server"
    )
    assert_equal(
        {
            k.lower(): v
            for k, v in next(
                r for r in reversed(fixture.requests) if r["path"] == path
            )["headers"].items()
        }.get("if-none-match"),
        '"worker-cache-probe"',
        "ETag validation sent",
    )


async def _check_auth_extra_info(
    state: SmokeState, fixture: _Fixture, page: Any, targets: _Targets, child: _Child
) -> None:
    await targets.page_cdp.send(
        "Fetch.enable",
        {"handleAuthRequests": True, "patterns": [{"urlPattern": "*native-probe*"}]},
    )
    url = fixture.origin + "/native-probe/basic-auth-success"
    await page.evaluate(
        'url=>{nativeMessages=[];nativePort.postMessage({kind:"fetch",url});}', url
    )
    pause = await targets.pause(url, child, False)
    await targets.page_cdp.send(
        "Fetch.continueRequest",
        {"requestId": pause["requestId"], "interceptResponse": True},
    )
    auth = await targets.event("Fetch.authRequired")
    # A command round trip drains facts preceding the live authentication pause.
    await targets.page_cdp.send("Network.getCookies", {"urls": [url]})

    def extras():
        return [
            e
            for e in targets.events
            if e["method"] == "Network.responseReceivedExtraInfo"
            and e["params"]["requestId"] == pause["networkId"]
        ]

    assert_equal(extras(), [], "retryable challenge has no response ExtraInfo")
    await targets.page_cdp.send(
        "Fetch.continueWithAuth",
        {
            "requestId": auth["params"]["requestId"],
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "user",
                "password": "pass",
            },
        },
    )
    response = await targets.pause(url, child, False, response=True)
    extra = await targets.event(
        "Network.responseReceivedExtraInfo", requestId=pause["networkId"]
    )
    assert_equal(
        extra["params"]["statusCode"],
        200,
        "selected authenticated response before pause",
    )
    await targets.page_cdp.send(
        "Fetch.continueResponse", {"requestId": response["requestId"]}
    )
    await wait_until(
        lambda: page.evaluate("nativeMessages.length>0"),
        "authenticated Worker response",
    )
    result = await page.evaluate("nativeMessages[0]")
    assert_equal(result["body"], "authenticated", "authenticated body")
    await targets.event("Network.loadingFinished", requestId=pause["networkId"])
    assert_equal(len(extras()), 1, "exactly one selected response ExtraInfo")
    state.record(
        "native_worker_auth_selected_extra_info",
        {"response": result, "extras": extras()},
    )


async def _check_large_worker_reader(
    state: SmokeState, fixture: _Fixture, page: Any, targets: _Targets, child: _Child
) -> None:
    path = "/native-probe/read-large"
    barrier = "/reader-barrier.js"
    fixture.hold(path)
    fixture.hold(barrier)
    try:
        await page.evaluate(
            '([url,barrier])=>{nativeMessages=[];nativePort.postMessage({kind:"read-large",url,barrier});}',
            [fixture.origin + path, fixture.origin + barrier],
        )
        pause = await targets.pause(fixture.origin + path, child, False)
        await targets.page_cdp.send(
            "Fetch.continueRequest", {"requestId": pause["requestId"]}
        )
        await wait_until(
            lambda: fixture.hits[barrier] == 1,
            "Worker JS blocked before consuming the response",
        )
        fixture.release(path)
        fixture.release(barrier)
        await wait_until(
            lambda: page.evaluate("nativeMessages.length>0"),
            "bounded Worker stream consumption",
        )
        result = await page.evaluate("nativeMessages[0]")
        assert_equal(
            result.get("large"),
            4 * 1024 * 1024,
            "all captured bytes delivered before stream close",
        )
        await targets.event("Network.loadingFinished", requestId=pause["networkId"])
        state.record("native_worker_bounded_reader", result)
    finally:
        fixture.release(path)
        fixture.release(barrier)


async def _check_empty_http_redirects(
    state: SmokeState, fixture: _Fixture, page: Any, targets: _Targets, child: _Child
) -> None:
    for rewritten in (False, True):
        for mode in ("error", "follow", "manual"):
            path = f"/native-probe/http-empty-{rewritten}-{mode}"
            url = fixture.origin + path
            await page.evaluate(
                '([url,redirect])=>{nativeMessages=[];nativePort.postMessage({kind:"fetch",url,redirect});}',
                [url, mode],
            )
            pause = await targets.pause(url, child, False)
            await targets.page_cdp.send(
                "Fetch.continueRequest",
                {"requestId": pause["requestId"], "interceptResponse": True},
            )
            pause = await targets.pause(url, child, False, response=True)
            params: dict[str, Any] = {"requestId": pause["requestId"]}
            if rewritten:
                params.update(
                    responseCode=302,
                    responsePhrase="Found",
                    responseHeaders=pause["responseHeaders"],
                )
            await targets.page_cdp.send("Fetch.continueResponse", params)
            await wait_until(
                lambda: page.evaluate("nativeMessages.length>0"),
                "HTTP empty Location response",
            )
            result = await page.evaluate("nativeMessages[0]")
            expected = (
                {
                    "status": 0,
                    "type": "opaqueredirect",
                    "bodyNull": True,
                    "body": "",
                    "clone": "",
                }
                if mode == "manual"
                else {
                    "status": 302,
                    "type": "basic",
                    "bodyNull": False,
                    "body": "native-worker-body",
                    "clone": "native-worker-body",
                }
            )
            assert_equal(
                result,
                expected,
                "HTTP 302 empty Location with original/rewritten headers",
            )
            await targets.event("Network.loadingFinished", requestId=pause["networkId"])
            assert_equal(
                fixture.hits[path], 1, "empty Location must not refetch the same URL"
            )
            state.record(
                f"native_worker_http_empty_{rewritten}_{mode}", {"response": result}
            )

    await targets.page_cdp.send("Fetch.disable")
    try:
        for location_kind, foreign in (
            ("empty", False),
            ("missing", False),
            ("empty", True),
            ("missing", True),
        ):
            for mode in ("error", "follow", "manual"):
                path = f"/native-probe/http-{location_kind}-ordinary-{mode}-{foreign}"
                url = (
                    fixture.origin.replace("127.0.0.1", "localhost")
                    if foreign
                    else fixture.origin
                ) + path
                await page.evaluate(
                    '([url,redirect])=>{nativeMessages=[];nativePort.postMessage({kind:"fetch",url,redirect});}',
                    [url, mode],
                )
                await wait_until(
                    lambda: page.evaluate("nativeMessages.length>0"),
                    "ordinary empty-Location response",
                )
                result = await page.evaluate("nativeMessages[0]")
                expected = (
                    {
                        "status": 0,
                        "type": "opaqueredirect",
                        "bodyNull": True,
                        "body": "",
                        "clone": "",
                    }
                    if mode == "manual"
                    else {
                        "status": 302,
                        "type": "cors" if foreign else "basic",
                        "bodyNull": False,
                        "body": "native-worker-body",
                        "clone": "native-worker-body",
                    }
                )
                assert_equal(
                    result, expected, "ordinary Location " + location_kind + "/" + mode
                )
                request = await targets.request(url, child)
                await targets.event(
                    "Network.loadingFinished",
                    requestId=request,
                )
                assert_equal(
                    fixture.hits[path], 1, "one ordinary request with empty Location"
                )
                state.record(
                    "native_worker_ordinary_"
                    + location_kind
                    + "_"
                    + mode
                    + "_"
                    + str(foreign),
                    {"response": result},
                )
    finally:
        await targets.page_cdp.send(
            "Fetch.enable",
            {
                "patterns": [
                    {"urlPattern": "*native-probe*", "resourceType": kind}
                    for kind in ("Fetch", "XHR")
                ]
            },
        )


async def _check_nested_worker(
    state: SmokeState,
    fixture: _Fixture,
    page: Any,
    targets: _Targets,
    outer: _Child,
    shared: bool,
) -> None:
    marker = fixture.origin + f"/native-probe/nested-{shared}"
    worker_url = fixture.origin + f"/worker.js?inner-{shared}"
    await page.evaluate(
        '([worker,url])=>{nativeMessages=[];nativePort.postMessage({kind:"nested",worker,url});}',
        [worker_url, marker],
    )
    await wait_until(
        lambda: any(info["url"] == worker_url for info in targets.targets.values()),
        "nested Worker target",
    )
    inner = next(
        targets.children[session]
        for session, info in targets.targets.items()
        if info["url"] == worker_url
    )
    if inner.parent is not outer:
        raise SmokeError("nested Worker must attach through its creator's session")
    # Playwright has its own attachment too; another debugger may resume the
    # Worker before our Network.enable completes. Start the probe explicitly
    # after this observer is installed, rather than racing initial script code.
    await asyncio.gather(*targets.tasks)
    await _message(page, "nestedReady", True)
    await page.evaluate(
        'url=>nativePort.postMessage({kind:"nested-fetch",url})', marker
    )
    request = await targets.request(marker, inner)
    if not shared:
        pause = await targets.pause(marker, inner, False)
        await targets.page_cdp.send(
            "Fetch.continueRequest", {"requestId": pause["requestId"]}
        )
    await wait_until(
        lambda: page.evaluate("nativeMessages.some(m=>'nested' in m)"),
        "nested Worker response",
    )
    result = await page.evaluate("nativeMessages.find(m=>'nested' in m)")
    assert_equal(
        result,
        {
            "nested": {
                "status": 200,
                "type": "basic",
                "bodyNull": False,
                "body": "native-worker-body",
                "clone": "native-worker-body",
            }
        },
        "nested Worker response",
    )
    await targets.event("Network.loadingFinished", requestId=request)
    observers = [
        e["sessionId"]
        for e in targets.events
        if e["method"] == "Network.requestWillBeSent"
        and e["params"]["request"]["url"] == marker
    ]
    assert_equal(
        observers, [inner.session], "inner request belongs only to inner Worker"
    )
    state.record(
        "native_nested_worker_" + str(shared),
        {"response": result, "events": targets.events.copy()},
    )


async def _check_network_extra_info(
    state: SmokeState, fixture: _Fixture, page: Any, targets: _Targets, child: _Child
) -> None:
    await page.evaluate("document.cookie='native_worker_probe=present; Path=/'")
    url = fixture.origin + "/post-redirect"
    await page.evaluate(
        'url=>{nativeMessages=[];nativePort.postMessage({kind:"post",url});}', url
    )
    await _message(page, "post", "native-worker-body")
    request = await targets.request(url, child)
    await targets.event("Network.loadingFinished", requestId=request)
    await wait_until(
        lambda: (
            len(
                [
                    e
                    for e in targets.events
                    if e["method"] == "Network.responseReceivedExtraInfo"
                    and e["params"]["requestId"] == request
                ]
            )
            == 2
        ),
        "both redirect response ExtraInfo events",
    )
    routed_extras = [
        e
        for e in targets.events
        if e["method"] == "Network.requestWillBeSentExtraInfo"
        and e["params"]["requestId"] == request
    ]
    assert_equal(
        [e["sessionId"] for e in routed_extras],
        ["page", "page"],
        "DedicatedWorker request ExtraInfo belongs to Page",
    )
    response_extras = [
        e
        for e in targets.events
        if e["method"] == "Network.responseReceivedExtraInfo"
        and e["params"]["requestId"] == request
    ]
    assert_equal(
        [e["sessionId"] for e in response_extras],
        ["page", "page"],
        "DedicatedWorker response ExtraInfo belongs to Page",
    )
    assert_equal(
        [e["params"]["statusCode"] for e in response_extras],
        [303, 200],
        "ExtraInfo for every HTTP response, without Set-Cookie",
    )
    redirected = [
        e
        for e in targets.events
        if e["method"] == "Network.requestWillBeSent"
        and e["sessionId"] == child.session
        and e["params"]["requestId"] == request
        and "redirectResponse" in e["params"]
    ]
    assert_equal(len(redirected), 1, "one Worker redirect transition")
    assert_equal(
        redirected[0]["params"]["redirectHasExtraInfo"],
        True,
        "redirect ExtraInfo availability is independent of cookies",
    )
    extras = [
        e["params"]
        for e in targets.events
        if e["method"] == "Network.requestWillBeSentExtraInfo"
        and e["params"]["requestId"] == request
    ]
    assert_equal(len(extras), 2, "one ExtraInfo per HTTP hop")
    first, last = (
        {k.lower(): v for k, v in info["headers"].items()} for info in extras
    )
    assert_equal(first.get("content-type"), "text/plain", "POST wire content type")
    assert_equal(
        first.get("content-length"),
        str(len("worker-upload")),
        "POST wire content length",
    )
    for header in ("content-type", "content-length"):
        assert_equal(last.get(header), None, "GET must not reuse POST " + header)
    for info in extras:
        sent = [
            entry["cookie"]["name"]
            for entry in info["associatedCookies"]
            if not entry["blockedReasons"]
        ]
        if "native_worker_probe" not in sent:
            raise SmokeError(f"missing actual request Cookie report: {info}")
    received = [
        r for r in fixture.requests if r["path"] in ("/post-redirect", "/post-final")
    ]
    assert_equal(
        [r["method"] for r in received], ["POST", "GET"], "actual redirect methods"
    )
    assert_equal(
        [r["body"] for r in received], ["worker-upload", ""], "actual redirect bodies"
    )
    state.record(
        "native_worker_redirect_extra_info", {"events": extras, "requests": received}
    )


async def _check_service_worker_redirects(state: SmokeState, fixture: _Fixture) -> None:
    context = await state.browser.new_context()
    page = await context.new_page()
    await page.goto(fixture.origin + "/page")
    page_cdp = await new_cdp_session(context, page)
    browser_cdp = await state.browser.new_browser_cdp_session()
    targets = _Targets(page_cdp, browser_cdp)
    try:
        await page.evaluate("""async()=>{
          await navigator.serviceWorker.register('/sw.js');
          await navigator.serviceWorker.ready;
          if(!navigator.serviceWorker.controller)await new Promise(resolve=>navigator.serviceWorker.addEventListener('controllerchange',resolve,{once:true}));
        }""")
        await targets.start()
        child = await targets.worker(
            page, fixture.origin + "/worker.js?controlled", False
        )
        for path, body, network_hits in (
            ("/sw-controlled", "controlled", 0),
            ("/sw-network-start", "HTTP final", 1),
            ("/sw-synthetic-start", "SW final", 1),
        ):
            await page.evaluate(
                'url=>{nativeMessages=[];nativePort.postMessage({kind:"fetch",url});}',
                fixture.origin + path,
            )
            await _message(page, "body", body)
            assert_equal(
                fixture.hits["/sw-final"], network_hits, "network vs SW redirect chain"
            )
            state.record("native_worker_service_worker_" + path[1:])
        await targets.event(
            "Network.loadingFinished",
            requestId=await targets.request(
                fixture.origin + "/sw-network-start", child
            ),
        )
    finally:
        await targets.cleanup()
        await browser_cdp.detach()
        await context.close()


async def run_native_worker_loading_group(state: SmokeState) -> None:
    with _Fixture() as fixture:
        for shared in (False, True):
            context = await state.browser.new_context()
            page = await context.new_page()
            await page.goto(fixture.origin + "/page")
            page_cdp = await new_cdp_session(context, page)
            browser_cdp = await state.browser.new_browser_cdp_session()
            targets = _Targets(page_cdp, browser_cdp)
            try:
                # Chromium 156's SharedWorker FetchHandler explicitly leaves
                # updating renderer loader factories unimplemented. Compare its
                # native Network/JS behavior without claiming a Fetch pause.
                # SharedWorker target interception has separate protocol tests.
                await targets.start(intercept=not shared)
                child = await targets.worker(
                    page, fixture.origin + f"/worker.js?native-probe-{shared}", shared
                )
                control = child if shared else page_cdp
                base = fixture.origin + f"/native-probe-{shared}"
                fetch_path = f"/native-probe-{shared}/overlap-fetch"
                xhr_path = f"/native-probe-{shared}/overlap-xhr"
                for path in (fetch_path, xhr_path):
                    fixture.hold(path)
                await page.evaluate(
                    '(base)=>nativePort.postMessage({kind:"overlap",base})', base
                )
                if shared:
                    fetch_pause = {
                        "networkId": await targets.request(
                            base + "/overlap-fetch", child
                        )
                    }
                    xhr_pause = {
                        "networkId": await targets.request(base + "/overlap-xhr", child)
                    }
                else:
                    fetch_pause = await targets.pause(
                        base + "/overlap-fetch", child, False
                    )
                    xhr_pause = await targets.pause(base + "/overlap-xhr", child, False)
                assert fetch_pause["networkId"] != xhr_pause["networkId"]
                for pause in () if shared else (fetch_pause, xhr_pause):
                    await control.send(
                        "Fetch.continueRequest", {"requestId": pause["requestId"]}
                    )
                await wait_until(
                    lambda fetch_path=fetch_path, xhr_path=xhr_path: (
                        fixture.hits[fetch_path] == 1 and fixture.hits[xhr_path] == 1
                    ),
                    "both native transports entered",
                )
                await page.evaluate('nativePort.postMessage({kind:"abort-xhr"})')
                await _message(page, "xhr", "aborted")
                failed = await targets.event(
                    "Network.loadingFailed", requestId=xhr_pause["networkId"]
                )
                assert_equal(
                    failed["sessionId"], child.session, "canceled XHR observer"
                )
                assert_equal(
                    failed["params"]["errorText"],
                    "net::ERR_ABORTED",
                    "canceled XHR error",
                )
                fixture.release(fetch_path)
                await _message(page, "fetch", "native-worker-body")
                await targets.event(
                    "Network.loadingFinished", requestId=fetch_pause["networkId"]
                )
                state.record(
                    "native_worker_overlap_" + str(shared),
                    {"events": targets.events.copy()},
                )

                stream_path = f"/native-probe-{shared}/stream"
                fixture.hold(stream_path)
                await page.evaluate(
                    '(url)=>nativePort.postMessage({kind:"stream",url})',
                    fixture.origin + stream_path,
                )
                if shared:
                    pause = {
                        "networkId": await targets.request(
                            fixture.origin + stream_path, child
                        )
                    }
                else:
                    pause = await targets.pause(
                        fixture.origin + stream_path, child, False
                    )
                    await control.send(
                        "Fetch.continueRequest", {"requestId": pause["requestId"]}
                    )
                await _message(page, "phase", "headers")
                await page.evaluate('nativePort.postMessage({kind:"abort-body"})')
                body = await _message(page, "phase", "body")
                assert_equal(
                    body,
                    {
                        "phase": "body",
                        "sameReason": True,
                        "reason": "native-body-abort",
                    },
                    "body abort preserves signal reason",
                )
                await targets.event(
                    "Network.loadingFailed", requestId=pause["networkId"]
                )
                fixture.release(stream_path)
                state.record("native_worker_body_abort_" + str(shared))

                if not shared:
                    await _check_redirect_responses(
                        state, fixture, page, targets, child
                    )
                    await _check_continued_real_redirects(
                        state, fixture, page, targets, child
                    )
                    await _check_auth_extra_info(state, fixture, page, targets, child)
                    await _check_large_worker_reader(
                        state, fixture, page, targets, child
                    )
                    await _check_empty_http_redirects(
                        state, fixture, page, targets, child
                    )
                    # Chromium exposes Worker on Window/DedicatedWorker only.
                    await _check_nested_worker(
                        state, fixture, page, targets, child, shared
                    )

                if not shared:
                    await page_cdp.send("Fetch.disable")
                    await _check_network_extra_info(
                        state, fixture, page, targets, child
                    )
                    await _check_cache_revalidation(
                        state, fixture, page, targets, child
                    )
                    await page_cdp.send(
                        "Fetch.enable",
                        {
                            "patterns": [
                                {"urlPattern": "*native-probe*", "resourceType": kind}
                                for kind in ("Fetch", "XHR")
                            ]
                        },
                    )
                # The second client retains the SharedWorker's target and request.
                if shared:
                    peer = await context.new_page()
                    await peer.goto(fixture.origin + "/page")
                    await peer.evaluate(
                        """url=>{nativeMessages=[];nativeWorker=new SharedWorker(url);nativePort=nativeWorker.port;nativeReady=new Promise(resolve=>nativePort.onmessage=e=>{nativeMessages.push(e.data);if(e.data.ready)resolve(true);});nativePort.start();}""",
                        fixture.origin + f"/worker.js?native-probe-{shared}",
                    )
                    await peer.evaluate("nativeReady")
                    survivor = base + "/surviving-client"
                    fixture.hold(f"/native-probe-{shared}/surviving-client")
                    await peer.evaluate(
                        'url=>{nativeMessages=[];nativePort.postMessage({kind:"fetch",url});}',
                        survivor,
                    )
                    pause = {"networkId": await targets.request(survivor, child)}
                    await wait_until(
                        lambda shared=shared: (
                            fixture.hits[f"/native-probe-{shared}/surviving-client"]
                            == 1
                        ),
                        "SharedWorker transport before client closes",
                    )
                    await page.close()
                    fixture.release(f"/native-probe-{shared}/surviving-client")
                    await wait_until(
                        lambda peer=peer: peer.evaluate("nativeMessages.length>0"),
                        "surviving SharedWorker client",
                    )
                    assert_equal(
                        (await peer.evaluate("nativeMessages[0]"))["body"],
                        "native-worker-body",
                        "SharedWorker survives initiating client",
                    )
                    await targets.event(
                        "Network.loadingFinished", requestId=pause["networkId"]
                    )
                    state.record("native_shared_worker_surviving_client")
                if not shared:
                    await page.evaluate("nativeWorker.terminate()")
                    for decision in ("fulfill", "continue"):
                        terminating = await targets.worker(
                            page,
                            fixture.origin + "/worker.js?terminate-" + decision,
                            False,
                        )
                        url = base + "/terminate-" + decision
                        fixture.hold(f"/native-probe-{shared}/terminate-{decision}")
                        await page.evaluate(
                            'url=>nativePort.postMessage({kind:"fetch",url})', url
                        )
                        paused = await targets.pause(url, terminating, False)
                        params = {"requestId": paused["requestId"]}
                        if decision == "fulfill":
                            params.update(
                                responseCode=200,
                                body=base64.b64encode(b"done").decode(),
                            )
                        await page_cdp.send(
                            "Fetch.fulfillRequest"
                            if decision == "fulfill"
                            else "Fetch.continueRequest",
                            params,
                        )
                        await page.evaluate("nativeWorker.terminate()")
                        await targets.event(
                            "Target.detachedFromTarget", sessionId=terminating.session
                        )
                        terminals = [
                            e
                            for e in targets.events
                            if e["method"]
                            in ("Network.loadingFinished", "Network.loadingFailed")
                            and e["params"]["requestId"] == paused["networkId"]
                        ]
                        if len(terminals) > 1:
                            raise SmokeError(
                                f"duplicate terminal during terminate: {terminals}"
                            )
                        fixture.release(f"/native-probe-{shared}/terminate-{decision}")
                        state.record(
                            "native_worker_terminate_" + decision,
                            {"terminals": terminals},
                        )
                for request in [
                    fetch_pause["networkId"],
                    xhr_pause["networkId"],
                    pause["networkId"],
                ]:
                    terminal = [
                        e
                        for e in targets.events
                        if e["method"]
                        in ("Network.loadingFinished", "Network.loadingFailed")
                        and e["params"]["requestId"] == request
                    ]
                    assert_equal(len(terminal), 1, "one terminal per native request")
                state.subresource_events.extend(
                    e for e in targets.events if e["method"].startswith("Network.")
                )
            finally:
                # Keep target routing evidence even when a scenario fails.
                state.record(
                    "native_worker_target_trace_" + str(shared),
                    {"events": targets.events.copy()},
                )
                if not page.is_closed():
                    await targets.cleanup()
                await browser_cdp.detach()
                await context.close()

        from .workers_nested import run_nested_worker_protocol_checks

        await run_nested_worker_protocol_checks(state, fixture)
        await _check_service_worker_redirects(state, fixture)
