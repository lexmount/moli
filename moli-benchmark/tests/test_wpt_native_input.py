from __future__ import annotations

import asyncio
import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock

from moli_benchmark.raw_cdp import RawCdpConnectionClosed, RawCdpError
from moli_benchmark.wpt_cross.__main__ import _case_references_testdriver
from moli_benchmark.wpt_cross.case_set import WptCase
from moli_benchmark.wpt_cross.native_input import NativeInput, Pointer, key_description, permission_origin
from moli_benchmark.wpt_cross.runner import _AttachedPage, _close_page, _close_target


class RecordingClient:
    def __init__(self) -> None:
        self.commands = []
        self.close = AsyncMock()

    async def command(self, method, params=None, **kwargs):
        self.commands.append((method, params, kwargs))
        return SimpleNamespace(response={"result": {}})


class NativeInputTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        self.client = RecordingClient()
        self.driver = NativeInput(self.client, "input-session")

    def input_events(self):
        return [params for method, params, _ in self.client.commands if method.startswith("Input.")]

    async def test_send_keys_keeps_shift_for_adjacent_capitals_and_releases_it(self):
        await self.driver.send_keys("ABc!")
        events = self.input_events()
        self.assertEqual([event["text"] for event in events if "text" in event], ["A", "B", "c", "!"])
        self.assertEqual(
            [(event["type"], event["modifiers"]) for event in events if event["key"] == "Shift"],
            [("rawKeyDown", 8), ("keyUp", 0), ("rawKeyDown", 8), ("keyUp", 0)],
        )
        self.assertEqual(self.driver.modifiers(), 0)

    async def test_send_keys_null_releases_control_before_typing(self):
        await self.driver.send_keys("\ue009a\ue000x\ue00c")
        events = self.input_events()
        a_down = next(event for event in events if event["key"] == "a" and event["type"] != "keyUp")
        self.assertEqual(a_down["type"], "rawKeyDown")
        self.assertEqual(a_down["modifiers"], 2)
        self.assertNotIn("text", a_down)
        self.assertEqual([event["text"] for event in events if "text" in event], ["x"])
        escape = next(event for event in events if event["key"] == "Escape")
        self.assertEqual((escape["type"], escape["code"], escape["windowsVirtualKeyCode"]), ("rawKeyDown", "Escape", 27))
        self.assertEqual(self.driver.modifiers(), 0)

    async def test_failed_send_keys_releases_pressed_modifiers(self):
        with self.assertRaises(ValueError):
            await self.driver.send_keys("\ue009\ue0ff")
        self.assertEqual([(event["type"], event["key"]) for event in self.input_events()], [("rawKeyDown", "Control"), ("keyUp", "Control")])
        self.assertEqual(self.driver.modifiers(), 0)

    async def test_repeated_key_down_is_marked_as_repeat(self):
        await self.driver.key("keyboard", "x", True)
        await self.driver.key("keyboard", "x", True)
        await self.driver.key("keyboard", "x", False)
        await self.driver.key("keyboard", "x", False)
        self.assertEqual([event["autoRepeat"] for event in self.input_events()], [False, True, False])

    async def test_action_ticks_preserve_modifiers_across_pointer_actions(self):
        await self.driver.perform(1, {"id": 1, "token": "document", "kind": "actions", "actions": [
            {"id": "keyboard", "type": "key", "actions": [
                {"type": "keyDown", "value": "\ue008"}, {"type": "pause"}, {"type": "keyUp", "value": "\ue008"},
            ]},
            {"id": "mouse", "type": "pointer", "actions": [
                {"type": "pause"}, {"type": "pointerDown", "button": 0}, {"type": "pointerUp", "button": 0},
            ]},
        ]})
        self.assertEqual([(event["type"], event["modifiers"]) for event in self.input_events()], [
            ("rawKeyDown", 8), ("mousePressed", 8), ("keyUp", 0), ("mouseReleased", 0),
        ])

    async def test_invalid_later_action_has_no_partial_input_side_effects(self):
        with self.assertRaises(ValueError):
            await self.driver.perform(1, {"id": 1, "token": "document", "kind": "actions", "actions": [
                {"id": "keyboard", "type": "key", "actions": [
                    {"type": "keyDown", "value": "x"}, {"type": "invalid"},
                ]},
            ]})
        self.assertEqual(self.input_events(), [])

    async def test_failed_tick_stops_other_actions_before_rejecting(self):
        moving = asyncio.Event()
        stopped = asyncio.Event()

        async def action(context_id, request_id, token, source, item):
            if source["id"] == "moving":
                moving.set()
                try:
                    await asyncio.Event().wait()
                finally:
                    stopped.set()
            else:
                await moving.wait()
                raise ValueError("move target out of bounds")

        self.driver.action = action
        with self.assertRaisesRegex(ValueError, "move target out of bounds"):
            await self.driver.perform(1, {"id": 1, "token": "document", "kind": "actions", "actions": [
                {"id": source_id, "type": "pointer", "actions": [
                    {"type": "pointerMove", "x": 10, "y": 20},
                ]}
                for source_id in ("moving", "failing")
            ]})
        self.assertTrue(stopped.is_set())

    async def test_next_action_sequence_releases_previous_held_keys(self):
        await self.driver.key("keyboard", "\ue008", True)
        await self.driver.perform(1, {"id": 1, "token": "document", "kind": "actions", "actions": []})
        self.assertEqual([event["type"] for event in self.input_events()], ["rawKeyDown", "keyUp"])
        self.assertEqual(self.driver.modifiers(), 0)

    async def test_pen_move_keeps_pressure_without_reporting_a_button_change(self):
        pointer = Pointer(kind="pen", x=10, y=20)
        await self.driver.mouse(pointer, "mousePressed", 0, force=0.36)
        await self.driver.mouse(pointer, "mouseMoved")
        await self.driver.mouse(pointer, "mouseReleased", 0)
        events = self.input_events()
        self.assertEqual([event["force"] for event in events], [0.36, 0.5, 0])
        self.assertEqual(events[1]["button"], "none")
        self.assertEqual(events[1]["buttons"], 1)

    async def test_binding_replies_stay_in_their_originating_realm(self):
        def binding(sequence, context, token):
            return SimpleNamespace(sequence=sequence, payload={"params": {
                "executionContextId": context,
                "payload": json.dumps({"id": 1, "token": token, "kind": "click", "x": 1, "y": 1}),
            }})
        self.client.wait_for_event = AsyncMock(side_effect=[
            binding(1, 7, "retired"), binding(2, 8, "current"), RawCdpConnectionClosed("closed"),
        ])
        evaluations = []
        async def evaluate(context_id, expression):
            evaluations.append((context_id, expression))
            if context_id == 7:
                raise ValueError("context is retired")
        self.driver.evaluate = evaluate
        self.driver.perform = AsyncMock()
        await self.driver.run()
        self.driver.perform.assert_awaited_once()
        self.assertEqual(self.driver.perform.await_args.args[0], 8)
        self.assertTrue(any(context == 8 and '.finish(1, "current", null)' in expression for context, expression in evaluations))
        self.assertFalse(any(context == 8 and '"retired"' in expression for context, expression in evaluations))

    async def test_close_cancels_the_binding_listener(self):
        self.driver.task = asyncio.create_task(asyncio.Event().wait())
        await self.driver.close()
        self.assertTrue(self.driver.task.cancelled())
        self.client.close.assert_awaited_once()
        self.assertEqual([command[0] for command in self.client.commands], ["Runtime.removeBinding", "Target.detachFromTarget"])
        await self.driver.close()
        self.client.close.assert_awaited_once()

    async def test_close_releases_connection_after_listener_and_cleanup_fail(self):
        async def failed_listener():
            raise RuntimeError("engine exited")

        self.driver.task = asyncio.create_task(failed_listener())
        await asyncio.sleep(0)
        self.client.command = AsyncMock(side_effect=RawCdpConnectionClosed("closed"))
        await self.driver.close()
        self.assertEqual(self.client.command.await_count, 2)
        self.client.close.assert_awaited_once()

    async def test_permissions_use_the_selected_frame_origin_and_case_browser_context(self):
        self.driver.browser_context_id = "case-context"
        self.driver.evaluate = AsyncMock(return_value=[1, 0])
        self.driver.command = AsyncMock(side_effect=[{"frameTree": {
            "frame": {"securityOrigin": "https://top.test"},
            "childFrames": [
                {"frame": {"securityOrigin": "https://sibling.test"}},
                {"frame": {"securityOrigin": "https://outer.test"}, "childFrames": [
                    {"frame": {"id": "child-frame", "securityOrigin": "https://child.test", "url": "about:blank"}},
                ]},
            ],
        }}, {"backendNodeId": 42}, {"object": {"objectId": "owner-object"}},
            {"result": {"value": True}}, {}])
        descriptor = {"name": "clipboard-write", "allowWithoutSanitization": True}
        await self.driver.perform(9, {
            "id": 4, "token": "realm", "kind": "set_permission", "descriptor": descriptor, "state": "denied",
        })
        method, params, kwargs = self.client.commands[-1]
        self.assertEqual(method, "Browser.setPermission")
        self.assertEqual(params, {
            "permission": descriptor, "setting": "denied", "browserContextId": "case-context",
            "origin": "https://top.test", "embeddedOrigin": "https://child.test",
        })
        self.assertNotIn("session_id", kwargs)
        self.driver.command.assert_any_await("DOM.getFrameOwner", {"frameId": "child-frame"})
        self.driver.command.assert_any_await("Runtime.releaseObject", {"objectId": "owner-object"})
        self.driver.command.side_effect = None
        self.driver.command.return_value = {}
        await self.driver.close()
        reset = next(command for command in self.client.commands if command[0] == "Browser.resetPermissions")
        self.assertEqual(reset[1], {"browserContextId": "case-context"})
        self.assertNotIn("session_id", reset[2])

    async def test_permissions_reject_malformed_requests_before_any_protocol_mutation(self):
        for descriptor, state in [({}, "granted"), ({"name": ""}, "granted"), ({"name": 1}, "granted"),
                                  ({"name": "geolocation"}, "invalid"), ({"name": "geolocation"}, {})]:
            with self.subTest(descriptor=descriptor, state=state), self.assertRaises(ValueError):
                await self.driver.perform(1, {"id": 1, "token": "realm", "kind": "set_permission",
                                              "descriptor": descriptor, "state": state})
        self.assertEqual(self.client.commands, [])

    async def test_permissions_reject_detached_moved_and_opaque_frames(self):
        request = {"id": 1, "token": "realm", "kind": "set_permission", "descriptor": {"name": "geolocation"}, "state": "granted"}
        for paths, frame in [
            ([[0]], {"securityOrigin": "https://top.test"}),
            ([[], [0]], {"securityOrigin": "https://top.test"}),
            ([[]], {"securityOrigin": "null", "url": "https://opaque.test/"}),
        ]:
            with self.subTest(paths=paths, frame=frame):
                self.driver.evaluate = AsyncMock(side_effect=paths)
                self.driver.command = AsyncMock(return_value={"frameTree": {"frame": frame}})
                with self.assertRaises(ValueError):
                    await self.driver.perform(1, request)
        self.assertEqual(self.client.commands, [])

    async def test_failed_permission_command_propagates_and_still_schedules_reset(self):
        self.driver.evaluate = AsyncMock(return_value=[])
        self.driver.command = AsyncMock(return_value={"frameTree": {"frame": {"securityOrigin": "https://top.test"}}})
        self.client.command = AsyncMock(side_effect=RawCdpError("unsupported permission"))
        with self.assertRaisesRegex(RawCdpError, "unsupported permission"):
            await self.driver.perform(1, {"id": 1, "token": "realm", "kind": "set_permission",
                                          "descriptor": {"name": "unsupported"}, "state": "granted"})
        with self.assertRaisesRegex(RawCdpError, "reset WPT permissions"):
            await self.driver.close()
        self.assertEqual([call.args[0] for call in self.client.command.await_args_list], [
            "Browser.setPermission", "Browser.resetPermissions", "Runtime.removeBinding", "Target.detachFromTarget",
        ])
        self.client.close.assert_awaited_once()
        with self.assertRaisesRegex(RawCdpError, "reset WPT permissions"):
            await self.driver.close()
        self.client.close.assert_awaited_once()

    async def test_omitted_frame_cannot_grant_permission_to_a_sibling(self):
        self.driver.evaluate = AsyncMock(return_value=[0])
        self.driver.command = AsyncMock(side_effect=[
            {"frameTree": {"frame": {"securityOrigin": "https://top.test"}, "childFrames": [
                {"frame": {"id": "sibling", "securityOrigin": "https://sibling.test"}},
            ]}},
            {"backendNodeId": 42}, {"object": {"objectId": "sibling-owner"}},
            {"result": {"value": False}}, {},
        ])
        with self.assertRaisesRegex(ValueError, "does not match"):
            await self.driver.perform(1, {"id": 1, "token": "realm", "kind": "set_permission",
                                          "descriptor": {"name": "geolocation"}, "state": "granted"})
        self.assertEqual(self.client.commands, [])
        self.driver.command.assert_awaited_with("Runtime.releaseObject", {"objectId": "sibling-owner"})

    async def test_failed_permission_cleanup_still_disposes_case_targets(self):
        class PageClient:
            def __init__(self):
                self.commands = []

            async def send(self, method, params=None, **kwargs):
                self.commands.append((method, params))
                return len(self.commands)

            async def recv_until_id(self, command_id, **kwargs):
                method, _ = self.commands[command_id - 1]
                result = {"targetInfos": []} if method == "Target.getTargets" else {"success": True}
                return {"result": result}, []

        for browser_context_id in (None, "isolated-context"):
            with self.subTest(browser_context_id=browser_context_id):
                client = PageClient()
                native = SimpleNamespace(close=AsyncMock(side_effect=RawCdpError("reset failed")))
                page = _AttachedPage(browser_context_id, "test-target", "test-session", frozenset(), native)
                if browser_context_id is None:
                    with self.assertRaisesRegex(RawCdpError, "reset failed"):
                        await _close_page(client, page)
                    self.assertIn(("Target.closeTarget", {"targetId": "test-target"}), client.commands)
                else:
                    await _close_page(client, page)
                    self.assertIn(("Target.disposeBrowserContext", {"browserContextId": browser_context_id}), client.commands)


    async def test_target_cleanup_accepts_a_concurrently_closed_target(self):
        client = SimpleNamespace(
            send=AsyncMock(side_effect=[1, 2]),
            recv_until_id=AsyncMock(side_effect=[
                RawCdpError("No target with given id found"),
                ({"result": {"targetInfos": [{"targetId": "unrelated"}]}}, []),
            ]),
        )
        await _close_target(client, "case-target")

    async def test_target_cleanup_preserves_errors_for_a_live_target(self):
        client = SimpleNamespace(
            send=AsyncMock(side_effect=[1, 2]),
            recv_until_id=AsyncMock(side_effect=[
                RawCdpError("close rejected"),
                ({"result": {"targetInfos": [{"targetId": "case-target"}]}}, []),
            ]),
        )
        with self.assertRaisesRegex(RawCdpError, "close rejected"):
            await _close_target(client, "case-target")


class NativeInputSelectionTests(unittest.TestCase):
    def test_permission_origins_keep_inherited_and_opaque_origin_semantics(self):
        self.assertEqual(permission_origin({"url": "about:blank", "securityOrigin": "https://parent.test:8443"}), "https://parent.test:8443")
        self.assertEqual(permission_origin({"url": "https://top.test/path?q=1"}), "https://top.test")
        for origin in ("null", "", "file://", "https://user:password@top.test"):
            with self.subTest(origin=origin), self.assertRaises(ValueError):
                permission_origin({"url": "https://top.test", "securityOrigin": origin})

    def test_testdriver_detection_in_html_and_generated_variants(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "input.html").write_text('<script src="/resources/testdriver.js"></script>')
            (root / "keys.any.js").write_text('// META: script=/resources/testdriver.js\n')
            (root / "plain.html").write_text('<script src="/resources/testharness.js"></script>')
            self.assertTrue(_case_references_testdriver(root, WptCase("input.html?variant")))
            self.assertTrue(_case_references_testdriver(root, WptCase("keys.any.html?variant")))
            self.assertFalse(_case_references_testdriver(root, WptCase("plain.html")))
            self.assertFalse(_case_references_testdriver(root, WptCase("../input.html")))

    def test_special_keys_preserve_physical_location(self):
        self.assertEqual((key_description("\ue007").code, key_description("\ue007").location), ("NumpadEnter", 3))
        self.assertEqual((key_description("\ue050").code, key_description("\ue050").location), ("ShiftRight", 2))
