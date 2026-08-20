from __future__ import annotations

import asyncio
import base64
import math
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..config import reserve_port
from ..jpeg_image import jpeg_dimensions
from ..png_image import decode_png
from ..raw_cdp import RawCdpClient, connect_raw_cdp, discover_websocket_url
from ..serve import start_moli_serve, stop_moli_serve, wait_for_cdp_server


SCREENSHOT_UNSUPPORTED = (
    "Page.captureScreenshot is not supported: renderer screenshots are not implemented."
)
SCREENCAST_UNSUPPORTED = (
    "Page.startScreencast is not supported: renderer layout is disabled."
)


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
                f"timed out waiting for screenshot CDP response id={message_id}: {seen[-20:]}"
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
) -> tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]:
    message_id = await client.send(method, params, session_id=session_id)
    response, seen = await _response_for_id(client, message_id)
    if "error" in response:
        raise SmokeError(f"{method} failed: {response['error']!r}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise SmokeError(f"{method} returned no result object: {response!r}")
    return result, response, seen


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
            raise SmokeError("timed out waiting for screenshot Page.loadEventFired")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        if (
            message.get("sessionId") == session_id
            and message.get("method") == "Page.loadEventFired"
        ):
            return


async def _open_target(
    client: RawCdpClient,
    fixture_url: str,
) -> tuple[str, str]:
    created, _, _ = await _success(
        client,
        "Target.createTarget",
        {"url": "about:blank"},
    )
    target_id = created.get("targetId")
    if not isinstance(target_id, str) or not target_id:
        raise SmokeError(f"Target.createTarget returned no targetId: {created!r}")

    attached, _, _ = await _success(
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
            "width": 800,
            "height": 600,
            "deviceScaleFactor": 1,
            "mobile": False,
        },
        session_id=session_id,
    )
    navigation, _, seen = await _success(
        client,
        "Page.navigate",
        {"url": fixture_url},
        session_id=session_id,
    )
    if navigation.get("errorText"):
        raise SmokeError(f"Page.navigate failed: {navigation!r}")
    await _wait_for_load(client, session_id, seen)
    return target_id, session_id


async def _capture(
    client: RawCdpClient,
    session_id: str,
    params: dict[str, Any] | None = None,
) -> tuple[bytes, dict[str, Any]]:
    message_id = await client.send(
        "Page.captureScreenshot",
        params
        or {
            "format": "png",
            "quality": 100,
            "fromSurface": True,
            "captureBeyondViewport": False,
        },
        session_id=session_id,
    )
    response, _ = await _response_for_id(client, message_id)
    assert_equal(response.get("id"), message_id, "captureScreenshot response id")
    assert_equal(
        response.get("sessionId"),
        session_id,
        "captureScreenshot response session route",
    )
    if "error" in response:
        raise SmokeError(f"Page.captureScreenshot failed: {response['error']!r}")
    data = response.get("result", {}).get("data")
    if not isinstance(data, str) or not data:
        raise SmokeError(f"Page.captureScreenshot returned no base64 data: {response!r}")
    try:
        return base64.b64decode(data, validate=True), response
    except ValueError as error:
        raise SmokeError("Page.captureScreenshot returned invalid base64") from error


async def _evaluate(client: RawCdpClient, session_id: str, expression: str) -> None:
    result, _, _ = await _success(
        client,
        "Runtime.evaluate",
        {"expression": expression, "returnByValue": True},
        session_id=session_id,
    )
    if result.get("exceptionDetails"):
        raise SmokeError(f"Runtime.evaluate mutation failed: {result!r}")


async def _run_capture_surface_matrix(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    evaluation, _, _ = await _success(
        client,
        "Runtime.evaluate",
        {
            "expression": """
              (() => {
                const node = document.createElement('div');
                node.id = 'capture-surface-node';
                node.style.cssText = [
                  'display:block',
                  'width:80px',
                  'height:30px',
                  'margin-top:500px',
                  'background:rgb(0,255,0)',
                ].join(';');
                document.body.append(node);
                return node;
              })()
            """,
        },
        session_id=session_id,
    )
    remote = evaluation.get("result")
    object_id = remote.get("objectId") if isinstance(remote, dict) else None
    if not isinstance(object_id, str) or not object_id:
        raise SmokeError(f"capture node evaluation returned no objectId: {evaluation!r}")

    # Geometry commands intentionally read the most recently published layout
    # snapshot, which may predate the mutation above. A fresh screenshot is the
    # explicit render demand that publishes the new node before DevTools asks
    # for its box and converts that box into a page-coordinate clip.
    await _capture(client, session_id, {"format": "png"})

    box_result, _, _ = await _success(
        client,
        "DOM.getBoxModel",
        {"objectId": object_id},
        session_id=session_id,
    )
    model = box_result.get("model")
    border = model.get("border") if isinstance(model, dict) else None
    if not isinstance(border, list) or len(border) != 8:
        raise SmokeError(f"DOM.getBoxModel returned no border quad: {box_result!r}")
    xs = [float(value) for value in border[0::2]]
    ys = [float(value) for value in border[1::2]]
    viewport_x = min(xs)
    viewport_y = min(ys)
    width = max(xs) - viewport_x
    height = max(ys) - viewport_y

    metrics, _, _ = await _success(
        client,
        "Page.getLayoutMetrics",
        session_id=session_id,
    )
    layout_viewport = metrics.get("cssLayoutViewport") or metrics.get("layoutViewport")
    content_size = metrics.get("cssContentSize") or metrics.get("contentSize")
    if not isinstance(layout_viewport, dict) or not isinstance(content_size, dict):
        raise SmokeError(f"Page.getLayoutMetrics returned incomplete geometry: {metrics!r}")
    page_x = float(layout_viewport.get("pageX", 0))
    page_y = float(layout_viewport.get("pageY", 0))
    document_x = viewport_x + page_x
    document_y = viewport_y + page_y

    node_png, _ = await _capture(
        client,
        session_id,
        {
            "format": "png",
            "captureBeyondViewport": True,
            "optimizeForSpeed": True,
            "clip": {
                "x": document_x,
                "y": document_y,
                "width": width,
                "height": height,
                "scale": 1,
            },
        },
    )
    node = decode_png(node_png)
    assert_equal(
        (node.width, node.height),
        (math.ceil(width), math.ceil(height)),
        "DevTools node screenshot dimensions",
    )
    assert_equal(
        node.pixel(node.width // 2, node.height // 2),
        (0, 255, 0, 255),
        "DevTools node screenshot pixel",
    )

    full_png, _ = await _capture(
        client,
        session_id,
        {
            "format": "png",
            "captureBeyondViewport": True,
            "optimizeForSpeed": True,
        },
    )
    full = decode_png(full_png)
    expected_full = (
        math.ceil(float(content_size["width"])),
        math.ceil(float(content_size["height"])),
    )
    assert_equal(
        (full.width, full.height),
        expected_full,
        "full-page screenshot uses Page.getLayoutMetrics content extent",
    )
    assert_equal(
        full.pixel(math.floor(document_x + width / 2), math.floor(document_y + height / 2)),
        (0, 255, 0, 255),
        "full-page screenshot includes below-viewport node",
    )
    record(
        results,
        "layout_screenshot_capture_surfaces",
        {
            "node": [node.width, node.height],
            "full": [full.width, full.height],
            "realBoxModel": True,
            "realLayoutMetrics": True,
        },
    )


def _is_screencast_frame(
    message: dict[str, Any],
    session_id: str,
    generation: int,
) -> bool:
    return (
        message.get("sessionId") == session_id
        and message.get("method") == "Page.screencastFrame"
        and message.get("params", {}).get("sessionId") == generation
    )


async def _next_screencast_frame(
    client: RawCdpClient,
    session_id: str,
    generation: int,
    *,
    initial_messages: list[dict[str, Any]] | None = None,
    timeout: float = 5.0,
) -> dict[str, Any]:
    for message in initial_messages or []:
        if _is_screencast_frame(message, session_id, generation):
            return message
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out waiting for screencast generation {generation} frame"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        if _is_screencast_frame(message, session_id, generation):
            return message


async def _assert_no_screencast_frame(
    client: RawCdpClient,
    session_id: str,
    generation: int,
    duration: float,
    label: str,
) -> None:
    deadline = asyncio.get_running_loop().time() + duration
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            return
        try:
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
        except TimeoutError:
            return
        if _is_screencast_frame(message, session_id, generation):
            raise SmokeError(f"{label} unexpectedly received a screencast frame")


def _decode_screencast_frame(
    event: dict[str, Any],
) -> tuple[bytes, dict[str, Any]]:
    params = event.get("params")
    if not isinstance(params, dict):
        raise SmokeError(f"screencast frame has no params object: {event!r}")
    data = params.get("data")
    metadata = params.get("metadata")
    if not isinstance(data, str) or not data:
        raise SmokeError(f"screencast frame has no base64 data: {event!r}")
    if not isinstance(metadata, dict):
        raise SmokeError(f"screencast frame has no metadata object: {event!r}")
    try:
        return base64.b64decode(data, validate=True), metadata
    except ValueError as error:
        raise SmokeError("screencast frame returned invalid base64") from error


async def _run_moli_screencast(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    _, start_response, start_seen = await _success(
        client,
        "Page.startScreencast",
        {
            "format": "jpeg",
            "quality": 80,
            "maxWidth": 400,
            "maxHeight": 300,
            "everyNthFrame": 1,
        },
        session_id=session_id,
    )
    visibility = next(
        (
            message
            for message in start_seen
            if message.get("sessionId") == session_id
            and message.get("method") == "Page.screencastVisibilityChanged"
        ),
        None,
    )
    if visibility is None:
        raise SmokeError("startScreencast did not emit screencastVisibilityChanged")
    assert_equal(visibility.get("params", {}).get("visible"), True, "screencast visibility")

    generation = 1
    initial_event = await _next_screencast_frame(
        client,
        session_id,
        generation,
        initial_messages=start_seen,
    )
    initial_jpeg, initial_metadata = _decode_screencast_frame(initial_event)
    assert_equal(jpeg_dimensions(initial_jpeg), (400, 300), "screencast JPEG dimensions")
    assert_equal(
        (initial_metadata.get("deviceWidth"), initial_metadata.get("deviceHeight")),
        (800.0, 600.0),
        "screencast source viewport metadata",
    )

    await _assert_no_screencast_frame(
        client,
        session_id,
        generation,
        1.15,
        "unacknowledged frame backpressure",
    )
    await _success(
        client,
        "Page.screencastFrameAck",
        {"sessionId": generation},
        session_id=session_id,
    )
    await _assert_no_screencast_frame(
        client,
        session_id,
        generation,
        1.25,
        "clean visual-state poll",
    )

    await _evaluate(
        client,
        session_id,
        "document.querySelector('#cards').style.display = 'none'",
    )
    mutated_event = await _next_screencast_frame(
        client,
        session_id,
        generation,
    )
    mutated_jpeg, mutated_metadata = _decode_screencast_frame(mutated_event)
    if mutated_metadata.get("timestamp", 0) - initial_metadata.get("timestamp", 0) < 0.9:
        raise SmokeError("screencast mutation frame violated the 1 FPS deadline")
    if mutated_jpeg == initial_jpeg:
        raise SmokeError("screencast mutation reused the previous encoded frame")

    await _success(
        client,
        "Page.screencastFrameAck",
        {"sessionId": generation},
        session_id=session_id,
    )
    await _success(client, "Page.stopScreencast", session_id=session_id)
    await _assert_no_screencast_frame(
        client,
        session_id,
        generation,
        1.1,
        "stopped screencast",
    )
    record(
        results,
        "layout_screencast_1fps",
        {
            "responseId": start_response["id"],
            "sessionRoute": session_id,
            "generation": generation,
            "format": "jpeg",
            "encodedWidth": 400,
            "encodedHeight": 300,
            "ackBackpressure": True,
            "cleanFrameSuppressed": True,
            "cleanPollRequiresNoAck": True,
            "mutationChangedFrame": True,
        },
    )


async def _run_moli_tree_scope_universe_capture(
    client: RawCdpClient,
    session_id: str,
    results: list[dict[str, Any]],
) -> None:
    await _evaluate(
        client,
        session_id,
        r"""
(() => {
  document.documentElement.style.cssText = 'margin:0;background:white';
  document.body.style.cssText = 'margin:0;width:800px';
  document.body.replaceChildren();
  for (let index = 0; index < 104; index += 1) {
    const host = document.createElement('section');
    host.style.cssText = 'display:inline-block;width:4px;height:4px;background:rgb(20,90,160)';
    document.body.appendChild(host);
    const root = host.attachShadow({mode: index % 2 === 0 ? 'open' : 'closed'});
    root.append(String(index % 10));
    if (index % 16 === 0) {
      const nestedHost = document.createElement('article');
      root.appendChild(nestedHost);
      nestedHost.attachShadow({mode: 'open'}).append('nested');
    }
  }
  const frame = document.createElement('iframe');
  frame.style.cssText = 'display:block;width:320px;height:120px;border:0';
  document.body.appendChild(frame);
  const child = frame.contentDocument;
  child.documentElement.style.cssText = 'margin:0;background:rgb(240,245,250)';
  child.body.style.cssText = 'margin:0';
  for (let index = 0; index < 24; index += 1) {
    const host = child.createElement('section');
    host.style.cssText = 'display:inline-block;width:8px;height:8px';
    child.body.appendChild(host);
    host.attachShadow({mode: index % 2 === 0 ? 'open' : 'closed'}).append('child');
  }
})()
""",
    )

    loop = asyncio.get_running_loop()
    started = loop.time()
    first_png, _ = await _capture(client, session_id)
    first_elapsed_ms = round((loop.time() - started) * 1000)
    started = loop.time()
    second_png, _ = await _capture(client, session_id)
    second_elapsed_ms = round((loop.time() - started) * 1000)
    if second_png != first_png:
        raise SmokeError("stable Shadow Root universe changed consecutive screenshot pixels")
    decoded = decode_png(second_png)
    assert_equal(
        (decoded.width, decoded.height),
        (800, 600),
        "Shadow Root universe screenshot dimensions",
    )
    record(
        results,
        "layout_shadow_tree_scope_universe",
        {
            "documentShadowRoots": 111,
            "childDocumentShadowRoots": 24,
            "openAndClosedRoots": True,
            "stableConsecutivePixels": True,
            "firstCaptureMs": first_elapsed_ms,
            "secondCaptureMs": second_elapsed_ms,
        },
    )


async def run_layout_screenshot_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    websocket_url = await discover_websocket_url(endpoint)
    is_moli = websocket_url.endswith("/devtools/browser/moli-browser")
    client = await connect_raw_cdp(endpoint)
    target_id: str | None = None
    try:
        target_id, session_id = await _open_target(
            client,
            f"{fixture}/layout-screenshot-poc",
        )
        initial_png, initial_response = await _capture(client, session_id)
        initial = decode_png(initial_png)
        assert_equal(
            (initial.width, initial.height),
            (800, 600),
            "viewport screenshot IHDR dimensions",
        )
        assert_equal(initial.pixel(50, 20), (240, 40, 40, 255), "red flex card")
        assert_equal(initial.pixel(150, 20), (40, 200, 80, 255), "green flex card")
        assert_equal(initial.pixel(250, 20), (40, 100, 240, 255), "blue flex card")
        assert_equal(initial.pixel(700, 500), (255, 255, 255, 255), "initial canvas")
        if initial.distinct_color_count() <= 8:
            raise SmokeError("fixture screenshot should contain more than eight colors")

        await _evaluate(
            client,
            session_id,
            "document.documentElement.style.backgroundColor = 'rgb(20, 30, 40)'",
        )
        paint_png, _ = await _capture(client, session_id)
        paint = decode_png(paint_png)
        assert_equal(paint.pixel(700, 500), (20, 30, 40, 255), "paint mutation canvas")
        if paint_png == initial_png:
            raise SmokeError("paint mutation reused the initial screenshot bytes")

        await _evaluate(
            client,
            session_id,
            "document.querySelector('#cards').style.flexDirection = 'column'",
        )
        layout_png, _ = await _capture(client, session_id)
        layout = decode_png(layout_png)
        assert_equal(layout.pixel(150, 20), (255, 255, 255, 255), "row slot after column mutation")
        assert_equal(layout.pixel(50, 60), (40, 200, 80, 255), "green card after column mutation")
        if layout_png == paint_png:
            raise SmokeError("layout mutation did not rebuild screenshot geometry")

        await _run_capture_surface_matrix(client, session_id, results)

        record(
            results,
            "layout_screenshot_websocket_mutations",
            {
                "engine": "moli" if is_moli else "chromium",
                "width": initial.width,
                "height": initial.height,
                "distinctColors": initial.distinct_color_count(),
                "responseId": initial_response["id"],
                "sessionRoute": initial_response["sessionId"],
                "devtoolsPngQuality": 100,
            },
        )
        if is_moli:
            await _run_moli_screencast(client, session_id, results)
            await _run_moli_tree_scope_universe_capture(client, session_id, results)
    finally:
        if target_id is not None:
            try:
                await _success(client, "Target.closeTarget", {"targetId": target_id})
            except Exception:
                pass
        await client.websocket.close()

    if is_moli:
        await _run_default_mock_boundary(fixture, results)


async def _run_default_mock_boundary(
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    port = reserve_port()
    serve = await start_moli_serve(port, layout=False)
    endpoint = f"http://127.0.0.1:{port}"
    client: RawCdpClient | None = None
    target_id: str | None = None
    try:
        await wait_for_cdp_server(endpoint, serve)
        client = await connect_raw_cdp(endpoint)
        target_id, session_id = await _open_target(
            client,
            f"{fixture}/layout-screenshot-poc",
        )
        message_id = await client.send(
            "Page.captureScreenshot",
            {},
            session_id=session_id,
        )
        response, _ = await _response_for_id(client, message_id)
        assert_equal(response.get("id"), message_id, "default-mock screenshot response id")
        assert_equal(
            response.get("sessionId"),
            session_id,
            "default-mock screenshot response session route",
        )
        assert_equal(
            response.get("error"),
            {"code": -32000, "message": SCREENSHOT_UNSUPPORTED},
            "default-mock screenshot capability error",
        )
        if "result" in response:
            raise SmokeError(f"default-mock screenshot exposed a result: {response!r}")
        screenshot_error = response["error"]

        message_id = await client.send(
            "Page.startScreencast",
            {"format": "jpeg", "quality": 80},
            session_id=session_id,
        )
        response, _ = await _response_for_id(client, message_id)
        assert_equal(response.get("id"), message_id, "default-mock screencast response id")
        assert_equal(
            response.get("sessionId"),
            session_id,
            "default-mock screencast response session route",
        )
        assert_equal(
            response.get("error"),
            {"code": -32000, "message": SCREENCAST_UNSUPPORTED},
            "default-mock screencast capability error",
        )
        if "result" in response:
            raise SmokeError(f"default-mock screencast exposed a result: {response!r}")
        record(
            results,
            "layout_screenshot_default_mock_boundary",
            {
                "error": screenshot_error,
                "screencastError": response["error"],
                "resultOmitted": True,
            },
        )
    finally:
        if client is not None:
            if target_id is not None:
                try:
                    await _success(client, "Target.closeTarget", {"targetId": target_id})
                except Exception:
                    pass
            await client.websocket.close()
        await stop_moli_serve(serve)
