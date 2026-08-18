from __future__ import annotations

import asyncio
import sys
from typing import Awaitable, TypeVar


ProgressResult = TypeVar("ProgressResult")


async def await_with_progress(
    label: str,
    awaitable: Awaitable[ProgressResult],
) -> ProgressResult:
    loop = asyncio.get_running_loop()
    started_at = loop.time()
    print(f"[moli-cdp-smoke] START {label}", file=sys.stderr, flush=True)
    try:
        result = await awaitable
    except BaseException as error:
        elapsed = loop.time() - started_at
        print(
            f"[moli-cdp-smoke] FAIL {label} elapsed={elapsed:.3f}s "
            f"error={type(error).__name__}",
            file=sys.stderr,
            flush=True,
        )
        raise
    elapsed = loop.time() - started_at
    print(
        f"[moli-cdp-smoke] DONE {label} elapsed={elapsed:.3f}s",
        file=sys.stderr,
        flush=True,
    )
    return result
