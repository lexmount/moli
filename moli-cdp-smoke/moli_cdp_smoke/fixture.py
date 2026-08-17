from __future__ import annotations

import base64
import hashlib
import html
import json
import os
import socket
import socketserver
import struct
import sys
import threading
import time
import urllib.parse
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

from .config import REPO_ROOT


def websocket_accept_key(key: str) -> str:
    digest = hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
    return base64.b64encode(digest).decode("ascii")


def websocket_frame(opcode: int, payload: bytes) -> bytes:
    if len(payload) < 126:
        return bytes([0x80 | opcode, len(payload)]) + payload
    if len(payload) <= 0xFFFF:
        return bytes([0x80 | opcode, 126]) + len(payload).to_bytes(2, "big") + payload
    raise RuntimeError(f"fixture websocket frame too large: {len(payload)}")


def websocket_text_frame(text: str) -> bytes:
    return websocket_frame(0x1, text.encode("utf-8"))


def websocket_close_frame(code: int = 1000, reason: str = "") -> bytes:
    return websocket_frame(0x8, code.to_bytes(2, "big") + reason.encode("utf-8"))


def recv_exact(sock: socket.socket, length: int) -> bytes:
    chunks: list[bytes] = []
    remaining = length
    while remaining > 0:
        chunk = sock.recv(remaining)
        if not chunk:
            raise ConnectionError("websocket peer closed while reading frame")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def read_websocket_frame(sock: socket.socket) -> tuple[int, bytes]:
    first, second = recv_exact(sock, 2)
    opcode = first & 0x0F
    masked = (second & 0x80) != 0
    payload_length = second & 0x7F
    if payload_length == 126:
        payload_length = int.from_bytes(recv_exact(sock, 2), "big")
    elif payload_length == 127:
        raise RuntimeError("fixture websocket does not support 64-bit frames")
    mask = recv_exact(sock, 4) if masked else b""
    payload = bytearray(recv_exact(sock, payload_length))
    if masked:
        for index in range(len(payload)):
            payload[index] ^= mask[index % 4]
    return opcode, bytes(payload)


WORKER_SCRIPT = r"""
self.onmessage = async event => {
  const data = event.data;
  try {
    if (data && data.kind === 'fetch') {
      const response = await fetch(data.url);
      self.postMessage({ ok: response.ok, status: response.status, text: await response.text() });
      return;
    }
    if (data && data.kind === 'xhr') {
      const result = await new Promise(resolve => {
        const xhr = new XMLHttpRequest();
        xhr.open('GET', data.url, true);
        xhr.onload = () => resolve({ ok: true, status: xhr.status, text: xhr.responseText });
        xhr.onerror = () => resolve({ ok: false, status: xhr.status, error: 'xhr error' });
        xhr.send();
      });
      self.postMessage(result);
      return;
    }
    self.postMessage({
      echoed: data,
      pathname: self.location.pathname,
      selfEqualsGlobal: self === globalThis,
    });
  } catch (error) {
    self.postMessage({
      ok: false,
      error: `${error && error.constructor && error.constructor.name || 'Error'}:${error && error.message || String(error)}`,
    });
  }
};
"""

SHARED_WORKER_SCRIPT = r"""
globalThis.__sharedWorkerSmokeBoot = {
  name,
  pathname: self.location.pathname,
  isSharedWorker: typeof SharedWorkerGlobalScope !== 'undefined' && self instanceof SharedWorkerGlobalScope,
};
globalThis.__sharedWorkerSmokeConnectionCount = 0;
console.log('shared-worker-smoke-ready');
self.onconnect = event => {
  const port = event.ports[0];
  const connectionId = ++globalThis.__sharedWorkerSmokeConnectionCount;
  port.onmessage = event => {
    const data = event.data;
    if (data && data.kind === 'ready') {
      port.postMessage({
        ready: true,
        name,
        pathname: self.location.pathname,
        selfEqualsGlobal: self === globalThis,
        isSharedWorker: globalThis.__sharedWorkerSmokeBoot.isSharedWorker,
        connectionId,
        connectionCount: globalThis.__sharedWorkerSmokeConnectionCount,
      });
      return;
    }
    if (data && data.kind === 'probe') {
      port.postMessage({
        kind: 'probe-result',
        echoed: data.value,
        name,
        pathname: self.location.pathname,
        selfEqualsGlobal: self === globalThis,
        isSharedWorker: globalThis.__sharedWorkerSmokeBoot.isSharedWorker,
        connectionId,
        connectionCount: globalThis.__sharedWorkerSmokeConnectionCount,
      });
      return;
    }
    if (data && data.kind === 'cpu-trace') {
      function moliCpuTraceSharedWorkerHotFunction() {
        const deadline = performance.now() + 120;
        let value = 1;
        while (performance.now() < deadline) {
          value = Math.imul(value + 3, 1103515245) | 0;
        }
        return value;
      }
      port.postMessage({
        kind: 'cpu-trace-result',
        value: moliCpuTraceSharedWorkerHotFunction(),
      });
      return;
    }
    port.postMessage({ echoed: data, connectionId, connectionCount: globalThis.__sharedWorkerSmokeConnectionCount });
  };
  port.start();
};
"""

TRANSPARENT_PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII="
)
RESOURCE_MEDIA_BYTES = b"\x00\xffmoli-media"
RESOURCE_XHR_BYTES = b"\x00\xffmoli-xhr"

LAYOUT_SCREENSHOT_FIXTURE = (
    REPO_ROOT
    / "moli-renderer-v8"
    / "tests"
    / "fixtures"
    / "layout-screenshot-poc.html"
).read_text(encoding="utf-8")

ACTION_WINDOW_DEADLINE_FIXTURE = """<!doctype html>
<style>
html, body { margin: 0; }
body { height: 1200px; }
#target { position: absolute; top: 250px; width: 20px; height: 20px; }
</style>
<div id="target"></div>
<script>
globalThis.__actionWindowWheelLog = [];
globalThis.__actionWindowIoLog = [];
addEventListener("wheel", event => {
  __actionWindowWheelLog.push("event:" + event.deltaY);
  Promise.resolve().then(() => {
    __actionWindowWheelLog.push("microtask:" + event.deltaY);
  });
}, { capture: true });
globalThis.__actionWindowObserver = new IntersectionObserver(entries => {
  const entry = entries.find(candidate => candidate.target.id === "target");
  if (!entry) return;
  __actionWindowIoLog.push(entry.isIntersecting);
  if (__actionWindowIoLog.length === 2 && entry.isIntersecting) {
    fetch("/action-window-witness/entered?source=deadline");
  }
});
__actionWindowObserver.observe(document.getElementById("target"));
</script>
"""

ACTION_WINDOW_OVERFLOW_FIXTURE = """<!doctype html>
<style>
html, body { margin: 0; }
body { width: 1000px; height: 1000px; }
#scroller {
  position: absolute;
  left: 20px;
  top: 20px;
  width: 100px;
  height: 100px;
  overflow: auto;
}
#content { width: 500px; height: 500px; }
</style>
<div id="scroller"><div id="content"></div></div>
<script>
globalThis.__actionWindowOverflowDeltas = [];
const scroller = document.getElementById("scroller");
scroller.addEventListener("wheel", event => {
  __actionWindowOverflowDeltas.push([event.deltaX, event.deltaY]);
  fetch(
    "/action-window-witness/entered?source=overflow" +
    "&deltaX=" + event.deltaX + "&deltaY=" + event.deltaY
  );
});
</script>
"""

ACTION_WINDOW_CAPTURE_FIXTURE = """<!doctype html>
<style>
html, body { margin: 0; background: white; }
body { height: 1200px; }
#witness { position: fixed; inset: 0; background: white; }
</style>
<div id="witness"></div>
<script>
globalThis.__actionWindowCaptureDeltas = [];
addEventListener("wheel", event => {
  __actionWindowCaptureDeltas.push(event.deltaY);
  document.getElementById("witness").style.background =
    __actionWindowCaptureDeltas.length === 1 ? "rgb(255, 0, 0)" : "rgb(0, 255, 0)";
  fetch("/action-window-witness/entered?source=capture&delta=" + event.deltaY);
}, { capture: true });
</script>
"""

ACTION_WINDOW_REPLACEMENT_FIXTURE = """<!doctype html>
<style>html, body { margin: 0; } body { height: 1200px; }</style>
<script>
globalThis.__actionWindowRetiredDeltas = [];
globalThis.__actionWindowReplacementDeltas = [];
document.addEventListener("wheel", event => {
  __actionWindowRetiredDeltas.push(event.deltaY);
  document.open();
  document.write("<!doctype html><body style='height:1200px'>replacement</body>");
  document.close();
  document.addEventListener("wheel", replacementEvent => {
    __actionWindowReplacementDeltas.push(replacementEvent.deltaY);
  }, { capture: true });
}, { capture: true, once: true });
</script>
"""

LDM0_TOP_DOM_WHITESPACE_FIXTURE = """<!doctype html>
<html>
    <head>
        <title>ldm0.top DOM whitespace fixture</title>
    </head>
    <body>
        <input id="ua-search-control" type="search" value="needle">
        <div id="whitespace-mutation">   </div>
        <iframe id="whitespace-frame" srcdoc="<!doctype html><html><body>
  <main id='inside-whitespace-frame'>child</main>
</body></html>"></iframe>
        <!-- DOM shape adapted from https://ldm0.top/, fetched 2026-08-03. -->
        <div id="widget_plate">
            <div class="widget" id="widget_home_page"></div>
            <div class="widget" id="widget_blog_cluster"></div>
            <div class="widget" id="widget_fun"></div>
        </div>
        <div class="blog">
            <!--Blog chunks usually sorted with timeline.-->
            <!--Blog chunks below should be generated by the generator.-->

            <div class="blog_chunk" data-entry="wtf-8">
                <div class="blog_chunk_left">
                    <div class="blog_chunk_title">WTF-8</div>
                    <div class="blog_chunk_preview"></div>
                </div>
                <div class="blog_chunk_right"></div>
            </div>

            <div class="blog_chunk" data-entry="recent">
                <div class="blog_chunk_left"></div>
                <div class="blog_chunk_right"></div>
            </div>

            <div class="blog_chunk" data-entry="interview">
                <div class="blog_chunk_left"></div>
                <div class="blog_chunk_right"></div>
            </div>

            <div class="blog_chunk" data-entry="gsoc-2020">
                <div class="blog_chunk_left"></div>
                <div class="blog_chunk_right"></div>
            </div>

            <div class="blog_chunk" data-entry="ffmpeg-rust">
                <div class="blog_chunk_left"></div>
                <div class="blog_chunk_right"></div>
            </div>

            <div class="blog_chunk" data-entry="web-ml">
                <div class="blog_chunk_left"></div>
                <div class="blog_chunk_right"></div>
            </div>

            <div class="blog_chunk" data-entry="virtual-constructor">
                <div class="blog_chunk_left"></div>
                <div class="blog_chunk_right"></div>
            </div>

            <div class="blog_chunk" data-entry="bloom">
                <div class="blog_chunk_left"></div>
                <div class="blog_chunk_right"></div>
            </div>
        </div>
        <div id="tail"></div>
    </body>
</html>
"""

DOM_SHADOW_OUTER_HTML_FIXTURE = """<!doctype html>
<html><body>
<x-host id="host">light</x-host>
<x-declarative id="declarative"><template shadowrootmode="open"><i>declarative</i></template>declarative-light</x-declarative>
<input id="control">
<iframe id="shadow-child" src="/dom-shadow-outer-html-child"></iframe>
<script>
const host = document.getElementById('host');
const root = host.attachShadow({
  mode: 'closed',
  delegatesFocus: true,
  serializable: true,
  slotAssignment: 'manual',
  clonable: true,
});
root.innerHTML = '<span data-x="&amp;">shadow &lt;</span><x-inner>inner-light</x-inner>';
root.querySelector('x-inner').attachShadow({ mode: 'open' }).innerHTML = '<b>nested</b>';
const detached = document.createElement('x-detached');
detached.textContent = 'detached-light';
detached.attachShadow({ mode: 'open' }).innerHTML = '<em>detached-shadow</em>';
globalThis.__outerHtmlDetached = detached;
</script>
</body></html>
"""

DOM_SHADOW_OUTER_HTML_CHILD_FIXTURE = """<!doctype html>
<html><body>
<x-child id="child-host">child-light</x-child>
<script>
document.getElementById('child-host').attachShadow({ mode: 'closed' }).innerHTML =
  '<span>child-shadow</span>';
</script>
</body></html>
"""

DOM_HIT_TEST_FIXTURE = """<!doctype html>
<html><head><style>html, body { margin: 0; }</style></head><body>
<div id="hit-target" style="position:absolute;left:0;top:0;width:100px;height:100px"></div>
<div id="hit-overlay" style="position:absolute;left:0;top:0;width:200px;height:200px;pointer-events:none"></div>
<x-hit id="author-host" style="position:absolute;left:220px;top:0;width:100px;height:100px"></x-hit>
<input id="ua-hit" type="search" value="needle" style="position:absolute;left:0;top:220px;width:200px;height:40px">
<iframe id="hit-frame" style="position:absolute;left:220px;top:220px;width:200px;height:150px;border:0"
  srcdoc="<!doctype html><style>html,body{margin:0}</style><div id='frame-hit' style='position:absolute;left:5px;top:5px;width:100px;height:100px'></div>"></iframe>
<script>
document.getElementById('author-host').attachShadow({mode: 'open'}).innerHTML =
  '<button id="author-hit" style="position:fixed;left:220px;top:0;width:100px;height:100px">inside</button>';
</script>
</body></html>
"""

PARSER_DOM_MUTATION_FIXTURE = """<!doctype html>
<html><head><script src="/parser-dom-mutation-held.js"></script></head>
<body id="late-body"><main>ready</main></body></html>
"""


class FixtureResponseGate:
    def __init__(self) -> None:
        self.request_seen = threading.Event()
        self.release_response = threading.Event()
        self.response_completed = threading.Event()

    def reset(self) -> None:
        self.request_seen.clear()
        self.release_response.clear()
        self.response_completed.clear()


class ProxyAuthFixtureServer:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._requests: list[str] = []
        handler = self._handler_class()
        self.tcpd = socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler)
        self.tcpd.daemon_threads = True
        self.thread = threading.Thread(
            target=self.tcpd.serve_forever,
            name="moli-cdp-smoke-proxy",
            daemon=True,
        )

    @property
    def url(self) -> str:
        host, port = self.tcpd.server_address
        return f"http://{host}:{port}"

    @property
    def requests(self) -> list[str]:
        with self._lock:
            return list(self._requests)

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.tcpd.shutdown()
        self.tcpd.server_close()
        self.thread.join(timeout=5)

    def _record_request(self, request: str) -> None:
        with self._lock:
            self._requests.append(request)

    def _handler_class(self) -> type[socketserver.StreamRequestHandler]:
        outer = self

        class Handler(socketserver.StreamRequestHandler):
            def handle(self) -> None:
                lines: list[bytes] = []
                while True:
                    line = self.rfile.readline(65_537)
                    if not line or len(line) > 65_536:
                        return
                    lines.append(line)
                    if line in (b"\r\n", b"\n"):
                        break

                request = b"".join(lines).decode("latin-1")
                outer._record_request(request)
                request_line = request.splitlines()[0] if request else ""
                authorized = any(
                    line.lower().startswith("proxy-authorization: basic ")
                    for line in request.splitlines()[1:]
                )
                if request_line.startswith("GET http://") and authorized:
                    body = b"<!doctype html><title>proxy ok</title><main>proxy ok</main>"
                    response = (
                        b"HTTP/1.1 200 OK\r\n"
                        b"Content-Type: text/html; charset=utf-8\r\n"
                        + f"Content-Length: {len(body)}\r\n".encode("ascii")
                        + b"Connection: close\r\n\r\n"
                        + body
                    )
                else:
                    body = b"proxy auth required"
                    response = (
                        b"HTTP/1.1 407 Proxy Authentication Required\r\n"
                        b"Proxy-Authenticate: Basic realm=\"smoke-proxy\"\r\n"
                        b"Content-Type: text/plain; charset=utf-8\r\n"
                        + f"Content-Length: {len(body)}\r\n".encode("ascii")
                        + b"Connection: close\r\n\r\n"
                        + body
                    )
                self.wfile.write(response)
                self.wfile.flush()

        return Handler


class FixtureServer:
    def __init__(self) -> None:
        self.profile_requests: dict[str, dict[str, Any]] = {}
        self._counter_lock = threading.Lock()
        self._request_counts: dict[str, int] = {}
        self.document_content_stylesheet_gate = FixtureResponseGate()
        self.fetch_runtime_teardown_gate = FixtureResponseGate()
        self.navigation_suspension_gate = FixtureResponseGate()
        self.parser_dom_mutation_script_gate = FixtureResponseGate()
        handler = self._handler_class()
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        self.httpd.daemon_threads = True
        self.thread = threading.Thread(target=self.httpd.serve_forever, name="moli-cdp-smoke-fixture", daemon=True)

    @property
    def url(self) -> str:
        host, port = self.httpd.server_address
        return f"http://{host}:{port}"

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.document_content_stylesheet_gate.release_response.set()
        self.fetch_runtime_teardown_gate.release_response.set()
        self.navigation_suspension_gate.release_response.set()
        self.parser_dom_mutation_script_gate.release_response.set()
        self.httpd.shutdown()
        self.httpd.server_close()
        self.thread.join(timeout=5)

    def reset_request_count(self, route: str) -> None:
        with self._counter_lock:
            self._request_counts[route] = 0

    def request_count(self, route: str) -> int:
        with self._counter_lock:
            return self._request_counts.get(route, 0)

    def _increment_request_count(self, route: str) -> int:
        with self._counter_lock:
            count = self._request_counts.get(route, 0) + 1
            self._request_counts[route] = count
            return count

    def _handler_class(self) -> type[BaseHTTPRequestHandler]:
        outer = self

        class Handler(BaseHTTPRequestHandler):
            server_version = "MoliCdpSmokeFixture/0.1"

            def log_message(self, fmt: str, *args: Any) -> None:
                if os.environ.get("MOLI_SMOKE_TRACE_BG") == "1":
                    print(f"[fixture] {fmt % args}", file=sys.stderr, flush=True)

            def do_GET(self) -> None:
                self._dispatch()

            def do_POST(self) -> None:
                self._dispatch()

            def do_HEAD(self) -> None:
                self._dispatch()

            def _dispatch(self) -> None:
                parsed = urllib.parse.urlparse(self.path)
                route = parsed.path
                query = urllib.parse.parse_qs(parsed.query)
                _body = self._read_body()

                if route == "/ws-echo" and self.headers.get("Upgrade", "").lower() == "websocket":
                    self._handle_websocket()
                    return
                if route == "/ws-chatgpt-live" and self.headers.get("Upgrade", "").lower() == "websocket":
                    self._handle_chatgpt_live_websocket()
                    return
                if route == "/favicon.ico":
                    self.send_response(HTTPStatus.NO_CONTENT)
                    self.end_headers()
                    return
                if route == "/inspector-routing-witness/reset":
                    outer.reset_request_count("/inspector-routing-witness/entered")
                    self._send_json({"enteredCount": 0})
                    return
                if route == "/inspector-routing-witness/status":
                    self._send_json(
                        {
                            "enteredCount": outer.request_count(
                                "/inspector-routing-witness/entered"
                            )
                        }
                    )
                    return
                if route == "/inspector-routing-witness/entered":
                    count = outer._increment_request_count(route)
                    self._send_text(f"entered-{count}")
                    return
                if route == "/action-window-witness/reset":
                    outer.reset_request_count("/action-window-witness/entered")
                    self._send_json({"count": 0})
                    return
                if route == "/action-window-witness/status":
                    self._send_json(
                        {
                            "count": outer.request_count(
                                "/action-window-witness/entered"
                            )
                        }
                    )
                    return
                if route == "/action-window-witness/entered":
                    count = outer._increment_request_count(route)
                    self._send_json({"count": count})
                    return
                if route == "/chromium-network-redirect-before-reset":
                    outer._increment_request_count(route)
                    self.send_response(HTTPStatus.FOUND)
                    self.send_header("Location", "/chromium-network-reset-before-response")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if route == "/chromium-network-reset-before-response":
                    outer._increment_request_count(route)
                    self.connection.setsockopt(
                        socket.SOL_SOCKET,
                        socket.SO_LINGER,
                        struct.pack("ii", 1, 0),
                    )
                    self.close_connection = True
                    self.connection.close()
                    return
                if route == "/xhr-sync-redirect-foobar":
                    self.send_response(HTTPStatus.MOVED_PERMANENTLY)
                    self.send_header("Location", "foobar://abcd")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if route == "/xhr-sync-redirect-mailto":
                    self.send_response(HTTPStatus.FOUND)
                    self.send_header("Location", "mailto:someone@example.org")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if route == "/xhr-sync-redirect-tel":
                    self.send_response(HTTPStatus.SEE_OTHER)
                    self.send_header("Location", "tel:1234567890")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if route == "/xhr-sync-redirect-nonexistent-302":
                    self.send_response(HTTPStatus.FOUND)
                    self.send_header("Location", "https://doesnotexist.localhost/")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if route == "/xhr-sync-redirect-nonexistent-303":
                    self.send_response(HTTPStatus.SEE_OTHER)
                    self.send_header("Location", "https://doesnotexist.localhost/")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if route == "/xhr-sync-redirect-loop":
                    self.send_response(HTTPStatus.MOVED_PERMANENTLY)
                    self.send_header("Location", "/xhr-sync-redirect-loop")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if route == "/plain":
                    self._send_html("<!doctype html><main>plain ok</main>")
                elif route == "/layout-screenshot-poc":
                    self._send_html(LAYOUT_SCREENSHOT_FIXTURE)
                elif route == "/action-window-deadline":
                    self._send_html(ACTION_WINDOW_DEADLINE_FIXTURE)
                elif route == "/action-window-overflow":
                    self._send_html(ACTION_WINDOW_OVERFLOW_FIXTURE)
                elif route == "/action-window-capture":
                    self._send_html(ACTION_WINDOW_CAPTURE_FIXTURE)
                elif route == "/action-window-replacement":
                    self._send_html(ACTION_WINDOW_REPLACEMENT_FIXTURE)
                elif route == "/agent-episode-smoke":
                    self._send_html(
                        "<!doctype html><style>input,button{display:block;width:220px;height:32px}</style>"
                        "<main>agent episode source</main>"
                        "<input id='query' placeholder='Search'>"
                        "<button id='navigate'>Continue</button>"
                        "<output id='state'>value:;events:</output>"
                        "<script>"
                        "globalThis.__agentOldRealmMarker='source-realm';"
                        "globalThis.__agentEvents=[];"
                        "const input=document.querySelector('#query');"
                        "const output=document.querySelector('#state');"
                        "const render=()=>output.textContent=`value:${input.value};events:${__agentEvents.join(',')}`;"
                        "input.addEventListener('input',()=>{__agentEvents.push('input');queueMicrotask(render)});"
                        "input.addEventListener('change',()=>{__agentEvents.push('change');render()});"
                        "document.querySelector('#navigate').addEventListener('click',()=>{"
                        "history.pushState(null,'',location.pathname+'#queued');"
                        "queueMicrotask(()=>{location.href='/agent-episode-smoke-result?value='+encodeURIComponent(input.value)});"
                        "});"
                        "</script>"
                    )
                elif route == "/agent-episode-smoke-result":
                    value = html.escape(query.get("value", [""])[0])
                    self._send_html(
                        "<!doctype html><style>button{display:block;width:220px;height:32px}</style>"
                        f"<main>agent episode result {value}</main>"
                        "<button id='done'>Done</button>"
                        "<script>globalThis.__agentNewRealmMarker='result-realm';</script>"
                    )
                elif route == "/semantic-dom":
                    self._send_html(
                        "<!doctype html><main id='search-target'>semantic search needle</main>"
                        "<input id='focus-target' value='focus me'>"
                    )
                elif route == "/semantic-autofill-card":
                    self._send_html(
                        "<!doctype html><form id='card-form'>"
                        "<label for='CREDIT_CARD_NUMBER'>Card Number</label>"
                        "<input id='CREDIT_CARD_NUMBER' name='card_number'>"
                        "<label for='CREDIT_CARD_NAME_FULL'>Name on Card</label>"
                        "<input id='CREDIT_CARD_NAME_FULL'>"
                        "<label for='CREDIT_CARD_EXP_MONTH'>Expiry Month</label>"
                        "<input id='CREDIT_CARD_EXP_MONTH' name='ccmonth'>"
                        "<label for='CREDIT_CARD_EXP_4_DIGIT_YEAR'>Expiry Year</label>"
                        "<input id='CREDIT_CARD_EXP_4_DIGIT_YEAR' name='ccyear'>"
                        "<label for='CREDIT_CARD_VERIFICATION_CODE'>CVC</label>"
                        "<input id='CREDIT_CARD_VERIFICATION_CODE' autocomplete='cc-csc'>"
                        "</form><input id='ordinary-field'>"
                        "<script>"
                        "globalThis.__autofillEvents = [];"
                        "for (const control of document.querySelectorAll('#card-form input')) {"
                        " for (const type of ['beforeinput', 'input', 'change']) {"
                        "  control.addEventListener(type, event => __autofillEvents.push({"
                        "   type, id: control.id, trusted: event.isTrusted,"
                        "   bubbles: event.bubbles, composed: event.composed"
                        "  }));"
                        " }"
                        "}"
                        "</script>"
                    )
                elif route == "/semantic-frames":
                    self._send_html(
                        "<!doctype html><main>semantic frames</main>"
                        "<iframe id='first' name='first-frame' src='/semantic-frame-child?child=first&nested=1'></iframe>"
                        "<iframe id='second' name='second-frame' src='/semantic-frame-child?child=second'></iframe>"
                    )
                elif route == "/semantic-frame-child":
                    child = html.escape(query.get("child", [""])[0])
                    nested = (
                        "<iframe id='nested' name='nested-frame' "
                        "src='/semantic-frame-grandchild'></iframe>"
                        if query.get("nested") == ["1"]
                        else ""
                    )
                    self._send_html(f"<!doctype html><main>child {child}</main>{nested}")
                elif route == "/semantic-frame-grandchild":
                    self._send_html("<!doctype html><main>grandchild</main>")
                elif route == "/semantic-shadow-frame":
                    self._send_html(
                        "<!doctype html><div id='host'></div><script>"
                        "const root = document.querySelector('#host').attachShadow({mode:'open'});"
                        "const frame = document.createElement('iframe');"
                        "frame.id = 'shadow-frame'; frame.name = 'shadowNamed';"
                        "frame.src = '/semantic-frame-child?child=shadow';"
                        "root.appendChild(frame);"
                        "</script>"
                    )
                elif route == "/semantic-cache-page":
                    self._send_html(
                        "<!doctype html><main>cache page</main>"
                        "<script src='/semantic-cache.js'></script>"
                    )
                elif route == "/semantic-cache.js":
                    count = outer._increment_request_count(route)
                    self._send_cacheable_js(
                        f"globalThis.__semanticCacheRequest = {count};",
                        max_age=3600,
                    )
                elif route == "/semantic-resource-page":
                    self._send_cacheable(
                        "text/html; charset=utf-8",
                        (
                            "<!doctype html><head>"
                            "<link rel='stylesheet' href='/semantic-resource.css'>"
                            "<script src='/semantic-resource.js'></script>"
                            "</head><body><main>semantic original document token</main></body>"
                        ).encode(),
                    )
                elif route == "/semantic-resource.css":
                    outer._increment_request_count(route)
                    self._send_cacheable(
                        "text/css; charset=utf-8",
                        b"@import url('/semantic-resource-import.css'); main { color: rgb(1, 2, 3); }",
                    )
                elif route == "/semantic-resource-import.css":
                    outer._increment_request_count(route)
                    self._send_cacheable(
                        "text/css; charset=utf-8",
                        b"main { background-color: rgb(4, 5, 6); }",
                    )
                elif route == "/semantic-resource.js":
                    self._send_cacheable(
                        "application/javascript; charset=utf-8",
                        b"globalThis.__semanticResourceScriptToken = 'script-ready';",
                    )
                elif route == "/semantic-event-source":
                    self._send_event_stream("event: semantic\ndata: event-source-ready\n\n")
                elif route == "/semantic-document.xml":
                    self._send_common(
                        HTTPStatus.OK,
                        "application/xml; charset=utf-8",
                        b"<?xml version='1.0'?><semantic-root><semantic-child>xml-ready</semantic-child></semantic-root>",
                    )
                elif route == "/init-script-tamperable":
                    self._send_html(
                        "<!doctype html><main>init script page</main><script>"
                        "const own = name => Object.prototype.hasOwnProperty.call(globalThis, name) ? globalThis[name] : null;"
                        "globalThis.__initSnapshot = {"
                        "injected: own('__initInjected'),"
                        "contextTemp: own('__contextTemp'),"
                        "pageInjected: own('__pageInjected'),"
                        "scriptOne: own('__scriptOne'),"
                        "scriptTwo: own('__scriptTwo'),"
                        "trailingSecret: own('__initTrailingSecret')"
                        "};"
                        "</script>"
                    )
                elif route == "/iframe":
                    self._send_html('<!doctype html><main>parent</main><iframe src="/child"></iframe>')
                elif route == "/child":
                    self._send_html("<!doctype html><body>child body text<input value='inner'></body>")
                elif route == "/wait-for-function":
                    self._send_html(
                        "<!doctype html><body><script>setTimeout(() => { globalThis.__ready = true; }, 50);</script></body>"
                    )
                elif route == "/wait-states":
                    self._send_html(
                        "<!doctype html><body>"
                        "<main>wait states</main>"
                        "<p id='hide-me'>hide me</p>"
                        "<p id='detach-me'>detach me</p>"
                        "<p id='visible' style='display:none'>visible ready</p>"
                        "<button id='delayed-button' disabled "
                        "onclick=\"document.getElementById('clicked').textContent = 'clicked'\">click</button>"
                        "<p id='clicked'>not clicked</p>"
                        "<script>"
                        "setTimeout(() => {"
                        "  const attached = document.createElement('span');"
                        "  attached.id = 'attached';"
                        "  attached.textContent = 'attached ready';"
                        "  document.body.appendChild(attached);"
                        "}, 50);"
                        "setTimeout(() => { document.getElementById('visible').style.display = 'block'; }, 75);"
                        "setTimeout(() => { document.getElementById('hide-me').style.display = 'none'; }, 100);"
                        "setTimeout(() => { document.getElementById('detach-me')?.remove(); }, 125);"
                        "setTimeout(() => { document.getElementById('delayed-button').disabled = false; }, 150);"
                        "</script>"
                        "</body>"
                    )
                elif route == "/streamed-reveal-page":
                    self._send_html(
                        "<!doctype html><body>"
                        "<main id='conversation'>"
                        "<template id='B:0'></template><span id='fallback'>Thinking</span>"
                        "</main>"
                        "<div hidden id='S:0'><div data-message-author-role='assistant'>OK</div></div>"
                        "<script>"
                        "globalThis.__smokeRevealEvents = [];"
                        "const pairs = [];"
                        "function reveal(queue) {"
                        "  for (const [boundaryId, segmentId] of queue.splice(0)) {"
                        "    const boundary = document.getElementById(boundaryId);"
                        "    const segment = document.getElementById(segmentId);"
                        "    const fallback = document.getElementById('fallback');"
                        "    if (!boundary || !segment) continue;"
                        "    fallback?.remove();"
                        "    boundary.replaceWith(...Array.from(segment.childNodes));"
                        "    globalThis.__smokeRevealEvents.push('revealed');"
                        "  }"
                        "}"
                        "function completeBoundary(boundaryId, segmentId) {"
                        "  pairs.push([boundaryId, segmentId]);"
                        "  requestAnimationFrame(() => reveal(pairs));"
                        "}"
                        "completeBoundary('B:0', 'S:0');"
                        "</script></body>"
                    )
                elif route == "/fetch-stream-client-nav-page":
                    self._send_html(
                        "<!doctype html><body>"
                        "<main id='conversation'><div id='status'>waiting</div></main>"
                        "<script>"
                        "globalThis.__smokeFetchStreamEvents = [];"
                        "function record(event) { globalThis.__smokeFetchStreamEvents.push(event); }"
                        "document.addEventListener('DOMContentLoaded', () => record('dcl'));"
                        "addEventListener('load', () => { record('load'); setTimeout(sendPrompt, 0); });"
                        "async function readTextStream(response) {"
                        "  if (!response.body) throw new Error('missing response body');"
                        "  const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();"
                        "  let text = '';"
                        "  for (;;) {"
                        "    const chunk = await reader.read();"
                        "    if (chunk.done) break;"
                        "    text += chunk.value;"
                        "    record('chunk:' + chunk.value);"
                        "  }"
                        "  return text;"
                        "}"
                        "async function sendPrompt() {"
                        "  try {"
                        "    record('send');"
                        "    const response = await fetch('/conversation-stream', {"
                        "      method: 'POST',"
                        "      headers: { 'content-type': 'text/plain;charset=utf-8' },"
                        "      body: 'OK'"
                        "    });"
                        "    record('response:' + response.status);"
                        "    history.pushState({ smoke: true }, '', '/conversation/fetch-stream');"
                        "    const text = await readTextStream(response);"
                        "    const assistant = document.createElement('div');"
                        "    assistant.setAttribute('data-message-author-role', 'assistant');"
                        "    assistant.textContent = text;"
                        "    document.getElementById('conversation').replaceChildren(assistant);"
                        "    record('rendered:' + text);"
                        "  } catch (error) {"
                        "    const message = `${error && error.name || 'Error'}:${error && error.message || String(error)}`;"
                        "    document.body.dataset.error = message;"
                        "    record('error:' + message);"
                        "  }"
                        "}"
                        "</script></body>"
                    )
                elif route == "/chatgpt-live-channel-page":
                    self._send_html(
                        "<!doctype html><body>"
                        "<main id='conversation'><div id='status'>waiting</div></main>"
                        "<script>"
                        "globalThis.__smokeChatGptLiveEvents = [];"
                        "function record(event) { globalThis.__smokeChatGptLiveEvents.push(event); }"
                        "function render(text) {"
                        "  const assistant = document.createElement('div');"
                        "  assistant.setAttribute('data-message-author-role', 'assistant');"
                        "  assistant.textContent = text;"
                        "  document.getElementById('conversation').replaceChildren(assistant);"
                        "  record('rendered:' + text);"
                        "}"
                        "document.addEventListener('DOMContentLoaded', () => record('dcl'));"
                        "addEventListener('load', () => { record('load'); setTimeout(sendPrompt, 0); });"
                        "async function sendPrompt() {"
                        "  try {"
                        "    record('prepare:start');"
                        "    const prepare = await fetch('/backend-api/f/conversation/prepare', { method: 'POST' });"
                        "    record('prepare:' + prepare.status);"
                        "    const prepared = await prepare.json();"
                        "    const conversationId = prepared.conversation_id || 'smoke-live';"
                        "    const wsUrl = new URL('/ws-chatgpt-live', location.href);"
                        "    wsUrl.protocol = wsUrl.protocol === 'https:' ? 'wss:' : 'ws:';"
                        "    wsUrl.searchParams.set('conversation_id', conversationId);"
                        "    const socket = new WebSocket(wsUrl.href, 'smoke');"
                        "    let text = '';"
                        "    socket.onopen = () => {"
                        "      record('ws:open');"
                        "      socket.send(JSON.stringify({ type: 'subscribe', conversation_id: conversationId }));"
                        "    };"
                        "    socket.onmessage = event => {"
                        "      const message = JSON.parse(event.data);"
                        "      record('ws:' + message.type + (message.text ? ':' + message.text : ''));"
                        "      if (message.type === 'delta') text += message.text || '';"
                        "      if (message.type === 'done') render(text);"
                        "    };"
                        "    record('conversation:start');"
                        "    const response = await fetch('/backend-api/f/conversation', {"
                        "      method: 'POST',"
                        "      headers: { 'content-type': 'application/json' },"
                        "      body: JSON.stringify({ conversation_id: conversationId, prompt: 'OK' })"
                        "    });"
                        "    record('conversation:' + response.status);"
                        "    history.pushState({ conversationId }, '', '/c/' + conversationId);"
                        "  } catch (error) {"
                        "    const message = `${error && error.name || 'Error'}:${error && error.message || String(error)}`;"
                        "    document.body.dataset.error = message;"
                        "    record('error:' + message);"
                        "  }"
                        "}"
                        "</script></body>"
                    )
                elif route == "/chatgpt-client-id-map-page":
                    self._send_html(
                        "<!doctype html><body>"
                        "<main id='conversation'><div id='status'>waiting</div></main>"
                        "<script>"
                        "globalThis.__smokeChatGptClientMapEvents = [];"
                        "function record(event) { globalThis.__smokeChatGptClientMapEvents.push(event); }"
                        "const clientId = 'client-new-thread';"
                        "const store = {"
                        "  threads: {[clientId]: {id: clientId, turns: []}},"
                        "  clientNewThreadIdToServerIdMapping: {},"
                        "  listeners: []"
                        "};"
                        "function resolveThreadId(id) {"
                        "  return store.clientNewThreadIdToServerIdMapping[id] || id;"
                        "}"
                        "function selectThread(id) {"
                        "  const resolved = resolveThreadId(id);"
                        "  record('selector:' + resolved);"
                        "  return store.threads[resolved];"
                        "}"
                        "function subscribe(listener) { store.listeners.push(listener); }"
                        "function setState(mutator) {"
                        "  mutator(store);"
                        "  for (const listener of store.listeners.slice()) listener();"
                        "}"
                        "function render() {"
                        "  const thread = selectThread(clientId);"
                        "  const turn = thread && thread.turns && thread.turns[0];"
                        "  if (!turn) {"
                        "    document.getElementById('conversation').textContent = 'empty';"
                        "    record('rendered:empty');"
                        "    return;"
                        "  }"
                        "  const assistant = document.createElement('div');"
                        "  assistant.setAttribute('data-message-author-role', 'assistant');"
                        "  assistant.textContent = turn.text;"
                        "  document.getElementById('conversation').replaceChildren(assistant);"
                        "  record('rendered:' + turn.text);"
                        "}"
                        "function mapServerThread(serverId, text) {"
                        "  setState(state => {"
                        "    const thread = state.threads[clientId] || {id: clientId, turns: []};"
                        "    state.threads[serverId] = thread;"
                        "    delete state.threads[clientId];"
                        "    state.clientNewThreadIdToServerIdMapping[clientId] = serverId;"
                        "    thread.serverId = serverId;"
                        "    thread.turns = [{role: 'assistant', text}];"
                        "    record('mapped:' + clientId + '->' + serverId);"
                        "  });"
                        "}"
                        "subscribe(render);"
                        "document.addEventListener('DOMContentLoaded', () => record('dcl'));"
                        "addEventListener('load', () => { record('load'); render(); setTimeout(sendPrompt, 0); });"
                        "async function sendPrompt() {"
                        "  try {"
                        "    record('prepare:start');"
                        "    const prepare = await fetch('/backend-api/f/conversation/prepare', { method: 'POST' });"
                        "    record('prepare:' + prepare.status);"
                        "    const prepared = await prepare.json();"
                        "    const serverId = prepared.conversation_id || 'smoke-live';"
                        "    const wsUrl = new URL('/ws-chatgpt-live', location.href);"
                        "    wsUrl.protocol = wsUrl.protocol === 'https:' ? 'wss:' : 'ws:';"
                        "    wsUrl.searchParams.set('conversation_id', serverId);"
                        "    const socket = new WebSocket(wsUrl.href, 'smoke');"
                        "    let text = '';"
                        "    socket.onopen = () => {"
                        "      record('ws:open');"
                        "      socket.send(JSON.stringify({ type: 'subscribe', conversation_id: serverId }));"
                        "    };"
                        "    socket.onmessage = event => {"
                        "      const message = JSON.parse(event.data);"
                        "      record('ws:' + message.type + (message.text ? ':' + message.text : ''));"
                        "      if (message.type === 'delta') text += message.text || '';"
                        "      if (message.type === 'done') mapServerThread(serverId, text);"
                        "    };"
                        "    record('conversation:start');"
                        "    const response = await fetch('/backend-api/f/conversation', {"
                        "      method: 'POST',"
                        "      headers: { 'content-type': 'application/json' },"
                        "      body: JSON.stringify({ conversation_id: clientId, prompt: 'OK' })"
                        "    });"
                        "    record('conversation:' + response.status);"
                        "    history.pushState({ conversationId: serverId, clientId }, '', '/c/' + serverId);"
                        "  } catch (error) {"
                        "    const message = `${error && error.name || 'Error'}:${error && error.message || String(error)}`;"
                        "    document.body.dataset.error = message;"
                        "    record('error:' + message);"
                        "  }"
                        "}"
                        "</script></body>"
                    )
                elif route == "/websocket-client-nav-page":
                    self._send_html(
                        "<!doctype html><body>"
                        "<main id='conversation'><div id='status'>waiting</div></main>"
                        "<script>"
                        "globalThis.__smokeWsEvents = [];"
                        "document.addEventListener('DOMContentLoaded', () => globalThis.__smokeWsEvents.push('dcl'));"
                        "addEventListener('load', () => globalThis.__smokeWsEvents.push('load'));"
                        "const url = new URL('/ws-echo', location.href);"
                        "url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';"
                        "const socket = new WebSocket(url.href, 'smoke');"
                        "socket.onopen = () => { globalThis.__smokeWsEvents.push('open'); socket.send('OK'); };"
                        "socket.onmessage = event => {"
                        "  globalThis.__smokeWsEvents.push(event.data);"
                        "  history.pushState({}, '', '/conversation/smoke');"
                        "  document.getElementById('conversation').innerHTML = "
                        "    '<div data-message-author-role=\"assistant\">' + event.data.replace(/^echo:/, '') + '</div>';"
                        "};"
                        "</script></body>"
                    )
                elif route == "/websocket-open-page":
                    self._send_html(
                        "<!doctype html><body>"
                        "<main id='conversation'><div id='status'>waiting</div></main>"
                        "<script>"
                        "globalThis.__smokeWsEvents = [];"
                        "document.addEventListener('DOMContentLoaded', () => globalThis.__smokeWsEvents.push('dcl'));"
                        "addEventListener('load', () => globalThis.__smokeWsEvents.push('load'));"
                        "const url = new URL('/ws-echo', location.href);"
                        "url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';"
                        "const socket = new WebSocket(url.href, 'smoke');"
                        "socket.onopen = () => { globalThis.__smokeWsEvents.push('open'); socket.send('OK'); };"
                        "socket.onmessage = event => {"
                        "  globalThis.__smokeWsEvents.push(event.data);"
                        "  document.getElementById('conversation').innerHTML = "
                        "    '<div data-message-author-role=\"assistant\">' + event.data.replace(/^echo:/, '') + '</div>';"
                        "};"
                        "</script></body>"
                    )
                elif route == "/lifecycle-load-state":
                    self._send_html(
                        "<!doctype html><body data-dcl='0' data-load='0'>"
                        "<main>lifecycle load state</main>"
                        "<img id='delayed' src='/delayed-image.png?delay=0.2' alt='delayed'>"
                        "<script>"
                        "document.addEventListener('DOMContentLoaded', () => { document.body.dataset.dcl = '1'; });"
                        "window.addEventListener('load', () => { document.body.dataset.load = '1'; });"
                        "</script></body>"
                    )
                elif route == "/chromium-cdp-lifecycle-page":
                    self._send_html(
                        "<!doctype html><body>"
                        "<main>chromium lifecycle page</main>"
                        "<script>document.body.dataset.scriptRan = '1';</script>"
                        "</body>"
                    )
                elif route == "/chromium-cdp-idle-page":
                    self._send_html(
                        "<!doctype html><title>idle detector</title>"
                        "<iframe src='/chromium-cdp-idle-child'></iframe>"
                    )
                elif route == "/chromium-cdp-idle-child":
                    self._send_html("<!doctype html><title>idle detector child</title>")
                elif route == "/chromium-audits-quirks-page":
                    self._send_html("<html><body>chromium Audits quirks page</body></html>")
                elif route == "/chromium-audits-csp-page":
                    self._send_html(
                        "<!doctype html>"
                        '<meta http-equiv="Content-Security-Policy" content="script-src \'none\'">'
                        "<body>chromium Audits CSP page</body>"
                    )
                elif route == "/chromium-cdp-dom-page":
                    self._send_html(
                        '<!doctype html><body><p class="class1" attr1="attr1">Paragraph Text</p></body>'
                    )
                elif route == "/chromium-cdp-dom-query-page":
                    self._send_html(
                        "<!doctype html><title>Example Domain</title><body>"
                        '<div class="testClass" id="firstDiv"></div>'
                        '<div class="testClass" id="secondDiv"></div>'
                        '<div class="testClass"></div>'
                        '<div class="testClass"></div>'
                        '<div class="testClass"></div>'
                        '<div id="depth-1"><div id="depth-2"><div id="targetDiv"></div></div>'
                        '<div id="targetUncle"><div id="targetCousin"></div></div></div>'
                        '<div id="singleTextChild">Only child</div>'
                        '<div id="multipleChildren">first<span>second</span></div>'
                        "</body>"
                    )
                elif route == "/ldm0-top-dom-whitespace":
                    self._send_html(LDM0_TOP_DOM_WHITESPACE_FIXTURE)
                elif route == "/parser-dom-mutation-page":
                    self._send_html(PARSER_DOM_MUTATION_FIXTURE)
                elif route == "/navigation-suspension-gated-document":
                    gate = outer.navigation_suspension_gate
                    gate.request_seen.set()
                    try:
                        if not gate.release_response.wait(timeout=10):
                            self._send_text(
                                "navigation suspension gate timed out",
                                status=HTTPStatus.REQUEST_TIMEOUT,
                            )
                        else:
                            self._send_html(
                                "<!doctype html><title>navigation committed</title>"
                                "<main id='navigation-committed'>replacement</main>"
                            )
                    finally:
                        gate.response_completed.set()
                elif route == "/navigation-suspension-gate/reset":
                    outer.navigation_suspension_gate.reset()
                    self._send_json({"ok": True})
                elif route == "/navigation-suspension-gate/release":
                    outer.navigation_suspension_gate.release_response.set()
                    self._send_json({"ok": True})
                elif route == "/navigation-suspension-gate/status":
                    gate = outer.navigation_suspension_gate
                    self._send_json(
                        {
                            "requestSeen": gate.request_seen.is_set(),
                            "released": gate.release_response.is_set(),
                            "responseCompleted": gate.response_completed.is_set(),
                        }
                    )
                elif route == "/parser-dom-mutation-held.js":
                    gate = outer.parser_dom_mutation_script_gate
                    gate.request_seen.set()
                    try:
                        if not gate.release_response.wait(timeout=10):
                            self._send_text(
                                "parser DOM mutation script gate timed out",
                                status=HTTPStatus.REQUEST_TIMEOUT,
                            )
                        else:
                            self._send_js(
                                "globalThis.__parserDomMutationScriptRan = true;"
                            )
                    finally:
                        gate.response_completed.set()
                elif route == "/fetch-runtime-teardown-held.mjs":
                    gate = outer.fetch_runtime_teardown_gate
                    gate.request_seen.set()
                    try:
                        if not gate.release_response.wait(timeout=10):
                            self._send_text(
                                "fetch runtime teardown gate timed out",
                                status=HTTPStatus.REQUEST_TIMEOUT,
                            )
                        else:
                            try:
                                self._send_js("export default 'completed';")
                            except (BrokenPipeError, ConnectionResetError):
                                # The teardown smoke intentionally retires the
                                # request owner before releasing this fixture.
                                pass
                    finally:
                        gate.response_completed.set()
                elif route == "/parser-dom-mutation-gate/reset":
                    outer.parser_dom_mutation_script_gate.reset()
                    self._send_json({"ok": True})
                elif route == "/parser-dom-mutation-gate/release":
                    outer.parser_dom_mutation_script_gate.release_response.set()
                    self._send_json({"ok": True})
                elif route == "/parser-dom-mutation-gate/status":
                    gate = outer.parser_dom_mutation_script_gate
                    self._send_json(
                        {
                            "requestSeen": gate.request_seen.is_set(),
                            "released": gate.release_response.is_set(),
                            "responseCompleted": gate.response_completed.is_set(),
                        }
                    )
                elif route == "/dom-shadow-outer-html":
                    self._send_html(DOM_SHADOW_OUTER_HTML_FIXTURE)
                elif route == "/dom-shadow-outer-html-child":
                    self._send_html(DOM_SHADOW_OUTER_HTML_CHILD_FIXTURE)
                elif route == "/chromium-cdp-hit-test-page":
                    self._send_html(DOM_HIT_TEST_FIXTURE)
                elif route == "/chromium-cdp-layout-page":
                    self._send_html(
                        "<!doctype html><body><div style='height:10000px;width:10000px'>content</div></body>"
                    )
                elif route == "/chromium-app-manifest-none/path/page":
                    self._send_html("<!doctype html><title>no manifest</title>")
                elif route == "/chromium-app-manifest-valid/page":
                    self._send_html(
                        '<!doctype html><link rel="manifest" href="/chromium-app-manifests/app.webmanifest">'
                    )
                elif route == "/chromium-app-manifest-invalid/page":
                    self._send_html(
                        '<!doctype html><link rel="manifest" href="/chromium-app-manifests/invalid.webmanifest">'
                    )
                elif route == "/chromium-app-manifest-missing/page":
                    self._send_html(
                        '<!doctype html><link rel="manifest" href="/chromium-app-manifests/missing.webmanifest">'
                    )
                elif route == "/chromium-app-manifest-redirect/page":
                    self._send_html(
                        '<!doctype html><link rel="manifest" href="/chromium-app-manifests/redirect.webmanifest">'
                    )
                elif route == "/chromium-app-manifest-dynamic/page":
                    self._send_html("<!doctype html><title>dynamic manifest</title>")
                elif route == "/chromium-app-manifests/app.webmanifest":
                    outer._increment_request_count(route)
                    self._send_common(
                        HTTPStatus.OK,
                        "application/manifest+json; charset=utf-8",
                        json.dumps(
                            {
                                "name": "Manifest Name",
                                "description": "Manifest Description",
                                "id": "./identity?x=1#ignored",
                                "start_url": "./start?x=2#fragment",
                                "scope": "./",
                                "display": "standalone",
                                "display_override": ["fullscreen", "browser", "bogus"],
                                "orientation": "portrait-primary",
                                "prefer_related_applications": True,
                                "background_color": "#11223380",
                                "theme_color": "red",
                                "icons": [
                                    {
                                        "src": "icons/app.png",
                                        "sizes": "16x16 32x32",
                                        "type": "image/png",
                                    }
                                ],
                                "shortcuts": [{"name": "Open", "url": "./shortcut"}],
                                "related_applications": [
                                    {"platform": "play", "url": "https://store.test/app", "id": "pkg"}
                                ],
                                "protocol_handlers": [
                                    {"protocol": "web+smoke", "url": "./handler?url=%s"}
                                ],
                            },
                            separators=(",", ":"),
                        ).encode(),
                    )
                elif route == "/chromium-app-manifests/invalid.webmanifest":
                    outer._increment_request_count(route)
                    self._send_common(
                        HTTPStatus.OK,
                        "application/manifest+json; charset=utf-8",
                        b'{"name":',
                    )
                elif route == "/chromium-app-manifests/missing.webmanifest":
                    self._send_text("missing manifest", status=HTTPStatus.NOT_FOUND)
                elif route == "/chromium-app-manifests/redirect.webmanifest":
                    self.send_response(HTTPStatus.FOUND)
                    self.send_header("Location", "/chromium-app-manifest-final/final.webmanifest")
                    self.send_header("Cache-Control", "no-store")
                    self.end_headers()
                elif route == "/chromium-app-manifest-final/final.webmanifest":
                    self._send_common(
                        HTTPStatus.OK,
                        "application/manifest+json; charset=utf-8",
                        b'{"id":"./identity","start_url":"./start","scope":"./"}',
                    )
                elif route == "/playwright-route-times":
                    self._send_html("<!doctype html><main>server fallback</main>")
                elif route == "/playwright-fallback-chain":
                    self._send_html("<!doctype html><main>fallback chain</main>")
                elif route == "/auth-email":
                    self._send_html(
                        "<!doctype html><main>email page</main>"
                        "<form id='login' method='get' action='/auth-email'>"
                        "<label>Email <input id='email' name='email' type='email' autocomplete='email'></label>"
                        "<button id='continue' type='submit'>Continue</button>"
                        "</form>"
                        "<script>"
                        "const form = document.getElementById('login');"
                        "form.addEventListener('submit', event => {"
                        "  event.preventDefault();"
                        "  document.body.dataset.submitted = '1';"
                        "  const email = new FormData(form).get('email') || '';"
                        "  location.href = '/auth-password?email=' + encodeURIComponent(email);"
                        "});"
                        "</script>"
                    )
                elif route == "/auth-password":
                    email = html.escape(query.get("email", [""])[0], quote=True)
                    self._send_html(
                        f"<!doctype html><main>password page</main>"
                        f'<input id="password" type="password" data-email="{email}">'
                    )
                elif route == "/delayed-image.png":
                    try:
                        delay = min(max(float(query.get("delay", ["0"])[0]), 0.0), 2.0)
                    except ValueError:
                        delay = 0.0
                    if delay:
                        time.sleep(delay)
                    self._send_png(TRANSPARENT_PNG)
                elif route == "/document-content-gated.css":
                    gate = outer.document_content_stylesheet_gate
                    gate.request_seen.set()
                    try:
                        if not gate.release_response.wait(timeout=10):
                            self._send_text(
                                "document-content stylesheet gate timed out",
                                status=HTTPStatus.REQUEST_TIMEOUT,
                            )
                        else:
                            self._send_css(
                                "#document-content-after-sheet { "
                                "color: rgb(71, 72, 73); "
                                "}"
                            )
                    finally:
                        gate.response_completed.set()
                elif route == "/dialog":
                    self._send_html(
                        "<!doctype html><button id='alert' onclick=\"alert('fixture alert')\">alert</button>"
                    )
                elif route == "/set-cookie":
                    self._send_html("<!doctype html><main>set cookie</main>", headers={"Set-Cookie": "serverCookie=server; Path=/"})
                elif route == "/echo-cookie":
                    cookie = html.escape(self.headers.get("Cookie", ""))
                    self._send_html(f"<!doctype html><body>{cookie}</body>")
                elif route == "/profile-headers":
                    token = query.get("token", [""])[0]
                    outer.profile_requests[token] = {
                        "userAgent": self.headers.get("User-Agent"),
                        "acceptLanguage": self.headers.get("Accept-Language"),
                        "extraHeader": self.headers.get("x-moli-profile-smoke"),
                        "referer": self.headers.get("Referer"),
                    }
                    self._send_html("<!doctype html><main>profile headers</main>")
                elif route == "/profile-result":
                    token = query.get("token", [""])[0]
                    self._send_json(outer.profile_requests.get(token))
                elif route == "/redirect-start":
                    self.send_response(HTTPStatus.FOUND)
                    self.send_header("Location", "/redirect-final")
                    self.send_header("Cache-Control", "no-store")
                    self.end_headers()
                elif route == "/redirect-final":
                    self._send_html("<!doctype html><main>redirect final</main>")
                elif route == "/chromium-network-revalidate":
                    self.protocol_version = "HTTP/1.1"
                    if self.headers.get("If-None-Match") == '"smoke-v1"':
                        self.send_response(HTTPStatus.NOT_MODIFIED)
                        self.send_header("Cache-Control", "public, max-age=3600")
                        self.send_header("ETag", '"smoke-v1"')
                        self.send_header("X-Smoke-Raw-Revalidation", "yes")
                        self.end_headers()
                    else:
                        body = b"<!doctype html><main>revalidation body</main>"
                        self.send_response(HTTPStatus.OK)
                        self.send_header("Content-Type", "text/html; charset=utf-8")
                        self.send_header("Cache-Control", "public, max-age=3600")
                        self.send_header("ETag", '"smoke-v1"')
                        self.send_header("X-Smoke-Cached-Response", "yes")
                        self.send_header("Content-Length", str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)
                elif route == "/history-a":
                    self._send_html("<!doctype html><main>history a</main>")
                elif route == "/history-b":
                    self._send_html("<!doctype html><main>history b</main>")
                elif route == "/document-continue":
                    marker = self.headers.get("x-smoke-nav-route") or "missing-document-route-header"
                    self._send_html(f"<!doctype html><main>{marker}</main>")
                elif route == "/api":
                    self._send_text("fixture api body")
                elif route == "/api-response-body-budget-small":
                    self._send_text("bounded response body remains readable")
                elif route == "/api-response-body-budget-oversize":
                    self._send_common(
                        HTTPStatus.OK,
                        "application/octet-stream",
                        b"x" * 2_000_001,
                    )
                elif route in ("/api-continue", "/worker-route-continue"):
                    payload = {"method": self.command, "routeHeader": self.headers.get("x-smoke-route") or self.headers.get("x-smoke-worker-route")}
                    self._send_json(payload)
                elif route == "/api-abort":
                    self._send_text("api abort fallback")
                elif route == "/api-echo":
                    self._send_json(
                        {
                            "method": self.command,
                            "body": _body.decode("utf-8", errors="replace"),
                            "contentType": self.headers.get("content-type"),
                            "customHeader": self.headers.get("x-smoke-post"),
                        },
                        headers={
                            "X-Smoke-Request-Method": self.command,
                            "X-Smoke-Request-Body-Length": str(len(_body)),
                        },
                    )
                elif route == "/api-response-headers":
                    self._send_json(
                        {"ok": True, "kind": self.headers.get("x-smoke-response-kind")},
                        headers={
                            "X-Smoke-Response": "header-visible",
                            "X-Smoke-Request-Kind": self.headers.get("x-smoke-response-kind")
                            or "missing",
                        },
                    )
                elif route == "/conversation-stream":
                    self._send_streaming_text(["O", "K"], delay=0.05)
                elif route == "/backend-api/f/conversation/prepare":
                    self._send_json({"conversation_id": "smoke-live"})
                elif route == "/backend-api/f/conversation":
                    self._send_json({"conversation_id": "smoke-live", "status": "queued"})
                elif route == "/api-response-stage":
                    self._send_text(
                        "response-stage body",
                        headers={"X-Smoke-Response-Stage": "paused"},
                    )
                elif route == "/api-binary":
                    self._send_common(
                        HTTPStatus.OK,
                        "application/octet-stream",
                        b"\x00\xffa",
                        {"X-Smoke-Binary": "ok"},
                    )
                elif route == "/api-auth":
                    if self.headers.get("Authorization") == "Basic dXNlcjpwYXNz":
                        self._send_text(
                            "authenticated fetch",
                            headers={"X-Smoke-Auth-Stage": "ok"},
                        )
                    else:
                        realm = query.get("realm", ["smoke-auth"])[0]
                        escaped_realm = realm.replace("\\", "\\\\").replace('"', '\\"')
                        self._send_text(
                            "auth required",
                            status=HTTPStatus.UNAUTHORIZED,
                            headers={"WWW-Authenticate": f'Basic realm="{escaped_realm}"'},
                        )
                elif route == "/api-redirect-start":
                    self.send_response(HTTPStatus.FOUND)
                    self.send_header("Location", "/api-redirect-final")
                    self.send_header("X-Smoke-Redirect", "start")
                    self.send_header("Cache-Control", "no-store")
                    self.end_headers()
                elif route == "/api-redirect-final":
                    self._send_json({"redirected": True, "method": self.command})
                elif route == "/parser-script-page":
                    self._send_html('<!doctype html><script src="/parser-script.js"></script><main>parser script page</main>')
                elif route == "/parser-script.js":
                    self._send_js('globalThis.__smokeParserScriptValue = "parser script loaded";')
                elif route == "/playwright-injected.js":
                    self._send_js('globalThis.__playwrightInjectedFromUrl = 42;')
                elif route == "/playwright-injected.css":
                    self._send_css("body { background-color: rgb(255, 0, 0); }")
                elif route == "/stylesheet-resource-page":
                    self._send_html(
                        "<!doctype html><head>"
                        '<link rel="stylesheet" href="/resource-link.css">'
                        "<style>@import url('/resource-import.css'); main { border-top-width: 1px; }</style>"
                        '<script src="/resource-after-style.js"></script>'
                        "</head><body><main id='styled'>stylesheet resource page</main></body>"
                    )
                elif route == "/stylesheet-resource-no-script-page":
                    self._send_html(
                        "<!doctype html><head>"
                        '<link rel="stylesheet" href="/resource-link.css">'
                        "<style>@import url('/resource-import.css'); main { border-top-width: 1px; }</style>"
                        "</head><body><main id='styled'>stylesheet resource page</main></body>"
                    )
                elif route == "/resource-link.css":
                    self._send_css("main { color: rgb(12, 34, 56); }")
                elif route == "/resource-import.css":
                    self._send_css("main { background-color: rgb(210, 220, 230); }")
                elif route == "/resource-after-style.js":
                    self._send_js("globalThis.__smokeAfterStylesheet = true;")
                elif route == "/chromium-resource-type-page":
                    self._send_html(
                        "<!doctype html><head>"
                        '<link rel="stylesheet" href="/chromium-resource-style.css">'
                        '<script src="/chromium-resource-script.js"></script>'
                        "</head><body>"
                        '<img id="resource-image" src="/chromium-resource-image.png" alt="resource">'
                        '<audio id="resource-audio" src="/chromium-resource-audio.wav"></audio>'
                        "<video id='resource-video'>"
                        '<source src="/chromium-resource-video.ogv" type="video/ogg">'
                        '<track default kind="captions" src="/chromium-resource-captions.vtt">'
                        "</video>"
                        "<script>"
                        "globalThis.__smokeResourceXhrDone = new Promise(resolve => {"
                        "  const xhr = new XMLHttpRequest();"
                        "  xhr.open('GET', '/chromium-resource-xhr.bin', true);"
                        "  xhr.responseType = 'arraybuffer';"
                        "  xhr.onload = () => resolve({ status: xhr.status, length: xhr.response.byteLength });"
                        "  xhr.onerror = () => resolve({ status: xhr.status, error: 'xhr error' });"
                        "  xhr.send();"
                        "});"
                        "</script>"
                        "<main>chromium resource type page</main></body>"
                    )
                elif route == "/chromium-resource-style.css":
                    self._send_css("main { color: rgb(31, 41, 59); }")
                elif route == "/chromium-resource-script.js":
                    self._send_js("globalThis.__smokeChromiumResourceScript = true;")
                elif route == "/chromium-resource-image.png":
                    self._send_png(TRANSPARENT_PNG)
                elif route == "/chromium-resource-audio.wav":
                    self._send_common(HTTPStatus.OK, "audio/wav", RESOURCE_MEDIA_BYTES)
                elif route == "/chromium-resource-video.ogv":
                    self._send_common(HTTPStatus.OK, "video/ogg", RESOURCE_MEDIA_BYTES)
                elif route == "/chromium-resource-captions.vtt":
                    self._send_common(HTTPStatus.OK, "text/vtt; charset=utf-8", b"WEBVTT\n\n00:00.000 --> 00:01.000\ncaption\n")
                elif route == "/chromium-resource-xhr.bin":
                    self._send_common(HTTPStatus.OK, "application/octet-stream", RESOURCE_XHR_BYTES)
                elif route == "/worker.js":
                    self._send_js(WORKER_SCRIPT)
                elif route == "/shared-worker.js":
                    self._send_js(SHARED_WORKER_SCRIPT)
                elif route == "/worker-route-fulfill":
                    self._send_text("worker route fulfill fallback")
                elif route == "/worker-route-abort":
                    self._send_text("worker route abort fallback")
                elif route == "/download-page":
                    self._send_html(
                        '<!doctype html><a id="download" href="/download" download>download</a>'
                        '<a id="slow-download" href="/slow-download" download>slow</a>'
                    )
                elif route == "/download":
                    self._send_download("smoke-download.txt", b"download contents")
                elif route == "/slow-download":
                    self._send_slow_download()
                else:
                    self._send_text(f"not found: {route}", status=HTTPStatus.NOT_FOUND)

            def _read_body(self) -> bytes:
                length = int(self.headers.get("Content-Length") or "0")
                return self.rfile.read(length) if length > 0 else b""

            def _send_common(self, status: HTTPStatus, content_type: str, body: bytes, headers: dict[str, str] | None = None) -> None:
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Length", str(len(body)))
                for key, value in (headers or {}).items():
                    self.send_header(key, value)
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(body)

            def _send_html(self, body: str, *, headers: dict[str, str] | None = None) -> None:
                self._send_common(HTTPStatus.OK, "text/html; charset=utf-8", body.encode(), headers)

            def _send_js(self, body: str) -> None:
                self._send_common(HTTPStatus.OK, "application/javascript; charset=utf-8", body.encode())

            def _send_cacheable_js(self, body: str, *, max_age: int) -> None:
                self._send_cacheable(
                    "application/javascript; charset=utf-8",
                    body.encode(),
                    max_age=max_age,
                )

            def _send_cacheable(
                self,
                content_type: str,
                payload: bytes,
                *,
                max_age: int = 3600,
            ) -> None:
                self.send_response(HTTPStatus.OK)
                self.send_header("Content-Type", content_type)
                self.send_header("Cache-Control", f"public, max-age={max_age}")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def _send_event_stream(self, body: str) -> None:
                payload = body.encode()
                self.send_response(HTTPStatus.OK)
                self.send_header("Content-Type", "text/event-stream; charset=utf-8")
                self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def _send_css(self, body: str) -> None:
                self._send_common(HTTPStatus.OK, "text/css; charset=utf-8", body.encode())

            def _send_text(
                self,
                body: str,
                *,
                status: HTTPStatus = HTTPStatus.OK,
                headers: dict[str, str] | None = None,
            ) -> None:
                self._send_common(status, "text/plain; charset=utf-8", body.encode(), headers)

            def _send_json(self, value: Any, *, headers: dict[str, str] | None = None) -> None:
                self._send_common(
                    HTTPStatus.OK,
                    "application/json; charset=utf-8",
                    json.dumps(value, separators=(",", ":")).encode(),
                    headers,
                )

            def _send_streaming_text(self, chunks: list[str], *, delay: float) -> None:
                self.send_response(HTTPStatus.OK)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.close_connection = True
                for chunk in chunks:
                    self.wfile.write(chunk.encode())
                    self.wfile.flush()
                    if delay:
                        time.sleep(delay)

            def _send_png(self, body: bytes) -> None:
                self._send_common(HTTPStatus.OK, "image/png", body)

            def _send_download(self, filename: str, body: bytes) -> None:
                self._send_common(
                    HTTPStatus.OK,
                    "text/plain; charset=utf-8",
                    body,
                    {"Content-Disposition": f'attachment; filename="{filename}"'},
                )

            def _send_slow_download(self) -> None:
                body = b"slow download contents"
                self.send_response(HTTPStatus.OK)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Disposition", 'attachment; filename="slow-smoke-download.txt"')
                self.end_headers()
                self.wfile.write(body[:4])
                self.wfile.flush()
                time.sleep(5)
                self.wfile.write(body[4:])

            def _handle_websocket(self) -> None:
                key = self.headers.get("Sec-WebSocket-Key")
                if not key:
                    self.send_error(HTTPStatus.BAD_REQUEST, "missing websocket key")
                    return
                protocol = "smoke" if "smoke" in (self.headers.get("Sec-WebSocket-Protocol") or "") else ""
                response = [
                    "HTTP/1.1 101 Switching Protocols",
                    "Upgrade: websocket",
                    "Connection: Upgrade",
                    f"Sec-WebSocket-Accept: {websocket_accept_key(key)}",
                ]
                if protocol:
                    response.append(f"Sec-WebSocket-Protocol: {protocol}")
                response.append("\r\n")
                self.request.sendall("\r\n".join(response).encode("ascii"))
                self.close_connection = True
                while True:
                    try:
                        opcode, payload = read_websocket_frame(self.request)
                    except Exception:
                        return
                    if opcode == 0x1:
                        self.request.sendall(websocket_text_frame("echo:" + payload.decode("utf-8", errors="replace")))
                    elif opcode == 0x8:
                        self.request.sendall(websocket_close_frame(1000, "bye"))
                        return
                    elif opcode == 0x9:
                        self.request.sendall(websocket_frame(0xA, payload))

            def _handle_chatgpt_live_websocket(self) -> None:
                key = self.headers.get("Sec-WebSocket-Key")
                if not key:
                    self.send_error(HTTPStatus.BAD_REQUEST, "missing websocket key")
                    return
                protocol = "smoke" if "smoke" in (self.headers.get("Sec-WebSocket-Protocol") or "") else ""
                response = [
                    "HTTP/1.1 101 Switching Protocols",
                    "Upgrade: websocket",
                    "Connection: Upgrade",
                    f"Sec-WebSocket-Accept: {websocket_accept_key(key)}",
                ]
                if protocol:
                    response.append(f"Sec-WebSocket-Protocol: {protocol}")
                response.append("\r\n")
                self.request.sendall("\r\n".join(response).encode("ascii"))
                self.close_connection = True
                while True:
                    try:
                        opcode, _payload = read_websocket_frame(self.request)
                    except Exception:
                        return
                    if opcode == 0x1:
                        for payload in (
                            {"type": "delta", "text": "O"},
                            {"type": "delta", "text": "K"},
                            {"type": "done"},
                        ):
                            self.request.sendall(websocket_text_frame(json.dumps(payload, separators=(",", ":"))))
                            time.sleep(0.05)
                    elif opcode == 0x8:
                        self.request.sendall(websocket_close_frame(1000, "bye"))
                        return
                    elif opcode == 0x9:
                        self.request.sendall(websocket_frame(0xA, _payload))

        return Handler
