from __future__ import annotations

import asyncio
import inspect
from typing import Any, Awaitable, Callable

from .diagnostics import record_phase


class SmokeError(AssertionError):
    pass


def assert_equal(actual: Any, expected: Any, label: str) -> None:
    if actual != expected:
        raise SmokeError(f"{label}: expected {expected!r}, got {actual!r}")


def record(results: list[dict[str, Any]], name: str, data: dict[str, Any] | None = None) -> None:
    entry: dict[str, Any] = {"name": name, "ok": True}
    if data:
        entry.update(data)
    results.append(entry)


def record_contract(
    results: list[dict[str, Any]],
    name: str,
    *,
    contract: str,
    source: str,
    commands: list[str],
    observed: Any,
) -> None:
    record(
        results,
        name,
        {
            "contract": contract,
            "source": source,
            "commands": commands,
            "observed": observed,
        },
    )


async def wait_until(
    predicate: Callable[[], bool | Awaitable[bool]],
    label: str,
    *,
    timeout_ms: int = 10_000,
    interval_ms: int = 50,
) -> None:
    record_phase(label, "waiting")
    timer = asyncio.timeout(timeout_ms / 1000)
    try:
        async with timer:
            while True:
                result = predicate()
                if inspect.isawaitable(result):
                    result = await result
                if result:
                    record_phase(label, "observed")
                    return
                await asyncio.sleep(interval_ms / 1000)
    except TimeoutError as error:
        if not timer.expired():
            raise
        record_phase(label, "timed_out", error)
        raise SmokeError(f"timed out waiting for {label} after {timeout_ms}ms") from error
