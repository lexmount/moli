from __future__ import annotations

import asyncio
import json
import urllib.request
from dataclasses import dataclass
from typing import Any

import websockets
from websockets.asyncio.client import ClientConnection

MAX_CDP_MESSAGE_BYTES = 192 * 1024 * 1024


class RawCdpError(RuntimeError):
    pass


def _read_json_url(url: str) -> Any:
    with urllib.request.urlopen(url, timeout=2) as response:
        return json.loads(response.read().decode("utf-8"))


async def discover_websocket_url(endpoint: str) -> str:
    version_url = endpoint.rstrip("/") + "/json/version"
    payload = await asyncio.to_thread(_read_json_url, version_url)
    if not isinstance(payload, dict):
        raise RawCdpError(f"CDP discovery response was not an object: {payload!r}")
    websocket_url = payload.get("webSocketDebuggerUrl")
    if not isinstance(websocket_url, str) or not websocket_url:
        raise RawCdpError(f"CDP discovery response did not include webSocketDebuggerUrl: {payload}")
    return websocket_url


async def discover_page_websocket_url(endpoint: str) -> str:
    targets_url = endpoint.rstrip("/") + "/json/list"
    payload = await asyncio.to_thread(_read_json_url, targets_url)
    if not isinstance(payload, list):
        raise RawCdpError(f"CDP target discovery response was not a list: {payload!r}")
    for target in payload:
        if not isinstance(target, dict) or target.get("type") != "page":
            continue
        websocket_url = target.get("webSocketDebuggerUrl")
        if isinstance(websocket_url, str) and websocket_url:
            return websocket_url
    raise RawCdpError(f"CDP target discovery response included no page websocket: {payload!r}")


async def discover_target_websocket_url(endpoint: str, target_id: str) -> str:
    targets_url = endpoint.rstrip("/") + "/json/list"
    payload = await asyncio.to_thread(_read_json_url, targets_url)
    if not isinstance(payload, list):
        raise RawCdpError(f"CDP target discovery response was not a list: {payload!r}")
    for target in payload:
        if not isinstance(target, dict) or target.get("id") != target_id:
            continue
        websocket_url = target.get("webSocketDebuggerUrl")
        if isinstance(websocket_url, str) and websocket_url:
            return websocket_url
        raise RawCdpError(
            f"CDP target {target_id} did not include webSocketDebuggerUrl: {target!r}"
        )
    raise RawCdpError(
        f"CDP target {target_id} was not present in discovery: {payload!r}"
    )


@dataclass
class RawCdpClient:
    websocket: ClientConnection
    next_id: int = 1
    command_count: int = 0
    forbid_further_sends: bool = False

    async def send(self, method: str, params: dict[str, Any] | None = None, *, session_id: str | None = None) -> int:
        if self.forbid_further_sends:
            raise RawCdpError(f"attempted to send {method} after the no-followup boundary")
        message_id = self.next_id
        self.next_id += 1
        self.command_count += 1
        message: dict[str, Any] = {"id": message_id, "method": method}
        if params is not None:
            message["params"] = params
        if session_id is not None:
            message["sessionId"] = session_id
        await self.websocket.send(json.dumps(message, separators=(",", ":")))
        return message_id

    def mark_no_followup_boundary(self) -> None:
        self.forbid_further_sends = True

    def clear_no_followup_boundary(self) -> None:
        self.forbid_further_sends = False

    async def recv(self) -> dict[str, Any]:
        raw = await self.websocket.recv()
        if isinstance(raw, bytes):
            try:
                raw = raw.decode("utf-8")
            except UnicodeDecodeError as error:
                raise RawCdpError("received non-UTF-8 binary CDP WebSocket frame") from error
        payload = json.loads(raw)
        if not isinstance(payload, dict):
            raise RawCdpError(f"unexpected CDP payload: {payload!r}")
        return payload

    async def recv_until_id(self, message_id: int, *, timeout: float = 10.0) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        seen: list[dict[str, Any]] = []
        deadline = asyncio.get_running_loop().time() + timeout
        while True:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise RawCdpError(f"timed out waiting for CDP response id={message_id}; seen={seen[-20:]}")
            try:
                message = await asyncio.wait_for(self.recv(), timeout=remaining)
            except TimeoutError as error:
                raise RawCdpError(
                    f"timed out waiting for CDP response id={message_id}; "
                    f"seen={seen[-20:]}"
                ) from error
            seen.append(message)
            if message.get("id") == message_id:
                if "error" in message:
                    raise RawCdpError(f"CDP command id={message_id} failed: {message['error']}")
                return message, seen


async def connect_raw_cdp(endpoint: str) -> RawCdpClient:
    websocket_url = await discover_websocket_url(endpoint)
    return await connect_raw_cdp_websocket(websocket_url)


async def connect_raw_cdp_websocket(websocket_url: str) -> RawCdpClient:
    # Screenshot payloads are base64 inside a JSON frame, so the WebSocket
    # limit must cover the renderer's encoded-image budget plus that expansion.
    websocket = await websockets.connect(
        websocket_url,
        open_timeout=5,
        max_size=MAX_CDP_MESSAGE_BYTES,
    )
    return RawCdpClient(websocket=websocket)
