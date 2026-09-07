from __future__ import annotations

import unittest
from unittest.mock import patch

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.target_lifecycle import (
    LifecycleProbe, assert_resources_bounded, process_resources,
)


class TargetLifecycleTests(unittest.TestCase):
    def test_resource_budget_is_fixed_not_proportional_to_iterations(self) -> None:
        baseline = {"fds": 24, "eventpoll": 7, "eventfd": 4, "threads": 20}
        assert_resources_bounded(baseline, baseline)
        assert_resources_bounded(baseline, dict(baseline, fds=28, eventpoll=8, eventfd=5))
        for key, value in (("fds", 224), ("eventpoll", 10), ("eventfd", 7), ("threads", 23)):
            with self.subTest(key=key), self.assertRaises(SmokeError):
                assert_resources_bounded(baseline, dict(baseline, **{key: value}))

    def test_event_identity_is_target_and_session_specific(self) -> None:
        probe = LifecycleProbe(None)  # type: ignore[arg-type]
        probe.observe({"method": "Target.targetDestroyed", "params": {"targetId": "old"}})
        probe.observe({"method": "Page.loadEventFired", "sessionId": "session-A"})
        probe.observe({"method": "Target.targetCreated", "params": {"targetInfo": {"targetId": "new"}}})
        self.assertEqual(probe.destroyed, {"old"})
        self.assertEqual(probe.loaded, {"session-A"})
        self.assertNotIn("session-B", probe.loaded)

    def test_non_linux_fd_sampling_is_explicitly_unavailable(self) -> None:
        with patch("moli_cdp_smoke.groups.target_lifecycle.sys.platform", "darwin"):
            self.assertIsNone(process_resources(1234))

    def test_missing_managed_process_is_not_silently_skipped(self) -> None:
        with patch("moli_cdp_smoke.groups.target_lifecycle.sys.platform", "linux"), \
             patch("moli_cdp_smoke.groups.target_lifecycle.Path.iterdir", side_effect=FileNotFoundError):
            with self.assertRaises(FileNotFoundError):
                process_resources(1234)
