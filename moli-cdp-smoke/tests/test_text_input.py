from __future__ import annotations

import asyncio
import unittest
from typing import Any

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.groups.text_input import (
    TextInputPageCrashed,
    _assert_invalidated_target,
    _input_command,
    _text_event,
)


class InvalidatedInputTests(unittest.TestCase):
    def beforeinput(self) -> dict[str, Any]:
        return {"value": "", "events": ["beforeinput:field"], "textEvents": [],
                "editEvents": [{"type": "beforeinput", "inputType": "insertText", "data": "X"}]}

    def test_frame_removal_at_beforeinput_requires_the_native_event_and_no_edit(self) -> None:
        observed = self.beforeinput()
        _assert_invalidated_target(observed, "X", "beforeinput", "frame")
        for field, value in [
            ("value", "X"),
            ("events", []),
            ("events", ["beforeinput:field", "input:field"]),
            ("editEvents", []),
            ("textEvents", [_text_event("field", "field", "X")]),
        ]:
            with self.subTest(field=field, value=value), self.assertRaises(SmokeError):
                _assert_invalidated_target({**observed, field: value}, "X", "beforeinput", "frame")

    def test_removal_at_textinput_requires_exactly_one_trusted_native_event(self) -> None:
        observed = {**self.beforeinput(), "events": ["beforeinput:field", "textInput:field"],
                    "textEvents": [_text_event("field", "field", "X")]}
        _assert_invalidated_target(observed, "X", "textInput", "frame")
        for events in [[], observed["textEvents"] * 2,
                       [{**observed["textEvents"][0], "trusted": False}]]:
            with self.subTest(events=events), self.assertRaises(SmokeError):
                _assert_invalidated_target({**observed, "textEvents": events}, "X", "textInput", "frame")


class Page:
    def __init__(self) -> None:
        self.listeners: list[Any] = []

    def on(self, event: str, handler: Any) -> None:
        self.listeners.append(handler)

    def remove_listener(self, event: str, handler: Any) -> None:
        self.listeners.remove(handler)

    def crash(self) -> None:
        for handler in tuple(self.listeners):
            handler()


class NativeInputLifetimeTests(unittest.IsolatedAsyncioTestCase):
    async def test_success_preserves_the_response_and_releases_the_listener(self) -> None:
        page = Page()
        response = {"result": 42}

        async def command() -> Any:
            return response

        self.assertIs(await _input_command(page, command()), response)
        self.assertEqual(page.listeners, [])

    async def test_protocol_errors_remain_failures(self) -> None:
        page = Page()
        error = RuntimeError("invalid input parameter")

        async def command() -> None:
            raise error

        with self.assertRaises(RuntimeError) as caught:
            await _input_command(page, command())
        self.assertIs(caught.exception, error)
        self.assertEqual(page.listeners, [])

    async def test_crash_cancels_a_command_that_never_gets_a_reply(self) -> None:
        page = Page()
        entered, released = asyncio.Event(), asyncio.Event()

        async def command() -> None:
            entered.set()
            try:
                await asyncio.Event().wait()
            finally:
                released.set()

        pending = asyncio.create_task(_input_command(page, command()))
        await entered.wait()
        page.crash()
        with self.assertRaisesRegex(TextInputPageCrashed, "page crashed"):
            await asyncio.wait_for(pending, 1)
        self.assertTrue(released.is_set())
        self.assertEqual(page.listeners, [])

    async def test_caller_cancellation_also_releases_the_pending_command(self) -> None:
        page = Page()
        entered, released = asyncio.Event(), asyncio.Event()

        async def command() -> None:
            entered.set()
            try:
                await asyncio.Event().wait()
            finally:
                released.set()

        pending = asyncio.create_task(_input_command(page, command()))
        await entered.wait()
        pending.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await pending
        self.assertTrue(released.is_set())
        self.assertEqual(page.listeners, [])
