from __future__ import annotations

import asyncio
import re
import unittest
from types import SimpleNamespace
from typing import Any

from moli_cdp_smoke.assertions import SmokeError
from moli_cdp_smoke.lifecycle import _pending_evaluation, pending_page_evaluation


class _Session:
    def __init__(self) -> None:
        self.handlers: list[Any] = []
        self.started = asyncio.Event()
        self.closed = asyncio.Event()
        self.canceled = asyncio.Event()
        self.token = ""
        self.block_enable = False

    def on(self, method: str, handler: Any) -> None:
        self.handlers.append(handler)

    def remove_listener(self, method: str, handler: Any) -> None:
        self.handlers.remove(handler)

    def emit(self, token: str) -> None:
        for handler in self.handlers:
            handler({"args": [{"value": token}]})

    async def send(self, method: str) -> dict[str, Any]:
        if self.block_enable:
            await asyncio.Event().wait()
        return {}

    async def evaluate(self, expression: str) -> None:
        self.token = re.search(r"moli-pending-evaluation-[0-9a-f]+", expression).group()
        self.started.set()
        try:
            await self.closed.wait()
            raise RuntimeError("owner closed")
        except asyncio.CancelledError:
            self.canceled.set()
            raise


class PendingEvaluationTests(unittest.IsolatedAsyncioTestCase):
    async def test_page_teardown_needs_no_auxiliary_session_or_closed_flag(self) -> None:
        class Page(_Session):
            @property
            def context(self) -> Any:
                raise AssertionError("a console marker needs no extra CDP session")

            def emit(self, token: str) -> None:
                for handler in self.handlers:
                    handler(SimpleNamespace(text=token))

        page = Page()

        async def close_after_ready() -> None:
            async with pending_page_evaluation(page) as pending:
                page.closed.set()
                with self.assertRaisesRegex(RuntimeError, "owner closed"):
                    await pending

        task = asyncio.create_task(close_after_ready())
        await page.started.wait()
        page.emit(page.token)
        await task
        self.assertEqual(page.handlers, [])

    async def test_only_the_matching_executor_marker_releases_the_gate(self) -> None:
        session, entered = _Session(), asyncio.Event()

        async def close_after_ready() -> None:
            async with _pending_evaluation(session, session.evaluate) as pending:
                entered.set()
                session.closed.set()
                with self.assertRaisesRegex(RuntimeError, "owner closed"):
                    await pending

        task = asyncio.create_task(close_after_ready())
        await session.started.wait()
        session.emit("unrelated-console-message")
        self.assertFalse(entered.is_set())
        session.emit(session.token)
        await task
        self.assertTrue(entered.is_set())
        self.assertEqual(session.handlers, [])

    async def test_returning_or_throwing_before_the_marker_is_a_failure(self) -> None:
        for throws in [False, True]:
            session = _Session()

            async def early(expression: str) -> Any:
                if throws:
                    raise ValueError("evaluation failed")
                return {"result": 42}

            with self.subTest(throws=throws), self.assertRaises(ValueError if throws else SmokeError):
                async with _pending_evaluation(session, early):
                    self.fail("an evaluation that never became pending passed the gate")
            self.assertEqual(session.handlers, [])

    async def test_marker_followed_by_completed_evaluation_is_not_pending(self) -> None:
        session = _Session()

        async def completed(expression: str) -> None:
            session.emit(re.search(r"moli-pending-evaluation-[0-9a-f]+", expression).group())

        with self.assertRaisesRegex(SmokeError, "completed before owner teardown"):
            async with _pending_evaluation(session, completed):
                self.fail("a completed command was treated as pending")
        self.assertEqual(session.handlers, [])

    async def test_missing_marker_fails_and_cancels_its_operation(self) -> None:
        session = _Session()
        with self.assertRaisesRegex(SmokeError, "did not enter its pending state"):
            async with _pending_evaluation(session, session.evaluate, timeout=0.01):
                self.fail("missing marker was accepted")
        self.assertTrue(session.canceled.is_set())
        self.assertEqual(session.handlers, [])

    async def test_runtime_enable_is_inside_the_setup_deadline(self) -> None:
        session = _Session()
        session.block_enable = True
        with self.assertRaisesRegex(SmokeError, "Runtime.enable"):
            async with _pending_evaluation(session, session.evaluate, timeout=0.01):
                self.fail("blocked Runtime.enable was accepted")
        self.assertFalse(session.started.is_set())
        self.assertEqual(session.handlers, [])

    async def test_canceling_setup_preserves_cancellation_and_releases_listeners(self) -> None:
        session = _Session()

        async def wait_for_ready() -> None:
            async with _pending_evaluation(session, session.evaluate):
                self.fail("setup unexpectedly completed")

        task = asyncio.create_task(wait_for_ready())
        await session.started.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(session.canceled.is_set())
        self.assertEqual(session.handlers, [])


if __name__ == "__main__":
    unittest.main()
