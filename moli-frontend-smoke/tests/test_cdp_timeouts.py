from __future__ import annotations

import asyncio
from contextlib import suppress
import json

import pytest
from websockets.asyncio.server import serve

from moli_frontend_smoke.cdp import CdpError, CdpTimeoutError, RawCdpClient, connect


@pytest.mark.parametrize("timeout", [0.25, 5.0])
def test_connection_deadline_covers_discovery_and_websocket_handshake(timeout: float) -> None:
    async def scenario() -> None:
        async def handshake(_connection, _request):
            await asyncio.sleep(0.15)

        async def websocket_handler(websocket):
            await websocket.wait_closed()

        async with serve(
            websocket_handler, "127.0.0.1", 0, process_request=handshake,
        ) as websocket_server:
            port = websocket_server.sockets[0].getsockname()[1]
            websocket_url = f"ws://127.0.0.1:{port}/devtools/browser/probe"
            handlers: set[asyncio.Task] = set()

            async def discovery(reader, writer):
                task = asyncio.current_task()
                handlers.add(task)
                try:
                    await reader.readuntil(b"\r\n\r\n")
                    await asyncio.sleep(0.15)
                    body = json.dumps({"webSocketDebuggerUrl": websocket_url}).encode()
                    writer.write(
                        b"HTTP/1.1 200 OK\r\nConnection: close\r\n"
                        + f"Content-Length: {len(body)}\r\n\r\n".encode()
                        + body
                    )
                    await writer.drain()
                finally:
                    writer.close()
                    with suppress(ConnectionError):
                        await writer.wait_closed()
                    handlers.discard(task)

            async with await asyncio.start_server(discovery, "127.0.0.1", 0) as server:
                port = server.sockets[0].getsockname()[1]
                endpoint = f"http://127.0.0.1:{port}"
                if timeout < 0.3:
                    # Each phase fits individually, but their combined duration
                    # exceeds the one connection deadline.
                    with pytest.raises(TimeoutError):
                        await connect(endpoint, timeout=timeout)
                else:
                    client = await connect(endpoint, timeout=timeout)
                    await client.websocket.close()
            await asyncio.gather(*handlers)

    asyncio.run(scenario())


class _SilentSocket:
    async def send(self, _message):
        pass

    async def recv(self):
        await asyncio.Event().wait()


@pytest.mark.parametrize("operation", ["command", "event"])
def test_timeout_diagnostic_identifies_the_pending_operation(operation: str) -> None:
    async def scenario() -> None:
        client = RawCdpClient(websocket=_SilentSocket())  # type: ignore[arg-type]
        if operation == "command":
            with pytest.raises(CdpError, match="Target.createTarget response after 0.01s") as raised:
                await client.command("Target.createTarget", session_id="page", timeout=0.01)
        else:
            with pytest.raises(CdpError, match="Page.loadEventFired event after 0.01s") as raised:
                await client.wait_event(
                    "Page.loadEventFired", session_id="page", timeout=0.01,
                )
        assert isinstance(raised.value, CdpTimeoutError)
        assert raised.value.context == {
            "method": "Target.createTarget" if operation == "command" else "Page.loadEventFired",
            "operation": "response" if operation == "command" else "event",
            "timeoutSeconds": 0.01,
            "commandId": 1 if operation == "command" else None,
            "sessionId": "page",
        }

    asyncio.run(scenario())
