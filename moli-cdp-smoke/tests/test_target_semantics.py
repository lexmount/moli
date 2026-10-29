from __future__ import annotations

import asyncio
import json
import unittest
from typing import Any
from unittest.mock import AsyncMock, patch

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups import target_semantics
from moli_cdp_smoke.raw_cdp import RawCdpClient


FIXTURE = "http://fixture.test"
SESSION = "page-session"
TARGET = "page-target"
LOADER = "requested-loader"


def lifecycle(frame: str = TARGET, loader: str = LOADER, session: str = SESSION) -> dict[str, Any]:
    return {
        "method": "Page.lifecycleEvent",
        "sessionId": session,
        "params": {"frameId": frame, "loaderId": loader, "name": "load"},
    }


class _TargetSocket:
    def __init__(
        self,
        phase: str,
        *,
        loader: str = LOADER,
        foreign_loads: bool = False,
        child_texts: list[str | None] | None = None,
    ) -> None:
        self.phase = phase
        self.loader = loader
        self.foreign_loads = foreign_loads
        self.child_texts = child_texts if child_texts is not None else ["child first", "child second"]
        self.messages: asyncio.Queue[str] = asyncio.Queue()
        self.commands: list[dict[str, Any]] = []
        self.closed = False

    async def send(self, raw: str) -> None:
        command = json.loads(raw)
        self.commands.append(command)
        method = command["method"]
        before: list[dict[str, Any]] = []
        after: list[dict[str, Any]] = []
        result: dict[str, Any] = {}
        if method == "Target.createBrowserContext":
            result = {"browserContextId": "context"}
        elif method == "Target.createTarget":
            result = {"targetId": TARGET}
            before.append({
                "method": "Target.attachedToTarget",
                "params": {
                    "sessionId": SESSION,
                    "waitingForDebugger": True,
                    "targetInfo": {
                        "targetId": TARGET,
                        "url": f"{FIXTURE}/semantic-frames?debugger-wait=1",
                    },
                },
            })
        elif method == "Page.setLifecycleEventsEnabled" and self.phase == "initial-document":
            before.append(lifecycle(loader="initial-about-blank-loader"))
        elif method == "Runtime.enable":
            if self.foreign_loads:
                before.extend([lifecycle(frame="child-frame"), lifecycle(session="other-session")])
            if self.phase == "runtime-enable":
                # Chromium's actual failure order: requested main load before
                # the Runtime.enable response, followed by a normal resume ACK.
                before.append(lifecycle(loader=self.loader))
        elif method == "Runtime.runIfWaitingForDebugger":
            if self.phase == "resume-before-reply":
                before.append(lifecycle(loader=self.loader))
            elif self.phase == "resume-after-reply":
                after.append(lifecycle(loader=self.loader))
        elif method == "Page.getFrameTree":
            result = {
                "frameTree": {
                    "frame": {
                        "id": TARGET,
                        "loaderId": LOADER,
                        "url": f"{FIXTURE}/semantic-frames?debugger-wait=1",
                    },
                    "childFrames": [
                        {"frame": {
                            "url": f"{FIXTURE}/semantic-frame-child?child=first&nested=1",
                        }},
                        {"frame": {"url": f"{FIXTURE}/semantic-frame-child?child=second"}},
                    ],
                },
            }
        elif method == "Runtime.evaluate":
            result = {"result": {"value": self.child_texts}}
        response: dict[str, Any] = {"id": command["id"], "result": result}
        if "sessionId" in command:
            response["sessionId"] = command["sessionId"]
        for message in [*before, response, *after]:
            self.messages.put_nowait(json.dumps(message))

    async def recv(self) -> str:
        return await self.messages.get()

    async def close(self) -> None:
        self.closed = True


class CreatedTargetLifecycleTests(unittest.IsolatedAsyncioTestCase):
    async def run_scenario(self, socket: _TargetSocket) -> dict[str, Any]:
        client = RawCdpClient(websocket=socket)  # type: ignore[arg-type]
        wait_for_event = target_semantics._raw_wait_for_event

        async def bounded_wait(*args: Any, **kwargs: Any) -> dict[str, Any]:
            return await wait_for_event(*args, **kwargs, timeout=0.01)

        with (
            patch.object(target_semantics, "connect_raw_cdp", AsyncMock(return_value=client)),
            patch.object(target_semantics, "_raw_wait_for_event", bounded_wait),
        ):
            return await target_semantics._created_target_debugger_wait_lifecycle("http://browser", FIXTURE)

    async def test_main_load_is_retained_across_command_response_boundaries(self) -> None:
        for phase in ["runtime-enable", "resume-before-reply", "resume-after-reply"]:
            with self.subTest(phase=phase):
                socket = _TargetSocket(phase)
                observed = await self.run_scenario(socket)
                self.assertEqual(observed["mainLoadLoaderId"], LOADER)
                self.assertEqual(observed["childTexts"], ["child first", "child second"])
                self.assertEqual(observed["loadObservedDuringRuntimeEnable"], phase == "runtime-enable")
                self.assertIn("Runtime.runIfWaitingForDebugger", [c["method"] for c in socket.commands])
                self.assertTrue(socket.closed)

    async def test_child_and_foreign_session_loads_do_not_satisfy_main_load(self) -> None:
        socket = _TargetSocket("resume-after-reply", foreign_loads=True)
        observed = await self.run_scenario(socket)
        self.assertEqual(observed["mainLoadLoaderId"], LOADER)
        self.assertFalse(observed["loadObservedDuringRuntimeEnable"])

    async def test_a_different_loader_does_not_satisfy_the_requested_navigation(self) -> None:
        socket = _TargetSocket("runtime-enable", loader="stale-loader")
        with self.assertRaisesRegex(SmokeError, "main load loader"):
            await self.run_scenario(socket)
        self.assertTrue(socket.closed)

    async def test_initial_placeholder_load_is_still_rejected(self) -> None:
        socket = _TargetSocket("initial-document")
        with self.assertRaisesRegex(SmokeError, "internal initial document"):
            await self.run_scenario(socket)
        self.assertNotIn("Runtime.runIfWaitingForDebugger", [c["method"] for c in socket.commands])
        self.assertTrue(socket.closed)

    async def test_missing_load_reports_the_expected_event_and_session(self) -> None:
        socket = _TargetSocket("missing", foreign_loads=True)
        with self.assertRaisesRegex(SmokeError, "Page.lifecycleEvent.*page-session"):
            await self.run_scenario(socket)
        self.assertTrue(socket.closed)

    async def test_child_documents_must_still_be_loaded(self) -> None:
        socket = _TargetSocket("resume-after-reply", child_texts=["child first", None])
        with self.assertRaisesRegex(SmokeError, "child frame documents"):
            await self.run_scenario(socket)
        self.assertTrue(socket.closed)


if __name__ == "__main__":
    unittest.main()
