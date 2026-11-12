"""Observe the demo login's request lifecycle, separately from its bot verdict.

No event hooks, request rewrites, identity overrides, or simulated-human input.
This uses the same dummy credentials and fixed 80 ms typing as the 12-site run.
"""
from __future__ import annotations

import argparse
import asyncio
import base64
from copy import deepcopy
import json
from pathlib import Path
import time

from .browser import browser_session, save

URL = "https://deviceandbrowserinfo.com/are_you_a_bot_interactions"
RESULT_URL = "https://deviceandbrowserinfo.com/fingerprint_bot_test"
INTERACTION_KEYS = (
    "numSuspiciousKeyEventsEmail", "numSuspiciousKeyEventsPassword",
    "emailFieldHasClick", "passwordFieldHasClick", "submitFieldHasClick",
    "hasClickedEmailFieldExactCenter", "hasClickedPasswordFieldExactCenter", "hasClickedSubmitExactCenter",
    "hasClickedEmailFieldSmallMargin", "hasClickedPasswordFieldSmallMargin", "hasClickedSubmitSmallMargin",
    "hasClickedEmailFieldZero", "hasClickedPasswordFieldZero", "hasClickedSubmitFieldZero",
    "hasModifierKeyPressed", "hasUntrustedEvent", "submittedWithEnter", "typeAtCharacter",
    "typeAtWithModifier", "timeToFill", "cdpMouseLeak",
)
TIMING_KEYS = ("numKeys", "averageTimeBetweenKeys", "standardDeviation", "medianTimeBetweenKeys")


def select_interactions(value: dict) -> dict:
    result = {key: value[key] for key in INTERACTION_KEYS
              if key in value and isinstance(value[key], (int, float, bool))}
    for field in ["emailTimingStats", "passwordTimingStats"]:
        stats = value.get(field)
        if stats is None:
            result[field] = None
        elif isinstance(stats, dict):
            result[field] = {key: stats[key] for key in TIMING_KEYS
                             if key in stats and isinstance(stats[key], (int, float))}
    # Count these without retaining script URLs, source text, or opaque tokens.
    if isinstance(value.get("nativeFunctionsStackTraces"), list):
        result["nativeFunctionsStackTraceCount"] = len(value["nativeFunctionsStackTraces"])
    return result


def select_verdict(value: dict) -> dict:
    result = {"isBot": value["isBot"]} if isinstance(value.get("isBot"), bool) else {}
    details = value.get("details", {})
    if isinstance(details, dict):
        result["details"] = {key: item for key, item in details.items() if isinstance(item, bool)}
    return result


def request_state(record: dict) -> str:
    if "failed_at" in record:
        return "failed"
    if "finished_at" in record:
        return "finished"
    if "headers_at" in record:
        return "body_pending"
    return "headers_pending"


class Observer:
    def __init__(self, cdp, clock):
        self.cdp, self.clock = cdp, clock
        self.requests = {}
        self.tasks = set()
        cdp.on("Network.requestWillBeSent", self.requested)
        cdp.on("Network.responseReceived", self.received)
        cdp.on("Network.loadingFinished", self.finished)
        cdp.on("Network.loadingFailed", self.failed)

    def start(self, coroutine):
        task = asyncio.create_task(coroutine)
        self.tasks.add(task)
        task.add_done_callback(self.tasks.discard)

    def requested(self, params):
        request = params["request"]
        if request.get("url") != RESULT_URL or request.get("method") != "POST":
            return
        record = {"sent_at": self.clock()}
        self.requests[params["requestId"]] = record
        self.start(self.read_submission(params["requestId"], request.get("postData"), record))

    async def read_submission(self, request_id, post_data, record):
        try:
            if post_data is None:
                post_data = (await asyncio.wait_for(self.cdp.send(
                    "Network.getRequestPostData", {"requestId": request_id}), 5))["postData"]
            record["interactions"] = select_interactions(json.loads(post_data)["interactions"])
        except Exception as error:
            record["submission_read_error"] = type(error).__name__

    def received(self, params):
        if (record := self.requests.get(params["requestId"])) is not None:
            response = params["response"]
            record.update(headers_at=self.clock(), status=response["status"], mime=response.get("mimeType"))

    def finished(self, params):
        if (record := self.requests.get(params["requestId"])) is not None:
            record["finished_at"] = self.clock()
            self.start(self.read_verdict(params["requestId"], record))

    def failed(self, params):
        if (record := self.requests.get(params["requestId"])) is not None:
            record.update(failed_at=self.clock(), network_error=params.get("errorText"),
                          canceled=params.get("canceled", False))

    async def read_verdict(self, request_id, record):
        try:
            body = await asyncio.wait_for(self.cdp.send("Network.getResponseBody", {"requestId": request_id}), 5)
            text = base64.b64decode(body["body"]) if body.get("base64Encoded") else body["body"]
            record["verdict"] = select_verdict(json.loads(text))
            record["verdict_read_at"] = self.clock()
        except Exception as error:
            record["verdict_read_error"] = type(error).__name__

    def snapshot(self):
        # Freeze evidence before any DOM/CDP await can advance the observation
        # window. A later reply must not retroactively fill an earlier sample.
        return [{**deepcopy(record), "state": request_state(record)} for record in self.requests.values()]

    async def close(self):
        for task in tuple(self.tasks):
            task.cancel()
        await asyncio.gather(*tuple(self.tasks), return_exceptions=True)


async def checkpoint(page, observer, clock):
    sample = {"cutoff": clock(), "requests": observer.snapshot()}
    try:
        # Read only the result container, not all page globals or event handlers.
        text = await asyncio.wait_for(page.evaluate(
            "() => document.getElementById('jsonResult')?.textContent || ''"), 5)
        sample["widget_verdict"] = select_verdict(json.loads(text)) if text.strip() else None
    except Exception as error:
        sample["widget_read_error"] = type(error).__name__
    sample["widget_read_at"] = clock()
    return sample


async def capture(args):
    result = {"url": URL, "typing_delay_ms": 80, "credentials": "dummy only",
              "human_simulation": False, "wait_before_input": 20, "baseline_wait_after_submit": 20,
              "extended_wait_after_baseline": args.extra_wait, "errors": []}
    async with browser_session(args.binary, args.engine, args.output) as (page, cdp):
        started = time.monotonic()
        clock = lambda: time.monotonic() - started
        observer = Observer(cdp, clock)
        try:
            await page.goto(URL, wait_until="domcontentloaded", timeout=45000)
            await asyncio.sleep(20)
            await page.locator('input[type=email]').first.click(timeout=10000)
            await page.keyboard.type("moli-cdp-demo@example.com", delay=80)
            await page.locator('input[type=password]').first.click(timeout=10000)
            await page.keyboard.type("Moli-Demo-Only-2026!", delay=80)
            await page.get_by_role("button", name="Login", exact=True).click(timeout=10000)
            result["submit_clicked_at"] = clock()
            print("submitted demo login", flush=True)
            await asyncio.sleep(20)
            result["baseline"] = await checkpoint(page, observer, clock)
            save(args.output / "result.json", result)
            print("baseline", json.dumps(result["baseline"]), flush=True)
            await asyncio.sleep(args.extra_wait)
            result["extended"] = await checkpoint(page, observer, clock)
            print("extended", json.dumps(result["extended"]), flush=True)
        except Exception as error:
            result["errors"].append({"elapsed": clock(), "error_type": type(error).__name__})
        finally:
            result["final_requests"] = observer.snapshot()
            await observer.close()
            save(args.output / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--engine", choices=["chromium", "moli"], required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--extra-wait", type=int, default=40)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output must be a new directory")
    if not 0 <= args.extra_wait <= 120:
        parser.error("extra-wait must be between 0 and 120 seconds")
    result = asyncio.run(capture(args))
    if result["errors"]:
        raise SystemExit("Observation failed; see result.json")


if __name__ == "__main__":
    main()
