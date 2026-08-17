from __future__ import annotations

import asyncio
import base64
import json
import urllib.request
from typing import Any

from ..assertions import SmokeError, assert_equal, record, record_contract
from ..png_image import DecodedPng, decode_png
from ..raw_cdp import RawCdpClient, connect_raw_cdp, discover_websocket_url


async def _response_for_id(
    client: RawCdpClient,
    message_id: int,
    *,
    timeout: float = 10.0,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out waiting for action-window response id={message_id}: {seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == message_id:
            return message, seen


async def _success(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    message_id = await client.send(method, params, session_id=session_id)
    response, seen = await _response_for_id(client, message_id)
    if session_id is not None:
        assert_equal(response.get("sessionId"), session_id, f"{method} response session")
    if "error" in response:
        raise SmokeError(f"{method} failed: {response['error']!r}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise SmokeError(f"{method} returned no result object: {response!r}")
    return result, seen


async def _wait_for_load(
    client: RawCdpClient,
    session_id: str,
    seen: list[dict[str, Any]],
) -> None:
    if any(
        message.get("sessionId") == session_id
        and message.get("method") == "Page.loadEventFired"
        for message in seen
    ):
        return
    deadline = asyncio.get_running_loop().time() + 10.0
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError("timed out waiting for action-window Page.loadEventFired")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        if (
            message.get("sessionId") == session_id
            and message.get("method") == "Page.loadEventFired"
        ):
            return


async def _open_target(
    client: RawCdpClient,
) -> tuple[str, str]:
    context, _ = await _success(client, "Target.createBrowserContext")
    context_id = context.get("browserContextId")
    if not isinstance(context_id, str) or not context_id:
        raise SmokeError(f"Target.createBrowserContext returned no id: {context!r}")
    created, _ = await _success(
        client,
        "Target.createTarget",
        {"browserContextId": context_id, "url": "about:blank"},
    )
    target_id = created.get("targetId")
    if not isinstance(target_id, str) or not target_id:
        raise SmokeError(f"Target.createTarget returned no targetId: {created!r}")
    attached, _ = await _success(
        client,
        "Target.attachToTarget",
        {"targetId": target_id, "flatten": True},
    )
    session_id = attached.get("sessionId")
    if not isinstance(session_id, str) or not session_id:
        raise SmokeError(f"Target.attachToTarget returned no sessionId: {attached!r}")
    await _success(client, "Page.enable", session_id=session_id)
    await _success(
        client,
        "Emulation.setDeviceMetricsOverride",
        {
            "width": 200,
            "height": 200,
            "deviceScaleFactor": 1,
            "mobile": False,
        },
        session_id=session_id,
    )
    return context_id, session_id


async def _navigate(
    client: RawCdpClient,
    session_id: str,
    url: str,
) -> None:
    navigation, seen = await _success(
        client,
        "Page.navigate",
        {"url": url},
        session_id=session_id,
    )
    if navigation.get("errorText"):
        raise SmokeError(f"Page.navigate failed: {navigation!r}")
    await _wait_for_load(client, session_id, seen)


async def _evaluate_json(
    client: RawCdpClient,
    session_id: str,
    expression: str,
) -> Any:
    evaluation, _ = await _success(
        client,
        "Runtime.evaluate",
        {"expression": expression, "returnByValue": True},
        session_id=session_id,
    )
    if evaluation.get("exceptionDetails"):
        raise SmokeError(f"Runtime.evaluate failed: {evaluation!r}")
    remote = evaluation.get("result")
    value = remote.get("value") if isinstance(remote, dict) else None
    if not isinstance(value, str):
        raise SmokeError(f"Runtime.evaluate returned no JSON string: {evaluation!r}")
    try:
        return json.loads(value)
    except json.JSONDecodeError as error:
        raise SmokeError(f"Runtime.evaluate returned invalid JSON: {value!r}") from error


async def _dispatch_wheel(
    client: RawCdpClient,
    session_id: str,
    delta_y: float,
    *,
    delta_x: float = 0,
    x: float = 20,
    y: float = 20,
) -> None:
    await _success(
        client,
        "Input.dispatchMouseEvent",
        {
            "type": "mouseWheel",
            "x": x,
            "y": y,
            "deltaX": delta_x,
            "deltaY": delta_y,
        },
        session_id=session_id,
    )


async def _capture_png(client: RawCdpClient, session_id: str) -> DecodedPng:
    capture, _ = await _success(
        client,
        "Page.captureScreenshot",
        {
            "format": "png",
            "quality": 100,
            "fromSurface": True,
            "captureBeyondViewport": False,
        },
        session_id=session_id,
    )
    encoded = capture.get("data")
    if not isinstance(encoded, str) or not encoded:
        raise SmokeError(f"Page.captureScreenshot returned no image: {capture!r}")
    try:
        return decode_png(base64.b64decode(encoded, validate=True))
    except ValueError as error:
        raise SmokeError("Page.captureScreenshot returned an invalid PNG") from error


def _read_json_url(url: str) -> dict[str, Any]:
    with urllib.request.urlopen(url, timeout=2) as response:
        value = json.loads(response.read().decode("utf-8"))
    if not isinstance(value, dict):
        raise SmokeError(f"fixture returned a non-object from {url}: {value!r}")
    return value


async def _fixture_json(url: str) -> dict[str, Any]:
    return await asyncio.to_thread(_read_json_url, url)


async def _reset_witness(fixture: str) -> None:
    result = await _fixture_json(f"{fixture}/action-window-witness/reset")
    assert_equal(result.get("count"), 0, "action-window witness reset")


async def _witness_count(fixture: str) -> int:
    result = await _fixture_json(f"{fixture}/action-window-witness/status")
    count = result.get("count")
    if not isinstance(count, int):
        raise SmokeError(f"action-window witness returned no count: {result!r}")
    return count


async def _wait_for_witness(
    fixture: str,
    expected: int,
    *,
    timeout: float = 3.0,
) -> None:
    deadline = asyncio.get_running_loop().time() + timeout
    while asyncio.get_running_loop().time() < deadline:
        if await _witness_count(fixture) >= expected:
            return
        await asyncio.sleep(0.025)
    raise SmokeError(
        f"timed out waiting for action-window witness count {expected}; "
        f"observed {await _witness_count(fixture)}"
    )


async def _wait_for_initial_intersection(
    client: RawCdpClient,
    session_id: str,
) -> None:
    deadline = asyncio.get_running_loop().time() + 2.0
    while asyncio.get_running_loop().time() < deadline:
        state = await _evaluate_json(
            client,
            session_id,
            "JSON.stringify(__actionWindowIoLog)",
        )
        if state == [False]:
            return
        await asyncio.sleep(0.025)
    raise SmokeError("initial IntersectionObserver state was not published")


async def _run_deadline_contract(
    client: RawCdpClient,
    session_id: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    await _navigate(client, session_id, f"{fixture}/action-window-deadline")
    await _wait_for_initial_intersection(client, session_id)

    opened_at = asyncio.get_running_loop().time()
    for index, delta_y in enumerate((100, -100, 100)):
        await _dispatch_wheel(client, session_id, delta_y)
        assert_equal(
            await _witness_count(fixture),
            0,
            f"wheel {index + 1} remains delayed",
        )
        if index != 2:
            # The final admission lands about half a window after the first.
            # A fixed deadline still releases near 1.0s; a sliding debounce
            # would move the release to roughly 1.5s.
            await asyncio.sleep(0.25)

    await _wait_for_witness(fixture, 1)
    elapsed = asyncio.get_running_loop().time() - opened_at
    if elapsed < 0.85:
        raise SmokeError(f"action window applied before its one-second deadline: {elapsed:.3f}s")
    if elapsed > 1.4:
        raise SmokeError(f"later wheel input moved the fixed deadline: {elapsed:.3f}s")
    state = await _evaluate_json(
        client,
        session_id,
        """JSON.stringify({
          scrollY,
          wheelLog: __actionWindowWheelLog,
          ioLog: __actionWindowIoLog
        })""",
    )
    assert_equal(
        state,
        {
            "scrollY": 100,
            "wheelLog": [
                "event:100",
                "event:-100",
                "event:100",
                "microtask:100",
                "microtask:-100",
                "microtask:100",
            ],
            "ioLog": [False, True],
        },
        "fixed action-window batch state",
    )
    record_contract(
        results,
        "raw_cdp_action_window_fixed_deadline_batch",
        contract=(
            "wheel acknowledgements remain delayed until one fixed deadline, then preserve "
            "event order and commit IntersectionObserver work once"
        ),
        source="Moli on-demand rendering policy, mirrored by renderer and action-window Rust tests",
        commands=[
            "Input.dispatchMouseEvent(mouseWheel) x3",
            "Runtime.evaluate",
        ],
        observed={"elapsedSeconds": round(elapsed, 3), **state},
    )


async def _run_overflow_container_contract(
    client: RawCdpClient,
    session_id: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    await _navigate(client, session_id, f"{fixture}/action-window-overflow")
    initial = await _evaluate_json(
        client,
        session_id,
        """JSON.stringify({
          container: [scroller.scrollLeft, scroller.scrollTop],
          page: [scrollX, scrollY],
          deltas: __actionWindowOverflowDeltas
        })""",
    )
    assert_equal(
        initial,
        {"container": [0, 0], "page": [0, 0], "deltas": []},
        "overflow container initial state",
    )

    opened_at = asyncio.get_running_loop().time()
    await _dispatch_wheel(client, session_id, 60, x=50, y=50)
    await _dispatch_wheel(client, session_id, 0, delta_x=45, x=50, y=50)
    assert_equal(
        await _witness_count(fixture),
        0,
        "vertical and horizontal container wheels remain delayed",
    )

    await _wait_for_witness(fixture, 2)
    elapsed = asyncio.get_running_loop().time() - opened_at
    if elapsed < 0.85:
        raise SmokeError(
            f"overflow-container wheel batch applied before its deadline: {elapsed:.3f}s"
        )
    state = await _evaluate_json(
        client,
        session_id,
        """JSON.stringify({
          container: [scroller.scrollLeft, scroller.scrollTop],
          page: [scrollX, scrollY],
          deltas: __actionWindowOverflowDeltas
        })""",
    )
    assert_equal(
        state,
        {
            "container": [45, 60],
            "page": [0, 0],
            "deltas": [[0, 60], [45, 0]],
        },
        "overflow container wheel batch state",
    )
    record_contract(
        results,
        "raw_cdp_action_window_overflow_container_axes",
        contract=(
            "delayed vertical and horizontal wheels hit the innermost overflow container "
            "at their CDP coordinates without scrolling the page"
        ),
        source="Moli coordinate hit testing and fixed action-window scheduling",
        commands=[
            "Input.dispatchMouseEvent(mouseWheel deltaY)",
            "Input.dispatchMouseEvent(mouseWheel deltaX)",
            "Runtime.evaluate",
        ],
        observed={"elapsedSeconds": round(elapsed, 3), **state},
    )


async def _run_capture_barrier_contract(
    client: RawCdpClient,
    session_id: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _reset_witness(fixture)
    await _navigate(client, session_id, f"{fixture}/action-window-capture")
    initial = await _capture_png(client, session_id)
    assert_equal(initial.pixel(10, 10), (255, 255, 255, 255), "initial capture pixel")

    await _dispatch_wheel(client, session_id, 10)
    assert_equal(await _witness_count(fixture), 0, "first wheel remains pending")
    flushed = await _capture_png(client, session_id)
    assert_equal(flushed.pixel(10, 10), (255, 0, 0, 255), "screenshot flush pixel")
    await _wait_for_witness(fixture, 1)

    # These two half-window waits straddle the retired window's old deadline
    # while remaining before the second wheel's fresh deadline.
    await asyncio.sleep(0.55)
    second_opened_at = asyncio.get_running_loop().time()
    await _dispatch_wheel(client, session_id, 20)
    assert_equal(await _witness_count(fixture), 1, "second wheel starts a fresh window")
    await asyncio.sleep(0.55)
    assert_equal(
        await _witness_count(fixture),
        1,
        "reset window has no inherited periodic deadline",
    )
    await _wait_for_witness(fixture, 2)
    second_elapsed = asyncio.get_running_loop().time() - second_opened_at
    if second_elapsed < 0.85:
        raise SmokeError(
            f"post-screenshot wheel reused the retired window deadline: {second_elapsed:.3f}s"
        )

    settled = await _capture_png(client, session_id)
    assert_equal(settled.pixel(10, 10), (0, 255, 0, 255), "fresh-window capture pixel")
    state = await _evaluate_json(
        client,
        session_id,
        "JSON.stringify({ scrollY, deltas: __actionWindowCaptureDeltas })",
    )
    assert_equal(
        state,
        {"scrollY": 30, "deltas": [10, 20]},
        "capture barrier action state",
    )
    record_contract(
        results,
        "raw_cdp_action_window_screenshot_barrier_reset",
        contract=(
            "Page.captureScreenshot flushes pending wheel work before paint and resets the "
            "window so later input receives a fresh one-second deadline"
        ),
        source="Moli on-demand screenshot/read barrier policy",
        commands=[
            "Input.dispatchMouseEvent(mouseWheel)",
            "Page.captureScreenshot",
            "Input.dispatchMouseEvent(mouseWheel)",
            "Page.captureScreenshot",
        ],
        observed={
            "firstPixel": list(flushed.pixel(10, 10)),
            "secondPixel": list(settled.pixel(10, 10)),
            "secondElapsedSeconds": round(second_elapsed, 3),
            **state,
        },
    )


async def _run_replacement_contract(
    client: RawCdpClient,
    session_id: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    await _navigate(client, session_id, f"{fixture}/action-window-replacement")
    for delta_y in (10, 20, 30):
        await _dispatch_wheel(client, session_id, delta_y)
    await asyncio.sleep(1.1)
    state = await _evaluate_json(
        client,
        session_id,
        """JSON.stringify({
          retired: __actionWindowRetiredDeltas,
          replacement: __actionWindowReplacementDeltas
        })""",
    )
    assert_equal(
        state,
        {"retired": [10], "replacement": []},
        "in-batch Document replacement isolation",
    )
    record_contract(
        results,
        "raw_cdp_action_window_document_replacement_isolation",
        contract=(
            "wheel work admitted for one exact Document stops when its first handler calls "
            "document.open and never continues in the replacement Document"
        ),
        source="Moli exact RendererDocumentLifecycleIdentity action scope",
        commands=[
            "Input.dispatchMouseEvent(mouseWheel) x3",
            "Runtime.evaluate",
        ],
        observed=state,
    )


async def run_action_window_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    websocket_url = await discover_websocket_url(endpoint)
    if not websocket_url.endswith("/devtools/browser/moli-browser"):
        record(
            results,
            "raw_cdp_action_window_moli_policy_not_applicable",
            {"engine": "chromium", "applicable": False},
        )
        return

    client = await connect_raw_cdp(endpoint)
    context_id: str | None = None
    try:
        context_id, session_id = await _open_target(client)
        await _run_deadline_contract(client, session_id, fixture, results)
        await _run_overflow_container_contract(client, session_id, fixture, results)
        await _run_capture_barrier_contract(client, session_id, fixture, results)
        await _run_replacement_contract(client, session_id, fixture, results)
    finally:
        if context_id is not None:
            try:
                await _success(
                    client,
                    "Target.disposeBrowserContext",
                    {"browserContextId": context_id},
                )
            except Exception:
                pass
        await client.websocket.close()
