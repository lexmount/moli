"""Child request ownership and Network/Page contracts, using held responses."""

from __future__ import annotations

import asyncio
import contextlib
import json
import threading
from collections import Counter
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Iterator
from urllib.parse import urlsplit

from ..assertions import SmokeError, assert_equal, record_contract
from ..raw_cdp import connect_raw_cdp
from .cdp_ordering import _Wire
from .protocol_progress import _page


@contextmanager
def _server() -> Iterator[tuple[str, dict[str, tuple[threading.Event, threading.Event]], Counter]]:
    gates = {path: (threading.Event(), threading.Event()) for path in (
        "/shared.js", "/slow", "/sw-document-gate", "/sw-script-gate",
    )}
    counts: Counter = Counter()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            path = urlsplit(self.path).path
            counts[path] += 1
            if path in gates:
                arrived, release = gates[path]
                arrived.set()
                release.wait()
            status, mime, headers = 200, "text/html", {}
            body = b"<!doctype html><title>child-network</title><body>loaded"
            if path in ("/shared.js", "/final.js", "/worker-script.js"):
                mime, body = "application/javascript", b"globalThis.scriptExecuted = 1;"
            if path in ("/shared.js", "/cached"):
                headers["Cache-Control"] = "public, max-age=3600"
            if path == "/redirect.js":
                status, headers["Location"] = 302, "/final.js"
                body = b""
            if path == "/empty":
                status, body = 204, b""
            if path == "/script-child":
                body = b"<!doctype html><script src='/worker-script.js'></script>"
            if path == "/worker.js":
                mime = "application/javascript"
                body = b"""
                    self.addEventListener('install', e => e.waitUntil(self.skipWaiting()));
                    self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));
                    self.addEventListener('fetch', e => {
                        const path = new URL(e.request.url).pathname;
                        if (path === '/worker-document' || path === '/worker-script.js') {
                            e.respondWith(fetch(path === '/worker-document'
                                ? '/sw-document-gate' : '/sw-script-gate').then(() => new Response(
                                path === '/worker-document' ? '<!doctype html><body>worker child'
                                    : 'globalThis.scriptExecuted = 1;',
                                {headers: {'Content-Type': path === '/worker-document'
                                    ? 'text/html' : 'application/javascript'}})));
                        }
                    });
                """
            try:
                self.send_response(status)
                self.send_header("Content-Type", mime)
                self.send_header("Content-Length", str(len(body)))
                for name, value in headers.items():
                    self.send_header(name, value)
                self.end_headers()
                self.wfile.write(body)
            except OSError:
                pass

        def log_message(self, *_args: Any) -> None:
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", gates, counts
    finally:
        for _arrived, release in gates.values():
            release.set()
        server.shutdown()
        thread.join(timeout=5)
        server.server_close()


async def _arrived(gates: dict, path: str) -> None:
    if not await asyncio.to_thread(gates[path][0].wait, 10):
        raise SmokeError(f"held request never arrived: {path}")


async def _eval(wire: _Wire, session: str, expression: str) -> Any:
    result = await wire.call("Runtime.evaluate", {
        "expression": expression, "returnByValue": True, "awaitPromise": True,
    }, session)
    if "exceptionDetails" in result:
        raise SmokeError(f"fixture script failed: {result!r}")
    return result.get("result", {}).get("value")


async def _frame(wire: _Wire, session: str, path: str, name: str = "child") -> str:
    start = len(wire.messages)
    await _eval(wire, session, f"""
        globalThis.{name} = document.createElement('iframe');
        {name}.loaded = new Promise(resolve => {name}.onload = resolve);
        {name}.src = {json.dumps(path)}; document.body.append({name}); void 0;
    """)
    event = await wire.until(lambda m: m.get("sessionId") == session
                            and m.get("method") == "Page.frameAttached",
                            "child attachment", after=start)
    return event["params"]["frameId"]


async def _request(wire: _Wire, session: str, url: str, *, after: int = 0) -> dict:
    return await wire.until(lambda m: m.get("sessionId") == session
                            and m.get("method") == "Network.requestWillBeSent"
                            and m["params"]["request"]["url"] == url,
                            f"request start {url}", after=after)


async def _terminal(wire: _Wire, request: dict, method: str | tuple[str, ...]) -> None:
    rid = request["params"]["requestId"]
    methods = (method,) if isinstance(method, str) else method
    await wire.until(lambda m: m.get("sessionId") == request.get("sessionId")
                     and m.get("method") in methods and m["params"].get("requestId") == rid,
                     f"request terminal {rid}")
    assert_equal(sum(m.get("method") in ("Network.loadingFinished", "Network.loadingFailed")
                     and m.get("sessionId") == request.get("sessionId")
                     and m.get("params", {}).get("requestId") == rid for m in wire.messages),
                 1, "exactly one request terminal")


async def _shared(wire: _Wire, session: str, url: str, gates: dict, counts: Counter) -> None:
    for name in ("first", "second"):
        await _eval(wire, session, f"""
            globalThis.{name} = document.createElement('iframe');
            {name}.loaded = new Promise(resolve => {name}.onload = resolve);
            {name}.srcdoc = '<script src="{url}/shared.js"></script>';
            document.body.append({name}); void 0;
        """)
        if name == "first":
            await _arrived(gates, "/shared.js")
            first = await _request(wire, session, url + "/shared.js")
        else:
            await wire.until(lambda m: m.get("method") == "Network.requestWillBeSent"
                             and m["params"]["request"]["url"] == url + "/shared.js"
                             and m["params"]["frameId"] != first["params"]["frameId"],
                             "second consumer is waiting for shared script")
    await _eval(wire, session, "first.remove()")
    await _terminal(wire, first, "Network.loadingFailed")
    gates["/shared.js"][1].set()
    assert_equal(await _eval(wire, session,
        "second.loaded.then(() => second.contentWindow.scriptExecuted)"), 1,
        "surviving shared-cache consumer executes")
    completed_transfers = counts["/shared.js"]
    await _eval(wire, session, f"""
        globalThis.third = document.createElement('iframe');
        third.loaded = new Promise(resolve => third.onload = resolve);
        third.srcdoc = '<script src="{url}/shared.js"></script>';
        document.body.append(third); void 0;
    """)
    assert_equal(await _eval(wire, session,
        "third.loaded.then(() => third.contentWindow.scriptExecuted)"), 1,
        "later cache consumer executes")
    assert_equal(counts["/shared.js"], completed_transfers, "later consumer uses completed cache")


async def _superseded(wire: _Wire, session: str, url: str, gates: dict, _counts: Counter) -> None:
    frame = await _frame(wire, session, url + "/initial")
    await _eval(wire, session, "child.loaded.then(() => true)")
    start = len(wire.messages)
    first = await wire.send("Page.navigate", {"frameId": frame, "url": url + "/slow"}, session)
    await _arrived(gates, "/slow")
    second = await wire.send("Page.navigate", {"frameId": frame, "url": url + "/second"}, session)
    response = await wire.response(second)
    second_loader = response["result"]["loaderId"]
    first_response = await wire.response(first, allow_error=True)
    assert_equal(bool(first_response.get("error") or first_response.get("result", {}).get("errorText")),
                 True, "superseded navigation has a cancellation terminal")
    await wire.until(lambda m: m.get("method") == "Page.frameNavigated"
                     and m["params"]["frame"]["id"] == frame
                     and m["params"]["frame"]["loaderId"] == second_loader,
                     "second navigation commits", after=start)
    assert_equal([m["params"]["frame"]["url"] for m in wire.messages[start:]
                  if m.get("method") == "Page.frameNavigated" and m["params"]["frame"]["id"] == frame],
                 [url + "/second"], "only winning navigation commits")


async def _worker(wire: _Wire, session: str) -> None:
    assert_equal(await _eval(wire, session, """
        (async () => {
            await navigator.serviceWorker.register('/worker.js');
            await navigator.serviceWorker.ready;
            if (!navigator.serviceWorker.controller) await new Promise(resolve =>
                navigator.serviceWorker.addEventListener('controllerchange', resolve, {once:true}));
            return true;
        })()
    """), True, "Service Worker controls parent")


async def _deep_navigation(wire: _Wire, session: str, url: str, _gates: dict, _counts: Counter) -> None:
    start = len(wire.messages)
    await _eval(wire, session, """
        let doc = document;
        for (let depth = 0; depth < 11; depth++) {
            const frame = doc.createElement('iframe');
            doc.body.append(frame);
            globalThis.deepChild = frame;
            doc = frame.contentDocument;
        }
        void 0;
    """)
    def attached() -> list[dict]:
        return [m for m in wire.messages[start:] if m.get("method") == "Page.frameAttached"]
    await wire.until(lambda _m: len(attached()) == 11, "deep child attachments", after=start)
    frame = attached()[-1]["params"]["frameId"]
    navigation = await wire.call("Page.navigate", {"frameId": frame, "url": url + "/deep"}, session)
    assert_equal(navigation.get("errorText"), None, "deep frame navigation succeeds")
    await wire.until(lambda m: m.get("method") == "Page.frameNavigated"
                     and m["params"]["frame"]["id"] == frame
                     and m["params"]["frame"]["loaderId"] == navigation["loaderId"],
                     "deep frame commit matches its command", after=start)


async def _worker_document(wire: _Wire, session: str, url: str, gates: dict, _counts: Counter) -> None:
    await _worker(wire, session)
    frame = await _frame(wire, session, url + "/worker-document")
    await _arrived(gates, "/sw-document-gate")
    request = await _request(wire, session, url + "/worker-document")
    await _eval(wire, session, "child.remove()")
    await wire.until(lambda m: m.get("method") == "Page.frameDetached"
                     and m["params"]["frameId"] == frame, "worker child detached")
    await _terminal(wire, request, "Network.loadingFailed")
    assert_equal(gates["/sw-document-gate"][1].is_set(), False,
                 "detach terminates request before worker response")


async def _worker_script(wire: _Wire, session: str, url: str, gates: dict, _counts: Counter) -> None:
    await _worker(wire, session)
    await wire.call("Fetch.enable", {"patterns": [{"resourceType": "Script"}]}, session)
    await _frame(wire, session, url + "/script-child")
    await _arrived(gates, "/sw-script-gate")
    request = await _request(wire, session, url + "/worker-script.js")
    assert_equal(gates["/sw-script-gate"][1].is_set(), False,
                 "Script start is visible before Service Worker response")
    gates["/sw-script-gate"][1].set()
    await _terminal(wire, request, ("Network.loadingFinished", "Network.loadingFailed"))
    assert_equal(await _eval(wire, session,
        "child.loaded.then(() => child.contentWindow.scriptExecuted)"), 1, "worker script executes")


async def _empty(wire: _Wire, session: str, url: str, _gates: dict, _counts: Counter) -> None:
    frame = await _frame(wire, session, url + "/initial")
    await _eval(wire, session, "child.loaded.then(() => true)")
    start = len(wire.messages)
    navigation = await wire.send("Page.navigate", {"frameId": frame, "url": url + "/empty"}, session)
    request = await _request(wire, session, url + "/empty", after=start)
    await _terminal(wire, request, ("Network.loadingFinished", "Network.loadingFailed"))
    await wire.until(lambda m: m.get("method") == "Page.frameStoppedLoading"
                     and m["params"]["frameId"] == frame, "204 stops child loading", after=start)
    await wire.response(navigation, allow_error=True)
    assert_equal(await _eval(wire, session, "child.contentWindow.location.pathname"),
                 "/initial", "204 retains existing Document")


async def _cache(wire: _Wire, session: str, url: str, _gates: dict, counts: Counter) -> None:
    frame = await _frame(wire, session, url + "/cached")
    await _eval(wire, session, "child.loaded.then(() => true)")
    start = len(wire.messages)
    await wire.call("Page.navigate", {"frameId": frame, "url": url + "/initial"}, session)
    nav = await wire.call("Page.navigate", {"frameId": frame, "url": url + "/cached"}, session)
    request = await _request(wire, session, url + "/cached", after=start)
    await _terminal(wire, request, "Network.loadingFinished")
    rid = request["params"]["requestId"]
    response = await wire.until(lambda m: m.get("method") == "Network.responseReceived"
        and m["params"]["requestId"] == rid, "cached response", after=start)
    assert_equal(response["params"]["response"].get("fromDiskCache"), True,
                 "HTTP cache provenance survives native observations")
    assert_equal(counts["/cached"], 1, "second navigation avoids HTTP")
    assert_equal(nav.get("errorText"), None, "cached navigation succeeds")


async def _redirect(wire: _Wire, session: str, url: str, _gates: dict, _counts: Counter) -> None:
    await wire.call("Fetch.enable", {"patterns": [{"resourceType": "Script"}]}, session)
    await _eval(wire, session, f"""
        globalThis.child = document.createElement('iframe');
        child.srcdoc = '<script src="{url}/redirect.js"></script>';
        document.body.append(child); void 0;
    """)
    first = await wire.until(lambda m: m.get("method") == "Fetch.requestPaused"
        and m["params"]["request"]["url"] == url + "/redirect.js", "first script hop")
    await wire.call("Fetch.continueRequest", {"requestId": first["params"]["requestId"]}, session)
    second = await wire.until(lambda m: m.get("method") == "Fetch.requestPaused"
        and m["params"]["request"]["url"] == url + "/final.js", "second script hop")
    assert_equal(second["params"].get("redirectedRequestId"), first["params"]["requestId"],
                 "redirected script pause identifies its previous hop")
    await wire.call("Fetch.continueRequest", {"requestId": second["params"]["requestId"]}, session)
    request = await _request(wire, session, url + "/redirect.js")
    await _terminal(wire, request, "Network.loadingFinished")
    starts = [message for message in wire.messages
              if message.get("method") == "Network.requestWillBeSent"
              and message["params"]["requestId"] == request["params"]["requestId"]]
    assert_equal([message["params"]["request"]["url"] for message in starts],
                 [url + "/redirect.js", url + "/final.js"], "one Network start per HTTP hop")


async def _resources(wire: _Wire, session: str, url: str, _gates: dict, _counts: Counter) -> None:
    frame = await _frame(wire, session, url + "/initial")
    await _eval(wire, session, "child.loaded.then(() => true)")
    for path in ("/second", "/third"):
        response = await wire.call("Page.navigate", {"frameId": frame, "url": url + path}, session)
        await wire.until(lambda m: m.get("method") == "Page.lifecycleEvent"
                         and m["params"].get("loaderId") == response["loaderId"]
                         and m["params"]["name"] == "load", "child navigation loaded")
    tree = await wire.call("Page.getResourceTree", session=session)
    children = tree["frameTree"].get("childFrames", [])
    child = next(item for item in children if item["frame"]["id"] == frame)
    assert_equal([item for item in child.get("resources", []) if item["type"] == "Document"],
                 [], "historical child Documents are not current subresources")


async def run_child_network_group(endpoint: str, _fixture: str, results: list[dict[str, Any]]) -> None:
    failures = []
    for name, scenario in (
        ("shared_script_owner_detach", _shared),
        ("superseded_navigation_identity", _superseded),
        ("deep_navigation_identity", _deep_navigation),
        ("service_worker_document_detach", _worker_document),
        ("service_worker_script_start", _worker_script),
        ("no_content_stops_loading", _empty),
        ("cached_document_provenance", _cache),
        ("intercepted_script_redirect_identity", _redirect),
        ("resource_tree_excludes_historical_documents", _resources),
    ):
        client = await connect_raw_cdp(endpoint)
        wire, context = _Wire(client, "child-network/" + name), None
        try:
            with _server() as (url, gates, counts):
                context = (await wire.call("Target.createBrowserContext"))["browserContextId"]
                _target, session = await _page(wire, context, url)
                async with asyncio.timeout(20):
                    await scenario(wire, session, url, gates, counts)
                # A redirect keeps its Network id, but every subsequent start
                # for that id must describe a new hop, never repeat admission.
                seen_requests = set()
                for message in wire.messages:
                    if message.get("method") != "Network.requestWillBeSent":
                        continue
                    params = message["params"]
                    key = (message.get("sessionId"), params["requestId"])
                    if key in seen_requests:
                        assert_equal(bool(params.get("redirectResponse")), True,
                                     "repeated Network start must be a redirect hop")
                    seen_requests.add(key)
            await wire.call("Target.disposeBrowserContext", {"browserContextId": context})
            context = None
            wire.assert_all_replied()
            record_contract(results, "child_network/" + name,
                contract="Child navigation and shared transport ownership survive independent cancellation.",
                source="Raw CDP browser comparison with gated HTTP/Service Worker responses.",
                commands=list(dict.fromkeys(method for method, _session in wire.requests.values())),
                observed={"wire": wire.trace})
        except Exception as error:
            results.append({"name": "child_network/" + name, "ok": False,
                            "error": str(error), "wire": wire.trace})
            failures.append(f"{name}: {error}")
        finally:
            if context:
                with contextlib.suppress(Exception):
                    async with asyncio.timeout(3):
                        await wire.call("Target.disposeBrowserContext", {"browserContextId": context})
            await client.websocket.close()
    if failures:
        raise SmokeError("; ".join(failures))
