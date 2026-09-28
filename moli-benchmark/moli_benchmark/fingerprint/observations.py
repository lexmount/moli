"""Bounded, allowlisted observations. Never save raw CDP messages or API bodies."""
from __future__ import annotations

import asyncio
import base64
from copy import deepcopy
import hashlib
import json
import math
import re
from urllib.parse import urlsplit


def is_fp_result(url: str) -> bool:
    parsed = urlsplit(url)
    return parsed.scheme == "https" and parsed.hostname == "demo.fingerprint.com" and parsed.path.startswith("/api/event/")


def select_fp_result(value: object) -> dict:
    if not isinstance(value, dict):
        return {}
    result = {key: value[key] for key in ["tampering", "virtual_machine", "developer_tools", "incognito"]
              if type(value.get(key)) is bool}
    if value.get("bot") in ("not_detected", "good", "bad"):
        result["bot"] = value["bot"]
    for key in ["suspect_score", "tampering_ml_score", "virtual_machine_ml_score"]:
        score = value.get(key)
        maximum = 100 if key == "suspect_score" else 1
        if type(score) in {int, float} and math.isfinite(score) and 0 <= score <= maximum:
            result[key] = score
    return result


def complete_fp_result(value: dict) -> bool:
    return all(key in value for key in ["bot", "tampering", "suspect_score", "virtual_machine"])


def error_record(phase: str, error: BaseException) -> dict:
    return {"phase": phase, "type": type(error).__name__,
            "network_codes": sorted(set(re.findall(r"(?:net::)?ERR_[A-Z_]+", str(error))))}


class Observer:
    def __init__(self, cdp, clock):
        self.cdp, self.clock = cdp, clock
        self.requests: dict[str, dict] = {}
        self.exception_count = 0
        self.network_failures: dict[str, int] = {}
        self.script_limit = 40
        self.script_count = 0
        self.script_limit_reached = False
        self.result_count = 0
        self.result_limit_reached = False
        cdp.on("Network.requestWillBeSent", self.requested)
        cdp.on("Network.responseReceived", self.received)
        cdp.on("Network.loadingFinished", self.finished)
        cdp.on("Network.loadingFailed", self.failed)
        cdp.on("Runtime.exceptionThrown", self.exception)

    def requested(self, params):
        url = params.get("request", {}).get("url", "")
        kind = "result" if is_fp_result(url) else "script" if params.get("type") == "Script" else None
        # Redirects reuse a request id; the final destination must still qualify.
        self.requests.pop(params["requestId"], None)
        if kind == "script":
            self.script_count += 1
            if self.script_count > self.script_limit:
                self.script_limit_reached = True
                return
        if kind == "result":
            self.result_count += 1
            if self.result_count > 20:
                self.result_limit_reached = True
                return
        if kind is not None:
            self.requests[params["requestId"]] = {"kind": kind, "host": urlsplit(url).hostname,
                                                   "sent_at": self.clock()}

    def received(self, params):
        if (record := self.requests.get(params["requestId"])) is not None:
            record.update(status=params["response"]["status"], headers_at=self.clock())

    def finished(self, params):
        if (record := self.requests.get(params["requestId"])) is not None:
            record["finished_at"] = self.clock()

    def failed(self, params):
        for code in set(re.findall(r"(?:net::)?ERR_[A-Z_]+", params.get("errorText", ""))):
            self.network_failures[code] = self.network_failures.get(code, 0) + 1
        if (record := self.requests.get(params["requestId"])) is not None:
            record["failed_at"] = self.clock()

    def exception(self, _params):
        self.exception_count += 1

    def snapshot(self, cutoff: float) -> dict:
        return {"cutoff": cutoff, "requests": deepcopy(self.requests), "exception_count": self.exception_count,
                "network_failures": dict(self.network_failures), "script_limit_reached": self.script_limit_reached,
                "result_limit_reached": self.result_limit_reached}

    async def resolve(self, frozen: dict) -> dict:
        result = {"responses": [], "scripts": [], "exception_count": frozen["exception_count"],
                  "network_failures": frozen["network_failures"], "script_limit_reached": frozen["script_limit_reached"],
                  "result_limit_reached": frozen["result_limit_reached"]}
        # Read only bodies that were complete at the cutoff; later responses can
        # never fill earlier samples. Result bodies have priority over script hashes.
        requests = sorted(frozen["requests"].items(), key=lambda item: item[1]["kind"] != "result")
        try:
            async with asyncio.timeout(15):
                for request_id, record in requests:
                    complete = (record.get("status") == 200 and "failed_at" not in record
                                and record.get("finished_at", math.inf) <= frozen["cutoff"])
                    projected = {**record, "within_window": complete}
                    if record["kind"] == "result":
                        result["responses"].append(projected)
                    elif not complete:
                        continue
                    else:
                        result["scripts"].append(projected)
                    if not complete:
                        continue
                    try:
                        response = await asyncio.wait_for(self.cdp.send("Network.getResponseBody", {"requestId": request_id}), 3)
                        body = base64.b64decode(response["body"]) if response.get("base64Encoded") else response["body"].encode()
                        if record["kind"] == "result":
                            projected["result"] = select_fp_result(json.loads(body))
                        else:
                            projected.update(sha256=hashlib.sha256(body).hexdigest(), bytes=len(body))
                    except Exception as error:
                        projected["read_error"] = type(error).__name__
        except TimeoutError:
            result["body_audit_incomplete"] = True
        return result
