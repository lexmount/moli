"""Observe an evaluation entering its pending state before destroying its owner."""

from __future__ import annotations

import asyncio
import uuid
from collections.abc import AsyncIterator, Awaitable, Callable
from contextlib import asynccontextmanager
from typing import Any

from .diagnostics import record_phase
from .assertions import SmokeError


@asynccontextmanager
async def _pending_task(
    evaluate: Callable[[str], Awaitable[Any]],
    token: str,
    ready: asyncio.Future[None],
    deadline: float,
) -> AsyncIterator[asyncio.Task[Any]]:
    record_phase(token, "evaluation_sent")
    pending = asyncio.create_task(evaluate(
        f"new Promise(() => {{ console.debug({token!r}); }})"
    ))
    try:
        done, _ = await asyncio.wait(
            (ready, pending), timeout=max(0, deadline - asyncio.get_running_loop().time()),
            return_when=asyncio.FIRST_COMPLETED,
        )
        if pending in done:
            result = pending.result()
            raise SmokeError(f"evaluation completed before owner teardown: {result!r}")
        if ready not in done:
            raise SmokeError(f"evaluation did not enter its pending state before the deadline; marker={token}")
        record_phase(token, "evaluation_pending")
        yield pending
    finally:
        record_phase(token, "evaluation_cleanup")
        ready.cancel()
        if not pending.done():
            pending.cancel()
        await asyncio.gather(pending, return_exceptions=True)


@asynccontextmanager
async def _pending_evaluation(
    session: Any,
    evaluate: Callable[[str], Awaitable[Any]],
    *,
    timeout: float = 5,
) -> AsyncIterator[asyncio.Task[Any]]:
    token = "moli-pending-evaluation-" + uuid.uuid4().hex
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeout
    ready = loop.create_future()

    def entered(params: dict[str, Any]) -> None:
        args = params.get("args", [])
        if args and args[0].get("value") == token and not ready.done():
            ready.set_result(None)

    timer = asyncio.timeout_at(deadline)
    try:
        async with timer:
            await session.send("Runtime.enable")
    except TimeoutError as error:
        if not timer.expired():
            raise
        raise SmokeError(f"Runtime.enable did not complete before pending evaluation after {timeout}s") from error
    session.on("Runtime.consoleAPICalled", entered)
    try:
        async with _pending_task(evaluate, token, ready, deadline) as pending:
            yield pending
    finally:
        session.remove_listener("Runtime.consoleAPICalled", entered)


@asynccontextmanager
async def pending_runtime_evaluation(session: Any) -> AsyncIterator[asyncio.Task[Any]]:
    async def evaluate(expression: str) -> Any:
        return await session.send("Runtime.evaluate", {
            "expression": expression, "awaitPromise": True, "returnByValue": True,
        })

    async with _pending_evaluation(session, evaluate) as pending:
        yield pending


@asynccontextmanager
async def pending_page_evaluation(page: Any, *, timeout: float = 5) -> AsyncIterator[asyncio.Task[Any]]:
    token = "moli-pending-evaluation-" + uuid.uuid4().hex
    loop = asyncio.get_running_loop()
    ready = loop.create_future()

    def entered(message: Any) -> None:
        if message.text == token and not ready.done():
            ready.set_result(None)

    page.on("console", entered)
    try:
        async with _pending_task(page.evaluate, token, ready, loop.time() + timeout) as pending:
            yield pending
    finally:
        page.remove_listener("console", entered)
