from __future__ import annotations

import base64
import urllib.parse
from typing import Any

from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..config import is_moli_browser
from ..diagnostics import new_cdp_session
from ..fixture import FETCH_MEDIA_CAPTIONS, FETCH_MEDIA_WAV
from ..helpers import attach_cdp_event_collector


async def run_fetch_media_group(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    for tag, stage, action, mixed in (
        ("audio", "Request", "continue", True),
        ("video", "Request", "continue", False),
        ("track", "Request", "continue", False),
        ("audio", "Request", "fulfill", True),
        ("track", "Request", "fulfill", True),
        ("video", "Request", "fail", True),
        ("track", "Request", "fail", True),
        ("audio", "Response", "continue", False),
        ("track", "Response", "continue", False),
        ("video", "Request", "disable", True),
        ("track", "Request", "disable", True),
    ):
        context = await browser.new_context()
        try:
            page = await context.new_page()
            cdp = await new_cdp_session(context, page)
            with attach_cdp_event_collector(cdp, [
                "Fetch.requestPaused", "Network.requestWillBeSent",
                "Network.loadingFinished", "Network.loadingFailed",
            ]) as events:
                resource_type = "TextTrack" if tag == "track" else "Media"
                case = f"{tag}-{stage.lower()}-{action}"
                name = "captions.vtt" if tag == "track" else "tone.wav"
                url = f"{fixture}/fetch-media/{name}?case={case}"
                body_bytes = FETCH_MEDIA_CAPTIONS if tag == "track" else FETCH_MEDIA_WAV
                content_type = "text/vtt; charset=utf-8" if tag == "track" else "audio/wav"
                # Chromium's loader reports text-track interceptions as Media;
                # Network retains the more precise TextTrack classification.
                patterns = [{"resourceType": "Media", "requestStage": stage}]
                if mixed:
                    patterns = [{"resourceType": kind, "requestStage": stage}
                                for kind in ("Image", "Media", "Font")]
                await cdp.send("Network.enable")
                await cdp.send("Fetch.enable", {"patterns": patterns})
                await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
                await page.evaluate("""({tag, url}) => {
                  const probe = globalThis.__fetchMedia = {result: 'pending'};
                  if (tag === 'track') {
                    const media = document.createElement('video');
                    const track = document.createElement('track');
                    probe.element = track;
                    track.kind = 'captions';
                    track.addEventListener('load', () => { probe.result = 'loaded'; });
                    track.addEventListener('error', () => { probe.result = 'error'; });
                    track.src = url;
                    media.append(track);
                    document.body.append(media);
                    track.track.mode = 'hidden';
                  } else {
                    const media = document.createElement(tag);
                    probe.element = media;
                    media.preload = 'auto';
                    media.addEventListener('loadedmetadata', () => { probe.result = 'loaded'; });
                    media.addEventListener('error', () => { probe.result = 'error'; });
                    document.body.append(media);
                    media.src = url;
                  }
                }""", {"tag": tag, "url": url})

                def pauses() -> list[dict[str, Any]]:
                    return [event["params"] for event in events if event["method"] == "Fetch.requestPaused"]

                await wait_until(lambda: bool(pauses()), f"{case} real resource pause")
                assert_equal(len(pauses()), 1, f"{case} pause count")
                pause = pauses()[0]
                network_id = pause.get("networkId")
                if not isinstance(network_id, str) or not network_id:
                    raise SmokeError(f"{case} pause missed networkId: {pause!r}")
                assert_equal((pause["resourceType"], pause["request"]["url"]),
                             ("Media", url), f"{case} pause identity")
                assert_equal(await page.evaluate("__fetchMedia.result"), "pending",
                             f"{case} resource remains paused")
                assert_equal(await page.evaluate("url => fetch(url).then(r => r.status)",
                                                 f"{fixture}/plain?case={case}"),
                             200, f"{case} nonmatching Fetch progresses")
                status_url = f"{fixture}/fetch-media/status?" + urllib.parse.urlencode({"name": name, "case": case})

                async def server_requests() -> int:
                    response = await page.evaluate("url => fetch(url).then(r => r.json())", status_url)
                    return response["requests"]

                assert_equal(await server_requests(), 1 if stage == "Response" else 0,
                             f"{case} no request reaches the server before continuation")
                params = {"requestId": pause["requestId"]}
                if action == "fulfill":
                    command = "Fetch.fulfillRequest"
                    params.update({"responseCode": 200,
                                   "responseHeaders": [{"name": "Content-Type", "value": content_type}],
                                   "body": base64.b64encode(body_bytes).decode("ascii")})
                elif action == "fail":
                    command = "Fetch.failRequest"
                    params["errorReason"] = "BlockedByClient"
                elif action == "disable":
                    command = "Fetch.disable"
                    params = {}
                elif stage == "Response":
                    command = "Fetch.continueResponse"
                    assert_equal(pause.get("responseStatusCode"), 200, f"{case} response stage")
                    body = await cdp.send("Fetch.getResponseBody", params)
                    decoded = (base64.b64decode(body["body"], validate=True)
                               if body.get("base64Encoded") else body["body"].encode())
                    assert_equal(decoded, body_bytes, f"{case} intercepted response bytes")
                else:
                    command = "Fetch.continueRequest"
                    assert_equal(pause.get("responseStatusCode"), None, f"{case} request stage")
                await cdp.send(command, params)

                failed = action == "fail" or (action == "disable" and is_moli_browser())
                await wait_until(lambda: page.evaluate("__fetchMedia.result !== 'pending'"),
                                 f"{case} element completion")
                observed = await page.evaluate("""() => ({
                  result: __fetchMedia.result,
                  readyState: __fetchMedia.element.readyState,
                  cues: __fetchMedia.element.track
                    ? [...(__fetchMedia.element.track.cues || [])].map(cue => cue.text) : null
                })""")
                assert_equal(observed["result"], "error" if failed else "loaded", f"{case} element outcome")
                if not failed and tag == "track":
                    assert_equal(observed["readyState"], 2, f"{case} loaded text track")
                    assert_equal(observed["cues"], ["intercepted caption"], f"{case} parsed WebVTT cue")
                elif not failed and observed["readyState"] < 1:
                    raise SmokeError(f"{case} media never acquired metadata: {observed!r}")

                def terminals() -> list[str]:
                    return [event["method"] for event in events
                            if event["params"].get("requestId") == network_id
                            and event["method"] in ("Network.loadingFinished", "Network.loadingFailed")]

                await wait_until(lambda: bool(terminals()), f"{case} Network terminal")
                assert_equal(terminals(), ["Network.loadingFailed" if failed else "Network.loadingFinished"],
                             f"{case} exactly one correlated terminal")
                requests = [event["params"] for event in events
                            if event["method"] == "Network.requestWillBeSent"
                            and event["params"].get("requestId") == network_id]
                assert_equal([(request["type"], request["request"]["url"]) for request in requests],
                             [(resource_type, url)], f"{case} Fetch/Network correlation")
                assert_equal(len(pauses()), 1, f"{case} no unexpected pauses")
                expected_requests = 0 if failed or action == "fulfill" else 1
                assert_equal(await server_requests(), expected_requests, f"{case} actual network dispatch count")
                if expected_requests:
                    server = await page.evaluate("url => fetch(url).then(r => r.json())", status_url)
                    assert_equal(server["headers"].get("sec-fetch-dest"), tag,
                                 f"{case} continuation preserves the request destination")
                    observed["requestHeaders"] = server["headers"]
                record_contract(
                    results, f"fetch_media_{case.replace('-', '_')}",
                    contract=("Audio, video and text tracks retain their element continuation while "
                              "Fetch actions control real network admission under single or mixed patterns."),
                    source="Executable Chromium CDP oracle; shared subresource registration",
                    commands=["Fetch.enable", command] + (["Fetch.getResponseBody"] if stage == "Response" else []),
                    observed={**observed, "type": resource_type, "terminal": terminals()[0],
                              "serverRequests": expected_requests, "mixedPatterns": mixed},
                )
        finally:
            await context.close()
