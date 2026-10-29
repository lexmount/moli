from __future__ import annotations

import html
import json
import os
import threading
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit


class FixtureServer:
    def __init__(self) -> None:
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), self._handler_class())
        self.thread = threading.Thread(
            target=self.httpd.serve_forever,
            name="moli-webdriver-smoke-fixture",
            daemon=True,
        )

    @property
    def url(self) -> str:
        host, port = self.httpd.server_address
        return f"http://{host}:{port}"

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.httpd.shutdown()
        self.thread.join(timeout=5)
        self.httpd.server_close()

    @staticmethod
    def _handler_class() -> type[BaseHTTPRequestHandler]:
        class Handler(BaseHTTPRequestHandler):
            server_version = "MoliWebDriverSmokeFixture/0.1"

            def log_message(self, format: str, *args: object) -> None:
                if os.environ.get("MOLI_WEBDRIVER_SMOKE_TRACE_BG") == "1":
                    super().log_message(format, *args)

            def do_GET(self) -> None:
                parsed = urlsplit(self.path)
                route = parsed.path
                if route == "/" or route == "/webdriver/basic":
                    self._send_html(self._basic_page())
                    return
                if route == "/webdriver/frame":
                    self._send_html("<!doctype html><title>Frame</title><main id='inside-frame'>frame ready</main>")
                    return
                if route in ("/webdriver/partitioned-cookie-top", "/webdriver/partitioned-cookie-frame"):
                    value = "part-a" if route.endswith("-top") else "part-b"
                    http_only = "; HttpOnly" if route.endswith("-top") else ""
                    self._send_html(
                        "<!doctype html><title>Partitioned Cookie</title><main>cookie ready</main>",
                        headers={
                            "Set-Cookie": (
                                f"bidiDeleteSmoke={value}; Path=/; Secure; SameSite=None; Partitioned{http_only}"
                            ),
                            "Cache-Control": "no-store",
                        },
                    )
                    return
                if route == "/webdriver/nested-frames":
                    self._send_html(self._nested_frames_page())
                    return
                if route == "/webdriver/outer-frame":
                    self._send_html(self._outer_frame_page())
                    return
                if route == "/webdriver/inner-frame":
                    self._send_html(
                        "<!doctype html><title>Inner Frame</title><main id='inner-main'>inner ready</main>"
                    )
                    return
                if route == "/webdriver/sibling-frame":
                    self._send_html(
                        "<!doctype html><title>Sibling Frame</title><main id='sibling-main'>sibling ready</main>"
                    )
                    return
                if route == "/webdriver/deleting-frame":
                    self._send_html(self._deleting_frame_page())
                    return
                if route == "/webdriver/deleting-frame-child":
                    self._send_html(self._deleting_frame_child_page())
                    return
                if route == "/webdriver/wait":
                    self._send_html(self._wait_page())
                    return
                if route == "/webdriver/implicit-wait":
                    self._send_html(self._implicit_wait_page())
                    return
                if route == "/webdriver/actions":
                    self._send_html(self._actions_page())
                    return
                if route == "/webdriver/input-navigation":
                    self._send_html(self._input_navigation_page())
                    return
                if route == "/webdriver/input-navigation-complete":
                    self._send_html(self._input_navigation_complete_page())
                    return
                if route == "/webdriver/form":
                    self._send_html(self._form_page())
                    return
                if route == "/webdriver/form-submitted":
                    self._send_html(self._form_submitted_page(parsed.query))
                    return
                if route == "/webdriver/profile-echo":
                    self._send_html(self._profile_echo_page())
                    return
                if route == "/webdriver/cookie-echo":
                    cookie = self.headers.get("Cookie", "")
                    self._send_html(f"<!doctype html><title>Cookie Echo</title><main id='cookie-echo'>{cookie}</main>")
                    return
                if route == "/webdriver/popup-page":
                    self._send_html(self._popup_page())
                    return
                if route == "/webdriver/popup-target":
                    self._send_html(self._popup_target_page(parsed.query))
                    return
                if route == "/webdriver/semantic-shadow-frame":
                    self._send_html(self._semantic_shadow_frame_page())
                    return
                if route == "/webdriver/network-data":
                    self._send_text(parse_qs(parsed.query).get("body", ["webdriver network body"])[0])
                    return
                if route == "/webdriver/shared-worker":
                    self._send_html(self._shared_worker_page())
                    return
                if route == "/webdriver/shared-worker.js":
                    self._send_js(self._shared_worker_script())
                    return
                if route == "/webdriver/basic-auth":
                    if self.headers.get("Authorization") != "Basic cG9zdG1hbjpwYXNzd29yZA==":
                        self.send_response(HTTPStatus.UNAUTHORIZED)
                        self.send_header("WWW-Authenticate", 'Basic realm="webdriver-smoke"')
                        self.send_header("Content-Length", "0")
                        self.end_headers()
                        return
                    self._send_html("<!doctype html><title>Basic Auth</title><main>authenticated</main>")
                    return
                if route == "/webdriver/download-page":
                    self._send_html(
                        '<!doctype html><title>Download</title>'
                        '<a id="download" href="/webdriver/download" download>download</a>'
                        '<a id="download-one" href="/webdriver/download-one" download>one</a>'
                        '<a id="download-two" href="/webdriver/download-two" download>two</a>'
                    )
                    return
                if route == "/webdriver/download":
                    self._send_download("smoke-download.txt", b"download contents")
                    return
                if route == "/webdriver/download-one":
                    self._send_download("file_1.txt", b"Hello, World!")
                    return
                if route == "/webdriver/download-two":
                    self._send_download("file_2.jpg", b"fake jpg")
                    return
                self.send_error(HTTPStatus.NOT_FOUND, "fixture route not found")

            def do_POST(self) -> None:
                route = urlsplit(self.path).path
                if route == "/webdriver/network-data":
                    length = int(self.headers.get("Content-Length", "0"))
                    body = self.rfile.read(length).decode("utf-8")
                    if body != "webdriver request body":
                        self.send_error(HTTPStatus.BAD_REQUEST, "unexpected network request body")
                        return
                    self._send_text("webdriver network body")
                    return
                self.send_error(HTTPStatus.NOT_FOUND, "fixture route not found")

            def _send_html(
                self,
                body: str,
                status: HTTPStatus = HTTPStatus.OK,
                *,
                headers: dict[str, str] | None = None,
            ) -> None:
                payload = body.encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Content-Length", str(len(payload)))
                for name, value in (headers or {}).items():
                    self.send_header(name, value)
                self.end_headers()
                self.wfile.write(payload)

            def _send_text(self, body: str, status: HTTPStatus = HTTPStatus.OK) -> None:
                payload = body.encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def _send_js(self, body: str, status: HTTPStatus = HTTPStatus.OK) -> None:
                payload = body.encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "text/javascript; charset=utf-8")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)

            def _send_download(self, filename: str, body: bytes, status: HTTPStatus = HTTPStatus.OK) -> None:
                self.send_response(status)
                self.send_header("Content-Type", "text/plain; charset=utf-8")
                self.send_header("Content-Disposition", f'attachment; filename="{filename}"')
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            @staticmethod
            def _basic_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke Basic</title></head>
<body>
  <main id="main" class="no cheese">Basic Ready</main>
  <button id="labelled" aria-label="Smoke Label">ignored</button>
  <article id="article-role">article body</article>
  <input id="field" value="">
  <input id="check" type="checkbox">
  <button id="clicker">Click</button>
  <output id="click-output"></output>
  <select id="select" name="select">
    <option value="one">One</option>
    <option value="two">Two</option>
  </select>
  <select id="selectomatic" name="selectomatic">
    <option value="one">One</option>
    <option value="two">Two</option>
    <option value="four">Four</option>
    <option value="still learning how to count, apparently">Still learning how to count, apparently</option>
  </select>
  <select id="select-with-spaces" name="select_with_spaces">
    <option value="one">One</option>
    <option value="two">Two</option>
    <option value="four">Four</option>
    <option value="still learning how to count, apparently">Still   learning how to count, apparently</option>
  </select>
  <select id="multi" name="multi" multiple>
    <option value="eggs" selected>Eggs</option>
    <option value="ham">Ham</option>
    <option value="sausages" selected>Sausages</option>
    <option value="onion gravy">Onion gravy</option>
  </select>
  <select id="select-empty-multiple" name="select_empty_multiple" multiple>
    <option value="select_1">select_1</option>
    <option value="select_2">select_2</option>
    <option value="select_3">select_3</option>
    <option value="select_4">select_4</option>
  </select>
  <select id="single-disabled" name="single_disabled">
    <option value="enabled">Enabled</option>
    <option value="disabled" disabled>Disabled</option>
  </select>
  <select id="multi-disabled" name="multi_disabled" multiple>
    <option value="enabled">Enabled</option>
    <option value="disabled" disabled>Disabled</option>
  </select>
  <select id="invisible-multi-select" name="invisible_multi_select" multiple>
    <option value="apples" style="display:none">Apples</option>
    <option value="pears" style="display:none">Pears</option>
  </select>
  <fieldset disabled>
    <input id="disabled-fieldset-child" value="blocked">
    <legend><input id="disabled-fieldset-legend" value="allowed"></legend>
  </fieldset>
  <input id="hidden-clear" type="hidden" value="hidden">
  <input id="upload-multiple" type="file" multiple style="display:none">
  <input id="upload-single" type="file">
  <p id="hidden-attribute" hidden>hidden text</p>
  <a id="relative-link" href="/webdriver/basic">Basic Link</a>
  <iframe id="child" src="/webdriver/frame"></iframe>
  <div id="host"></div>
  <script>
    window.__events = [];
    document.querySelector("#clicker").addEventListener("click", () => {
      document.body.dataset.clicked = "yes";
      document.querySelector("#click-output").textContent = "clicked";
    });
    window.__selectEvents = [];
    for (const select of document.querySelectorAll("select")) {
      select.addEventListener("input", () => window.__selectEvents.push(select.id + ":input:" + select.value));
      select.addEventListener("change", () => window.__selectEvents.push(select.id + ":change:" + select.value));
    }
    window.__clearEvents = [];
    for (const id of ["field", "disabled-fieldset-legend"]) {
      const input = document.getElementById(id);
      input.addEventListener("input", () => window.__clearEvents.push(id + ":input:" + input.value));
      input.addEventListener("change", () => window.__clearEvents.push(id + ":change:" + input.value));
    }
    window.__fileEvents = [];
    for (const id of ["upload-multiple", "upload-single"]) {
      const input = document.getElementById(id);
      input.addEventListener("input", () => window.__fileEvents.push(id + ":input:" + input.files.length));
      input.addEventListener("change", () => window.__fileEvents.push(id + ":change:" + input.files.length));
    }
    const root = document.querySelector("#host").attachShadow({ mode: "open" });
    root.innerHTML = '<span id="shadow-text" class="shadow-item">shadow ready</span><button id="shadow-button" class="shadow-item">Shadow</button>';
  </script>
</body>
</html>"""

            @staticmethod
            def _input_navigation_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Input Navigation</title></head>
<body>
  <input id="navigation-field" autofocus>
  <script>
    document.getElementById("navigation-field").addEventListener("keydown", event => {
      if (event.key === "Enter") location.href = "/webdriver/input-navigation-complete";
    });
  </script>
</body>
</html>"""

            @staticmethod
            def _input_navigation_complete_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Input Navigation Complete</title></head>
<body><main id="input-navigation-complete">input navigation complete</main></body>
</html>"""

            @staticmethod
            def _semantic_shadow_frame_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Semantic Shadow Frame</title></head>
<body>
  <main>shadow frame owner</main>
  <div id="shadow-frame-host"></div>
  <script>
    const root = document.querySelector("#shadow-frame-host").attachShadow({ mode: "open" });
    const frame = document.createElement("iframe");
    frame.id = "shadowFrameId";
    frame.name = "shadowFrameName";
    frame.src = "/webdriver/frame";
    root.appendChild(frame);
  </script>
</body>
</html>"""

            @staticmethod
            def _nested_frames_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke Nested Frames</title></head>
<body>
  <main id="top-main">top ready</main>
  <iframe id="outerById" name="outerByName" src="/webdriver/outer-frame"></iframe>
  <iframe id="siblingById" name="siblingByName" src="/webdriver/sibling-frame"></iframe>
</body>
</html>"""

            @staticmethod
            def _outer_frame_page() -> str:
                return """<!doctype html>
<html>
<head><title>Outer Frame</title></head>
<body>
  <main id="outer-main">outer ready</main>
  <iframe id="innerById" name="innerByName" src="/webdriver/inner-frame"></iframe>
</body>
</html>"""

            @staticmethod
            def _deleting_frame_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke Deleting Frame</title></head>
<body>
  <main id="top-marker">top ready</main>
  <button id="addBackFrame" onclick="addBackFrame()">add frame</button>
  <div id="frame-host"></div>
  <script>
    function addBackFrame() {
      if (document.getElementById("iframe1")) return;
      const frame = document.createElement("iframe");
      frame.id = "iframe1";
      frame.name = "iframe1";
      frame.src = "/webdriver/deleting-frame-child";
      document.getElementById("frame-host").appendChild(frame);
    }
    addBackFrame();
  </script>
</body>
</html>"""

            @staticmethod
            def _deleting_frame_child_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke Deleting Frame Child</title></head>
<body>
  <main id="success">success</main>
  <button id="killIframe" onclick="parent.document.getElementById('iframe1')?.remove()">kill frame</button>
</body>
</html>"""

            @staticmethod
            def _wait_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke Wait</title></head>
<body>
  <main id="wait-main">wait page</main>
  <p id="status">starting</p>
  <button id="wait-clicker" disabled>wait click</button>
  <iframe id="delayed-frame" src="/webdriver/frame"></iframe>
  <p id="remove-me">remove me</p>
  <p id="hide-me">hide me</p>
  <p id="hide-located">hide located</p>
  <input id="checky" type="checkbox">
  <input id="value-target" value="">
  <script>
    setTimeout(() => {
      const late = document.createElement("strong");
      late.id = "late";
      late.textContent = "late ready";
      document.body.appendChild(late);
    }, 50);
    setTimeout(() => {
      document.getElementById("status").textContent = "ready";
      document.getElementById("wait-clicker").disabled = false;
    }, 75);
    setTimeout(() => {
      document.getElementById("hide-me").style.display = "none";
      document.getElementById("hide-located").style.display = "none";
    }, 100);
    window.__removeWaitTarget = () => {
      document.getElementById("remove-me")?.remove();
    };
    setTimeout(() => {
      document.getElementById("checky").checked = true;
    }, 150);
    setTimeout(() => {
      const valueTarget = document.getElementById("value-target");
      valueTarget.value = "Example Expected text";
      valueTarget.setAttribute("data-ready", "true");
    }, 175);
  </script>
</body>
</html>"""

            @staticmethod
            def _implicit_wait_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke Implicit Wait</title></head>
<body>
  <main id="implicit-main">implicit wait page</main>
  <button id="adder">Add delayed boxes</button>
  <script>
    let nextBox = 0;
    document.getElementById("adder").addEventListener("click", () => {
      const firstIndex = nextBox++;
      const secondIndex = nextBox++;
      const delay = Number(window.__implicitAddDelayMs ?? 120);
      setTimeout(() => {
        for (const index of [firstIndex, secondIndex]) {
          const node = document.createElement("div");
          node.id = "box" + index;
          node.className = "redbox";
          node.textContent = "box " + index;
          document.body.appendChild(node);
        }
      }, delay);
    });
  </script>
</body>
</html>"""

            @staticmethod
            def _actions_page() -> str:
                return """<!doctype html>
<html>
<head>
  <title>WebDriver Smoke Actions</title>
  <style>
    body { margin: 0; padding: 20px; font-family: sans-serif; }
    #hover, #double-click, #context-click, #drag-source, #drop-target {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 120px;
      height: 48px;
      margin: 8px;
      border: 1px solid #444;
      user-select: none;
    }
    #drop-target { width: 150px; height: 70px; }
  </style>
</head>
<body>
  <button id="hover">Hover</button>
  <input id="double-click" value="">
  <input id="context-click" value="">
  <div id="drag-source" draggable="true">drag me</div>
  <div id="drop-target"><p>Drop here</p></div>
  <output id="event-log"></output>
  <script>
    window.__actionEvents = [];
    function log(value) {
      window.__actionEvents.push(value);
      document.getElementById("event-log").textContent = window.__actionEvents.join("|");
    }
    document.getElementById("hover").addEventListener("mouseover", event => {
      log("mouseover:" + event.button + ":" + event.buttons);
    });
    document.getElementById("double-click").addEventListener("dblclick", event => {
      event.currentTarget.value = "DoubleClicked";
      log("dblclick:" + event.detail + ":" + event.button);
    });
    document.getElementById("context-click").addEventListener("contextmenu", event => {
      event.preventDefault();
      event.currentTarget.value = "ContextClicked";
      log("contextmenu:" + event.button);
    });
    const source = document.getElementById("drag-source");
    const target = document.getElementById("drop-target");
    source.addEventListener("dragstart", event => {
      event.dataTransfer.setData("text/plain", "dragged");
      log("dragstart");
    });
    target.addEventListener("dragover", event => {
      event.preventDefault();
      log("dragover");
    });
    target.addEventListener("drop", event => {
      event.preventDefault();
      const value = event.dataTransfer.getData("text/plain") || "dragged";
      target.dataset.drop = value;
      target.querySelector("p").textContent = "Dropped!";
      log("drop:" + value);
    });
  </script>
</body>
</html>"""

            @staticmethod
            def _form_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke Form</title></head>
<body>
  <main id="form-main">form ready</main>
  <form id="submit-form" action="/webdriver/form-submitted" method="get">
    <input id="login" name="login" value="">
    <input id="working" name="working" value="">
    <input id="no-type" name="noType" value="">
    <textarea id="with-text" name="withText">textarea value</textarea>
    <textarea id="empty-textarea" name="emptyTextArea"></textarea>
    <input id="checky" name="checky" type="checkbox" value="checked">
    <input id="cheese-and-peas" name="food" type="radio" value="cheese_and_peas" checked>
    <input id="cheese" name="food" type="radio" value="cheese">
    <input id="peas" name="food" type="radio" value="peas">
    <p id="submit-paragraph">inside form</p>
    <input id="submit" name="submit_id" type="submit" value="Submit Input">
    <input id="named-submit" name="submit" type="submit" value="Named Submit">
    <button id="button-submit" name="buttonSubmit" type="submit" value="button">Button Submit</button>
  </form>
  <button id="outside-submit" name="SearchableText" type="button">outside</button>
  <input id="key-reporter" value="">
  <input id="event-one" value="">
  <input id="event-two" value="">
  <output id="result"></output>
  <output id="event-log"></output>
  <script>
    window.__formEvents = [];
    function log(value) {
      window.__formEvents.push(value);
      document.getElementById("event-log").textContent = window.__formEvents.join("|");
    }
    for (const id of ["key-reporter", "event-one", "event-two", "working", "with-text"]) {
      const element = document.getElementById(id);
      for (const type of ["focus", "blur", "input", "change", "keydown", "keypress", "keyup"]) {
        element.addEventListener(type, event => {
          const suffix = type === "keydown" || type === "keypress" || type === "keyup"
            ? ":" + event.key + ":" + event.keyCode
            : "";
          log(id + ":" + type + suffix);
        });
      }
    }
    document.getElementById("checky").addEventListener("change", () => {
      document.getElementById("result").textContent = "checkbox:" + document.getElementById("checky").checked;
    });
  </script>
</body>
</html>"""

            @staticmethod
            def _form_submitted_page(query: str) -> str:
                payload = json.dumps(parse_qs(query), sort_keys=True, separators=(",", ":"))
                return f"""<!doctype html>
<html>
<head><title>WebDriver Submitted</title></head>
<body>
  <main id="submitted">{html.escape(payload)}</main>
</body>
</html>"""

            @staticmethod
            def _popup_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke Popup</title></head>
<body>
  <main id="popup-source">popup source ready</main>
  <a id="anchor-popup" href="/webdriver/popup-target?kind=anchor" target="_blank">anchor popup</a>
  <a id="named-link" href="/webdriver/popup-target?kind=named-first" target="seleniumNamedPopup">named popup</a>
  <button id="script-popup" onclick="window.open('/webdriver/popup-target?kind=script', '_blank')">script popup</button>
  <button id="named-reuse" onclick="window.open('/webdriver/popup-target?kind=named-second', 'seleniumNamedPopup')">reuse named popup</button>
  <button id="self-open" onclick="window.open('/webdriver/popup-target?kind=self', '_self')">self popup</button>
</body>
</html>"""

            @staticmethod
            def _popup_target_page(query: str) -> str:
                kind = parse_qs(query).get("kind", ["unknown"])[0]
                escaped_kind = html.escape(kind)
                return f"""<!doctype html>
<html>
<head><title>WebDriver Smoke Popup {escaped_kind}</title></head>
<body>
  <main id="popup-main">popup {escaped_kind}</main>
  <script>
    document.body.dataset.hasOpener = window.opener ? "yes" : "no";
  </script>
</body>
</html>"""

            @staticmethod
            def _shared_worker_page() -> str:
                return """<!doctype html>
<html>
<head><title>WebDriver Smoke SharedWorker</title></head>
<body>
  <main id="shared-worker-main">shared worker ready</main>
  <script>
    globalThis.__webdriverSharedWorkerProbe = value => new Promise((resolve, reject) => {
      const worker = new SharedWorker('/webdriver/shared-worker.js', 'webdriver-shared-worker-smoke');
      globalThis.__webdriverSharedWorkers = globalThis.__webdriverSharedWorkers || [];
      globalThis.__webdriverSharedWorkers.push(worker);
      const timer = setTimeout(() => reject(new Error('shared worker timeout')), 5000);
      worker.port.onmessage = event => {
        clearTimeout(timer);
        resolve(event.data);
      };
      worker.port.start();
      worker.port.postMessage({ kind: 'probe', value });
    });
  </script>
</body>
</html>"""

            @staticmethod
            def _shared_worker_script() -> str:
                return """
globalThis.__webdriverSharedWorkerConnectionCount = 0;
console.log('webdriver-shared-worker-ready');
self.onconnect = event => {
  const port = event.ports[0];
  const connectionId = ++globalThis.__webdriverSharedWorkerConnectionCount;
  port.onmessage = event => {
    const data = event.data;
    if (data && data.kind === 'probe') {
      port.postMessage({
        kind: 'probe-result',
        echoed: data.value,
        name,
        pathname: self.location.pathname,
        selfEqualsGlobal: self === globalThis,
        isSharedWorker: typeof SharedWorkerGlobalScope !== 'undefined' && self instanceof SharedWorkerGlobalScope,
        connectionId,
        connectionCount: globalThis.__webdriverSharedWorkerConnectionCount,
      });
      return;
    }
    port.postMessage({ kind: 'unexpected', connectionId });
  };
  port.start();
};
"""

            def _profile_echo_page(self) -> str:
                payload = json.dumps(
                    {
                        "userAgent": self.headers.get("User-Agent", ""),
                        "acceptLanguage": self.headers.get("Accept-Language", ""),
                        "cookie": self.headers.get("Cookie", ""),
                    },
                    separators=(",", ":"),
                )
                return f"""<!doctype html>
<html>
<head><title>WebDriver Smoke Profile Echo</title></head>
<body><main id="profile-echo">{html.escape(payload)}</main></body>
</html>"""

        return Handler
