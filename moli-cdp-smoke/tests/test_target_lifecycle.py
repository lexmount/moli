from __future__ import annotations

import unittest
import os
import tempfile
import sys
from pathlib import Path
from unittest.mock import patch

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.navigation import DocumentLoad
from moli_cdp_smoke.groups.target_lifecycle import (
    LifecycleProbe, assert_resources_bounded, process_resources, wait_for_resources_bounded,
)


class TargetLifecycleTests(unittest.TestCase):
    def test_profile_startup_files_do_not_mask_transport_resource_leaks(self) -> None:
        baseline = {"fds": 100, "profileFileFds": 40, "eventpoll": 5, "eventfd": 8, "threads": 40}
        current = dict(baseline, fds=141, profileFileFds=80)
        assert_resources_bounded(baseline, current)
        for change in ({"fds": 149}, {"eventpoll": 8}, {"eventfd": 11}, {"threads": 43}):
            with self.subTest(change=change), self.assertRaises(SmokeError):
                assert_resources_bounded(baseline, dict(current, **change))

    @unittest.skipUnless(sys.platform.startswith("linux"), "requires Linux /proc")
    def test_profile_file_fds_are_counted_separately_and_total_is_retained(self) -> None:
        with tempfile.TemporaryDirectory() as profile:
            with (Path(profile) / "database").open("w"):
                resources = process_resources(os.getpid(), profile)
                self.assertEqual(resources["profileFileFds"], 1)
                self.assertGreater(resources["fds"], resources["profileFileFds"])
                self.assertEqual(process_resources(os.getpid())["profileFileFds"], 0)

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
        self.assertNotIn("new", probe.destroyed)
        document = DocumentLoad("session-A", "frame", "requested-loader")
        self.assertFalse(document.observe({"method": "Page.loadEventFired", "sessionId": "session-A"}))
        self.assertFalse(document.observe({"method": "Page.lifecycleEvent", "sessionId": "session-B",
                                          "params": {"frameId": "frame", "loaderId": "requested-loader", "name": "load"}}))

    def test_non_linux_fd_sampling_is_explicitly_unavailable(self) -> None:
        with patch("moli_cdp_smoke.groups.target_lifecycle.sys.platform", "darwin"):
            self.assertIsNone(process_resources(1234))

    def test_missing_managed_process_is_not_silently_skipped(self) -> None:
        with patch("moli_cdp_smoke.groups.target_lifecycle.sys.platform", "linux"), \
             patch("moli_cdp_smoke.groups.target_lifecycle.Path.iterdir", side_effect=FileNotFoundError):
            with self.assertRaises(FileNotFoundError):
                process_resources(1234)


class ResourceReclamationTests(unittest.IsolatedAsyncioTestCase):
    baseline = {"fds": 24, "eventpoll": 7, "eventfd": 4, "threads": 20}

    async def poll_twice(self, predicate, label):
        for _ in range(2):
            if predicate():
                return
        raise SmokeError(f"timed out waiting for {label}")

    async def test_target_destruction_can_precede_io_reclamation(self) -> None:
        pending = dict(self.baseline, fds=40)
        with patch("moli_cdp_smoke.groups.target_lifecycle.process_resources",
                   side_effect=[pending, self.baseline]), \
             patch("moli_cdp_smoke.groups.target_lifecycle.wait_until", self.poll_twice):
            self.assertEqual(await wait_for_resources_bounded(1234, None, self.baseline), self.baseline)

    async def test_persistent_leaks_still_fail_the_original_fixed_budgets(self) -> None:
        for key, value in (("fds", 224), ("eventpoll", 10), ("eventfd", 7), ("threads", 23)):
            with self.subTest(key=key), \
                 patch("moli_cdp_smoke.groups.target_lifecycle.process_resources",
                       return_value=dict(self.baseline, **{key: value})), \
                 patch("moli_cdp_smoke.groups.target_lifecycle.wait_until", self.poll_twice):
                with self.assertRaisesRegex(SmokeError, f"target teardown leaked {key}") as error:
                    await wait_for_resources_bounded(1234, None, self.baseline)
                self.assertIn("timed out", str(error.exception.__cause__))

    async def test_missing_or_inaccessible_managed_process_is_not_retried(self) -> None:
        for error in (FileNotFoundError, PermissionError):
            with self.subTest(error=error), \
                 patch("moli_cdp_smoke.groups.target_lifecycle.process_resources", side_effect=error) as sample:
                with self.assertRaises(error):
                    await wait_for_resources_bounded(1234, None, self.baseline)
                sample.assert_called_once_with(1234, None)
