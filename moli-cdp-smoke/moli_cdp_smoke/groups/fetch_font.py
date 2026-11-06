from __future__ import annotations

import asyncio
import base64
from typing import Any

from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..config import REPO_ROOT, is_moli_browser
from ..diagnostics import new_cdp_session
from ..helpers import attach_cdp_event_collector


async def run_fetch_font_group(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    font_bytes = (REPO_ROOT / "moli-layout/tests/fixtures/moli-ahem.woff2").read_bytes()
    for source, stage, action in (
        ("css", "Request", "continue"),
        ("api", "Request", "continue"),
        ("api", "Request", "fulfill"),
        ("api", "Request", "fail"),
        ("api", "Request", "disable"),
        ("api", "Response", "continue"),
    ):
        context = await browser.new_context()
        try:
            page = await context.new_page()
            cdp = await new_cdp_session(context, page)
            with attach_cdp_event_collector(cdp, [
                "Fetch.requestPaused", "Network.requestWillBeSent",
                "Network.responseReceived", "Network.loadingFinished", "Network.loadingFailed",
            ]) as events:
                await cdp.send("Network.enable")
                await cdp.send("Fetch.enable", {"patterns": [
                    {"urlPattern": "*fetch-font.woff2*", "resourceType": "Font", "requestStage": stage},
                    {"urlPattern": "*chromium-resource-image.png*", "resourceType": "Image"},
                ]})
                # Navigate with the mixed configuration already enabled: Document
                # requests must remain usable when a batch includes Font.
                await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
                case = f"{source}-{stage.lower()}-{action}"
                font_url = f"{fixture}/fetch-font.woff2?case={case}"
                image_url = f"{fixture}/chromium-resource-image.png?case={case}"
                await page.evaluate(
                    """({source, fontUrl, imageUrl, controlUrl}) => {
                      let font;
                      if (source === 'css') {
                        const style = document.createElement('style');
                        style.textContent = `@font-face{font-family:FetchSmoke;src:url("${fontUrl}")}`;
                        document.head.append(style);
                        font = [...document.fonts].find(face => face.family === 'FetchSmoke');
                      } else {
                        font = new FontFace('FetchSmoke', `url("${fontUrl}")`);
                        document.fonts.add(font);
                      }
                      const image = new Image();
                      const probe = globalThis.__fetchFont = {
                        font, image, fontResult: 'pending', fetchLength: null, imageResult: 'pending'
                      };
                      font.load().then(
                        () => { probe.fontResult = 'loaded'; },
                        error => { probe.fontResult = error.name; }
                      );
                      // Match the same URL pattern with a distinct cache key so
                      // the control cannot restart or coalesce the font load.
                      fetch(controlUrl, {cache: 'no-store'})
                        .then(response => response.arrayBuffer())
                        .then(bytes => { probe.fetchLength = bytes.byteLength; });
                      image.onload = () => { probe.imageResult = 'loaded'; };
                      image.onerror = () => { probe.imageResult = 'error'; };
                      image.src = imageUrl;
                    }""",
                    {"source": source, "fontUrl": font_url, "imageUrl": image_url,
                     "controlUrl": font_url + "&control=fetch"},
                )

                def pauses() -> list[dict[str, Any]]:
                    return [event["params"] for event in events
                            if event["method"] == "Fetch.requestPaused"]

                await wait_until(lambda: len(pauses()) >= 2, f"{case} Font and Image pauses")
                assert_equal(sorted(pause["resourceType"] for pause in pauses()),
                             ["Font", "Image"], f"{case} mixed pattern resource types")
                font_pause = next(pause for pause in pauses() if pause["resourceType"] == "Font")
                image_pause = next(pause for pause in pauses() if pause["resourceType"] == "Image")
                assert_equal(font_pause["request"]["url"], font_url, f"{case} paused font URL")
                assert_equal(image_pause["request"]["url"], image_url, f"{case} paused image URL")
                network_id = font_pause.get("networkId")
                if not isinstance(network_id, str) or not network_id:
                    raise SmokeError(f"{case} Font pause missed networkId: {font_pause!r}")
                await wait_until(
                    lambda: page.evaluate("__fetchFont.fetchLength !== null"),
                    f"{case} non-Font request completes while Font is paused",
                )
                assert_equal(await page.evaluate("__fetchFont.fetchLength"), len(font_bytes),
                             f"{case} unpaused Fetch body")
                assert_equal(await page.evaluate("__fetchFont.font.status"), "loading",
                             f"{case} font remains paused")
                counts = await page.evaluate(
                    "url => fetch(url).then(response => response.json())",
                    f"{fixture}/fetch-font/status?case={case}",
                )
                assert_equal(counts["requests"], 1 if font_pause.get("responseStatusCode") == 200 else 0,
                             f"{case} request pause prevents early network dispatch")
                if action != "disable":
                    await cdp.send("Fetch.continueRequest", {"requestId": image_pause["requestId"]})

                command = {
                    "continue": "Fetch.continueRequest", "fulfill": "Fetch.fulfillRequest",
                    "fail": "Fetch.failRequest", "disable": "Fetch.disable",
                }[action]
                handled = {image_pause["requestId"]}
                response_bodies = 0

                async def settle_resources() -> bool:
                    nonlocal response_bodies
                    # Chromium 145 publishes a second interception ID for a
                    # continued Font request, retaining the same Network ID.
                    # Settle each published pause, rather than assuming a 1:1
                    # mapping between Fetch interceptions and Network requests.
                    for pause in pauses():
                        request_id = pause["requestId"]
                        if request_id in handled:
                            continue
                        assert_equal((pause["resourceType"], pause["request"]["url"], pause.get("networkId")),
                                     ("Font", font_url, network_id), f"{case} subsequent pause identity")
                        if stage == "Request":
                            assert_equal(pause.get("responseStatusCode"), None, f"{case} request stage")
                            assert_equal(pause.get("responseErrorReason"), None, f"{case} no response error")
                        if action == "fulfill":
                            await cdp.send(command, {
                                "requestId": request_id,
                                "responseCode": 200,
                                "responseHeaders": [{"name": "Content-Type", "value": "font/woff2"}],
                                "body": base64.b64encode(font_bytes).decode("ascii"),
                            })
                        elif action == "fail":
                            await cdp.send(command, {"requestId": request_id, "errorReason": "BlockedByClient"})
                        elif action == "disable":
                            await cdp.send(command)
                        else:
                            if stage == "Response":
                                # Chromium exposes the failed first attempt at
                                # response stage before retrying the font load.
                                if "responseErrorReason" in pause:
                                    assert_equal(pause["responseErrorReason"], "Failed",
                                                 f"{case} initial font attempt")
                                    await cdp.send(command, {"requestId": request_id})
                                    handled.add(request_id)
                                    continue
                                assert_equal(pause.get("responseStatusCode"), 200, f"{case} response stage")
                                body = await cdp.send("Fetch.getResponseBody", {"requestId": request_id})
                                decoded = (base64.b64decode(body["body"], validate=True)
                                           if body.get("base64Encoded") else body["body"].encode())
                                assert_equal(decoded, font_bytes, f"{case} paused font response body")
                                response_bodies += 1
                            await cdp.send(command, {"requestId": request_id})
                        handled.add(request_id)
                    return await page.evaluate(
                        "__fetchFont.fontResult !== 'pending' && __fetchFont.imageResult !== 'pending'"
                    )

                await wait_until(settle_resources, f"{case} font and image outcomes")
                assert_equal(response_bodies, 1 if stage == "Response" else 0,
                             f"{case} reads exactly one successful Font response")
                # Existing Moli cleanup terminates paused subresources on
                # disable; Chromium resumes them. Keep Font and Image aligned
                # within each engine, as verified with the unmodified PR build.
                disabled_failure = action == "disable" and is_moli_browser()
                failed = action == "fail" or disabled_failure
                expected = {
                    "fontResult": "NetworkError" if failed else "loaded",
                    "fontStatus": "error" if failed else "loaded",
                    "imageResult": "error" if disabled_failure else "loaded",
                }
                observed = await page.evaluate("""() => ({
                  fontResult: __fetchFont.fontResult,
                  fontStatus: __fetchFont.font.status,
                  imageResult: __fetchFont.imageResult
                })""")
                assert_equal(observed, expected, f"{case} resource outcomes")

                def terminals() -> list[dict[str, Any]]:
                    return [event for event in events
                            if event["params"].get("requestId") == network_id
                            and event["method"] in ("Network.loadingFinished", "Network.loadingFailed")]

                await wait_until(lambda: bool(terminals()), f"{case} Font Network terminal")
                assert_equal([event["method"] for event in terminals()],
                             ["Network.loadingFailed" if failed else "Network.loadingFinished"],
                             f"{case} exactly one Font Network terminal")
                requests = [event["params"] for event in events
                            if event["method"] == "Network.requestWillBeSent"
                            and event["params"].get("requestId") == network_id]
                assert_equal([(request["type"], request["request"]["url"]) for request in requests],
                             [("Font", font_url)], f"{case} Fetch/Network Font correlation")
                if action == "fail":
                    expected_error = ("net::ERR_BLOCKED_BY_CLIENT" if is_moli_browser()
                                      else "net::ERR_BLOCKED_BY_CLIENT.Inspector")
                    assert_equal(terminals()[0]["params"]["errorText"], expected_error,
                                 f"{case} Network failure reason")
                elif disabled_failure:
                    assert_equal(terminals()[0]["params"]["errorText"], "Fetch interception disabled",
                                 f"{case} Network cleanup reason")
                counts = await page.evaluate(
                    "url => fetch(url).then(response => response.json())",
                    f"{fixture}/fetch-font/status?case={case}",
                )
                assert_equal(counts["requests"], 0 if failed or action == "fulfill" else 1,
                             f"{case} only continued Font reaches the server")
                if counts["requests"]:
                    assert_equal(counts["headers"].get("sec-fetch-dest"), "font",
                                 f"{case} continuation preserves the font request destination")
                if not disabled_failure:
                    assert_equal(counts["imageHeaders"].get("sec-fetch-dest"), "image",
                                 f"{case} continuation preserves the image request destination")
                assert_equal(len({pause["requestId"] for pause in pauses()}), len(pauses()),
                             f"{case} unique interception IDs")
                assert_equal({pause["requestId"] for pause in pauses()}, handled,
                             f"{case} no nonmatching or unhandled pauses")
                record_contract(
                    results,
                    f"fetch_font_{case.replace('-', '_')}",
                    contract=("Mixed Font/Image patterns preserve navigation and nonmatching Fetch; "
                              "Font actions settle the real font load with one correlated Network terminal."),
                    source="Executable Chromium CDP oracle; PR #2470 Font interception regression",
                    commands=["Fetch.enable", command] + (["Fetch.getResponseBody"] if stage == "Response" else []),
                    observed={**observed, "serverRequests": counts["requests"],
                              "networkId": network_id, "fontPauses": len(pauses()) - 1},
                )
        finally:
            await context.close()

    for resource_type in ("Image", "Font"):
        for action in ("continue", "fail"):
            await _verify_css_resource_interception(browser, fixture, results, action, resource_type)


async def _verify_css_resource_interception(
    browser: Any, fixture: str, results: list[dict[str, Any]], action: str, resource_type: str,
) -> None:
    context = await browser.new_context()
    screenshot = None
    try:
        page = await context.new_page()
        cdp = await new_cdp_session(context, page)
        with attach_cdp_event_collector(cdp, [
            "Fetch.requestPaused", "Network.loadingFinished", "Network.loadingFailed",
        ]) as events:
            label = f"CSS {resource_type.lower()} {action}"
            case = f"css-{action}" if resource_type == "Image" else f"css-font-{action}"
            asset = "chromium-resource-image.png" if resource_type == "Image" else "fetch-font.woff2"
            url = f"{fixture}/{asset}?case={case}"
            await cdp.send("Network.enable")
            await cdp.send("Fetch.enable", {"patterns": [
                {"urlPattern": url, "resourceType": resource_type, "requestStage": "Request"},
            ]})
            await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
            await page.evaluate("""url => {
              const stylesheet = document.createElement('link');
              globalThis.__cssImageStylesheet = 'pending';
              stylesheet.onload = () => { __cssImageStylesheet = 'loaded'; };
              stylesheet.onerror = () => { __cssImageStylesheet = 'error'; };
              stylesheet.rel = 'stylesheet';
              stylesheet.href = url;
              document.head.append(stylesheet);
            }""", f"{fixture}/fetch-font/style.css?case={case}&resource={resource_type.lower()}")
            await wait_until(lambda: page.evaluate("__cssImageStylesheet !== 'pending'"),
                             f"{label} stylesheet loaded")
            assert_equal(await page.evaluate("__cssImageStylesheet"), "loaded",
                         f"{label} stylesheet result")
            # Demand real paint resources. Merely defining @font-face does not
            # exercise the stylesheet font loader, and the snapshot may itself
            # be waiting for the intercepted image or font.
            screenshot = asyncio.create_task(cdp.send("Page.captureScreenshot", {"format": "png"}))
            await wait_until(
                lambda: any(event["method"] == "Fetch.requestPaused" for event in events),
                f"{label} pause",
            )
            pauses = [event["params"] for event in events if event["method"] == "Fetch.requestPaused"]
            pause = pauses[0]
            assert_equal((pause["resourceType"], pause["request"]["url"]), (resource_type, url),
                         f"{label} pause identity")
            network_id = pause.get("networkId")
            if not isinstance(network_id, str) or not network_id:
                raise SmokeError(f"{label} pause missed networkId: {pause!r}")
            command = "Fetch.continueRequest" if action == "continue" else "Fetch.failRequest"
            handled: set[str] = set()

            def terminals() -> list[str]:
                return [event["method"] for event in events
                        if event["params"].get("requestId") == network_id
                        and event["method"] in ("Network.loadingFinished", "Network.loadingFailed")]

            async def settle_and_capture() -> bool:
                for event in events:
                    if event["method"] != "Fetch.requestPaused":
                        continue
                    pending = event["params"]
                    if pending["requestId"] in handled:
                        continue
                    assert_equal((pending["resourceType"], pending["request"]["url"], pending.get("networkId")),
                                 (resource_type, url, network_id), f"{label} pause identity")
                    params = {"requestId": pending["requestId"]}
                    if action == "fail":
                        params["errorReason"] = "BlockedByClient"
                    await cdp.send(command, params)
                    handled.add(pending["requestId"])
                if not screenshot.done():
                    return False
                screenshot.result()
                if not terminals():
                    return False
                if resource_type == "Font":
                    return await page.evaluate(
                        "['loaded', 'error'].includes([...document.fonts].find(face => face.family === 'FetchPaint').status)"
                    )
                return True

            await wait_until(settle_and_capture, f"{label} resource actions and paint")
            capture = await screenshot
            if not capture.get("data"):
                raise SmokeError(f"{label} paint returned no image")

            assert_equal(terminals(), ["Network.loadingFinished" if action == "continue" else "Network.loadingFailed"],
                         f"{label} exactly one Network terminal")
            if resource_type == "Image":
                assert_equal(len(handled), 1, f"{label} pause count")
            else:
                font_status = await page.evaluate(
                    "[...document.fonts].find(face => face.family === 'FetchPaint').status"
                )
                assert_equal(font_status, "loaded" if action == "continue" else "error",
                             f"{label} real CSS font status")
            record_contract(
                results, f"fetch_css_{resource_type.lower()}_{action}",
                contract="Paint-demanded CSS images and fonts use the shared interception admission.",
                source="Executable Chromium CDP oracle",
                commands=["Fetch.enable", "Page.captureScreenshot", command],
                observed={"resourceType": pause["resourceType"], "terminal": terminals()[0], "pauses": len(handled)},
            )
    finally:
        if screenshot is not None:
            screenshot.cancel()
            await asyncio.gather(screenshot, return_exceptions=True)
        await context.close()
