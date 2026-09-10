from __future__ import annotations

import asyncio
from collections import deque
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Any

from .assertions import SmokeError
from .raw_cdp import connect_raw_cdp_websocket, discover_target_websocket_url


@asynccontextmanager
async def collect_worker_network_events(
    endpoint: str, page_cdp: Any,
) -> AsyncIterator[list[dict[str, Any]]]:
    """Observe each exact child Worker, enabling Network before it starts."""
    info = await page_cdp.send("Target.getTargetInfo")
    websocket_url = await discover_target_websocket_url(endpoint, info["targetInfo"]["targetId"])
    client = await connect_raw_cdp_websocket(websocket_url)
    events: list[dict[str, Any]] = []
    try:
        setup_id = await client.send("Target.setAutoAttach", {
            "autoAttach": True,
            "waitForDebuggerOnStart": True,
            "flatten": True,
            "filter": [{"type": "worker", "exclude": False}, {"exclude": True}],
        })
        _, setup_messages = await client.recv_until_id(setup_id)

        async def collect() -> None:
            pending = deque(setup_messages)
            sessions: set[str] = set()
            enabling: dict[int, str] = {}
            while True:
                message = pending.popleft() if pending else await client.recv()
                if "error" in message:
                    raise SmokeError(f"Worker Network observer command failed: {message}")
                if message.get("method") == "Target.attachedToTarget":
                    params = message["params"]
                    if params["targetInfo"]["type"] != "worker":
                        raise SmokeError(f"Worker Network observer attached to another type: {message}")
                    session = params["sessionId"]
                    sessions.add(session)
                    command_id = await client.send("Network.enable", session_id=session)
                    enabling[command_id] = session
                elif message.get("id") in enabling:
                    session = enabling.pop(message["id"])
                    await client.send("Runtime.runIfWaitingForDebugger", session_id=session)
                elif str(message.get("method", "")).startswith("Network."):
                    if message.get("sessionId") not in sessions:
                        raise SmokeError(f"Worker Network event has no exact Worker session: {message}")
                    events.append(message)

        async with asyncio.TaskGroup() as tasks:
            observer = tasks.create_task(collect())
            try:
                yield events
            finally:
                observer.cancel()
    finally:
        await client.websocket.close()


def attach_cdp_event_collector(client: Any, methods: list[str]) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for method in methods:
        client.on(method, lambda params, method=method: events.append({"method": method, "params": params}))
    return events


async def run_worker_command(page: Any, payload: Any, timeout_ms: int = 10_000) -> Any:
    return await page.evaluate(
        """
        async ({ payload, timeout }) => {
          return await new Promise((resolve, reject) => {
            const worker = new Worker('/worker.js');
            const timer = setTimeout(() => {
              worker.terminate();
              reject(new Error(`worker command timed out after ${timeout}ms`));
            }, timeout);
            worker.onmessage = event => {
              clearTimeout(timer);
              worker.terminate();
              resolve(event.data);
            };
            worker.onerror = event => {
              clearTimeout(timer);
              worker.terminate();
              reject(new Error(event.message || 'worker error'));
            };
            worker.postMessage(payload);
          });
        }
        """,
        {"payload": payload, "timeout": timeout_ms},
    )


async def evaluate_xhr(page: Any, url: str, method: str = "GET", body: str | None = None) -> Any:
    return await page.evaluate(
        """
        async ({ url, method, body }) => {
          return await new Promise(resolve => {
            const xhr = new XMLHttpRequest();
            const events = [];
            xhr.addEventListener('load', () => events.push('load'));
            xhr.addEventListener('error', () => events.push('error'));
            xhr.addEventListener('abort', () => events.push('abort'));
            xhr.addEventListener('loadend', () => {
              resolve({
                events,
                phase: events.includes('load') ? 'load' : events.includes('error') ? 'error' : 'other',
                status: xhr.status,
                readyState: xhr.readyState,
                text: xhr.responseText,
              });
            });
            xhr.open(method, url, true);
            xhr.send(body);
          });
        }
        """,
        {"url": url, "method": method, "body": body},
    )
