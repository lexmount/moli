from __future__ import annotations

import asyncio
import bisect
import contextlib
import json
import time
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

import websockets
from websockets.asyncio.client import ClientConnection


class RawCdpError(RuntimeError):
    pass


class RawCdpTimeoutError(RawCdpError, TimeoutError):
    def __init__(
        self,
        message: str,
        *,
        method: str | None = None,
        message_id: int | None = None,
        messages: list[dict[str, Any]] | None = None,
    ) -> None:
        super().__init__(message)
        self.method = method
        self.message_id = message_id
        self.messages = list(messages or ())


class RawCdpCommandError(RawCdpError):
    def __init__(
        self,
        *,
        method: str,
        message_id: int,
        error: dict[str, Any],
        messages: list[dict[str, Any]],
        elapsed_ms: float,
    ) -> None:
        super().__init__(f"CDP {method} failed: {error}")
        self.method = method
        self.message_id = message_id
        self.error = error
        self.messages = messages
        self.elapsed_ms = elapsed_ms


class RawCdpConnectionClosed(RawCdpError):
    pass


def _read_json_url(url: str) -> dict[str, Any]:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(url, timeout=2) as response:
        return json.loads(response.read().decode("utf-8"))


async def discover_websocket_url(endpoint: str) -> str:
    payload = await asyncio.to_thread(_read_json_url, endpoint.rstrip("/") + "/json/version")
    websocket_url = payload.get("webSocketDebuggerUrl")
    if not isinstance(websocket_url, str) or not websocket_url:
        raise RawCdpError(f"CDP discovery response did not include webSocketDebuggerUrl: {payload}")
    return websocket_url


@dataclass
class RawCdpClient:
    websocket: ClientConnection
    next_id: int = 1
    command_count: int = 0

    async def send(self, method: str, params: dict[str, Any] | None = None, *, session_id: str | None = None) -> int:
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

    async def recv(self) -> dict[str, Any]:
        raw = await self.websocket.recv()
        if isinstance(raw, bytes):
            raw = raw.decode("utf-8")
        payload = json.loads(raw)
        if not isinstance(payload, dict):
            raise RawCdpError(f"unexpected CDP payload: {payload!r}")
        return payload

    async def recv_until_id(
        self,
        message_id: int,
        *,
        timeout: float = 10.0,
        method: str | None = None,
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        seen: list[dict[str, Any]] = []
        deadline = asyncio.get_running_loop().time() + timeout
        try:
            while True:
                remaining = deadline - asyncio.get_running_loop().time()
                if remaining <= 0:
                    raise RawCdpTimeoutError(
                        f"timed out waiting for CDP response id={message_id}; seen={seen[-20:]}"
                    )
                message = await asyncio.wait_for(self.recv(), timeout=remaining)
                seen.append(message)
                if message.get("id") == message_id:
                    if "error" in message:
                        raise RawCdpError(
                            f"CDP command id={message_id} failed: {message['error']}"
                        )
                    return message, seen
        except BaseException as error:
            # Keep the same exception (including cancellation) and receiver.
            # A failed response/receive must not discard already observed events.
            error.cdp_receive_evidence = {
                "message_id": message_id,
                "method": method,
                "messages": list(seen),
            }
            raise


async def connect_raw_cdp(endpoint: str) -> RawCdpClient:
    websocket_url = await discover_websocket_url(endpoint)
    websocket = await websockets.connect(websocket_url, open_timeout=5, proxy=None, max_size=None)
    return RawCdpClient(websocket=websocket)


@dataclass(frozen=True)
class RecordedCdpMessage:
    sequence: int
    received_monotonic: float
    received_epoch: float
    payload: dict[str, Any]

    def json_value(self) -> dict[str, Any]:
        return {
            "sequence": self.sequence,
            "received_monotonic": self.received_monotonic,
            "received_epoch": self.received_epoch,
            "payload": self.payload,
        }


@dataclass(frozen=True)
class RoutedCommandResult:
    message_id: int
    response: dict[str, Any]
    messages: list[RecordedCdpMessage]
    elapsed_ms: float


class RoutedRawCdpClient:
    """Raw CDP client with one receiver and lossless event recording.

    ``RawCdpClient`` intentionally mirrors the small sequential harness used by
    older suites. Agent episodes need a stronger boundary: a background reader
    owns the WebSocket, responses are routed by id, and every intervening event
    remains available for ordering checks and failure artifacts.
    """

    def __init__(self, websocket: ClientConnection) -> None:
        self.websocket = websocket
        self.next_id = 1
        self.command_count = 0
        self._send_lock = asyncio.Lock()
        self._condition = asyncio.Condition()
        self._pending: dict[int, asyncio.Future[RecordedCdpMessage]] = {}
        self._messages: list[RecordedCdpMessage] = []
        self._receiver_task: asyncio.Task[None] | None = None
        self._closed_error: BaseException | None = None
        self._closing = False

    def start(self) -> None:
        if self._receiver_task is not None:
            raise RuntimeError("routed CDP receiver already started")
        self._receiver_task = asyncio.create_task(
            self._receive_loop(),
            name="moli-benchmark-cdp-receiver",
        )

    @property
    def current_sequence(self) -> int:
        return self._messages[-1].sequence if self._messages else 0

    def recorded_messages(self) -> list[dict[str, Any]]:
        return [message.json_value() for message in self._messages]

    def _message_index_after(self, sequence: int) -> int:
        return bisect.bisect_right(
            self._messages,
            sequence,
            key=lambda message: message.sequence,
        )

    def messages_since(self, sequence: int) -> list[RecordedCdpMessage]:
        return self._messages[self._message_index_after(sequence) :]

    async def command(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        session_id: str | None = None,
        timeout: float = 10.0,
    ) -> RoutedCommandResult:
        if timeout <= 0:
            raise ValueError("CDP command timeout must be positive")
        if self._closed_error is not None:
            raise RawCdpConnectionClosed(str(self._closed_error))

        loop = asyncio.get_running_loop()
        future: asyncio.Future[RecordedCdpMessage] = loop.create_future()
        async with self._send_lock:
            if self._closed_error is not None:
                raise RawCdpConnectionClosed(str(self._closed_error))
            message_id = self.next_id
            self.next_id += 1
            self.command_count += 1
            sequence_before_send = self.current_sequence
            message: dict[str, Any] = {"id": message_id, "method": method}
            if params is not None:
                message["params"] = params
            if session_id is not None:
                message["sessionId"] = session_id
            self._pending[message_id] = future
            started = time.perf_counter()
            try:
                await self.websocket.send(json.dumps(message, separators=(",", ":")))
            except BaseException:
                self._pending.pop(message_id, None)
                future.cancel()
                raise
        try:
            record = await asyncio.wait_for(asyncio.shield(future), timeout=timeout)
        except TimeoutError as error:
            self._pending.pop(message_id, None)
            future.cancel()
            seen = [message.json_value() for message in self.messages_since(sequence_before_send)]
            raise RawCdpTimeoutError(
                f"timed out waiting for CDP {method} response id={message_id}",
                method=method,
                message_id=message_id,
                messages=seen,
            ) from error

        elapsed_ms = (time.perf_counter() - started) * 1000.0
        messages = self.messages_since(sequence_before_send)
        response = record.payload
        error_value = response.get("error")
        if isinstance(error_value, dict):
            raise RawCdpCommandError(
                method=method,
                message_id=message_id,
                error=error_value,
                messages=[message.json_value() for message in messages],
                elapsed_ms=elapsed_ms,
            )
        return RoutedCommandResult(
            message_id=message_id,
            response=response,
            messages=messages,
            elapsed_ms=elapsed_ms,
        )

    async def wait_for_event(
        self,
        method: str,
        *,
        after_sequence: int = 0,
        session_id: str | None = None,
        predicate: Callable[[dict[str, Any]], bool] | None = None,
        timeout: float = 10.0,
    ) -> RecordedCdpMessage:
        deadline = asyncio.get_running_loop().time() + timeout
        async with self._condition:
            scan_index = self._message_index_after(after_sequence)
            while True:
                while scan_index < len(self._messages):
                    message = self._messages[scan_index]
                    scan_index += 1
                    payload = message.payload
                    if payload.get("method") != method:
                        continue
                    if session_id is not None and payload.get("sessionId") != session_id:
                        continue
                    if predicate is not None and not predicate(payload):
                        continue
                    return message
                if self._closed_error is not None:
                    raise RawCdpConnectionClosed(str(self._closed_error))
                remaining = deadline - asyncio.get_running_loop().time()
                if remaining <= 0:
                    raise RawCdpTimeoutError(
                        f"timed out waiting for CDP event {method}",
                        method=method,
                        messages=[
                            message.json_value()
                            for message in self.messages_since(after_sequence)
                        ],
                    )
                try:
                    await asyncio.wait_for(self._condition.wait(), timeout=remaining)
                except TimeoutError as error:
                    raise RawCdpTimeoutError(
                        f"timed out waiting for CDP event {method}",
                        method=method,
                        messages=[
                            message.json_value()
                            for message in self.messages_since(after_sequence)
                        ],
                    ) from error

    async def close(self) -> None:
        if self._closing:
            return
        self._closing = True
        with contextlib.suppress(Exception):
            await self.websocket.close()
        task = self._receiver_task
        if task is not None:
            with contextlib.suppress(asyncio.CancelledError, Exception):
                await task

    async def _receive_loop(self) -> None:
        failure: BaseException | None = None
        try:
            while True:
                raw = await self.websocket.recv()
                if isinstance(raw, bytes):
                    try:
                        raw = raw.decode("utf-8")
                    except UnicodeDecodeError as error:
                        raise RawCdpError("received non-UTF-8 binary CDP frame") from error
                payload = json.loads(raw)
                if not isinstance(payload, dict):
                    raise RawCdpError(f"unexpected CDP payload: {payload!r}")
                record = RecordedCdpMessage(
                    sequence=self.current_sequence + 1,
                    received_monotonic=time.perf_counter(),
                    received_epoch=time.time(),
                    payload=payload,
                )
                response_id = payload.get("id")
                future = self._pending.pop(response_id, None) if isinstance(response_id, int) else None
                async with self._condition:
                    self._messages.append(record)
                    self._condition.notify_all()
                if future is not None and not future.done():
                    future.set_result(record)
        except asyncio.CancelledError:
            if not self._closing:
                failure = RawCdpConnectionClosed("CDP receiver was cancelled")
        except BaseException as error:
            if not self._closing:
                failure = RawCdpConnectionClosed(
                    f"CDP WebSocket receiver stopped: {type(error).__name__}: {error}"
                )
        finally:
            if failure is not None:
                self._closed_error = failure
                pending = list(self._pending.values())
                self._pending.clear()
                for future in pending:
                    if not future.done():
                        future.set_exception(failure)
                async with self._condition:
                    self._condition.notify_all()


async def connect_routed_raw_cdp(endpoint: str) -> RoutedRawCdpClient:
    websocket_url = await discover_websocket_url(endpoint)
    websocket = await websockets.connect(
        websocket_url,
        open_timeout=5,
        proxy=None,
        max_size=None,
    )
    client = RoutedRawCdpClient(websocket)
    client.start()
    return client
