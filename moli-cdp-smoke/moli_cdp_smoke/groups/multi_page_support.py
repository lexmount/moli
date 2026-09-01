from __future__ import annotations

import asyncio
import json
import os
import urllib.request
from collections.abc import Awaitable, Callable, Sequence
from typing import Any

from ..assertions import SmokeError
from ..progress import await_with_progress

MultiPageCase = Callable[[Any, str, list[dict[str, Any]]], Awaitable[None]]
_CASE_ENV = "MOLI_MULTI_PAGE_CASES"
_SHARD_ENV = "MOLI_MULTI_PAGE_SHARD"


def multi_page_case_name(case: MultiPageCase) -> str:
    return case.__name__.removeprefix("_")


def select_multi_page_cases(
    cases: Sequence[MultiPageCase],
    *,
    requested: str | None = None,
    shard: str | None = None,
) -> tuple[MultiPageCase, ...]:
    by_name = {multi_page_case_name(case): case for case in cases}
    if len(by_name) != len(cases):
        raise ValueError("multi-page case names must be unique")

    requested = os.environ.get(_CASE_ENV, "") if requested is None else requested
    requested_names = tuple(
        dict.fromkeys(name.strip() for name in requested.split(",") if name.strip())
    )
    if requested_names:
        unknown = sorted(set(requested_names) - by_name.keys())
        if unknown:
            available = ", ".join(by_name)
            raise ValueError(
                f"unknown {_CASE_ENV} value(s): {', '.join(unknown)}; "
                f"available cases: {available}"
            )
        selected = tuple(by_name[name] for name in requested_names)
    else:
        selected = tuple(cases)

    shard = os.environ.get(_SHARD_ENV, "") if shard is None else shard
    shard = shard.strip()
    if shard:
        shard_index, shard_count = _parse_shard(shard)
        selected = tuple(
            case
            for index, case in enumerate(selected)
            if index % shard_count == shard_index
        )
        if not selected:
            raise ValueError(f"{_SHARD_ENV}={shard!r} selected no multi-page cases")

    return selected


async def run_multi_page_cases(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
    cases: Sequence[MultiPageCase],
    *,
    timeout_seconds: float = 20,
) -> None:
    selected = select_multi_page_cases(cases)
    failures: list[Exception] = []
    for case in selected:
        name = multi_page_case_name(case)
        try:
            await await_with_progress(
                f"multi-page/{name}",
                case(browser, fixture, results),
                timeout_seconds=timeout_seconds,
            )
        except Exception as error:  # noqa: BLE001 - keep running independent cases.
            error.add_note(f"multi-page case: {name}")
            failures.append(error)

    if failures:
        raise ExceptionGroup("multi-page cases failed", failures)


async def close_context(context: Any) -> None:
    try:
        await asyncio.wait_for(context.close(), timeout=5)
    except Exception as error:
        raise SmokeError(
            f"BrowserContext.close failed: {type(error).__name__}: {error}"
        ) from error


async def expect_protocol_error(awaitable: Awaitable[Any], label: str) -> str:
    try:
        result = await asyncio.wait_for(awaitable, timeout=5)
    except Exception as error:  # noqa: BLE001 - any CDP error is the expected result.
        return str(error)
    raise SmokeError(f"{label} unexpectedly succeeded: {result!r}")


def read_fixture_json(url: str) -> Any:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(url, timeout=2) as response:
        return json.load(response)


def runtime_value(response: dict[str, Any]) -> Any:
    return response.get("result", {}).get("value")


def _parse_shard(value: str) -> tuple[int, int]:
    try:
        raw_index, raw_count = value.split("/", maxsplit=1)
        index = int(raw_index)
        count = int(raw_count)
    except ValueError as error:
        raise ValueError(
            f"{_SHARD_ENV} must use one-based INDEX/COUNT syntax, got {value!r}"
        ) from error
    if count <= 0 or index <= 0 or index > count:
        raise ValueError(
            f"{_SHARD_ENV} must satisfy 1 <= INDEX <= COUNT, got {value!r}"
        )
    return index - 1, count
