from __future__ import annotations

import asyncio
import base64
import weakref
from contextlib import asynccontextmanager
from dataclasses import replace
from typing import Any, AsyncIterator


from .diagnostics import new_cdp_session
from .assertions import SmokeError, wait_until
from .state import SmokeState


@asynccontextmanager
async def isolated_network_page(state: SmokeState) -> AsyncIterator[SmokeState]:
    # Auth caches belong to a browser context. Different challenge realms on
    # the same path do not prevent Chromium from sending cached credentials.
    context = await state.browser.new_context()
    try:
        page = await context.new_page()
        cdp = await new_cdp_session(context, page)
        events = attach_cdp_event_collector(cdp, [
            "Network.requestWillBeSent", "Network.responseReceived", "Network.dataReceived",
            "Network.loadingFinished", "Network.loadingFailed",
        ])
        await cdp.send("Network.enable")
        await page.goto(state.fixture + "/plain", wait_until="load", timeout=10_000)
        yield replace(state, context=context, page=page, cdp=cdp, subresource_events=events)
    finally:
        await context.close()


async def read_io_to_eof(client: Any, handle: str) -> dict[str, Any]:
    """Read through the explicit EOF marker, which can follow the last data chunk."""
    data: list[bytes] = []
    encoding: bool | None = None
    async with asyncio.timeout(5):
        while True:
            chunk = await client.send("IO.read", {"handle": handle})
            if not isinstance(chunk.get("data"), str) or not isinstance(chunk.get("eof"), bool):
                raise SmokeError(f"invalid IO.read response: {chunk}")
            current = chunk.get("base64Encoded", False)
            if encoding is None and chunk["data"]:
                encoding = current
            if chunk["data"] and current != encoding:
                raise SmokeError(f"IO.read changed encoding within a stream: {chunk}")
            data.append(base64.b64decode(chunk["data"], validate=True) if current else chunk["data"].encode())
            if chunk["eof"]:
                body = b"".join(data)
                return {"data": base64.b64encode(body).decode() if encoding else body.decode(),
                        "base64Encoded": bool(encoding), "eof": True}


async def capture_layout(page: Any) -> bytes:
    """Publish fixture layout for geometry and input assertions.

    Screenshot tests must call page.screenshot() directly on a fresh page;
    prewarming here would bypass Playwright's getLayoutMetrics preflight.
    """
    client = await new_cdp_session(page.context, page)
    try:
        result = await client.send("Page.captureScreenshot", {"format": "png"})
        return base64.b64decode(result["data"], validate=True)
    finally:
        await client.detach()


def _remove_event_listeners(client: Any, handlers: list[tuple[str, Any]]) -> None:
    for method, handler in handlers:
        client.remove_listener(method, handler)


def _defer_event_listener_removal(
    loop: asyncio.AbstractEventLoop, client: Any, handlers: list[tuple[str, Any]],
) -> None:
    # GC can run inside the emitter's non-reentrant lock. Never call back into
    # that emitter from a finalizer. A closed loop cannot deliver more events.
    if not loop.is_closed():
        loop.call_soon_threadsafe(_remove_event_listeners, client, handlers)


class CdpEvents(list[dict[str, Any]]):
    """Keep listeners while their events are owned, with explicit scoped cleanup."""

    def __init__(self, client: Any, methods: list[str]) -> None:
        super().__init__()
        reference = weakref.ref(self)
        self._client = client
        self._handlers: list[tuple[str, Any]] = []
        self._detach = weakref.finalize(
            self, _defer_event_listener_removal, asyncio.get_running_loop(),
            client, self._handlers,
        )
        for method in methods:
            def collect(params: Any, method: str = method) -> None:
                events = reference()
                if events is not None:
                    events.append({"method": method, "params": params})
            client.on(method, collect)
            self._handlers.append((method, collect))

    def close(self) -> None:
        if self._detach.detach() is not None:
            _remove_event_listeners(self._client, self._handlers)

    def __enter__(self) -> CdpEvents:
        return self

    def __exit__(self, *args: Any) -> None:
        self.close()


def attach_cdp_event_collector(client: Any, methods: list[str]) -> CdpEvents:
    return CdpEvents(client, methods)


async def wait_for_document_request(
    events: list[dict[str, Any]], url: str, *, loader_id: str, timeout_ms: int = 10_000,
) -> str:
    """A navigation's load event does not replace this frontend's Network terminal."""
    request_id: str | None = None

    def finished() -> bool:
        nonlocal request_id
        for event in events:
            params = event.get("params", {})
            if params.get("type") != "Document" or params.get("loaderId") != loader_id:
                continue
            resource = params.get("request") or params.get("response") or {}
            if resource.get("url") == url:
                candidate = params.get("requestId")
                if not isinstance(candidate, str) or not candidate:
                    raise SmokeError(f"Document event has no requestId: {event!r}")
                request_id = candidate
                break
        if request_id is None:
            return False
        terminals = [event for event in events
                     if event.get("params", {}).get("requestId") == request_id
                     and event.get("method") in {"Network.loadingFailed", "Network.loadingFinished"}]
        if len(terminals) > 1:
            raise SmokeError(f"duplicate Network terminals request={request_id}: {terminals!r}")
        for event in terminals:
            params = event.get("params", {})
            if event.get("method") == "Network.loadingFailed":
                raise SmokeError(f"Document request failed url={url} loader={loader_id} "
                                 f"request={request_id}: {params!r}")
            if event.get("method") == "Network.loadingFinished":
                return True
        return False

    try:
        await wait_until(finished, f"Network terminal url={url} loader={loader_id}", timeout_ms=timeout_ms)
    except SmokeError as error:
        raise SmokeError(f"{error}; request={request_id}; events={events[-20:]!r}") from error
    assert request_id is not None
    return request_id


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
