from __future__ import annotations

import asyncio
import unittest
from typing import Any

from moli_cdp_smoke.groups.text_input import TextInputPageCrashed, _input_command


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
