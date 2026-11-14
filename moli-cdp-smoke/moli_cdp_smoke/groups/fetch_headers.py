from __future__ import annotations

import asyncio
import json
from typing import Any
from urllib.parse import parse_qs, urlencode, urlsplit

from ..assertions import assert_equal, record_contract, wait_until
from ..diagnostics import new_cdp_session
from ..fixture import FixtureServer
from ..helpers import attach_cdp_event_collector


def _url(origin: str, route: str, case: str, *, hop: str = "final", target: str = "", status: int = 302) -> str:
    return origin + "/fetch-headers/" + route + "?" + urlencode({"case": case, "hop": hop, "to": target, "status": status})


async def run_fetch_headers_group(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    peer, escape = FixtureServer(), FixtureServer()
    peer.start()
    escape.start()
    try:
        cases = [(f"replay-{status}", status) for status in (301, 302, 303, 307, 308)]
        cases += [(case, 302) for case in ("noop-method", "noop-url", "rewrite-url", "empty-postdata", "new-content-type", "chain-replay",
                                          "drop-then-replay", "fresh-then-replay", "origin-echo", "authorization")]
        for api in ("fetch", "xhr", "sync-xhr"):
            for case, status in cases:
                await _case(browser, fixture, peer, escape.url, api, case, status, results)
    finally:
        peer.stop()
        escape.stop()


async def _case(browser: Any, source: str, peer: FixtureServer, escape: str,
                api: str, case: str, status: int, results: list[dict[str, Any]]) -> None:
    label = f"{api}-{case}"
    context = await browser.new_context()
    operation = None
    pending_pause = None
    try:
        page = await context.new_page()
        await page.goto(source + "/plain", wait_until="load", timeout=10_000)
        cdp = await new_cdp_session(context, page)
        await cdp.send("Network.enable")
        await cdp.send("Fetch.enable", {"patterns": [
            {"urlPattern": "*fetch-headers/*", "resourceType": resource}
            for resource in ("Fetch", "XHR")
        ]})
        queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue()
        cdp.on("Fetch.requestPaused", queue.put_nowait)
        pauses = []
        preflights = []
        original = None
        target = _url(source, "echo", label)
        next_url = target
        if case in ("chain-replay", "drop-then-replay", "fresh-then-replay"):
            next_url = _url(source, "redirect", label, hop="middle", target=target)
        peer_url = _url(peer.url, "redirect", label, hop="authorized", target=_url(escape, "echo", label))
        url = _url(source, "redirect", label, hop="source", target=next_url, status=status)
        if case == "origin-echo":
            url = _url(peer.url, "echo", label)
        elif case == "authorization":
            url = _url(source, "redirect", label, hop="source", target=peer_url)
        method = "GET" if case == "authorization" else "POST"
        author_headers = {"Authorization": "author-secret"} if case == "authorization" else {"Content-Type": "text/plain", "X-Author": "kept"}
        body = "undefined" if method == "GET" else json.dumps("abc")
        if api == "fetch":
            script = f"fetch({json.dumps(url)},{{method:{json.dumps(method)},headers:{json.dumps(author_headers)},body:{body}}}).then(r=>r.text())"
        else:
            script = f"""new Promise((resolve,reject)=>{{const x=new XMLHttpRequest();
              x.open({json.dumps(method)},{json.dumps(url)},{str(api != 'sync-xhr').lower()});
              for(const [k,v] of Object.entries({json.dumps(author_headers)}))x.setRequestHeader(k,v);
              x.onload=()=>resolve(x.responseText);x.onerror=()=>reject(new Error('XHR failed'));x.send({body});}})"""
        with attach_cdp_event_collector(cdp, ["Network.requestWillBeSent", "Network.responseReceived",
                                              "Network.loadingFinished", "Network.loadingFailed"]) as events:
            operation = asyncio.create_task(page.evaluate(script))
            async with asyncio.timeout(10):
                while True:
                    pending_pause = asyncio.create_task(queue.get())
                    done, _ = await asyncio.wait((operation, pending_pause), return_when=asyncio.FIRST_COMPLETED)
                    if operation in done:
                        pending_pause.cancel()
                        await asyncio.gather(pending_pause, return_exceptions=True)
                        break
                    pause = pending_pause.result()
                    pending_pause = None
                    if pause["request"]["method"] == "OPTIONS":
                        # Chromium also labels intercepted preflights as XHR.
                        # They have independent Network IDs, outside this chain.
                        preflights.append(pause)
                        await cdp.send("Fetch.continueRequest", {"requestId": pause["requestId"]})
                        continue
                    pauses.append(pause)
                    query = parse_qs(urlsplit(pause["request"]["url"]).query)
                    hop = query["hop"][0]
                    headers = None
                    params = {"requestId": pause["requestId"]}
                    if hop == "source" and case != "authorization":
                        original = dict(pause["request"]["headers"])
                    elif case == "origin-echo":
                        headers = dict(pause["request"]["headers"])
                        origin = next((value for name, value in headers.items() if name.lower() == "origin"), None)
                        assert_equal(origin, source, label + " visible generated Origin")
                    elif case == "authorization":
                        if hop == "authorized":
                            headers = {"Authorization": "interceptor-secret"}
                    else:
                        headers = dict(original)
                        if case == "noop-method":
                            params["method"] = pause["request"]["method"]
                        if case in ("noop-url", "rewrite-url"):
                            # Chromium restarts the transport whenever url is
                            # supplied, including an unchanged URL. Headers now
                            # apply to that new request rather than FollowRedirect.
                            params["url"] = pause["request"]["url"]
                            if case == "rewrite-url":
                                params["url"] = params["url"].replace("/fetch-headers/echo?", "/fetch-headers/rewritten?")
                        if case == "empty-postdata":
                            params["postData"] = ""
                        if case == "new-content-type" or (case == "fresh-then-replay" and hop == "middle"):
                            headers = {name: value for name, value in headers.items() if name.lower() != "content-type"}
                            headers["Content-Type"] = "application/x-new"
                        if case == "drop-then-replay" and hop == "middle":
                            headers = None
                    if headers is not None:
                        params["headers"] = [{"name": name, "value": value} for name, value in headers.items()]
                    await cdp.send("Fetch.continueRequest", params)
                value = json.loads(await operation)
            wire = value["headers"]
            restarted = case in ("noop-url", "rewrite-url")
            keep_body = status in (307, 308) or case == "origin-echo"
            expected_type = "text/plain" if keep_body or restarted or case in ("drop-then-replay", "fresh-then-replay") else None
            if case == "new-content-type":
                expected_type = "application/x-new"
            assert_equal(value["method"], "POST" if keep_body else "GET", label + " wire method")
            assert_equal(value["body"], "abc" if keep_body else "", label + " wire body")
            assert_equal(wire.get("content-type"), expected_type, label + " wire Content-Type")
            assert_equal(wire.get("content-length"), "3" if keep_body else None, label + " wire Content-Length")
            if restarted:
                assert_equal(value["path"], "/fetch-headers/rewritten" if case == "rewrite-url" else "/fetch-headers/echo",
                             label + " actual overridden request path")
                assert_equal(wire.get("origin"), source, label + " restarted request accepts explicit Origin")
            if case == "authorization":
                peer_path = urlsplit(peer_url).path + "?" + urlsplit(peer_url).query
                assert_equal(peer.resource_request_headers(peer_path).get("authorization"), "interceptor-secret",
                             label + " explicit second-hop Authorization")
                assert_equal(wire.get("authorization"), None, label + " third origin has no Authorization")
            else:
                assert_equal(wire.get("x-author"), "kept", label + " retained custom header")
                if not keep_body and not restarted:
                    assert_equal(wire.get("origin"), None, label + " no replayed POST Origin")
            ids = {pause.get("networkId") for pause in pauses}
            assert_equal(len(ids), 1, label + " stable Network identity")
            request_id = next(iter(ids))
            if not request_id:
                raise AssertionError(label + " missing Network identity")
            await wait_until(lambda: any(event["method"] in ("Network.loadingFinished", "Network.loadingFailed")
                                         and event["params"].get("requestId") == request_id for event in events),
                             label + " Network terminal")
            terminals = [event["method"] for event in events
                         if event["method"] in ("Network.loadingFinished", "Network.loadingFailed")
                         and event["params"].get("requestId") == request_id]
            assert_equal(terminals, ["Network.loadingFinished"], label + " one terminal")
            record_contract(results, "intercepted_headers_" + label,
                            contract="A redirect removes replayed upload headers; a supplied URL restarts header application. Fresh overrides and single-hop Authorization remain supported.",
                            source="Executable Chromium CDP oracle",
                            commands=["Fetch.enable", "Fetch.continueRequest"],
                            observed={"api": api, "case": case, "method": value["method"], "contentType": expected_type,
                                      "pauses": len(pauses), "preflightPauses": len(preflights), "terminals": terminals})
    finally:
        for task in (pending_pause, operation):
            if task is not None and not task.done():
                task.cancel()
                await asyncio.gather(task, return_exceptions=True)
        await context.close()
