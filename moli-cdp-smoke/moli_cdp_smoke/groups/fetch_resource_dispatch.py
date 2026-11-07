from __future__ import annotations

import asyncio
import base64
import urllib.parse
from typing import Any

from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..config import REPO_ROOT, is_moli_browser
from ..diagnostics import new_cdp_session
from ..fixture import FETCH_DISPATCH_PNG, FETCH_MEDIA_CAPTIONS, FETCH_MEDIA_WAV
from ..helpers import attach_cdp_event_collector
from ..png_image import decode_png


async def run_fetch_resource_dispatch_group(
    browser: Any, fixture: str, results: list[dict[str, Any]],
) -> None:
    # The origin deliberately returns 404 for worker-owned URLs: a successful
    # load with zero origin requests proves that continuation retained SW routing.
    for kind in ("font", "css-font", "css-image", "audio", "video", "track"):
        for admission in ("direct", "unmatched", "request"):
            await _resource_case(browser, fixture, results, kind, admission, worker=True)
    for kind in ("font", "track"):
        await _resource_case(browser, fixture, results, kind, "response", worker=True)
    for admission in ("direct", "unmatched", "request", "response"):
        await _resource_case(browser, fixture, results, "css-image", admission)
        await _resource_case(browser, fixture, results, "font", admission, worker=True, cross_origin=True)
    for cross_origin in (False, True):
        for policy in ("headers-replaced", "headers-empty"):
            await _resource_case(browser, fixture, results, "font", "request", policy=policy, cross_origin=cross_origin)
    if is_moli_browser():
        await _resource_case(browser, fixture, results, "font", "override", worker=True)
    for kind in ("font", "css-image", "audio", "track"):
        for admission in ("unmatched", "request"):
            await _resource_case(browser, fixture, results, kind, admission, policy="headers")
    for admission in ("unmatched", "request"):
        await _resource_case(browser, fixture, results, "font", admission, policy="blocked")
    for admission in ("unmatched", "request"):
        await _resource_case(browser, fixture, results, "font", admission, policy="cache")
    await _resource_case(browser, fixture, results, "font", "request", policy="auth")
    for admission in ("unmatched", "request"):
        for worker in (False, True):
            await _resource_case(browser, fixture, results, "font", admission, policy="offline", worker=worker)
    for policy in ("headers", "blocked", "redirect"):
        await _resource_case(browser, fixture, results, "font", "request", policy=policy, worker=True)


async def _resource_case(
    browser: Any, fixture: str, results: list[dict[str, Any]], kind: str,
    admission: str, *, worker: bool = False, policy: str | None = None,
    cross_origin: bool = False,
) -> None:
    case = f"{kind}-{admission}-{policy or ('worker' if worker else 'network')}" + ("-controlled" if worker and policy == "offline" else "-fallback" if worker and policy else "")
    if cross_origin:
        case += "-cross-origin"
    served_by_worker = worker and policy in (None, "offline")
    name = ("font.woff2" if kind in ("font", "css-font") else "image.png"
            if kind == "css-image" else "captions.vtt" if kind == "track" else "tone.wav")
    route = "sw" if served_by_worker else "cache" if policy == "cache" else "auth" if policy == "auth" else "network"
    if policy == "redirect":
        name = "redirect-font.woff2"
    url = f"{fixture}/fetch-dispatch/{route}/{name}?case={case}"
    if cross_origin:
        url = url.replace("127.0.0.1", "localhost")
    continued_url = url.replace("font.woff2", "continued.woff2") if admission == "override" else url
    if policy == "redirect":
        continued_url = url.replace("redirect-font.woff2", "font.woff2")
    if admission == "override":
        url = url.replace("font.woff2", "missing.woff2")
    fetch_type = "Font" if kind in ("font", "css-font") else "Image" if kind == "css-image" else "Media"
    expected_body = ((REPO_ROOT / "moli-layout/tests/fixtures/moli-ahem.woff2").read_bytes()
                     if fetch_type == "Font" else FETCH_DISPATCH_PNG if fetch_type == "Image"
                     else FETCH_MEDIA_CAPTIONS if kind == "track" else FETCH_MEDIA_WAV)
    context = await browser.new_context()
    screenshot: asyncio.Task[Any] | None = None
    try:
        page = await context.new_page()
        cdp = await new_cdp_session(context, page)
        await page.goto(f"{fixture}/fetch-dispatch/page", wait_until="load", timeout=10_000)
        if worker:
            await page.evaluate("""() => {
              globalThis.__workerReady = false;
              navigator.serviceWorker.register('/fetch-dispatch/sw.js', {scope:'/fetch-dispatch/'})
                .then(() => navigator.serviceWorker.ready)
                .then(() => { globalThis.__workerReady = true; });
            }""")
            await wait_until(lambda: page.evaluate("__workerReady && !!navigator.serviceWorker.controller"),
                             f"{case} controlling service worker")
        await cdp.send("Network.enable")
        if policy in ("headers", "headers-replaced", "headers-empty", "auth", "redirect"):
            await cdp.send("Network.setExtraHTTPHeaders", {"headers": {"X-Fetch-Policy": case}})
        elif policy == "blocked":
            await cdp.send("Network.setBlockedURLs", {"urls": ["*" + url.split(fixture, 1)[1]]})
        elif policy == "offline":
            await cdp.send("Network.emulateNetworkConditions", {"offline":True, "latency":0,
                           "downloadThroughput":-1, "uploadThroughput":-1})
        elif policy == "cache":
            await cdp.send("Network.setCacheDisabled", {"cacheDisabled": True})
        if admission != "direct":
            pattern = "*never-match-resource*" if admission == "unmatched" else "*fetch-dispatch/*"
            await cdp.send("Fetch.enable", {"patterns": [{"resourceType": fetch_type,
                            "urlPattern": pattern, "requestStage": "Response" if admission == "response" else "Request"}],
                                            "handleAuthRequests": policy == "auth"})
        with attach_cdp_event_collector(cdp, [
            "Fetch.requestPaused", "Fetch.authRequired", "Network.requestWillBeSent", "Network.responseReceived",
            "Network.loadingFinished", "Network.loadingFailed",
        ]) as events:
            async def start_resource() -> None:
                nonlocal screenshot
                await page.evaluate("""({kind, url}) => {
                  const probe = globalThis.__dispatchResource = {result:'pending'};
                  if (kind === 'font') {
                    const font = new FontFace('DispatchFont', `url("${url}")`);
                    document.fonts.add(font);
                    probe.element = font;
                    font.load().then(() => { probe.result='loaded'; }, () => { probe.result='error'; });
                  } else if (kind === 'css-font' || kind === 'css-image') {
                    const style = document.createElement('style');
                    style.textContent = kind === 'css-font'
                      ? `@font-face{font-family:DispatchPaint;src:url("${url}")}body{font-family:DispatchPaint}`
                      : `body{background-image:url("${url}")}`;
                    document.head.append(style);
                    document.body.textContent = 'paint-demanded resource';
                  } else if (kind === 'track') {
                    const media = document.createElement('video');
                    const track = document.createElement('track');
                    probe.element = track;
                    track.kind = 'captions';
                    track.onload = () => { probe.result='loaded'; };
                    track.onerror = () => { probe.result='error'; };
                    track.src = url;
                    media.append(track); document.body.append(media);
                    track.track.mode = 'hidden';
                  } else {
                    const media = document.createElement(kind);
                    probe.element = media;
                    media.preload = 'auto';
                    media.onloadedmetadata = () => { probe.result='loaded'; };
                    media.onerror = () => { probe.result='error'; };
                    document.body.append(media); media.src = url;
                  }
                }""", {"kind": kind, "url": url})
                if kind.startswith("css-"):
                    screenshot = asyncio.create_task(cdp.send("Page.captureScreenshot", {"format": "png"}))

            handled: set[str] = set()
            response_bodies = 0
            auth_requests: set[str] = set()

            def resource_requests() -> list[dict[str, Any]]:
                return [event["params"] for event in events
                        if event["method"] == "Network.requestWillBeSent"
                        and event["params"].get("type") == ("TextTrack" if kind == "track" else fetch_type)
                        and event["params"]["request"]["url"] in (url, continued_url)]

            def terminals() -> list[str]:
                ids = {request["requestId"] for request in resource_requests()}
                return [event["method"] for event in events
                        if event["method"] in ("Network.loadingFinished", "Network.loadingFailed")
                        and event["params"].get("requestId") in ids]

            async def settle() -> bool:
                nonlocal response_bodies
                for event in list(events):
                    if event["method"] == "Fetch.authRequired":
                        auth = event["params"]
                        if auth["requestId"] not in auth_requests:
                            assert_equal(policy, "auth", f"{case} expected authentication")
                            await cdp.send("Fetch.continueWithAuth", {"requestId":auth["requestId"],
                                           "authChallengeResponse":{"response":"ProvideCredentials",
                                                                    "username":"font", "password":"secret"}})
                            auth_requests.add(auth["requestId"])
                        continue
                    if event["method"] != "Fetch.requestPaused":
                        continue
                    pause = event["params"]
                    if pause["requestId"] in handled:
                        continue
                    assert_equal(admission in ("request", "response", "override"), True,
                                 f"{case} only matching filters expose pauses")
                    assert_equal(pause["resourceType"], fetch_type, f"{case} pause type")
                    if pause["request"]["url"] not in (url, continued_url):
                        raise SmokeError(f"{case} unexpected pause URL: {pause!r}")
                    params: dict[str, Any] = {"requestId": pause["requestId"]}
                    if admission == "response":
                        if pause.get("responseStatusCode") == 200:
                            body = await cdp.send("Fetch.getResponseBody", params)
                            data = base64.b64decode(body["body"]) if body.get("base64Encoded") else body["body"].encode()
                            assert_equal(data, expected_body, f"{case} SW response body")
                            response_bodies += 1
                        await cdp.send("Fetch.continueResponse", params)
                    else:
                        if admission == "override":
                            params.update({"url": continued_url, "method":"POST",
                                           "postData":base64.b64encode(b"font override").decode("ascii"),
                                           "headers":[{"name":"X-Fetch-Override", "value":"kept"}]})
                        if policy == "redirect" and pause["request"]["url"] == url:
                            params["headers"] = [{"name":"X-Fetch-Hop", "value":"initial-only"}]
                        if policy in ("headers-replaced", "headers-empty"):
                            params["headers"] = ([] if policy == "headers-empty" else
                                                 [{"name":"X-Fetch-Hop", "value":"only-header"}])
                        await cdp.send("Fetch.continueRequest", params)
                    handled.add(pause["requestId"])
                if screenshot is not None:
                    if not screenshot.done():
                        return False
                    screenshot.result()
                if not terminals():
                    return False
                if kind == "css-font":
                    return await page.evaluate("['loaded','error'].includes([...document.fonts].find(f => f.family === 'DispatchPaint').status)")
                if kind == "css-image":
                    return True
                return await page.evaluate("__dispatchResource.result !== 'pending'")

            await start_resource()
            await wait_until(settle, f"{case} resource completion")
            failed = policy == "blocked" or (policy == "offline" and not served_by_worker)
            expected_terminal = "Network.loadingFailed" if failed else "Network.loadingFinished"
            failures = [event["params"] for event in events if event["method"] == "Network.loadingFailed"]
            assert_equal(terminals(), [expected_terminal], f"{case} single network terminal ({failures!r})")
            if not kind.startswith("css-"):
                assert_equal(await page.evaluate("__dispatchResource.result"),
                             "error" if failed else "loaded", f"{case} resource result")
            elif kind == "css-font":
                assert_equal(await page.evaluate("[...document.fonts].find(f => f.family === 'DispatchPaint').status"),
                             "loaded", f"{case} CSS font decoded")
            elif kind == "css-image":
                request_id = resource_requests()[-1]["requestId"]
                body = await cdp.send("Network.getResponseBody", {"requestId": request_id})
                data = base64.b64decode(body["body"]) if body.get("base64Encoded") else body["body"].encode()
                assert_equal(data, expected_body, f"{case} complete image response body")
                painted = await cdp.send("Page.captureScreenshot", {"format": "png"})
                background_pixel = decode_png(base64.b64decode(painted["data"])).pixel(5, 5)
                assert_equal(background_pixel,
                             (255, 0, 0, 255), f"{case} decoded background image pixels")
            # Chromium serves controlled resources before its network-service
            # Fetch hook. Moli also supports pausing before SW dispatch, and
            # must preserve that dispatch when the client resumes the request.
            needs_pause = admission in ("request", "response", "override") and ((not served_by_worker and policy not in ("blocked", "offline")) or is_moli_browser())
            if needs_pause and not handled:
                raise SmokeError(f"{case} did not exercise interception")
            if admission == "response" and needs_pause:
                assert_equal(response_bodies, 1, f"{case} SW body exposed once")
            resource_path = urllib.parse.urlsplit(continued_url)
            status_url = f"{fixture}/fetch-dispatch/status?path=" + urllib.parse.quote(resource_path.path + "?" + resource_path.query, safe='')
            async def status() -> dict[str, Any]:
                return await page.evaluate("url => fetch(url).then(r => r.json())", status_url)
            if policy == "cache":
                # A fresh Document avoids FontFace's per-document cache. The
                # disabled HTTP cache must still send the same URL to origin.
                await page.goto(f"{fixture}/fetch-dispatch/page", wait_until="load", timeout=10_000)
                previous_terminals = len(terminals())
                await start_resource()
                async def settle_again() -> bool:
                    return await settle() and len(terminals()) > previous_terminals
                await wait_until(settle_again, f"{case} cache bypass reload")
            if policy == "offline":
                await cdp.send("Network.emulateNetworkConditions", {"offline":False, "latency":0,
                               "downloadThroughput":-1, "uploadThroughput":-1})
            observed = await status()
            assert_equal(observed["requests"], 0 if served_by_worker or policy in ("blocked", "offline") else 2 if policy in ("cache", "auth") else 1,
                         f"{case} origin request count")
            if policy in ("headers", "auth", "redirect"):
                assert_equal(observed["headers"].get("x-fetch-policy"), case, f"{case} page headers retained")
            if policy == "redirect":
                assert_equal(observed["headers"].get("x-fetch-hop"), None,
                             f"{case} current-request override stops at redirect")
                initial = await page.evaluate("url => fetch(url).then(r => r.json())",
                                             f"{fixture}/fetch-dispatch/status?path=" + urllib.parse.quote(url.split(fixture, 1)[1], safe=''))
                assert_equal(initial["requests"], 1, f"{case} initial redirect dispatch count")
                assert_equal(initial["headers"].get("x-fetch-hop"), "initial-only", f"{case} initial header override")
                assert_equal(initial["headers"].get("x-fetch-policy"), None,
                             f"{case} omitted page header stays absent on initial hop")
            if policy in ("headers-replaced", "headers-empty"):
                assert_equal(observed["headers"].get("x-fetch-policy"), None,
                             f"{case} explicit header list replaces page defaults")
                assert_equal(observed["headers"].get("x-fetch-hop"),
                             None if policy == "headers-empty" else "only-header",
                             f"{case} explicit header list reaches origin")
            if policy == "auth":
                assert_equal(len(auth_requests), 1, f"{case} one authentication challenge")
            if policy == "cache":
                assert_equal(observed["headers"].get("cache-control"), "no-cache", f"{case} cache disable policy retained")
            record_contract(results, f"fetch_dispatch_{case.replace('-', '_')}",
                            contract="Direct and resumed resource loads preserve SW routing and page network policy.",
                            source="Executable Chromium CDP oracle; origin counts and real resource completion",
                            commands=["Network.enable"] + (["Fetch.enable"] if admission != "direct" else [])
                                     + (["Fetch.continueResponse", "Fetch.getResponseBody"] if admission == "response" and handled
                                        else ["Fetch.continueRequest"] if handled else [])
                                     + ([{"headers": "Network.setExtraHTTPHeaders", "blocked": "Network.setBlockedURLs",
                                          "headers-replaced": "Network.setExtraHTTPHeaders", "headers-empty": "Network.setExtraHTTPHeaders",
                                          "cache": "Network.setCacheDisabled", "auth": "Fetch.continueWithAuth", "redirect": "Network.setExtraHTTPHeaders", "offline": "Network.emulateNetworkConditions"}[policy]] if policy else []),
                            observed={"originRequests":observed["requests"], "pauses":len(handled),
                                      "responseBodies": response_bodies, "crossOrigin": cross_origin,
                                      "imageBodyBytes": len(data) if kind == "css-image" else None,
                                      "backgroundPixel": background_pixel if kind == "css-image" else None,
                                      "terminals":terminals(), "headers":observed["headers"]})
    finally:
        if screenshot is not None:
            screenshot.cancel()
            await asyncio.gather(screenshot, return_exceptions=True)
        await context.close()
