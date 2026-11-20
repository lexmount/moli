"""Response Document origin lifetime and URL contracts, using real HTTP and SW."""

from __future__ import annotations

import contextlib
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

from ..assertions import SmokeError, assert_equal, record_contract
from ..state import SmokeState


@contextlib.contextmanager
def _server():
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_GET(self):
            url = urlsplit(self.path)
            status, headers = 200, {"Origin-Agent-Cluster": "?0", "Cache-Control": "no-store"}
            if url.path == "/redirect":
                status, body = 302, b""
                headers["Location"] = parse_qs(url.query)["to"][0]
            elif url.path == "/worker.js":
                headers["Content-Type"] = "text/javascript"
                body = b"""
                    self.addEventListener('install', e => e.waitUntil(self.skipWaiting()));
                    self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));
                    self.addEventListener('fetch', e => {
                        const path = new URL(e.request.url).pathname;
                        if (path === '/sw/entry')
                            e.respondWith(Response.redirect('/sw/alias', 302));
                        else if (path === '/sw/alias')
                            e.respondWith(fetch('/nested/doc.xml#nested'));
                    });
                """
            elif url.path.endswith((".xml", ".html")):
                headers["Content-Type"] = "text/html" if url.path.endswith(".html") else "application/xml"
                headers["Access-Control-Allow-Origin"] = "*"
                body = b'<html xmlns="http://www.w3.org/1999/xhtml"><body><a id="link" href="child">link</a></body></html>'
            else:
                headers["Content-Type"] = "text/html"
                body = b"<!doctype html><html><head></head><body></body></html>"
            self.send_response(status)
            for name, value in headers.items():
                self.send_header(name, value)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server.server_port
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


async def run_xhr_document_metadata_group(state: SmokeState) -> None:
    failures = []
    with _server() as port:
        origin = f"http://localhost:{port}"
        await state.page.goto(f"{origin}/page", wait_until="load")
        for name, expression in [
            ("clone-requester-origin", _CLONE),
            ("retained-requester-origin", _RETAINED),
            ("redirect-fragments", _REDIRECT),
            ("service-worker-response-url", _SERVICE_WORKER),
        ]:
            try:
                observed = await state.page.evaluate(expression, port)
                assert_equal(observed, "ok", name)
                record_contract(state.results, f"xhr-document/{name}",
                                contract="Response Documents preserve requester origin and actual response URL.",
                                source="WHATWG XHR document response; DOM Document cloning; Fetch redirect Location URL",
                                commands=["Runtime.evaluate"], observed=observed)
            except Exception as error:
                failures.append(f"{name}: {error}")
    if failures:
        raise SmokeError("; ".join(failures))


_XHR = r"""
    const check = (actual, expected, label) => {
        if (actual !== expected) throw new Error(`${label}: ${actual} !== ${expected}`);
    };
    const load = (realm, url, async = true) => new Promise((resolve, reject) => {
        const xhr = new realm.XMLHttpRequest();
        xhr.open('GET', url, async);
        if (async) xhr.responseType = 'document';
        xhr.onload = () => resolve(xhr);
        xhr.onerror = () => reject(new Error(`XHR failed: ${url}`));
        try { xhr.send(); if (!async) resolve(xhr); } catch (e) { reject(e); }
    });
"""

_CLONE = "async port => {" + _XHR + r"""
    for (const kind of ['xml', 'html']) {
        const xhr = await load(window, `http://127.0.0.1:${port}/doc.${kind}`);
        const doc = xhr.responseXML;
        check(doc.domain, 'localhost', 'response origin');
        for (const deep of [false, true]) {
            const clone = doc.cloneNode(deep);
            check(clone.domain, 'localhost', `${kind} clone origin`);
            check(clone.URL, doc.URL, 'clone URL');
            check(clone.cloneNode(deep).domain, 'localhost', 'second clone origin');
        }
    }
    return 'ok';
}"""

_RETAINED = "async port => {" + _XHR + r"""
    for (const kind of ['xml', 'html']) {
        for (const retire of ['detach', 'navigate']) {
            const frame = document.createElement('iframe');
            document.body.append(frame);
            let step = 'create request';
            try {
                const realm = frame.contentWindow;
                const xhr = await load(realm, `/doc.${kind}`);
                const doc = xhr.responseXML;
                const clones = [doc, doc.cloneNode(false), doc.cloneNode(true)];
                step = 'retire frame';
                if (retire === 'detach') frame.remove();
                else await new Promise(resolve => {
                    frame.onload = resolve;
                    frame.src = '/replacement';
                });
                for (const retained of clones) {
                    step = 'read retained domain';
                    check(retained.domain, 'localhost', `${kind} ${retire} origin`);
                    step = 'clone retired document';
                    check(retained.cloneNode(false).domain, 'localhost', 'clone after retirement');
                }
            } catch (error) {
                throw new Error(`${kind} ${retire} ${step}: ${error.name}: ${error.message}`);
            } finally { frame.remove(); }
        }
    }
    return 'ok';
}"""

_REDIRECT = "async port => {" + _XHR + r"""
    for (const async of [false, true]) {
        for (const [target, fragment] of [
            ['/doc.xml', '#anchor'], ['/doc.xml#', '#'], ['/doc.xml#own', '#own'],
            ['/redirect?to=%2Fdoc.xml', '#anchor']
        ]) {
            const xhr = await load(window, `/redirect?to=${encodeURIComponent(target)}#anchor`, async);
            const expected = `${location.origin}/doc.xml${fragment}`;
            check(xhr.responseXML.URL, expected, `redirect ${async} ${target}`);
            check(xhr.responseXML.baseURI, expected, 'response base');
            check(xhr.responseURL, `${location.origin}/doc.xml`, 'fragment-free responseURL');
        }
    }
    return 'ok';
}"""

_SERVICE_WORKER = "async port => {" + _XHR + r"""
    const registration = await navigator.serviceWorker.register('/worker.js');
    try {
        await navigator.serviceWorker.ready;
        if (!navigator.serviceWorker.controller) await new Promise(resolve => {
            navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true});
        });
        const xhr = await load(window, '/sw/entry');
        const expected = `${location.origin}/nested/doc.xml#nested`;
        check(xhr.responseXML.URL, expected, 'worker actual response URL');
        check(xhr.responseURL, expected.split('#')[0], 'worker responseURL');
        check(xhr.responseXML.getElementById('link').href,
              `${location.origin}/nested/child`, 'worker relative link');
        return 'ok';
    } finally { await registration.unregister(); }
}"""
