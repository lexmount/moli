from __future__ import annotations

import asyncio
import json
import time
import urllib.request
from dataclasses import dataclass, field
from typing import Any, Callable

import websockets
from websockets.asyncio.client import ClientConnection

from .models import DomFrameObservation, EngineObservation, SmokeCase


class CdpError(RuntimeError):
    pass


class FixtureObservationError(CdpError):
    """The page or fixture violated its deterministic observation contract."""


class CdpCommandError(CdpError):
    def __init__(self, method: str, error: dict[str, Any]) -> None:
        super().__init__(f"{method} failed: {error}")
        self.method = method
        self.error = error


_DOM_ENABLE_PARAMS = {"includeWhitespace": "all"}


def _read_json_url(url: str) -> dict[str, Any]:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(url, timeout=2) as response:
        value = json.loads(response.read().decode("utf-8"))
    if not isinstance(value, dict):
        raise CdpError(f"unexpected CDP discovery response: {value!r}")
    return value


async def discover_websocket_url(endpoint: str) -> str:
    payload = await asyncio.to_thread(_read_json_url, endpoint.rstrip("/") + "/json/version")
    url = payload.get("webSocketDebuggerUrl")
    if not isinstance(url, str) or not url:
        raise CdpError(f"CDP discovery did not include webSocketDebuggerUrl: {payload}")
    return url


@dataclass
class RawCdpClient:
    websocket: ClientConnection
    next_id: int = 1
    events: list[dict[str, Any]] = field(default_factory=list)

    async def send(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        session_id: str | None = None,
    ) -> int:
        message_id = self.next_id
        self.next_id += 1
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
        value = json.loads(raw)
        if not isinstance(value, dict):
            raise CdpError(f"unexpected CDP message: {value!r}")
        return value

    async def command(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        session_id: str | None = None,
        timeout: float = 10.0,
    ) -> dict[str, Any]:
        message_id = await self.send(method, params, session_id=session_id)
        deadline = asyncio.get_running_loop().time() + timeout
        while True:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise CdpError(f"timed out waiting for {method} response")
            message = await asyncio.wait_for(self.recv(), timeout=remaining)
            if message.get("id") == message_id:
                if "error" in message:
                    raise CdpCommandError(method, message["error"])
                result = message.get("result")
                return result if isinstance(result, dict) else {}
            self.events.append(message)

    async def wait_event(
        self,
        method: str,
        *,
        session_id: str,
        timeout: float,
        predicate: Callable[[dict[str, Any]], bool] | None = None,
    ) -> dict[str, Any]:
        def matches(message: dict[str, Any]) -> bool:
            return (
                message.get("sessionId") == session_id
                and message.get("method") == method
                and (predicate is None or predicate(message.get("params") or {}))
            )

        for message in self.events:
            if matches(message):
                return message
        deadline = asyncio.get_running_loop().time() + timeout
        while True:
            remaining = deadline - asyncio.get_running_loop().time()
            if remaining <= 0:
                raise CdpError(f"timed out waiting for {method}")
            message = await asyncio.wait_for(self.recv(), timeout=remaining)
            self.events.append(message)
            if matches(message):
                return message


async def connect(endpoint: str) -> RawCdpClient:
    url = await discover_websocket_url(endpoint)
    websocket = await websockets.connect(
        url,
        open_timeout=5,
        max_size=32 * 1024 * 1024,
        proxy=None,
    )
    return RawCdpClient(websocket=websocket)


def _state_expression(case_id: str, timeout_ms: int, after_token: str | None) -> str:
    return f"""
(() => {{
  const expectedId = {json.dumps(case_id)};
  const timeoutMs = {timeout_ms};
  const afterToken = {json.dumps(after_token)};
  const readState = () => globalThis.__MOLI_FRONTEND_SMOKE__ || null;
  const usable = value => value && value.id === expectedId && (
    value.phase === "ready" ||
    value.phase === "error" ||
    (value.phase === "checkpoint" &&
      value.pendingFrame &&
      value.pendingFrame.token &&
      value.pendingFrame.token !== afterToken)
  );
  const current = readState();
  if (usable(current)) {{
    return current;
  }}
  return new Promise(resolve => {{
    let settled = false;
    const finish = value => {{
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      removeEventListener("moli-frontend-smoke-ready", onState);
      resolve(value);
    }};
    const onState = () => {{
      const value = readState();
      if (usable(value)) {{
        finish(value);
      }}
    }};
    addEventListener("moli-frontend-smoke-ready", onState);
    const timer = setTimeout(() => finish({{
      id: expectedId,
      phase: "timeout",
      observed: readState()
    }}), timeoutMs);
    onState();
  }});
}})()
"""


def _resume_expression(token: str) -> str:
    return f"""
(() => {{
  const resume = globalThis.__MOLI_FRONTEND_SMOKE_RESUME__;
  return typeof resume === "function" && resume({json.dumps(token)});
}})()
"""


_OBSERVABLE_TREE_BARRIER_EXPRESSION = r"""
(() => {
  if (typeof globalThis.getComputedStyle !== "function") {
    throw new Error("getComputedStyle is unavailable");
  }
  let elements = 0;
  for (const element of document.querySelectorAll("*")) {
    getComputedStyle(element).display;
    getComputedStyle(element, "::before").content;
    getComputedStyle(element, "::after").content;
    getComputedStyle(element, "::marker").content;
    elements += 1;
  }
  return elements;
})()
"""


async def _capture_document(
    client: RawCdpClient,
    *,
    session_id: str,
    timeout: float,
) -> dict[str, Any]:
    barrier = await client.command(
        "Runtime.evaluate",
        {
            "expression": _OBSERVABLE_TREE_BARRIER_EXPRESSION,
            "returnByValue": True,
        },
        session_id=session_id,
        timeout=timeout,
    )
    if barrier.get("exceptionDetails"):
        raise FixtureObservationError(
            "observable-tree materialization threw: "
            f"{barrier['exceptionDetails']}"
        )
    materialized = (barrier.get("result") or {}).get("value")
    if not isinstance(materialized, int) or materialized < 1:
        raise FixtureObservationError(
            "observable-tree materialization returned invalid element count: "
            f"{materialized!r}"
        )
    document = await client.command(
        "DOM.getDocument",
        {"depth": -1, "pierce": True},
        session_id=session_id,
        timeout=timeout,
    )
    root = document.get("root")
    if not isinstance(root, dict):
        raise CdpError(f"DOM.getDocument returned no root: {document}")
    return root


def _diagnostics(events: list[dict[str, Any]], session_id: str) -> dict[str, Any]:
    exceptions = []
    console_errors = []
    network_failures = []
    http_errors = []
    request_urls: dict[str, str] = {}
    for event in events:
        if event.get("sessionId") != session_id:
            continue
        method = event.get("method")
        params = event.get("params") or {}
        if method == "Network.requestWillBeSent":
            request_id = params.get("requestId")
            request_url = (params.get("request") or {}).get("url")
            if isinstance(request_id, str) and isinstance(request_url, str):
                request_urls[request_id] = request_url
        elif method == "Runtime.exceptionThrown":
            details = params.get("exceptionDetails") or {}
            exceptions.append(
                {
                    "text": details.get("text"),
                    "lineNumber": details.get("lineNumber"),
                    "columnNumber": details.get("columnNumber"),
                    "url": details.get("url"),
                    "exception": (details.get("exception") or {}).get("description"),
                }
            )
        elif method == "Runtime.consoleAPICalled" and params.get("type") in {"error", "assert"}:
            console_errors.append(
                {
                    "type": params.get("type"),
                    "args": [
                        argument.get("value", argument.get("description"))
                        for argument in params.get("args") or []
                        if isinstance(argument, dict)
                    ],
                }
            )
        elif method == "Network.loadingFailed":
            network_failures.append(
                {
                    "requestId": params.get("requestId"),
                    "url": request_urls.get(str(params.get("requestId"))),
                    "errorText": params.get("errorText"),
                    "type": params.get("type"),
                    "canceled": params.get("canceled"),
                    "blockedReason": params.get("blockedReason"),
                }
            )
        elif method == "Network.responseReceived":
            response = params.get("response") or {}
            status = response.get("status")
            if isinstance(status, (int, float)) and status >= 400:
                http_errors.append(
                    {
                        "requestId": params.get("requestId"),
                        "url": response.get("url"),
                        "status": status,
                        "statusText": response.get("statusText"),
                        "type": params.get("type"),
                    }
                )
    return {
        "exceptions": exceptions[-20:],
        "consoleErrors": console_errors[-20:],
        "networkFailures": network_failures[-20:],
        "httpErrors": http_errors[-20:],
    }


def _reconcile_expected_diagnostics(
    diagnostics: dict[str, Any],
    ready_state: dict[str, Any],
) -> None:
    expected_container = ready_state.get("expectedDiagnostics") or {}
    if not isinstance(expected_container, dict):
        raise FixtureObservationError("fixture expectedDiagnostics is not an object")
    expected = expected_container.get("networkFailures") or []
    if not isinstance(expected, list):
        raise FixtureObservationError("fixture expected network failures are not a list")
    unmatched = list(diagnostics.get("networkFailures") or [])
    matched: list[dict[str, Any]] = []
    missing: list[dict[str, Any]] = []
    labels: set[str] = set()
    for position, item in enumerate(expected):
        if not isinstance(item, dict):
            raise FixtureObservationError(
                f"expected network failure {position} is not an object"
            )
        label = item.get("label")
        url = item.get("url")
        resource_type = item.get("type")
        canceled = item.get("canceled")
        if (
            not isinstance(label, str)
            or not label
            or label in labels
            or not isinstance(url, str)
            or not url
            or not isinstance(resource_type, str)
            or not resource_type
            or not isinstance(canceled, bool)
        ):
            raise FixtureObservationError(
                f"invalid expected network failure {position}: {item!r}"
            )
        labels.add(label)
        match_index = next(
            (
                index
                for index, failure in enumerate(unmatched)
                if failure.get("url") == url
                and failure.get("type") == resource_type
                and failure.get("canceled") is canceled
            ),
            None,
        )
        if match_index is None:
            missing.append(
                {
                    "label": label,
                    "url": url,
                    "type": resource_type,
                    "canceled": canceled,
                }
            )
            continue
        failure = unmatched.pop(match_index)
        matched.append(
            {
                "label": label,
                "errorText": failure.get("errorText"),
                "type": failure.get("type"),
                "canceled": failure.get("canceled"),
                "blockedReason": failure.get("blockedReason"),
            }
        )
    diagnostics["networkFailures"] = unmatched
    diagnostics["expectedNetworkFailures"] = matched
    diagnostics["missingExpectedNetworkFailures"] = missing


def _diagnostics_have_errors(diagnostics: dict[str, Any]) -> bool:
    return any(
        diagnostics.get(key)
        for key in (
            "exceptions",
            "consoleErrors",
            "networkFailures",
            "httpErrors",
            "missingExpectedNetworkFailures",
        )
    )


async def observe_case(
    *,
    engine: str,
    endpoint: str,
    case: SmokeCase,
    url: str,
    timeout_ms: int,
    client: RawCdpClient | None = None,
) -> EngineObservation:
    started = time.perf_counter()
    owns_client = client is None
    context_id: str | None = None
    session_id: str | None = None
    ready_state: dict[str, Any] | None = None
    frames: list[DomFrameObservation] = []
    final_root: dict[str, Any] | None = None
    diagnostics: dict[str, Any] = {}
    try:
        if client is None:
            client = await connect(endpoint)
        else:
            client.events.clear()
        context = await client.command("Target.createBrowserContext")
        context_id = context.get("browserContextId")
        if not isinstance(context_id, str) or not context_id:
            raise CdpError(f"Target.createBrowserContext returned {context}")
        target = await client.command(
            "Target.createTarget",
            {"browserContextId": context_id, "url": "about:blank"},
        )
        target_id = target.get("targetId")
        if not isinstance(target_id, str) or not target_id:
            raise CdpError(f"Target.createTarget returned {target}")
        attached = await client.command(
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        session_id = attached.get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            raise CdpError(f"Target.attachToTarget returned {attached}")

        command_timeout = max(10.0, timeout_ms / 1000 + 2.0)
        await client.command("Runtime.enable", session_id=session_id, timeout=command_timeout)
        await client.command("Page.enable", session_id=session_id, timeout=command_timeout)
        await client.command(
            "DOM.enable",
            _DOM_ENABLE_PARAMS,
            session_id=session_id,
            timeout=command_timeout,
        )
        await client.command("Network.enable", session_id=session_id, timeout=command_timeout)
        await client.command(
            "Page.navigate",
            {"url": url},
            session_id=session_id,
            timeout=command_timeout,
        )
        await client.wait_event(
            "Page.loadEventFired",
            session_id=session_id,
            timeout=command_timeout,
        )
        after_token: str | None = None
        while True:
            evaluation = await client.command(
                "Runtime.evaluate",
                {
                    "expression": _state_expression(case.id, timeout_ms, after_token),
                    "awaitPromise": True,
                    "returnByValue": True,
                },
                session_id=session_id,
                timeout=command_timeout,
            )
            if evaluation.get("exceptionDetails"):
                raise FixtureObservationError(
                    f"state Runtime.evaluate threw: {evaluation['exceptionDetails']}"
                )
            state_value = (evaluation.get("result") or {}).get("value")
            if not isinstance(state_value, dict):
                raise FixtureObservationError(
                    f"fixture returned invalid state: {state_value!r}"
                )
            ready_state = state_value
            if ready_state.get("id") != case.id:
                raise FixtureObservationError(
                    f"fixture state id mismatch: {ready_state!r}"
                )
            phase = ready_state.get("phase")
            if phase == "checkpoint":
                pending = ready_state.get("pendingFrame")
                if not isinstance(pending, dict):
                    raise FixtureObservationError(
                        f"checkpoint has no pending frame: {ready_state!r}"
                    )
                index = pending.get("index")
                name = pending.get("name")
                token = pending.get("token")
                if index != len(frames) or not isinstance(name, str) or not isinstance(token, str):
                    raise FixtureObservationError(
                        f"invalid checkpoint sequence at frame {len(frames)}: {pending!r}"
                    )
                if len(frames) >= 32:
                    raise FixtureObservationError(
                        "fixture exceeded the 32-frame safety limit"
                    )
                root = await _capture_document(
                    client,
                    session_id=session_id,
                    timeout=command_timeout,
                )
                frames.append(
                    DomFrameObservation(index=index, name=name, token=token, dom=root)
                )
                resumed = await client.command(
                    "Runtime.evaluate",
                    {
                        "expression": _resume_expression(token),
                        "returnByValue": True,
                    },
                    session_id=session_id,
                    timeout=command_timeout,
                )
                if resumed.get("exceptionDetails"):
                    raise FixtureObservationError(
                        f"frame resume Runtime.evaluate threw: {resumed['exceptionDetails']}"
                    )
                if (resumed.get("result") or {}).get("value") is not True:
                    raise FixtureObservationError(
                        f"fixture refused frame resume token {token!r}"
                    )
                after_token = token
                continue
            if phase != "ready":
                raise FixtureObservationError(
                    f"fixture did not become ready: {ready_state!r}"
                )
            if ready_state.get("errors"):
                raise FixtureObservationError(
                    f"fixture reported errors: {ready_state!r}"
                )
            reported_frames = ready_state.get("frames")
            observed_names = [frame.name for frame in frames]
            if reported_frames != observed_names:
                raise FixtureObservationError(
                    "fixture frame history mismatch: "
                    f"reported {reported_frames!r}, observed {observed_names!r}"
                )
            break

        final_root = await _capture_document(
            client,
            session_id=session_id,
            timeout=command_timeout,
        )
        frames.append(
            DomFrameObservation(
                index=len(frames),
                name="settled",
                token=f"{case.id}:settled",
                dom=final_root,
            )
        )
        diagnostics = _diagnostics(client.events, session_id)
        _reconcile_expected_diagnostics(diagnostics, ready_state)
        if _diagnostics_have_errors(diagnostics):
            raise FixtureObservationError(
                "fixture emitted browser diagnostics: "
                + json.dumps(diagnostics, ensure_ascii=False, separators=(",", ":"))
            )
        return EngineObservation(
            engine=engine,
            ok=True,
            duration_ms=(time.perf_counter() - started) * 1000,
            ready_state=ready_state,
            dom=final_root,
            frames=frames,
            diagnostics=diagnostics,
        )
    except Exception as error:
        if not diagnostics:
            diagnostics = _diagnostics(client.events, session_id) if client and session_id else {}
        return EngineObservation(
            engine=engine,
            ok=False,
            duration_ms=(time.perf_counter() - started) * 1000,
            ready_state=ready_state,
            dom=final_root,
            frames=frames,
            diagnostics=diagnostics,
            error_type=type(error).__name__,
            error=str(error),
            failure_kind=(
                "fixture"
                if isinstance(error, FixtureObservationError)
                else "infrastructure"
            ),
        )
    finally:
        if client is not None:
            if context_id is not None:
                try:
                    await client.command(
                        "Target.disposeBrowserContext",
                        {"browserContextId": context_id},
                        timeout=3,
                    )
                except Exception:
                    pass
            if owns_client:
                await client.websocket.close()
