"""Compare native FP Pro observations without changing identity or site probes."""
from __future__ import annotations

import argparse
import asyncio
import base64
import json
from pathlib import Path
import time
from urllib.parse import urlsplit

from .browser import browser_session, save

URL = "https://demo.fingerprint.com/playground"
ATTRIBUTE_KEYS = ("fonts", "font_preferences", "touch_support", "platform", "vendor", "languages",
                  "screen_resolution", "color_depth", "hardware_concurrency", "device_memory",
                  "timezone", "os_cpu", "architecture", "audio", "math", "webgl_basics")
IDENTITY = """async () => ({
    userAgent: navigator.userAgent, platform: navigator.platform, languages: navigator.languages,
    webdriver: navigator.webdriver, hardwareConcurrency: navigator.hardwareConcurrency,
    deviceMemory: navigator.deviceMemory,
    metadata: navigator.userAgentData ? await navigator.userAgentData.getHighEntropyValues([
      'platformVersion','architecture','bitness','fullVersionList','wow64']) : null
})"""


def select_result(value: dict) -> dict:
    # No visitor/request IDs, IPs, request bodies, cookies or opaque SDK tokens.
    result = {key: value[key] for key in ["bot", "tampering", "suspect_score", "tampering_ml_score"]
              if key in value}
    raw = value.get("raw_device_attributes", {})
    result["attributes"] = {key: raw[key] for key in ATTRIBUTE_KEYS if key in raw}
    return result


def is_result_url(url: str) -> bool:
    parsed = urlsplit(url)
    return parsed.hostname == "demo.fingerprint.com" and parsed.path.startswith("/api/event/")


async def capture(args) -> dict:
    result = {"url": URL, "wait_seconds": args.wait_seconds, "responses": [],
              "errors": [], "valid_verdict": False}
    try:
        async with browser_session(args.binary, args.engine, args.output, args.require_windows) as (page, cdp):
            started = time.monotonic()
            responses = []

            def received(params):
                response = params.get("response", {})
                if is_result_url(response.get("url", "")):
                    responses.append({"requestId": params["requestId"], "status": response["status"],
                                      "elapsed": time.monotonic() - started})

            cdp.on("Network.responseReceived", received)
            try:
                await page.goto(URL, wait_until="domcontentloaded", timeout=45000)
            except Exception as error:
                result["errors"].append({"phase": "navigation", "error_type": type(error).__name__})
            await asyncio.sleep(args.wait_seconds)
            cutoff = time.monotonic() - started
            result["observation_cutoff"] = cutoff
            for response in list(responses):
                if response["elapsed"] > cutoff:
                    continue
                observation = {"status": response["status"], "elapsed": response["elapsed"]}
                try:
                    body = await asyncio.wait_for(cdp.send("Network.getResponseBody", {"requestId": response["requestId"]}), 5)
                    if body.get("base64Encoded"):
                        value = json.loads(base64.b64decode(body["body"]))
                    else:
                        value = json.loads(body["body"])
                    observation["result"] = select_result(value)
                except Exception as error:
                    observation["error_type"] = type(error).__name__
                result["responses"].append(observation)
            # This post-verdict read does not override or instrument any probe.
            result["identity"] = await asyncio.wait_for(page.evaluate(IDENTITY), 10)
            result["valid_verdict"] = any(isinstance(r.get("result", {}).get("tampering"), bool)
                                          for r in result["responses"])
    finally:
        if args.output.exists():
            save(args.output / "result.json", result)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--engine", choices=["chromium", "moli"], required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--require-windows", action="store_true", help="Reject a Linux/macOS runner even if its UA claims Windows")
    parser.add_argument("--wait-seconds", type=int, default=12)
    args = parser.parse_args()
    if args.wait_seconds < 1 or args.wait_seconds > 120:
        parser.error("wait-seconds must be between 1 and 120")
    if args.output.exists():
        parser.error("output must be a new directory")
    result = asyncio.run(capture(args))
    print(json.dumps({"valid_verdict": result["valid_verdict"], "responses": result["responses"]}))


if __name__ == "__main__":
    main()
