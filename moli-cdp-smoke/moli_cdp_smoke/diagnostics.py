"""Bounded protocol checkpoints, without adding browser commands or retries."""

from __future__ import annotations

import asyncio
import json
import itertools
import os
import sys
import time
import traceback
import weakref
from collections import deque
from contextlib import contextmanager
from contextvars import ContextVar
from pathlib import Path
from typing import Any, Iterator


_ACTIVE: ContextVar[Diagnostics | None] = ContextVar("smoke_diagnostics", default=None)
_CHECKPOINT_INTERVAL = 0.25
_CALL_IDS = itertools.count(1)


def _bounded(value: Any, depth: int = 0) -> Any:
    if isinstance(value, str):
        return value if len(value) <= 2_048 else value[:2_048] + f"… ({len(value)} characters)"
    if depth >= 8:
        return "<nested payload omitted>"
    if isinstance(value, dict):
        return {key: _bounded(item, depth + 1) for key, item in list(value.items())[:40]}
    if isinstance(value, list):
        items = [_bounded(item, depth + 1) for item in value[:16]]
        return items + ([{"omittedItems": len(value) - 16}] if len(value) > 16 else [])
    return value


def _resources() -> dict[str, Any]:
    result: dict[str, Any] = {"cpuCount": os.cpu_count(), "python": sys.version}
    if hasattr(os, "sched_getaffinity"):
        result["cpuAffinity"] = sorted(os.sched_getaffinity(0))
    if hasattr(os, "getloadavg"):
        result["loadAverage"] = os.getloadavg()
    for name in ["cpu.max", "cpu.stat", "memory.max", "memory.events", "pids.max"]:
        try:
            result[name] = (Path("/sys/fs/cgroup") / name).read_text()[:4_096]
        except OSError as error:
            result[name] = {"unavailable": type(error).__name__}
    return result


class Diagnostics:
    def __init__(self, result_path: Path | None) -> None:
        self.directory = result_path.with_suffix(".diagnostics") if result_path else None
        self.started = time.monotonic()
        self.started_at = time.time()
        self.initial_resources = _resources()
        self.messages: deque[dict[str, Any]] = deque(maxlen=128)
        self.phases: deque[dict[str, Any]] = deque(maxlen=128)
        self.pending: dict[tuple[str, int], dict[str, Any]] = {}
        self.errors: list[str] = []
        self.artifacts: list[str] = []
        self.message_count = 0
        self.failure_count = 0
        self._checkpoint: asyncio.TimerHandle | None = None
        self._closed = False
        self.flush()

    def _schedule_checkpoint(self) -> None:
        if self.directory is not None and self._checkpoint is None and not self._closed:
            try:
                self._checkpoint = asyncio.get_running_loop().call_later(
                    _CHECKPOINT_INTERVAL, self.flush,
                )
            except RuntimeError:
                self.flush()

    def wire(self, client: Any, direction: str, message: dict[str, Any],
             *, transport: str = "raw-cdp") -> None:
        connection = f"{transport}:{id(client):x}"
        self.message_count += 1
        entry = {"sequence": self.message_count, "elapsed": time.monotonic() - self.started,
                 "connection": connection, "transport": transport,
                 "direction": direction, "message": _bounded(message)}
        self.messages.append(entry)
        message_id = message.get("id")
        if isinstance(message_id, int):
            key = connection, message_id
            if direction == "send":
                self.pending[key] = entry
            elif direction == "receive":
                self.pending.pop(key, None)
        self._schedule_checkpoint()

    def phase(self, label: str, status: str, error: BaseException | None = None) -> None:
        self.phases.append({"label": label, "status": status,
                            "elapsed": time.monotonic() - self.started,
                            **({"error": f"{type(error).__name__}: {error}"} if error else {})})
        if error is not None and self.directory is not None:
            self.failure_count += 1
            snapshot = self._snapshot()
            try:
                snapshot["taskStacks"] = [
                    {"name": task.get_name(),
                     "stack": [line for frame in task.get_stack()
                               for line in traceback.format_stack(frame)]}
                    for task in asyncio.all_tasks()
                ]
            except RuntimeError:  # A synchronous diagnostic has no running loop.
                snapshot["taskStacks"] = []
            self.write(f"failure-{self.failure_count:03d}.json", snapshot)
        self._schedule_checkpoint()

    def write(self, name: str, payload: Any) -> str | None:
        if self.directory is None:
            return None
        if Path(name).name != name:
            raise ValueError("diagnostic artifact names must be filenames")
        path = self.directory / name
        try:
            self.directory.mkdir(parents=True, exist_ok=True)
            temporary = path.with_suffix(path.suffix + ".tmp")
            # CDP can carry lone UTF-16 surrogates. Escape them losslessly so
            # legitimate JavaScript strings cannot break UTF-8 diagnostics.
            temporary.write_text(json.dumps(payload, ensure_ascii=True, indent=2) + "\n",
                                 encoding="utf-8")
            os.replace(temporary, path)
        except OSError as error:
            if len(self.errors) < 16:
                self.errors.append(f"{path}: {type(error).__name__}: {error}")
            return None
        if name != "checkpoint.json" and str(path) not in self.artifacts:
            self.artifacts.append(str(path))
        return str(path)

    def flush(self) -> None:
        if self._checkpoint is not None:
            self._checkpoint.cancel()
            self._checkpoint = None
        self.write("checkpoint.json", self._snapshot())

    def _snapshot(self) -> dict[str, Any]:
        return {
            "startedAt": self.started_at, "elapsed": time.monotonic() - self.started,
            "initialResources": self.initial_resources, "currentResources": _resources(),
            "messageCount": self.message_count, "lastMessages": list(self.messages),
            "pendingCommands": list(self.pending.values()), "phases": list(self.phases),
            "artifacts": self.artifacts, "diagnosticErrors": self.errors,
            "playwrightIds": {
                "playwright": "Local CDPSession call identifiers, not CDP wire request IDs.",
                "playwright-driver": "Python SDK/driver RPC identifiers, not CDP wire request IDs.",
            },
        }

    def finish(self) -> dict[str, Any]:
        if not self._closed:
            self._closed = True
            self.flush()
        return {"directory": str(self.directory) if self.directory else None,
                "messageCount": self.message_count, "artifacts": self.artifacts,
                "errors": self.errors}


@contextmanager
def diagnostics_scope(result_path: Path | None) -> Iterator[Diagnostics]:
    diagnostics = Diagnostics(result_path)
    token = _ACTIVE.set(diagnostics)
    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        loop = None
    previous_handler = loop.get_exception_handler() if loop is not None else None

    def unhandled(active_loop: asyncio.AbstractEventLoop, context: dict[str, Any]) -> None:
        error = context.get("exception")
        details = ("".join(traceback.format_exception(error))
                   if isinstance(error, BaseException) else repr(context))
        if len(diagnostics.errors) < 16:
            diagnostics.errors.append(f"unhandled asyncio error: {context.get('message')}: {details}")
        diagnostics.phase("asyncio/unhandled", "failed", error)
        if previous_handler is not None:
            previous_handler(active_loop, context)
        else:
            active_loop.default_exception_handler(context)

    if loop is not None:
        loop.set_exception_handler(unhandled)
    try:
        yield diagnostics
    finally:
        try:
            diagnostics.finish()
        finally:
            if loop is not None:
                loop.set_exception_handler(previous_handler)
            _ACTIVE.reset(token)


def current_diagnostics() -> Diagnostics | None:
    return _ACTIVE.get()


def record_wire(client: Any, direction: str, message: dict[str, Any],
                *, transport: str = "raw-cdp") -> None:
    if diagnostics := _ACTIVE.get():
        diagnostics.wire(client, direction, message, transport=transport)


def record_phase(label: str, status: str, error: BaseException | None = None) -> None:
    if diagnostics := _ACTIVE.get():
        diagnostics.phase(label, status, error)


def write_artifact(name: str, payload: Any) -> str | None:
    if diagnostics := _ACTIVE.get():
        return diagnostics.write(name, payload)
    return None


@contextmanager
def observe_playwright_connection(playwright: Any) -> Iterator[None]:
    """Observe the pinned Python SDK's existing transport, including Page.goto.

    CDPSession wrappers cannot see the SDK's own navigation and locator traffic.
    Fail explicitly if a future SDK changes this adapter; never silently lose
    that evidence. This delegates the same messages to the same reader/writer.
    """
    try:
        transport = playwright._impl_obj._connection._transport
        send, receive = transport.send, transport.on_message
        if not callable(send) or not callable(receive):
            raise AttributeError("transport callbacks are not callable")
    except AttributeError as error:
        raise RuntimeError("Playwright transport changed; update the smoke diagnostic adapter") from error

    def observed_send(message: dict[str, Any]) -> None:
        record_wire(transport, "send", message, transport="playwright-driver")
        send(message)

    def observed_receive(message: dict[str, Any]) -> None:
        record_wire(transport, "receive", message, transport="playwright-driver")
        receive(message)

    transport.send, transport.on_message = observed_send, observed_receive
    try:
        yield
    finally:
        transport.send, transport.on_message = send, receive


class ObservedCdpSession:
    """Delegate the SDK API while recording local calls and their outcomes."""

    def __init__(self, session: Any) -> None:
        self._session = session
        self._event_loggers: dict[str, Any] = {}

    def __getattr__(self, name: str) -> Any:
        return getattr(self._session, name)

    def _observe_event(self, method: str) -> None:
        if method not in self._event_loggers:
            reference = weakref.ref(self)

            def observed(params: Any) -> None:
                if client := reference():
                    record_wire(client, "event", {"method": method, "params": params},
                                transport="playwright")

            self._session.on(method, observed)
            self._event_loggers[method] = observed

    def on(self, method: str, handler: Any) -> Any:
        self._observe_event(method)
        return self._session.on(method, handler)

    def once(self, method: str, handler: Any) -> Any:
        self._observe_event(method)
        return self._session.once(method, handler)

    async def _call(self, method: str, params: Any, invoke: Any) -> Any:
        return await _observe_call(self, method, params, invoke)

    async def send(self, method: str, params: dict[str, Any] | None = None) -> Any:
        return await self._call(method, params, lambda: self._session.send(method, params))

    async def detach(self) -> None:
        try:
            return await self._call("CDPSession.detach", None, self._session.detach)
        finally:
            for method, handler in self._event_loggers.items():
                self._session.remove_listener(method, handler)
            self._event_loggers.clear()


async def new_cdp_session(context: Any, page: Any) -> ObservedCdpSession:
    session = await _observe_call(context, "BrowserContext.newCDPSession",
                                  {"url": getattr(page, "url", None)},
                                  lambda: context.new_cdp_session(page),
                                  result_description="CDPSession created")
    return ObservedCdpSession(session)


async def _observe_call(client: Any, method: str, params: Any, invoke: Any,
                        *, result_description: str | None = None) -> Any:
    call_id = next(_CALL_IDS)
    record_wire(client, "send", {"id": call_id, "method": method, "params": params},
                transport="playwright")
    try:
        result = await invoke()
    except BaseException as error:
        record_wire(client, "receive", {"id": call_id, "error": {
            "type": type(error).__name__, "message": str(error),
        }}, transport="playwright")
        raise
    record_wire(client, "receive", {"id": call_id, "result": result_description or result},
                transport="playwright")
    return result
