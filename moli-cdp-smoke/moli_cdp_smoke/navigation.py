"""Document readiness tied to a session, main frame, and navigation loader."""

from __future__ import annotations

import asyncio
from collections import deque
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any, Literal

from .assertions import SmokeError, wait_until
from .raw_cdp import RawCdpClient


Observer = Callable[[dict[str, Any]], None]


@dataclass
class DocumentLoad:
    session_id: str
    frame_id: str
    loader_id: str | None
    previous_loader_id: str | None = None
    committed: bool = False
    loaded: set[str] = field(default_factory=set)

    def observe(self, message: dict[str, Any]) -> bool:
        params = message.get("params", {})
        if (message.get("method") == "Target.detachedFromTarget"
                and params.get("sessionId") == self.session_id):
            raise SmokeError(f"navigation session={self.session_id} detached: {message!r}")
        if message.get("sessionId") != self.session_id:
            return False
        if message.get("method") == "Inspector.targetCrashed":
            raise SmokeError(f"navigation session={self.session_id} crashed: {message!r}")
        if message.get("method") == "Page.frameNavigated":
            frame = params.get("frame", {})
            candidate = frame.get("loaderId")
            if (frame.get("id") == self.frame_id and candidate
                    and candidate != self.previous_loader_id):
                if self.loader_id is None:
                    self.loader_id = candidate
                if candidate == self.loader_id:
                    self.committed = True
        if (message.get("method") == "Page.lifecycleEvent"
                and params.get("frameId") == self.frame_id
                and params.get("name") == "load" and params.get("loaderId")):
            self.loaded.add(params["loaderId"])
        return self.ready

    @property
    def ready(self) -> bool:
        return self.committed and self.loader_id in self.loaded

    def describe(self) -> str:
        return (f"session={self.session_id} frame={self.frame_id} loader={self.loader_id} "
                f"previousLoader={self.previous_loader_id} committed={self.committed} "
                f"loaded={sorted(self.loaded)!r}")


async def receive_response(
    client: RawCdpClient,
    message_id: int,
    *,
    timeout: float = 10,
    on_message: Observer | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    seen: list[dict[str, Any]] = []
    try:
        async with asyncio.timeout(timeout):
            while True:
                message = await client.recv()
                seen.append(message)
                if on_message is not None:
                    on_message(message)
                if message.get("id") == message_id:
                    return message, seen
    except TimeoutError as error:
        raise SmokeError(
            f"timed out waiting for CDP response id={message_id}; seen={seen[-20:]!r}"
        ) from error


async def command(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
    on_message: Observer | None = None,
) -> tuple[dict[str, Any], dict[str, Any], list[dict[str, Any]]]:
    message_id = await client.send(method, params, session_id=session_id)
    try:
        response, seen = await receive_response(client, message_id, on_message=on_message)
    except SmokeError as error:
        raise SmokeError(f"{method} session={session_id}: {error}") from error
    if "error" in response:
        raise SmokeError(f"{method} failed: {response['error']!r}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise SmokeError(f"{method} returned no result object: {response!r}")
    return result, response, seen


async def wait_for_load(
    client: RawCdpClient,
    session_id: str,
    seen: list[dict[str, Any]],
    *,
    frame_id: str,
    loader_id: str | None,
    previous_loader_id: str | None = None,
    timeout: float = 10,
    on_message: Observer | None = None,
) -> None:
    document = DocumentLoad(session_id, frame_id, loader_id, previous_loader_id)
    trace: deque[dict[str, Any]] = deque(maxlen=20)
    try:
        async with asyncio.timeout(timeout):
            for message in seen:
                trace.append(message)
                if document.observe(message):
                    return
            while True:
                message = await client.recv()
                trace.append(message)
                if on_message is not None:
                    on_message(message)
                if document.observe(message):
                    return
    except TimeoutError as error:
        raise SmokeError(
            f"timed out waiting for Document load {document.describe()}; seen={list(trace)!r}"
        ) from error


async def _navigate(
    client: RawCdpClient,
    session_id: str,
    params: dict[str, Any] | None = None,
    *,
    method: Literal["Page.navigate", "Page.reload"] = "Page.navigate",
    on_message: Observer | None = None,
) -> dict[str, Any]:
    await command(client, "Page.setLifecycleEventsEnabled", {"enabled": True},
                  session_id=session_id, on_message=on_message)
    previous_loader = None
    if method == "Page.reload":
        tree, _, _ = await command(client, "Page.getFrameTree", session_id=session_id,
                                   on_message=on_message)
        frame = tree["frameTree"]["frame"]
        previous_loader = frame["loaderId"]
    navigation, _, seen = await command(client, method, params, session_id=session_id,
                                        on_message=on_message)
    if navigation.get("errorText") or navigation.get("isDownload"):
        raise SmokeError(f"{method} did not load a Document: {navigation!r}")
    if method == "Page.navigate":
        if not navigation.get("frameId") or not navigation.get("loaderId"):
            raise SmokeError(f"{method} did not start a new Document: {navigation!r}")
        frame_id, loader_id = navigation["frameId"], navigation["loaderId"]
    else:
        frame_id, loader_id = frame["id"], None
    try:
        await wait_for_load(client, session_id, seen, frame_id=frame_id,
                            loader_id=loader_id, previous_loader_id=previous_loader,
                            on_message=on_message)
    except SmokeError as error:
        raise SmokeError(f"{method}: {error}") from error
    return navigation


async def navigate(
    client: RawCdpClient,
    session_id: str,
    params: dict[str, Any] | None = None,
    *,
    method: Literal["Page.navigate", "Page.reload"] = "Page.navigate",
    on_message: Observer | None = None,
    timeout: float = 10,
) -> dict[str, Any]:
    try:
        async with asyncio.timeout(timeout):
            return await _navigate(client, session_id, params, method=method, on_message=on_message)
    except TimeoutError as error:
        raise SmokeError(
            f"timed out in {method} session={session_id} params={params!r} after {timeout}s"
        ) from error


async def navigate_cdp(session: Any, url: str, *, timeout: float = 10) -> dict[str, Any]:
    """Observe before sending, retain early events, and release all listeners."""
    events: list[dict[str, Any]] = []
    handlers: list[tuple[str, Any]] = []
    for method in ["Page.frameNavigated", "Page.lifecycleEvent", "Inspector.targetCrashed"]:
        def observe(params: dict[str, Any], method: str = method) -> None:
            events.append({"method": method, "params": params, "sessionId": "current"})
        session.on(method, observe)
        handlers.append((method, observe))
    document: DocumentLoad | None = None
    cursor = 0

    def ready() -> bool:
        nonlocal cursor
        assert document is not None
        while cursor < len(events):
            message = events[cursor]
            cursor += 1
            if document.observe(message):
                return True
        return document.ready

    try:
        async with asyncio.timeout(timeout):
            await session.send("Page.enable")
            await session.send("Page.setLifecycleEventsEnabled", {"enabled": True})
            result = await session.send("Page.navigate", {"url": url})
            if result.get("errorText") or result.get("isDownload"):
                raise SmokeError(f"Page.navigate did not load a Document: {result!r}")
            if not result.get("frameId") or not result.get("loaderId"):
                raise SmokeError(f"Page.navigate did not start a new Document: {result!r}")
            document = DocumentLoad("current", result["frameId"], result["loaderId"])
            await wait_until(ready, f"Document load for {url}")
            return result
    except TimeoutError as error:
        detail = document.describe() if document else "navigation reply pending"
        raise SmokeError(f"timed out navigating {url}: {detail}; seen={events[-20:]!r}") from error
    finally:
        for method, handler in handlers:
            session.remove_listener(method, handler)
