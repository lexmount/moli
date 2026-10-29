from __future__ import annotations

import asyncio
import sys
from typing import Awaitable, TypeVar

from .diagnostics import record_phase


ProgressResult = TypeVar("ProgressResult")
_CANCELLATION_GRACE_SECONDS = 5.0


def _consume_task_result(task: asyncio.Future[object]) -> None:
    try:
        task.exception()
    except (asyncio.CancelledError, Exception):
        pass


async def _cancel_with_grace(task: asyncio.Future[object]) -> None:
    if task.done():
        return
    task.cancel()
    done, _ = await asyncio.wait((task,), timeout=_CANCELLATION_GRACE_SECONDS)
    if task in done:
        _consume_task_result(task)
    else:
        task.add_done_callback(_consume_task_result)


async def await_with_progress(
    label: str,
    awaitable: Awaitable[ProgressResult],
    *,
    timeout_seconds: float | None = None,
) -> ProgressResult:
    if timeout_seconds is not None and timeout_seconds <= 0:
        raise ValueError("timeout_seconds must be positive")
    loop = asyncio.get_running_loop()
    started_at = loop.time()
    record_phase(label, "started")
    print(f"[moli-cdp-smoke] START {label}", file=sys.stderr, flush=True)
    task = asyncio.ensure_future(awaitable)
    try:
        if timeout_seconds is None:
            result = await task
        else:
            done, _ = await asyncio.wait((task,), timeout=timeout_seconds)
            if task not in done:
                raise TimeoutError(
                    f"{label} timed out after {timeout_seconds:g}s"
                )
            result = task.result()
    except BaseException as error:
        record_phase(label, "failed", error)
        await _cancel_with_grace(task)
        elapsed = loop.time() - started_at
        print(
            f"[moli-cdp-smoke] FAIL {label} elapsed={elapsed:.3f}s "
            f"error={type(error).__name__}",
            file=sys.stderr,
            flush=True,
        )
        raise
    elapsed = loop.time() - started_at
    record_phase(label, "passed")
    print(
        f"[moli-cdp-smoke] DONE {label} elapsed={elapsed:.3f}s",
        file=sys.stderr,
        flush=True,
    )
    return result
