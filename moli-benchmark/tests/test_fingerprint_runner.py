from __future__ import annotations

import asyncio
import json
import unittest

from moli_benchmark.fingerprint.cases import CASES, select_cases
from moli_benchmark.fingerprint.observations import Observer, complete_fp_result, is_fp_result, select_fp_result
from moli_benchmark.fingerprint.runner import build_summary, classify


class FingerprintContractTests(unittest.TestCase):
    def test_canonical_matrix_and_selection(self):
        self.assertEqual(len(CASES), 12)
        self.assertEqual(len({case.id for case in CASES}), 12)
        self.assertEqual(select_cases(["browserscan-bot"])[0].wait_seconds, 10)
        for ids in [[], ["unknown"], ["creepjs", "creepjs"]]:
            with self.assertRaises(ValueError):
                select_cases(ids)

    def test_fp_requires_all_fields_and_retains_zero_and_false(self):
        raw = {"bot": "not_detected", "tampering": False, "virtual_machine": False, "suspect_score": 0,
               "visitorId": "secret", "requestId": "secret", "ip": "secret", "raw_device_attributes": {"ip": "secret"}}
        value = select_fp_result(raw)
        self.assertEqual(len(value), 4)
        self.assertTrue(complete_fp_result(value))
        for field in value:
            self.assertFalse(complete_fp_result({key: v for key, v in value.items() if key != field}))
        self.assertEqual(select_fp_result({"bot": {}, "suspect_score": True, "tampering": "false"}), {})
        self.assertNotIn("suspect_score", select_fp_result({"suspect_score": float("nan")}))

    def test_result_origin_is_exact(self):
        self.assertTrue(is_fp_result("https://demo.fingerprint.com/api/event/id?token=secret"))
        self.assertFalse(is_fp_result("https://demo.fingerprint.com.attacker.test/api/event/id"))
        self.assertFalse(is_fp_result("http://demo.fingerprint.com/api/event/id"))

    def test_negative_verdict_is_complete_not_a_collection_error(self):
        row = {"engine": "moli", "report": {"valid": True, "verdict": "Robot"}, "process_cleaned_up": True}
        row["status"] = classify(row)
        self.assertEqual(row["status"], "complete")
        summary = build_summary([row], select_cases(["browserscan-bot"]), ["moli", "chromium"])
        self.assertEqual(summary["coverage"]["moli"]["complete"], 1)
        self.assertEqual(summary["coverage"]["chromium"]["attempted"], 0)
        self.assertNotIn("pass_count", summary)

    def test_missing_does_not_become_pass_and_errors_do_not_disappear(self):
        row = {"report": {"valid": False}, "process_cleaned_up": True}
        self.assertEqual(classify(row), "missing")
        row["report"]["fields"] = {"value": None}
        self.assertEqual(classify(row), "partial")
        row["errors"] = [{"type": "TimeoutError"}]
        self.assertEqual(classify(row), "timeout")
        row["errors"] = [{"type": "Error", "network_codes": ["net::ERR_CERT_AUTHORITY_INVALID"]}]
        self.assertEqual(classify(row), "network_error")
        row["process_cleaned_up"] = False
        self.assertEqual(classify(row), "cleanup_error")


class FakeCDP:
    def __init__(self):
        self.calls = []

    def on(self, *_args):
        pass

    async def send(self, method, params):
        self.calls.append((method, params))
        return {"body": json.dumps({"bot": "not_detected", "tampering": False,
                                    "virtual_machine": False, "suspect_score": 6, "ip": "private"})}


class ObservationWindowTests(unittest.IsolatedAsyncioTestCase):
    async def test_headers_before_but_body_after_cutoff_does_not_fill_the_sample(self):
        cdp = FakeCDP()
        now = [1]
        observer = Observer(cdp, lambda: now[0])
        observer.requested({"requestId": "r", "request": {"url": "https://demo.fingerprint.com/api/event/secret"}})
        observer.received({"requestId": "r", "response": {"status": 200}})
        frozen = observer.snapshot(2)
        now[0] = 3
        observer.finished({"requestId": "r"})
        result = await observer.resolve(frozen)
        self.assertFalse(result["responses"][0]["within_window"])
        self.assertNotIn("result", result["responses"][0])
        self.assertEqual(cdp.calls, [])
        completed = await observer.resolve(observer.snapshot(4))
        self.assertTrue(complete_fp_result(completed["responses"][0]["result"]))
        encoded = json.dumps(completed)
        for forbidden in ["private", "secret", "requestId"]:
            self.assertNotIn(forbidden, encoded)

    async def test_redirect_outside_result_origin_cannot_supply_a_verdict(self):
        observer = Observer(FakeCDP(), lambda: 1)
        for url in ["https://demo.fingerprint.com/api/event/id", "https://example.com/redirected"]:
            observer.requested({"requestId": "r", "request": {"url": url}})
        observer.received({"requestId": "r", "response": {"status": 200}})
        observer.finished({"requestId": "r"})
        self.assertEqual((await observer.resolve(observer.snapshot(2)))["responses"], [])


if __name__ == "__main__":
    unittest.main()
