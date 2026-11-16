import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import statistics
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(os.environ['LIMO_TEST_ROOT'])
BIN = Path(os.environ['LIMO_TEST_BIN'])
WORK = Path(tempfile.mkdtemp(prefix='limo-release-smoke-'))
RESULTS = []


def run(name, args, *, stdin=None, contains=(), code=0, timeout=12, exact=None):
    started = time.perf_counter()
    process = subprocess.Popen([str(BIN), *args], cwd=ROOT,
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True,
                               start_new_session=True)
    timed_out = False
    try:
        stdout, stderr = process.communicate(stdin, timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGKILL)
        stdout, stderr = process.communicate()
    elapsed = (time.perf_counter() - started) * 1000
    status_ok = process.returncode != 0 if code == 'nonzero' else process.returncode == code
    passed = not timed_out and status_ok and all(text in stdout + stderr for text in contains)
    if exact is not None:
        passed = passed and stdout.strip() == exact
    result = dict(name=name, passed=passed, args=args, stdin=stdin,
                  returncode=process.returncode, elapsed_ms=round(elapsed, 2),
                  timed_out=timed_out, stdout=stdout, stderr=stderr)
    RESULTS.append(result)
    print(json.dumps(result, ensure_ascii=False), flush=True)
    return result


def free_port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


run('help', ['--help'], contains=('Usage:', '--profile-dir', '--timeout-ms'))
run('eval', ['-e', 'console.log(1 + 2)'], exact='3')
run('top-level await auto detection', ['-e', 'console.log(await Promise.resolve(3))'], exact='3')
run('stdin explicit dash', ['-'], stdin='console.log("stdin-ok")', exact='stdin-ok')
run('stdin implicit', [], stdin='console.log("pipe-ok")', exact='pipe-ok')
run('DOM example', ['examples/dom.js'], contains=('Hello from limo!', 'clicked: Click me', '<button>Click me</button>'))
run('timers and microtasks example', ['examples/async.js'],
    exact='start\nsynchronous end\nmicrotask\ninterval: 1\ninterval: 2\ninterval: 3\ntimeout\nlast microtask')
run('fetch JSON example', ['examples/fetch.mjs'], contains=('message: Hello from a local file!', 'items: DOM, fetch, Worker'))
run('ES module example', ['examples/modules/main.mjs'], exact='Hello, limo!')
run('eval relative module import', ['--input-type', 'module', '-e',
    'import { greet } from "./examples/modules/greet.mjs"; console.log(greet("limo"))'], exact='Hello, limo!')
run('Worker example', ['examples/workers/main.js'], exact='sum: 15')

profile_args = ['--port', str(free_port()), '--profile-dir', str(WORK / 'profile')]
for number in (1, 2):
    run(f'IndexedDB persistence run {number}', [*profile_args, 'examples/idb.mjs'], exact=f'IndexedDB runs: {number}')
for number in (1, 2):
    run(f'OPFS sync handle persistence run {number}', [*profile_args, 'examples/opfs-sah/main.js'], exact=f'OPFS runs: {number}')
for number in (1, 2):
    run(f'localStorage persistence run {number}', [*profile_args, '-e',
        'const count = Number(localStorage.getItem("count") || 0) + 1; localStorage.setItem("count", String(count)); console.log(count)'], exact=str(number))

expected_hash = hashlib.sha256('hello'.encode()).hexdigest()
run('WebCrypto SHA-256', ['-e',
    'const result = await crypto.subtle.digest("SHA-256", new TextEncoder().encode("hello")); console.log(Array.from(new Uint8Array(result), b => b.toString(16).padStart(2, "0")).join(""))'], exact=expected_hash)
run('WebAssembly execution', ['-e',
    'const bytes = new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,127,3,2,1,0,7,10,1,6,97,110,115,119,101,114,0,0,10,6,1,4,0,65,42,11]); const {instance} = await WebAssembly.instantiate(bytes); console.log(instance.exports.answer())'], exact='42')
run('canvas 2D', ['-e',
    'const canvas = document.createElement("canvas"); canvas.width=2; canvas.height=2; const ctx=canvas.getContext("2d"); ctx.fillStyle="red"; ctx.fillRect(0,0,2,2); console.log(Array.from(ctx.getImageData(0,0,1,1).data).join(","))'], exact='255,0,0,255')
run('console levels and object formatting', ['-e',
    'console.log("a: b", {value: 3}); console.warn("warning"); console.error("failure")'], contains=('a: b', 'warning', 'failure'))


class CorsHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        payload = b'{"ok":true}'
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(payload)))
        if self.path == '/allowed':
            self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *_):
        pass


server = ThreadingHTTPServer(('127.0.0.1', 0), CorsHandler)
threading.Thread(target=server.serve_forever, daemon=True).start()
origin = f'http://127.0.0.1:{server.server_port}'
run('CORS allowed response', ['-e',
    f'const response = await fetch("{origin}/allowed"); console.log(JSON.stringify(await response.json()))'], exact='{"ok":true}')
run('CORS denied response', ['-e',
    f'try {{ await fetch("{origin}/denied"); console.log("unexpectedly-allowed"); }} catch (error) {{ console.log("cors-blocked"); }}'], exact='cors-blocked')
server.shutdown()
server.server_close()

special = WORK / '入口 #&?.mjs'
special.write_text('console.log("special-path-ok")')
run('Unicode and reserved characters in file path', [str(special)], exact='special-path-ok')
cjs = WORK / 'unsupported.cjs'
cjs.write_text('console.log("override-ok")')
run('CommonJS rejection', [str(cjs)], code='nonzero', contains=('CommonJS',))
run('explicit input type overrides extension', ['--input-type', 'script', str(cjs)], exact='override-ok')
run('missing input file', [str(WORK / 'missing.js')], code='nonzero', contains=('not a file',))
run('conflicting file and eval options', ['examples/dom.js', '-e', '1'], code='nonzero')
run('syntax error', ['-e', 'const = ;'], code='nonzero')
run('synchronous uncaught exception', ['-e', 'console.log("before-error"); throw new Error("smoke-boom")'], code='nonzero')
run('module uncaught exception', ['--input-type', 'module', '-e', 'throw new Error("module-boom")'], code='nonzero')
run('timer uncaught exception', ['-e', 'setTimeout(() => { throw new Error("timer-boom") }, 10)'], code='nonzero')
run('unhandled Promise rejection', ['-e', 'Promise.reject(new Error("reject-boom"))'], code='nonzero')
run('unbacked top-level await', ['-e', 'console.log("before-await"); await new Promise(() => {}); console.log("after-await")'], exact='before-await')
run('live interval execution timeout', ['--timeout-ms', '100', '-e', 'setInterval(() => {}, 10)'], code='nonzero', contains=('timed out',), timeout=4)
run('synchronous infinite loop execution timeout', ['--timeout-ms', '100', '-e', 'while (true) {}'], code='nonzero', contains=('timed out',), timeout=4)

run('Intl timezone data', ['-e', 'console.log(new Intl.DateTimeFormat("zh-CN", {year:"numeric", timeZone:"Asia/Shanghai"}).format(new Date("2020-01-01T00:00:00Z")))'], exact='2020年')
run('Blob URL fetch', ['-e', 'const url = URL.createObjectURL(new Blob(["blob-ok"])); console.log(await (await fetch(url)).text()); URL.revokeObjectURL(url)'], exact='blob-ok')
run('MessageChannel delivery', ['-e', 'const c = new MessageChannel(); c.port1.onmessage = e => {console.log(e.data); c.port1.close(); c.port2.close()}; c.port2.postMessage("channel-ok")'], exact='channel-ok')
run('iframe realm and messaging', ['-e', r'const answer = await new Promise((resolve, reject) => {const timeout = setTimeout(() => reject(new Error("iframe-timeout")), 2000); addEventListener("message", e => {if (e.data === "frame-ok") {clearTimeout(timeout); resolve(e.data)}}); const frame = document.createElement("iframe"); frame.srcdoc = "<script>parent.postMessage(\"frame-ok\", \"*\")</script>"; document.body.appendChild(frame)}); console.log(answer)'], exact='frame-ok')
shared = WORK / 'shared-worker.js'
shared.write_text('onconnect = event => { const port = event.ports[0]; port.onmessage = e => { port.postMessage(e.data + 1); port.close(); close() }; port.start() }')
shared_main = WORK / 'shared-main.mjs'
shared_main.write_text('const worker = new SharedWorker("./shared-worker.js"); worker.port.onmessage = e => { console.log("shared:", e.data); worker.port.close() }; worker.port.start(); worker.port.postMessage(41)')
run('SharedWorker messaging', ['--timeout-ms', '4000', str(shared_main)], exact='shared: 42', timeout=6)
sw = WORK / 'service-worker.js'
sw.write_text('addEventListener("install", event => event.waitUntil(skipWaiting())); addEventListener("activate", event => event.waitUntil(clients.claim()))')
sw_main = WORK / 'service-main.mjs'
sw_main.write_text('const registration = await navigator.serviceWorker.register("./service-worker.js"); await navigator.serviceWorker.ready; console.log("service-worker:", await registration.unregister())')
run('ServiceWorker registration lifecycle', ['--timeout-ms', '4000', str(sw_main)], exact='service-worker: true', timeout=6)

timings = []
for number in range(10):
    result = run(f'startup sample {number + 1}', ['-e', 'console.log(1 + 2)'], exact='3')
    timings.append(result['elapsed_ms'])

report = dict(binary=str(BIN), results=RESULTS,
              passed=sum(result['passed'] for result in RESULTS), total=len(RESULTS),
              startup_ms=dict(median=round(statistics.median(timings), 2),
                              minimum=min(timings), maximum=max(timings)))
(WORK / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
print(json.dumps({'report': str(WORK / 'results.json'), 'passed': report['passed'],
                  'total': report['total'], 'startup_ms': report['startup_ms']}, ensure_ascii=False), flush=True)
