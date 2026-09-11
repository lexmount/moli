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


if __name__ == "__main__":
    unittest.main()
