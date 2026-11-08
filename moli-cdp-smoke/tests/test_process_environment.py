from __future__ import annotations

import asyncio
import unittest
from typing import Any
from unittest.mock import patch

from pyee import EventEmitter

from moli_cdp_smoke.diagnostics import ObservedCdpSession
from moli_cdp_smoke.groups.process_environment import _override_while_peer_paused


BASELINE = ["en-US", "UTC", 0, 0]


class _Control:
    def __init__(self) -> None:
        self.defaults = BASELINE

    async def send(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        if method == "Emulation.setTimezoneOverride":
            self.defaults = {
                "Europe/Paris": ["fr-FR", "Europe/Paris", -60, -120],
                "Asia/Shanghai": ["de-DE", "Asia/Shanghai", -480, -480],
                "": BASELINE,
            }[params["timezoneId"]]
        return {}


class _Peer(EventEmitter):
    def __init__(self, control: _Control, *, fail_cleanup: bool = False,
                 fail_evaluation: bool = False) -> None:
        super().__init__()
        self.control = control
        self.calls: list[str] = []
        self.resumed = asyncio.Event()
        self.evaluation_finished = False
        self.fail_cleanup = fail_cleanup
        self.fail_evaluation = fail_evaluation

    async def send(self, method: str, params: Any = None) -> dict[str, Any]:
        self.calls.append(method)
        if method == "Runtime.evaluate":
            self.emit("Debugger.paused", {"callFrames": [{"callFrameId": "frame"}]})
            await self.resumed.wait()
            self.evaluation_finished = True
        elif method == "Debugger.evaluateOnCallFrame":
            if self.fail_evaluation:
                raise RuntimeError("original frame failure")
            return {"result": {"value": [self.control.defaults, self.control.defaults[2]]}}
        elif method in {"Debugger.resume", "Debugger.disable"}:
            self.resumed.set()
        return {}

    def remove_listener(self, method: str, handler: Any) -> None:
        super().remove_listener(method, handler)
        if self.fail_cleanup:
            raise KeyError("listener cleanup failure")


class PausedEnvironmentTests(unittest.IsolatedAsyncioTestCase):
    async def test_independent_observer_cannot_break_pause_cleanup(self) -> None:
        control = _Control()
        peer = _Peer(control)
        with patch("moli_cdp_smoke.groups.process_environment.is_moli_browser", return_value=True):
            await _override_while_peer_paused(control, ObservedCdpSession(peer), BASELINE)
        self.assertEqual(peer.calls.count("Debugger.evaluateOnCallFrame"), 4)
        self.assertEqual(peer.calls[-2:], ["Debugger.resume", "Debugger.disable"])
        self.assertTrue(peer.evaluation_finished)

    async def test_assertion_failure_still_resumes_drains_and_disables(self) -> None:
        control = _Control()
        peer = _Peer(control, fail_evaluation=True)
        with patch("moli_cdp_smoke.groups.process_environment.is_moli_browser", return_value=True):
            with self.assertRaisesRegex(RuntimeError, "original frame failure"):
                await _override_while_peer_paused(control, ObservedCdpSession(peer), BASELINE)
        self.assertTrue(peer.evaluation_finished)
        self.assertEqual(peer.calls[-2:], ["Debugger.resume", "Debugger.disable"])

    async def test_cleanup_failure_cannot_prevent_resume_or_hide_primary_error(self) -> None:
        control = _Control()
        peer = _Peer(control, fail_cleanup=True, fail_evaluation=True)
        with patch("moli_cdp_smoke.groups.process_environment.is_moli_browser", return_value=True):
            with self.assertRaisesRegex(KeyError, "listener cleanup failure") as caught:
                await _override_while_peer_paused(control, ObservedCdpSession(peer), BASELINE)
        self.assertIsInstance(caught.exception.__context__, RuntimeError)
        self.assertIn("original frame failure", str(caught.exception.__context__))
        self.assertTrue(peer.evaluation_finished)
        self.assertEqual(peer.calls[-2:], ["Debugger.resume", "Debugger.disable"])
