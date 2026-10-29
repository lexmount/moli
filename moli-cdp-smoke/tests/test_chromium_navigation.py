from __future__ import annotations

import asyncio
import unittest
from types import SimpleNamespace
from typing import Any
from unittest.mock import patch

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups import chromium_cdp_support as navigation


URL = "http://fixture.test/plain?requested"
FRAME = "main-frame"
LOADER = "requested-loader"


class _NavigationSession:
    def __init__(self, *, early: bool = False, complete: bool = True,
                 same_document: bool = False, error: str | None = None,
                 block_method: str | None = None) -> None:
        self.early = early
        self.complete = complete
        self.same_document = same_document
        self.error = error
        self.block_method = block_method
        self.listeners: dict[str, list[Any]] = {}
        self.commands: list[str] = []
        self.ready = False

    def on(self, method: str, listener: Any) -> None:
        self.listeners.setdefault(method, []).append(listener)

    def remove_listener(self, method: str, listener: Any) -> None:
        self.listeners[method].remove(listener)

    def emit(self, method: str, params: dict[str, Any]) -> None:
        for listener in self.listeners.get(method, []):
            listener(params)

    def document_ready(self) -> None:
        self.ready = True
        if self.same_document:
            self.emit("Page.navigatedWithinDocument", {"frameId": FRAME, "url": URL})
        else:
            self.emit("Page.lifecycleEvent", {
                "frameId": FRAME, "loaderId": LOADER, "name": "DOMContentLoaded",
            })

    async def send(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        self.commands.append(method)
        if method == self.block_method:
            await asyncio.Event().wait()
        if method == "Page.setLifecycleEventsEnabled":
            self.emit("Page.lifecycleEvent", {
                "frameId": FRAME, "loaderId": "old-loader", "name": "DOMContentLoaded",
            })
        if method == "Page.navigate":
            if self.error:
                return {"frameId": FRAME, "errorText": self.error}
            self.emit("Page.lifecycleEvent", {
                "frameId": "child-frame", "loaderId": LOADER, "name": "DOMContentLoaded",
            })
            self.emit("Page.lifecycleEvent", {
                "frameId": FRAME, "loaderId": "old-loader", "name": "DOMContentLoaded",
            })
            if self.complete:
                if self.early:
                    self.document_ready()
                else:
                    asyncio.get_running_loop().call_soon(self.document_ready)
            return {"frameId": FRAME, **({} if self.same_document else {"loaderId": LOADER})}
        if method == "Runtime.evaluate":
            if not self.ready:
                raise AssertionError("evaluated while the requested loader was still loading")
            return {"result": {"value": {"url": URL, "readyState": "interactive"}}}
        return {}


class ChromiumNavigationTests(unittest.IsolatedAsyncioTestCase):
    async def test_does_not_evaluate_until_requested_loader_dom_ready(self) -> None:
        session = _NavigationSession()
        await navigation._navigate_with_cdp_until_dom_ready(SimpleNamespace(cdp=session), URL)
        self.assertEqual(session.commands.count("Runtime.evaluate"), 1)
        self.assertTrue(all(not listeners for listeners in session.listeners.values()))

    async def test_dom_ready_before_navigate_reply_is_retained(self) -> None:
        session = _NavigationSession(early=True)
        await navigation._navigate_with_cdp_until_dom_ready(SimpleNamespace(cdp=session), URL)
        self.assertEqual(session.commands.count("Runtime.evaluate"), 1)

    async def test_same_document_navigation_uses_its_frame_and_url(self) -> None:
        session = _NavigationSession(same_document=True)
        await navigation._navigate_with_cdp_until_dom_ready(SimpleNamespace(cdp=session), URL)
        self.assertEqual(session.commands.count("Runtime.evaluate"), 1)

    async def test_stale_and_child_events_cannot_satisfy_the_navigation(self) -> None:
        session = _NavigationSession(complete=False)
        with patch.object(navigation, "_NAVIGATION_TIMEOUT_SECONDS", 0.01):
            with self.assertRaisesRegex(SmokeError, "requested-loader"):
                await navigation._navigate_with_cdp_until_dom_ready(SimpleNamespace(cdp=session), URL)
        self.assertNotIn("Runtime.evaluate", session.commands)
        self.assertTrue(all(not listeners for listeners in session.listeners.values()))

    async def test_navigation_error_is_not_replaced_by_a_readiness_timeout(self) -> None:
        session = _NavigationSession(error="net::ERR_ABORTED")
        with self.assertRaisesRegex(SmokeError, "net::ERR_ABORTED"):
            await navigation._navigate_with_cdp_until_dom_ready(SimpleNamespace(cdp=session), URL)
        self.assertNotIn("Runtime.evaluate", session.commands)
        self.assertTrue(all(not listeners for listeners in session.listeners.values()))

    async def test_pending_cdp_commands_respect_the_navigation_deadline(self) -> None:
        for method in ("Page.navigate", "Runtime.evaluate"):
            with self.subTest(method=method):
                session = _NavigationSession(block_method=method)
                with patch.object(navigation, "_NAVIGATION_TIMEOUT_SECONDS", 0.01):
                    with self.assertRaisesRegex(SmokeError, method):
                        await navigation._navigate_with_cdp_until_dom_ready(SimpleNamespace(cdp=session), URL)
                self.assertTrue(all(not listeners for listeners in session.listeners.values()))
