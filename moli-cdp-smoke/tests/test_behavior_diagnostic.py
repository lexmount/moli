from __future__ import annotations

import asyncio
import json
import unittest

from moli_cdp_smoke.diagnostics.device_browser_behavior import (
    Observer, RESULT_URL, request_state, select_interactions, select_verdict,
)


class BehaviorDiagnosticTests(unittest.TestCase):
    def test_missing_response_is_pending_not_failure_or_pass(self):
        self.assertEqual(request_state({"sent_at": 1}), "headers_pending")
        self.assertEqual(request_state({"sent_at": 1, "headers_at": 2}), "body_pending")
        self.assertEqual(request_state({"finished_at": 3}), "finished")
        self.assertEqual(request_state({"headers_at": 2, "failed_at": 3}), "failed")

    def test_payload_allowlist_keeps_metrics_but_not_credentials_or_stacks(self):
        self.assertEqual(select_interactions({
            "email": "private", "password": "private", "opaqueToken": "private",
            "typeAtCharacter": True, "emailTimingStats": {"numKeys": 25, "token": "private"},
            "passwordTimingStats": None, "nativeFunctionsStackTraces": ["private URL"],
        }), {"typeAtCharacter": True, "emailTimingStats": {"numKeys": 25},
             "passwordTimingStats": None, "nativeFunctionsStackTraceCount": 1})

    def test_verdict_requires_a_real_boolean_and_redacts_non_boolean_details(self):
        self.assertEqual(select_verdict({"isBot": "false", "visitor": "private"}), {"details": {}})
        self.assertEqual(select_verdict({"isBot": False, "details": {"suspiciousClientSideBehavior": False,
                                                                      "token": "private"}}),
                         {"isBot": False, "details": {"suspiciousClientSideBehavior": False}})


class FakeCDP:
    def on(self, _event, _callback):
        pass

    async def send(self, method, _params):
        if method != "Network.getResponseBody":
            raise AssertionError(method)
        return {"body": json.dumps({"isBot": True, "details": {"suspiciousClientSideBehavior": True}})}


class BehaviorObserverTests(unittest.IsolatedAsyncioTestCase):
    async def test_late_verdict_cannot_mutate_the_frozen_baseline(self):
        now = [1]
        observer = Observer(FakeCDP(), lambda: now[0])
        observer.requested({"requestId": "r", "request": {"url": RESULT_URL, "method": "POST",
                           "postData": json.dumps({"interactions": {"typeAtCharacter": True}})}})
        await asyncio.gather(*tuple(observer.tasks))
        baseline = observer.snapshot()
        self.assertEqual(baseline[0]["state"], "headers_pending")
        now[0] = 30
        observer.received({"requestId": "r", "response": {"status": 200, "mimeType": "application/json"}})
        observer.finished({"requestId": "r"})
        await asyncio.gather(*tuple(observer.tasks))
        extended = observer.snapshot()
        self.assertEqual(extended[0]["state"], "finished")
        self.assertTrue(extended[0]["verdict"]["isBot"])
        self.assertNotIn("verdict", baseline[0])
        self.assertNotIn("headers_at", baseline[0])
        self.assertNotIn("requestId", baseline[0])
        await observer.close()

    async def test_unrelated_network_probe_is_not_a_demo_verdict(self):
        observer = Observer(FakeCDP(), lambda: 0)
        observer.requested({"requestId": "probe", "request": {
            "url": "https://fingerprint-scan.com/", "method": "POST", "postData": "{}"}})
        observer.received({"requestId": "probe", "response": {"status": 200}})
        observer.finished({"requestId": "probe"})
        self.assertEqual(observer.snapshot(), [])
        self.assertFalse(observer.tasks)


if __name__ == "__main__":
    unittest.main()
