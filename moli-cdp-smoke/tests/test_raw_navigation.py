from __future__ import annotations

import asyncio
import unittest
from collections import deque
from collections.abc import Sequence
from typing import Any

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups import layout_screenshot as navigation
from moli_cdp_smoke.navigation import navigate_cdp


class _Client:
    def __init__(self, messages: Sequence[dict[str, Any]] = ()) -> None:
        self.messages = deque(messages)
        self.commands: list[tuple[str, dict[str, Any] | None, str | None]] = []
        self.waiting = asyncio.Event()

    async def recv(self) -> dict[str, Any]:
        if self.messages:
            return self.messages.popleft()
        self.waiting.set()
        await asyncio.Event().wait()
        raise AssertionError("unreachable")

    async def send(self, method: str, params: dict[str, Any] | None = None,
                   *, session_id: str | None = None) -> int:
        self.commands.append((method, params, session_id))
        result = {
            "Target.createTarget": {"targetId": "target"},
            "Target.attachToTarget": {"sessionId": "page"},
            "Page.navigate": {"frameId": "frame", "errorText": "net::ERR_ABORTED"},
        }.get(method, {})
        self.messages.append({"id": len(self.commands), "result": result,
                              **({"sessionId": session_id} if session_id else {})})
        return len(self.commands)


def _commit(loader: str, *, frame: str = "frame", session: str = "page") -> dict[str, Any]:
    return {"method": "Page.frameNavigated", "sessionId": session,
            "params": {"frame": {"id": frame, "loaderId": loader}}}


def _load(loader: str, *, frame: str = "frame", session: str = "page") -> dict[str, Any]:
    return {"method": "Page.lifecycleEvent", "sessionId": session,
            "params": {"frameId": frame, "loaderId": loader, "name": "load"}}


class RawNavigationDiagnosticsTests(unittest.IsolatedAsyncioTestCase):
    async def test_response_timeout_keeps_the_command_id_and_last_wire_events(self) -> None:
        client = _Client([{"method": "Page.frameStartedLoading", "sessionId": "page",
                           "params": {"frameId": "frame"}}])
        with self.assertRaisesRegex(SmokeError, "id=17.*frameStartedLoading"):
            await navigation._response_for_id(client, 17, timeout=0.01)

    async def test_failed_initial_navigation_closes_its_owned_target(self) -> None:
        client = _Client()
        with self.assertRaisesRegex(SmokeError, "net::ERR_ABORTED"):
            await navigation._open_target(client, "http://fixture.test/failed")
        self.assertEqual(client.commands[-1],
                         ("Target.closeTarget", {"targetId": "target"}, None))


class DocumentLoadTests(unittest.IsolatedAsyncioTestCase):
    async def test_old_child_and_other_session_loads_cannot_release_the_gate(self) -> None:
        stale = [{"method": "Page.loadEventFired", "sessionId": "page"},
                 _commit("old"), _load("old"),
                 _commit("new", frame="child"), _load("new", frame="child"),
                 _commit("new", session="other"), _load("new", session="other")]
        client = _Client([*stale, _commit("new"), _load("new")])
        await navigation._wait_for_load(client, "page", [], frame_id="frame", loader_id="new")
        self.assertFalse(client.messages)

    async def test_events_received_before_the_navigation_reply_are_retained(self) -> None:
        client = _Client()
        await navigation._wait_for_load(client, "page", [_commit("new"), _load("new")],
                                        frame_id="frame", loader_id="new")
        self.assertFalse(client.waiting.is_set())

    async def test_load_and_commit_can_arrive_on_either_side_of_the_reply(self) -> None:
        for early, late in [(_commit("new"), _load("new")), (_load("new"), _commit("new"))]:
            with self.subTest(early=early["method"]):
                client = _Client([late])
                await navigation._wait_for_load(client, "page", [early],
                                                frame_id="frame", loader_id="new")
                self.assertFalse(client.messages)

    async def test_commit_alone_does_not_satisfy_load_and_timeout_has_identity(self) -> None:
        client = _Client([_commit("new")])
        with self.assertRaisesRegex(SmokeError, "session=page frame=frame loader=new.*committed=True"):
            await navigation._wait_for_load(client, "page", [], frame_id="frame",
                                            loader_id="new", timeout=0.01)

    async def test_load_without_this_navigation_commit_is_not_success(self) -> None:
        client = _Client([_load("new")])
        with self.assertRaisesRegex(SmokeError, "committed=False.*lifecycleEvent"):
            await navigation._wait_for_load(client, "page", [], frame_id="frame",
                                            loader_id="new", timeout=0.01)

    async def test_reload_identifies_the_new_loader_and_rejects_replay(self) -> None:
        client = _Client([_commit("old"), _load("old"), _load("new"), _commit("new")])
        await navigation._wait_for_load(client, "page", [], frame_id="frame",
                                        loader_id=None, previous_loader_id="old")
        self.assertFalse(client.messages)

    async def test_old_document_replay_cannot_satisfy_reload(self) -> None:
        client = _Client([_commit("old"), _load("old")])
        with self.assertRaisesRegex(SmokeError, "loader=None previousLoader=old.*committed=False"):
            await navigation._wait_for_load(client, "page", [], frame_id="frame",
                                            loader_id=None, previous_loader_id="old", timeout=0.01)

    async def test_crash_and_detach_report_the_actual_failure_without_waiting(self) -> None:
        for message, detail in [
            ({"method": "Inspector.targetCrashed", "sessionId": "page"}, "crashed"),
            ({"method": "Target.detachedFromTarget", "params": {"sessionId": "page"}}, "detached"),
        ]:
            with self.subTest(detail=detail):
                with self.assertRaisesRegex(SmokeError, detail):
                    await navigation._wait_for_load(_Client([message]), "page", [],
                                                    frame_id="frame", loader_id="new")

    async def test_cancellation_keeps_its_original_exception(self) -> None:
        client = _Client()
        task = asyncio.create_task(navigation._wait_for_load(
            client, "page", [], frame_id="frame", loader_id="new"))
        await client.waiting.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task


class _Session:
    def __init__(self, *, early: bool = False, complete: bool = True,
                 blocked: str | None = None) -> None:
        self.listeners: dict[str, list[Any]] = {}
        self.commands: list[str] = []
        self.early, self.complete, self.blocked = early, complete, blocked

    def on(self, method: str, handler: Any) -> None:
        self.listeners.setdefault(method, []).append(handler)

    def remove_listener(self, method: str, handler: Any) -> None:
        self.listeners[method].remove(handler)

    def emit(self, message: dict[str, Any]) -> None:
        for handler in self.listeners.get(message["method"], []):
            handler(message["params"])

    def load(self) -> None:
        self.emit(_commit("new"))
        self.emit(_load("new"))

    async def send(self, method: str, params: Any = None) -> dict[str, Any]:
        self.commands.append(method)
        if method == self.blocked:
            await asyncio.Event().wait()
        if method == "Page.setLifecycleEventsEnabled":
            self.emit(_commit("old"))
            self.emit(_load("old"))
        if method == "Page.navigate":
            self.emit(_load("new", frame="child"))
            if self.complete:
                if self.early:
                    self.load()
                else:
                    asyncio.get_running_loop().call_soon(self.load)
            return {"frameId": "frame", "loaderId": "new"}
        return {}


class SessionDocumentLoadTests(unittest.IsolatedAsyncioTestCase):
    async def test_reply_and_matching_events_can_arrive_in_either_order(self) -> None:
        for early in [True, False]:
            with self.subTest(early=early):
                session = _Session(early=early)
                await navigate_cdp(session, "http://fixture.test/plain")
                self.assertTrue(all(not handlers for handlers in session.listeners.values()))

    async def test_replayed_and_child_events_do_not_complete_the_navigation(self) -> None:
        session = _Session(complete=False)
        with self.assertRaisesRegex(SmokeError, "frame=frame loader=new.*committed=False"):
            await navigate_cdp(session, "http://fixture.test/plain", timeout=0.01)
        self.assertTrue(all(not handlers for handlers in session.listeners.values()))

    async def test_setup_and_navigation_share_the_same_deadline(self) -> None:
        for method in ["Page.enable", "Page.setLifecycleEventsEnabled", "Page.navigate"]:
            with self.subTest(method=method):
                session = _Session(blocked=method)
                with self.assertRaisesRegex(SmokeError, "navigation reply pending"):
                    await navigate_cdp(session, "http://fixture.test/plain", timeout=0.01)
                self.assertTrue(all(not handlers for handlers in session.listeners.values()))


if __name__ == "__main__":
    unittest.main()
