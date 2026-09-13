from __future__ import annotations

import unittest
from unittest.mock import patch

from moli_cdp_smoke.diagnostics.browser import require_native_windows
from moli_cdp_smoke.diagnostics.fingerprint_identity import is_result_url, select_result


class FingerprintIdentityDiagnosticTests(unittest.TestCase):
    def test_windows_baseline_rejects_non_windows_hosts(self):
        for system in ["Linux", "Darwin"]:
            with self.subTest(system=system), patch("platform.system", return_value=system):
                with self.assertRaisesRegex(ValueError, "not a Windows baseline"):
                    require_native_windows(True)
                require_native_windows(False)
        with patch("platform.system", return_value="Windows"):
            require_native_windows(True)

    def test_result_url_requires_the_demo_host_and_event_api(self):
        self.assertTrue(is_result_url("https://demo.fingerprint.com/api/event/opaque"))
        self.assertFalse(is_result_url("https://demo.fingerprint.com/playground"))
        self.assertFalse(is_result_url("https://example.com/api/event/opaque"))

    def test_result_snapshot_omits_identifiers_and_unselected_raw_attributes(self):
        value = {"tampering": True, "suspect_score": 22, "visitor_id": "secret",
                 "request_id": "secret", "ip_address": "secret",
                 "raw_device_attributes": {"fonts": {"value": ["Arial"]},
                                           "cookies": "secret", "ip": "secret"}}
        self.assertEqual(select_result(value), {
            "tampering": True, "suspect_score": 22,
            "attributes": {"fonts": {"value": ["Arial"]}},
        })

    def test_score_signals_preserve_vm_and_history_without_private_details(self):
        signals = {
            "bot": "not_detected", "tampering": True, "suspect_score": 28,
            "tampering_ml_score": 0.99, "virtual_machine": True,
            "virtual_machine_ml_score": 0.82, "developer_tools": False,
            "incognito": False, "high_activity_device": True, "rare_device": False,
            "rare_device_percentile_bucket": "<p95",
        }
        value = {**signals, "visitor_id": "secret", "event_id": "secret", "ip_address": "secret",
                 "tampering_details": {"anti_detect_browser": True, "anomaly_score": 0.05,
                                       "token": "secret", "extra": {"identifier": "secret"}},
                 "activity_details": {"visitor_id": "secret", "history": ["secret"]}}
        self.assertEqual(select_result(value), {
            **signals, "tampering_details": {"anti_detect_browser": True, "anomaly_score": 0.05},
            "attributes": {},
        })

    def test_score_signals_do_not_invent_absent_or_malformed_tampering_details(self):
        for details in [None, "secret", ["secret"]]:
            with self.subTest(details=details):
                self.assertEqual(select_result({"tampering_details": details}), {"attributes": {}})
        self.assertEqual(select_result({}), {"attributes": {}})


if __name__ == "__main__":
    unittest.main()
