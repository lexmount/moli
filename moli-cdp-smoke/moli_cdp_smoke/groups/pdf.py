from __future__ import annotations

import asyncio
import base64
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..pdf_document import assert_moli_pdf_text_layer, assert_pdf_envelope, inspect_moli_pdf
from ..raw_cdp import RawCdpClient, connect_raw_cdp, discover_websocket_url


async def _response(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    message_id = await client.send(method, params, session_id=session_id)
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 15.0
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for {method}: {seen[-20:]!r}")
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
    response, seen = await _response(client, method, params, session_id=session_id)
    if "error" in response:
        raise SmokeError(f"{method} failed: {response['error']!r}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise SmokeError(f"{method} returned no result object: {response!r}")
    return result, seen


async def _open_fixture_target(client: RawCdpClient, fixture: str) -> tuple[str, str]:
    created, _ = await _success(client, "Target.createTarget", {"url": "about:blank"})
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
            "width": 800,
            "height": 600,
            "deviceScaleFactor": 1,
            "mobile": False,
        },
        session_id=session_id,
    )
    navigation, seen = await _success(
        client,
        "Page.navigate",
        {"url": f"{fixture}/plain?pdf-smoke"},
        session_id=session_id,
    )
    if navigation.get("errorText"):
        raise SmokeError(f"Page.navigate failed: {navigation!r}")
    if not any(
        message.get("sessionId") == session_id
        and message.get("method") == "Page.loadEventFired"
        for message in seen
    ):
        deadline = asyncio.get_running_loop().time() + 10.0
        while True:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise SmokeError("timed out waiting for PDF fixture load")
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
            if (
                message.get("sessionId") == session_id
                and message.get("method") == "Page.loadEventFired"
            ):
                break
    await _success(
        client,
        "Runtime.evaluate",
        {
            "expression": """
              (() => {
                document.documentElement.style.cssText = 'margin:0;background:white';
                document.body.style.cssText = 'margin:0;min-height:1800px;background:#123456';
                const style = document.createElement('style');
                style.textContent = '@media print { body { background: #234567 } }';
                document.head.append(style);
              })()
            """,
        },
        session_id=session_id,
    )
    return target_id, session_id


def _decode_base64_pdf(result: dict[str, Any], label: str) -> bytes:
    data = result.get("data")
    if not isinstance(data, str) or not data:
        raise SmokeError(f"{label} returned no base64 PDF data: {result!r}")
    try:
        return base64.b64decode(data, validate=True)
    except ValueError as error:
        raise SmokeError(f"{label} returned invalid base64 PDF data") from error


async def _read_stream(
    client: RawCdpClient,
    session_id: str,
    handle: str,
) -> bytes:
    output = bytearray()
    while True:
        result, _ = await _success(
            client,
            "IO.read",
            {"handle": handle, "size": 64 * 1024},
            session_id=session_id,
        )
        chunk = result.get("data")
        if not isinstance(chunk, str):
            raise SmokeError(f"IO.read returned no data string: {result!r}")
        if result.get("base64Encoded"):
            try:
                output.extend(base64.b64decode(chunk, validate=True))
            except ValueError as error:
                raise SmokeError("IO.read returned invalid base64 stream data") from error
        else:
            output.extend(chunk.encode())
        if result.get("eof"):
            break
    await _success(client, "IO.close", {"handle": handle}, session_id=session_id)
    return bytes(output)


async def run_pdf_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    websocket_url = await discover_websocket_url(endpoint)
    is_moli = websocket_url.endswith("/devtools/browser/moli-browser")
    client = await connect_raw_cdp(endpoint)
    target_id: str | None = None
    try:
        target_id, session_id = await _open_fixture_target(client, fixture)
        common = {
            "printBackground": True,
            "paperWidth": 4,
            "paperHeight": 4,
            "marginTop": 0,
            "marginBottom": 0,
            "marginLeft": 0,
            "marginRight": 0,
        }

        base64_result, _ = await _success(
            client,
            "Page.printToPDF",
            {**common, "pageRanges": "1-2", "transferMode": "ReturnAsBase64"},
            session_id=session_id,
        )
        base64_pdf = _decode_base64_pdf(base64_result, "base64 printToPDF")
        assert_pdf_envelope(base64_pdf, "base64 printToPDF")

        stream_result, _ = await _success(
            client,
            "Page.printToPDF",
            {**common, "transferMode": "ReturnAsStream"},
            session_id=session_id,
        )
        assert_equal(stream_result.get("data"), "", "stream printToPDF data")
        stream = stream_result.get("stream")
        if not isinstance(stream, str) or not stream:
            raise SmokeError(f"stream printToPDF returned no handle: {stream_result!r}")
        stream_pdf = await _read_stream(client, session_id, stream)
        assert_pdf_envelope(stream_pdf, "stream printToPDF")

        landscape_result, _ = await _success(
            client,
            "Page.printToPDF",
            {
                **common,
                "paperHeight": 6,
                "landscape": True,
                "pageRanges": "1",
            },
            session_id=session_id,
        )
        landscape_pdf = _decode_base64_pdf(landscape_result, "landscape printToPDF")
        assert_pdf_envelope(landscape_pdf, "landscape printToPDF")

        range_response, _ = await _response(
            client,
            "Page.printToPDF",
            {**common, "pageRanges": "998-999"},
            session_id=session_id,
        )
        assert_equal(
            range_response.get("error"),
            {"code": -32000, "message": "Page range exceeds page count"},
            "printToPDF out-of-document page range",
        )
        scale_response, _ = await _response(
            client,
            "Page.printToPDF",
            {**common, "scale": 2.1},
            session_id=session_id,
        )
        assert_equal(
            scale_response.get("error"),
            {"code": -32602, "message": "scale is outside of [0.1 - 2] range"},
            "printToPDF scale validation",
        )

        detail: dict[str, Any] = {
            "engine": "moli" if is_moli else "chromium",
            "base64Bytes": len(base64_pdf),
            "streamBytes": len(stream_pdf),
            "streamRead": True,
            "pageRangeValidation": True,
        }
        if is_moli:
            assert_moli_pdf_text_layer(base64_pdf, "base64 printToPDF")
            assert_moli_pdf_text_layer(stream_pdf, "stream printToPDF")
            assert_moli_pdf_text_layer(landscape_pdf, "landscape printToPDF")
            base64_info = inspect_moli_pdf(base64_pdf, "base64 printToPDF")
            stream_info = inspect_moli_pdf(stream_pdf, "stream printToPDF")
            landscape_info = inspect_moli_pdf(landscape_pdf, "landscape printToPDF")
            assert_equal(base64_info.page_count, 2, "base64 selected PDF page count")
            assert_equal(stream_info.page_count, 3, "stream PDF page count")
            assert_equal(
                landscape_info.media_boxes,
                ((432.0, 288.0),),
                "landscape PDF MediaBox",
            )
            detail.update(
                {
                    "selectedPages": base64_info.page_count,
                    "allPages": stream_info.page_count,
                    "landscapeMediaBox": list(landscape_info.media_boxes[0]),
                    "xrefValidated": True,
                }
            )
        record(results, "page_print_to_pdf", detail)
    finally:
        if target_id is not None:
            try:
                await _success(client, "Target.closeTarget", {"targetId": target_id})
            except Exception:
                pass
        await client.websocket.close()
