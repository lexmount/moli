from __future__ import annotations

import asyncio
from contextlib import suppress
from typing import Any


from ..diagnostics import new_cdp_session
from ..assertions import SmokeError
from ..state import SmokeState


async def run_fetch_runtime_teardown_group(state: SmokeState) -> None:
    """Prove that an exact old fetch runtime can retire from its callback thread.

    A held module fetch retains the final request-side runtime lease while CDP
    disposes its BrowserContext. Cancellation completes on ``lm-fetch-semantics``.
    The owner/handle split must let that callback release the lease while an
    external owner performs the join, without closing the browser connection.
    """

    gate = state.fixture_server.fetch_runtime_teardown_gate
    gate.reset()
    context = await state.browser.new_context()
    page = await context.new_page()
    cdp = await new_cdp_session(context, page)
    browser_cdp = await state.browser.new_browser_cdp_session()
    context_closed = False
    replacement_context: Any | None = None

    try:
        await asyncio.wait_for(
            page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000),
            timeout=12,
        )
        await _runtime_value(
            cdp,
            """
            import('/fetch-runtime-teardown-held.mjs').catch(() => {});
            'started';
            """,
        )

        request_seen = await asyncio.to_thread(gate.request_seen.wait, 5)
        if not request_seen:
            raise SmokeError("held module request did not reach the fixture gate")

        await asyncio.wait_for(context.close(), timeout=5)
        context_closed = True

        gate.release_response.set()
        response_completed = await asyncio.to_thread(gate.response_completed.wait, 5)
        if not response_completed:
            raise SmokeError("held module response did not complete")

        version = await _send_cdp(browser_cdp, "Browser.getVersion")
        if not version.get("protocolVersion"):
            raise SmokeError(
                f"CDP connection did not survive fetch runtime teardown: {version}"
            )

        replacement_context = await state.browser.new_context()
        replacement_page = await replacement_context.new_page()
        response = await asyncio.wait_for(
            replacement_page.goto(
                f"{state.fixture}/plain?after-fetch-runtime-teardown",
                wait_until="load",
                timeout=10_000,
            ),
            timeout=12,
        )
        if response is None or not response.ok:
            raise SmokeError("replacement BrowserContext could not load after teardown")

        state.record(
            "cdp_fetch_runtime_teardown_callback_survives",
            {
                "retiredRequest": "/fetch-runtime-teardown-held.mjs",
                "replacementDocument": await replacement_page.text_content("main"),
                "protocolVersion": version["protocolVersion"],
            },
        )
    finally:
        gate.release_response.set()
        if replacement_context is not None:
            with suppress(Exception):
                await asyncio.wait_for(replacement_context.close(), timeout=5)
        if not context_closed:
            with suppress(Exception):
                await asyncio.wait_for(context.close(), timeout=5)
        with suppress(Exception):
            await asyncio.wait_for(browser_cdp.detach(), timeout=2)


async def _send_cdp(
    cdp: Any,
    method: str,
    params: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return await asyncio.wait_for(cdp.send(method, params or {}), timeout=5)


async def _runtime_value(
    cdp: Any,
    expression: str,
    *,
    await_promise: bool = False,
) -> Any:
    result = await _send_cdp(
        cdp,
        "Runtime.evaluate",
        {
            "expression": expression,
            "awaitPromise": await_promise,
            "returnByValue": True,
        },
    )
    if result.get("exceptionDetails"):
        raise SmokeError(f"Runtime.evaluate failed: {result}")
    return result.get("result", {}).get("value")
