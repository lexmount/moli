"""Per-engine WPT case runner.

Connects to a launched :class:`EngineDriver` and executes either testharness
cases (through ``window.__bench_wpt__``) or manifest-backed screenshot
reftests at a fixed viewport.

Classifies each result as ``pass`` / ``fail`` / ``timeout`` / ``crash`` /
``error`` so that the cross-engine matrix can be built without rewriting raw
engine behavior. Reftest failures retain test/reference/diff PNG artifacts.
"""

from __future__ import annotations

import asyncio
import base64
import hashlib
import io
import json
import re
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any
from urllib.parse import quote, urlsplit, urlunsplit

from PIL import Image, ImageChops

from ..raw_cdp import RawCdpClient, RawCdpError, connect_raw_cdp
from .case_set import FuzzyTolerance
from .engine import EngineDriver, EngineDriverHandle

LAYOUT_VIEWPORT_WIDTH = 800
LAYOUT_VIEWPORT_HEIGHT = 600
LAYOUT_DEVICE_SCALE_FACTOR = 1.0


@dataclass(frozen=True)
class Viewport:
    width: int = LAYOUT_VIEWPORT_WIDTH
    height: int = LAYOUT_VIEWPORT_HEIGHT
    device_scale_factor: float = LAYOUT_DEVICE_SCALE_FACTOR


LAYOUT_VIEWPORT = Viewport()


@dataclass(frozen=True)
class ReftestReferenceRun:
    reference_path: str
    url: str
    relation: str
    fuzzy: FuzzyTolerance | None = None


@dataclass(frozen=True)
class ReftestRun:
    case_path: str
    url: str
    timeout_seconds: float
    references: tuple[ReftestReferenceRun, ...]


CaseRun = tuple[str, str] | tuple[str, str, float] | ReftestRun


# WPT testharness.js status constants (testharness.js: TestsStatus)
HARNESS_STATUS_OK = 0
HARNESS_STATUS_ERROR = 1
HARNESS_STATUS_TIMEOUT = 2
HARNESS_STATUS_PRECONDITION_FAILED = 3

HARNESS_STATUS_NAMES = {
    HARNESS_STATUS_OK: "OK",
    HARNESS_STATUS_ERROR: "ERROR",
    HARNESS_STATUS_TIMEOUT: "TIMEOUT",
    HARNESS_STATUS_PRECONDITION_FAILED: "PRECONDITION_FAILED",
}

# WPT testharness.js per-test status constants (testharness.js: Test.statuses)
TEST_STATUS_PASS = 0
TEST_STATUS_FAIL = 1
TEST_STATUS_TIMEOUT = 2
TEST_STATUS_NOTRUN = 3
TEST_STATUS_PRECONDITION_FAILED = 4

TEST_STATUS_NAMES = {
    TEST_STATUS_PASS: "PASS",
    TEST_STATUS_FAIL: "FAIL",
    TEST_STATUS_TIMEOUT: "TIMEOUT",
    TEST_STATUS_NOTRUN: "NOTRUN",
    TEST_STATUS_PRECONDITION_FAILED: "PRECONDITION_FAILED",
}

MAX_RECORDED_FAILURES = 40
MAX_FAILURE_MESSAGE_CHARS = 500
FINAL_PAYLOAD_SOURCES = {"completion-callback", "done-hook", "done-hook-late"}


@dataclass
class CaseResult:
    case_path: str
    url: str
    status: str  # "pass" | "fail" | "timeout" | "crash" | "error"
    duration_ms: float | None
    harness_status: int | None = None
    harness_message: str | None = None
    subtests_total: int = 0
    subtests_pass: int = 0
    subtests_fail: int = 0
    subtests_timeout: int = 0
    subtests_notrun: int = 0
    error: str | None = None
    console_errors: int = 0
    js_exceptions: int = 0
    payload_source: str | None = None
    failures: list[dict[str, Any]] = field(default_factory=list)
    failure_names: list[str] = field(default_factory=list)
    test_type: str = "testharness"
    reftest_comparisons: list[dict[str, Any]] = field(default_factory=list)
    artifacts: dict[str, Any] = field(default_factory=dict)


@dataclass
class EngineRunResult:
    engine: str
    binary: str | None
    binary_sha256: str | None
    binary_version: str | None
    endpoint: str
    ready_ms: float | None
    cases: list[CaseResult] = field(default_factory=list)
    shutdown_info: dict[str, Any] = field(default_factory=dict)
    setup_error: str | None = None


class _PageSessionUnusable(RuntimeError):
    """Preserve the current case result while forcing page-session recovery."""

    def __init__(self, case_result: CaseResult) -> None:
        self.case_result = case_result
        super().__init__(case_result.error or "page session became unusable")


@dataclass(frozen=True)
class _AttachedPage:
    browser_context_id: str | None
    target_id: str
    session_id: str
    baseline_target_ids: frozenset[str] | None


@dataclass(frozen=True)
class _NavigationIdentity:
    session_id: str
    frame_id: str
    loader_id: str
    expected_url: str


_TRACE_METHODS = {"Runtime.consoleAPICalled", "Runtime.exceptionThrown", "Log.entryAdded"}


def _count_traces(messages: list[dict[str, Any]]) -> tuple[int, int]:
    console_errors = 0
    js_exceptions = 0
    for message in messages:
        method = message.get("method")
        if method == "Runtime.exceptionThrown":
            js_exceptions += 1
        elif method == "Runtime.consoleAPICalled":
            params = message.get("params") or {}
            if params.get("type") in {"error", "assert"}:
                console_errors += 1
        elif method == "Log.entryAdded":
            entry = (message.get("params") or {}).get("entry") or {}
            if entry.get("level") == "error":
                console_errors += 1
    return console_errors, js_exceptions


def _recorded_failures(tests: Any) -> list[dict[str, Any]]:
    if not isinstance(tests, list):
        return []
    failures: list[dict[str, Any]] = []
    for entry in tests:
        if not isinstance(entry, dict):
            continue
        status = entry.get("status")
        if status == TEST_STATUS_PASS:
            continue
        failure: dict[str, Any] = {
            "status": status,
            "status_name": TEST_STATUS_NAMES.get(status) if isinstance(status, int) else None,
        }
        name = entry.get("name")
        if isinstance(name, str):
            failure["name"] = name
        message = entry.get("message")
        if isinstance(message, str) and message:
            failure["message"] = message[:MAX_FAILURE_MESSAGE_CHARS]
            if len(message) > MAX_FAILURE_MESSAGE_CHARS:
                failure["message_truncated"] = True
        failures.append(failure)
        if len(failures) >= MAX_RECORDED_FAILURES:
            break
    return failures


def _failure_name(entry: dict[str, Any], index: int) -> str:
    name = entry.get("name")
    if isinstance(name, str) and name:
        return name
    status = entry.get("status_name") or entry.get("status") or "failure"
    return f"<unnamed {status} #{index + 1}>"


def _failure_names(tests: Any) -> list[str]:
    if not isinstance(tests, list):
        return []
    names: list[str] = []
    for index, entry in enumerate(tests):
        if not isinstance(entry, dict):
            names.append(f"<malformed failure #{index + 1}>")
            continue
        status = entry.get("status")
        if status == TEST_STATUS_PASS:
            continue
        names.append(_failure_name(entry, index))
    return names


def _exception_text(error: BaseException) -> str:
    return str(error) or type(error).__name__


async def _target_infos(client: RawCdpClient) -> list[dict[str, Any]]:
    command_id = await client.send("Target.getTargets")
    response, _ = await client.recv_until_id(command_id, timeout=5)
    infos = (response.get("result") or {}).get("targetInfos")
    if not isinstance(infos, list):
        raise RawCdpError(f"Target.getTargets returned invalid targetInfos: {response}")
    return [info for info in infos if isinstance(info, dict)]


def _target_ids(infos: list[dict[str, Any]]) -> frozenset[str]:
    return frozenset(
        target_id
        for info in infos
        if isinstance((target_id := info.get("targetId")), str) and target_id
    )


async def _close_target(client: RawCdpClient, target_id: str) -> None:
    command_id = await client.send("Target.closeTarget", {"targetId": target_id})
    response, _ = await client.recv_until_id(command_id, timeout=5)
    success = (response.get("result") or {}).get("success")
    if success is False:
        raise RawCdpError(f"Target.closeTarget rejected target {target_id}")


async def _close_page(client: RawCdpClient, page: _AttachedPage) -> None:
    """Dispose one case's storage context and every target created inside it."""

    try:
        before = await _target_infos(client)
    except (RawCdpError, asyncio.TimeoutError):
        before = []

    if page.browser_context_id is not None:
        command_id = await client.send(
            "Target.disposeBrowserContext",
            {"browserContextId": page.browser_context_id},
        )
        await client.recv_until_id(command_id, timeout=10)
    else:
        owned_ids = {
            target_id
            for info in before
            if isinstance((target_id := info.get("targetId")), str)
            and target_id
            and (
                target_id == page.target_id
                or (
                    page.baseline_target_ids is not None
                    and target_id not in page.baseline_target_ids
                )
            )
        }
        owned_ids.add(page.target_id)
        for target_id in sorted(owned_ids):
            await _close_target(client, target_id)

    try:
        after = await _target_infos(client)
    except (RawCdpError, asyncio.TimeoutError):
        return

    residual_ids = {
        target_id
        for info in after
        if isinstance((target_id := info.get("targetId")), str)
        and target_id
        and (
            (
                page.browser_context_id is not None
                and info.get("browserContextId") == page.browser_context_id
            )
            or (
                page.browser_context_id is None
                and (
                    target_id == page.target_id
                    or (
                        page.baseline_target_ids is not None
                        and target_id not in page.baseline_target_ids
                    )
                )
            )
        )
    }
    for target_id in sorted(residual_ids):
        await _close_target(client, target_id)

    if residual_ids:
        remaining = _target_ids(await _target_infos(client))
        leaked = residual_ids & remaining
        if leaked:
            raise RawCdpError(
                f"case cleanup left auxiliary targets alive: {sorted(leaked)}"
            )


async def _attach_page(
    client: RawCdpClient,
    *,
    viewport: Viewport | None = None,
) -> _AttachedPage:
    """Create an isolated BrowserContext + Target for exactly one WPT case."""

    try:
        baseline_target_ids: frozenset[str] | None = _target_ids(
            await _target_infos(client)
        )
    except (RawCdpError, asyncio.TimeoutError):
        baseline_target_ids = None

    browser_context_id: str | None = None
    target: str | None = None
    try:
        try:
            ctx_id = await client.send("Target.createBrowserContext")
            ctx_resp, _ = await client.recv_until_id(ctx_id, timeout=5)
            value = (ctx_resp.get("result") or {}).get("browserContextId")
            if isinstance(value, str) and value:
                browser_context_id = value
        except (RawCdpError, asyncio.TimeoutError):
            browser_context_id = None

        target_params: dict[str, Any] = {"url": "about:blank"}
        if browser_context_id:
            target_params["browserContextId"] = browser_context_id
        target_id = await client.send("Target.createTarget", target_params)
        target_resp, _ = await client.recv_until_id(target_id, timeout=10)
        target = (target_resp.get("result") or {}).get("targetId")
        if not isinstance(target, str) or not target:
            raise RuntimeError(f"missing targetId in createTarget response: {target_resp}")

        attach_id = await client.send(
            "Target.attachToTarget", {"targetId": target, "flatten": True}
        )
        attach_resp, _ = await client.recv_until_id(attach_id, timeout=5)
        session_id = (attach_resp.get("result") or {}).get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            raise RuntimeError(f"missing sessionId in attachToTarget response: {attach_resp}")

        for method, params in (
            ("Runtime.enable", None),
            ("Page.enable", None),
            ("Page.setLifecycleEventsEnabled", {"enabled": True}),
        ):
            cmd_id = await client.send(method, params, session_id=session_id)
            await client.recv_until_id(cmd_id, timeout=5)
        if viewport is not None:
            cmd_id = await client.send(
                "Emulation.setDeviceMetricsOverride",
                {
                    "width": viewport.width,
                    "height": viewport.height,
                    "deviceScaleFactor": viewport.device_scale_factor,
                    "mobile": False,
                },
                session_id=session_id,
            )
            await client.recv_until_id(cmd_id, timeout=5)
        try:
            cmd_id = await client.send("Log.enable", session_id=session_id)
            await client.recv_until_id(cmd_id, timeout=3)
        except (RawCdpError, asyncio.TimeoutError):
            pass

        return _AttachedPage(
            browser_context_id=browser_context_id,
            target_id=target,
            session_id=session_id,
            baseline_target_ids=baseline_target_ids,
        )
    except BaseException:
        if target is not None:
            try:
                await _close_page(
                    client,
                    _AttachedPage(
                        browser_context_id=browser_context_id,
                        target_id=target,
                        session_id="",
                        baseline_target_ids=baseline_target_ids,
                    ),
                )
            except BaseException:
                pass
        elif browser_context_id is not None:
            try:
                command_id = await client.send(
                    "Target.disposeBrowserContext",
                    {"browserContextId": browser_context_id},
                )
                await client.recv_until_id(command_id, timeout=5)
            except BaseException:
                pass
        raise


_HARNESS_PROBE_EXPRESSION = """
(function() {
  if (typeof window === 'undefined' || typeof location === 'undefined') return null;
  var trace = window.__bench_wpt_trace__;
  var bridgeInstalled = false;
  if (Array.isArray(trace)) {
    for (var i = 0; i < trace.length; i++) {
      if (trace[i] && trace[i].installing === true) {
        bridgeInstalled = true;
        break;
      }
    }
  }
  return {
    bridgeInstalled: bridgeInstalled,
    payload: typeof window.__bench_wpt__ === 'undefined' ? null : window.__bench_wpt__
  };
})()
"""


def _normalized_case_path(value: str) -> str:
    parts = urlsplit(value)
    path = quote(parts.path.lstrip("/"), safe="/%:@!$&'()*+,;=-._~")
    query = quote(parts.query, safe="=&?/%:@!$'()*+,;[-]._~")
    return urlunsplit(("", "", path, query, ""))


def _normalized_navigation_url(value: str) -> str:
    parts = urlsplit(value)
    path = quote(parts.path or "/", safe="/%:@!$&'()*+,;=-._~")
    query = quote(parts.query, safe="=&?/%:@!$'()*+,;[-]._~")
    return urlunsplit(
        (
            parts.scheme.lower(),
            parts.netloc.lower(),
            path,
            query,
            "",
        )
    )


def _navigation_identity(
    response: dict[str, Any],
    *,
    session_id: str,
    expected_url: str,
) -> _NavigationIdentity:
    response_session = response.get("sessionId")
    if response_session is not None and response_session != session_id:
        raise RawCdpError(
            f"Page.navigate response belonged to session {response_session!r}, "
            f"expected {session_id!r}"
        )
    result = response.get("result")
    if not isinstance(result, dict):
        raise RawCdpError(f"Page.navigate returned no result: {response}")
    error_text = result.get("errorText")
    if isinstance(error_text, str) and error_text:
        raise RawCdpError(f"Page.navigate failed for `{expected_url}`: {error_text}")
    if result.get("isDownload") is True:
        raise RawCdpError(f"Page.navigate produced a download for `{expected_url}`")
    frame_id = result.get("frameId")
    loader_id = result.get("loaderId")
    if not isinstance(frame_id, str) or not frame_id:
        raise RawCdpError(f"Page.navigate returned no frameId: {response}")
    if not isinstance(loader_id, str) or not loader_id:
        raise RawCdpError(f"Page.navigate returned no loaderId: {response}")
    return _NavigationIdentity(
        session_id=session_id,
        frame_id=frame_id,
        loader_id=loader_id,
        expected_url=expected_url,
    )


def _navigation_commit_url(
    messages: list[dict[str, Any]], identity: _NavigationIdentity
) -> str | None:
    for message in messages:
        if message.get("sessionId") != identity.session_id:
            continue
        if message.get("method") != "Page.frameNavigated":
            continue
        frame = (message.get("params") or {}).get("frame") or {}
        if (
            frame.get("id") == identity.frame_id
            and frame.get("loaderId") == identity.loader_id
            and isinstance(frame.get("url"), str)
        ):
            return frame["url"]
    return None


def _has_navigation_evidence(
    messages: list[dict[str, Any]], identity: _NavigationIdentity
) -> bool:
    committed_url = _navigation_commit_url(messages, identity)
    return committed_url is not None and (
        _normalized_navigation_url(committed_url)
        == _normalized_navigation_url(identity.expected_url)
    )


def _navigation_commit_error(
    identity: _NavigationIdentity, committed_url: str | None
) -> str | None:
    if committed_url is None or (
        _normalized_navigation_url(committed_url)
        == _normalized_navigation_url(identity.expected_url)
    ):
        return None
    return (
        f"Page.navigate committed loader {identity.loader_id} to unexpected URL "
        f"`{committed_url}`; expected `{identity.expected_url}`"
    )


async def _run_one_case(
    *,
    client: RawCdpClient,
    session_id: str,
    case_path: str,
    url: str,
    timeout_seconds: float,
) -> CaseResult:
    started = time.perf_counter()
    deadline = started + timeout_seconds
    seen_messages: list[dict[str, Any]] = []
    try:
        nav_id = await client.send("Page.navigate", {"url": url}, session_id=session_id)
        nav_response, nav_seen = await client.recv_until_id(
            nav_id,
            timeout=max(0.01, deadline - time.perf_counter()),
        )
        seen_messages.extend(nav_seen)
        identity = _navigation_identity(
            nav_response,
            session_id=session_id,
            expected_url=url,
        )
    except (RawCdpError, asyncio.TimeoutError) as error:
        raise _PageSessionUnusable(
            CaseResult(
                case_path=case_path,
                url=url,
                status="error",
                duration_ms=(time.perf_counter() - started) * 1000.0,
                error=f"navigate failed: {_exception_text(error)}",
            )
        ) from error

    payload: Any = None
    bridge_installed = False
    committed_url = _navigation_commit_url(seen_messages, identity)
    commit_error = _navigation_commit_error(identity, committed_url)
    if commit_error is not None:
        return CaseResult(
            case_path=case_path,
            url=url,
            status="error",
            duration_ms=(time.perf_counter() - started) * 1000.0,
            error=commit_error,
        )
    navigation_observed = committed_url is not None
    expected_case_path = _normalized_case_path(case_path)
    last_payload_case_path: str | None = None
    while time.perf_counter() < deadline:
        try:
            eval_id = await client.send(
                "Runtime.evaluate",
                {
                    "expression": _HARNESS_PROBE_EXPRESSION,
                    "returnByValue": True,
                    "awaitPromise": False,
                },
                session_id=session_id,
            )
            response, eval_seen = await client.recv_until_id(
                eval_id,
                # A synchronous test can keep the renderer busy for longer than
                # an ordinary CDP command timeout.  Once the probe is in flight,
                # let it use the case's remaining budget instead of turning a
                # progressing test into an infrastructure error after 5 seconds.
                timeout=max(0.01, deadline - time.perf_counter()),
            )
            seen_messages.extend(eval_seen)
        except (RawCdpError, asyncio.TimeoutError) as error:
            raise _PageSessionUnusable(
                CaseResult(
                    case_path=case_path,
                    url=url,
                    status="error",
                    duration_ms=(time.perf_counter() - started) * 1000.0,
                    error=f"evaluate failed: {_exception_text(error)}",
                )
            ) from error
        if committed_url is None:
            committed_url = _navigation_commit_url(eval_seen, identity)
            commit_error = _navigation_commit_error(identity, committed_url)
            if commit_error is not None:
                return CaseResult(
                    case_path=case_path,
                    url=url,
                    status="error",
                    duration_ms=(time.perf_counter() - started) * 1000.0,
                    error=commit_error,
                )
            navigation_observed = committed_url is not None
        result = ((response.get("result") or {}).get("result") or {})
        value = result.get("value")
        if isinstance(value, dict):
            if navigation_observed:
                bridge_installed = bridge_installed or value.get("bridgeInstalled") is True
                candidate = value.get("payload")
                if isinstance(candidate, dict):
                    # A bridge payload is stronger evidence than the optional
                    # trace array: some pages replace or mutate the trace after
                    # the bridge has already published its terminal snapshot.
                    bridge_installed = True
                    candidate_case_path = candidate.get("case_path")
                    if isinstance(candidate_case_path, str):
                        last_payload_case_path = candidate_case_path
                    if (
                        isinstance(candidate_case_path, str)
                        and _normalized_case_path(candidate_case_path)
                        == expected_case_path
                    ):
                        source = (
                            candidate.get("source")
                            if isinstance(candidate.get("source"), str)
                            else None
                        )
                        payload = candidate
                        if source in FINAL_PAYLOAD_SOURCES:
                            break
        await asyncio.sleep(0.05)

    duration_ms = (time.perf_counter() - started) * 1000.0
    console_errors, js_exceptions = _count_traces(seen_messages)
    error: str | None = None
    if not navigation_observed:
        error = (
            "Page.navigate returned loader/frame identity but no matching "
            "navigation event was observed"
        )
    elif last_payload_case_path is not None and (
        _normalized_case_path(last_payload_case_path) != expected_case_path
    ):
        error = (
            f"ignored harness payload for `{last_payload_case_path}` while "
            f"waiting for `{case_path}`"
        )
    return classify_payload(
        payload=payload if isinstance(payload, dict) else None,
        case_path=case_path,
        url=url,
        duration_ms=duration_ms,
        bridge_installed=bridge_installed,
        console_errors=console_errors,
        js_exceptions=js_exceptions,
        error=error,
    )


_REFTEST_LOCATION_EXPRESSION = """
(function(expectedUrl) {
  return {
    href: location.href,
    expectedHref: new URL(expectedUrl, location.href).href,
    readyState: document.readyState
  };
})(%s)
"""

_REFTEST_READY_EXPRESSION = r"""
(async function() {
  if (document.readyState !== 'complete') {
    await new Promise(function(resolve) {
      addEventListener('load', resolve, {once: true});
    });
  }
  if (document.fonts && document.fonts.ready) {
    try { await document.fonts.ready; } catch (_) {}
  }
  var waitForPaints = function() {
    return new Promise(function(resolve) {
      var settled = false;
      var timer = null;
      var done = function() {
        if (!settled) {
          settled = true;
          if (timer !== null) clearTimeout(timer);
          resolve();
        }
      };
      timer = setTimeout(done, 100);
      if (typeof requestAnimationFrame === 'function') {
        requestAnimationFrame(function() { requestAnimationFrame(done); });
      }
    });
  };
  await waitForPaints();
  var root = document.documentElement;
  if (root && root.classList.contains('reftest-wait')) {
    await new Promise(function(resolve) {
      var finish = function() {
        if (!root.classList.contains('reftest-wait')) {
          if (observer) observer.disconnect();
          if (timer) clearInterval(timer);
          resolve();
        }
      };
      var observer = null;
      var timer = null;
      if (typeof MutationObserver === 'function') {
        observer = new MutationObserver(finish);
        observer.observe(root, {attributes: true, attributeFilter: ['class']});
      } else {
        timer = setInterval(finish, 10);
      }
      root.dispatchEvent(new Event('TestRendered', {bubbles: true}));
      finish();
    });
    await waitForPaints();
  }
  return {
    href: location.href,
    readyState: document.readyState,
    width: window.innerWidth,
    height: window.innerHeight,
    deviceScaleFactor: window.devicePixelRatio
  };
})()
"""


@dataclass(frozen=True)
class CapturedScreenshot:
    png: bytes
    sha256: str
    width: int
    height: int


@dataclass
class _ReftestEvidence:
    reference: ReftestReferenceRun
    screenshot: CapturedScreenshot
    diff_image: Image.Image


def _remote_object_value(response: dict[str, Any]) -> Any:
    command_result = response.get("result") or {}
    if command_result.get("exceptionDetails"):
        return None
    remote_object = command_result.get("result") or {}
    return remote_object.get("value")


async def _wait_for_reftest_ready(
    client: RawCdpClient,
    session_id: str,
    url: str,
    timeout_seconds: float,
    identity: _NavigationIdentity,
    navigation_observed: bool,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    deadline = time.perf_counter() + timeout_seconds
    seen_messages: list[dict[str, Any]] = []
    location_expression = _REFTEST_LOCATION_EXPRESSION % json.dumps(url)

    while time.perf_counter() < deadline:
        remaining = deadline - time.perf_counter()
        try:
            eval_id = await client.send(
                "Runtime.evaluate",
                {"expression": location_expression, "returnByValue": True},
                session_id=session_id,
            )
            response, messages = await client.recv_until_id(
                eval_id,
                timeout=max(0.01, min(5.0, remaining)),
            )
            seen_messages.extend(messages)
            navigation_observed = navigation_observed or _has_navigation_evidence(
                messages, identity
            )
        except (RawCdpError, asyncio.TimeoutError):
            await asyncio.sleep(0.025)
            continue
        value = _remote_object_value(response)
        if (
            isinstance(value, dict)
            and value.get("href") == value.get("expectedHref")
            and navigation_observed
        ):
            break
        await asyncio.sleep(0.025)
    else:
        raise asyncio.TimeoutError(f"navigation did not commit to {url}")

    remaining = deadline - time.perf_counter()
    if remaining <= 0:
        raise asyncio.TimeoutError(f"reftest readiness timed out for {url}")
    eval_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": _REFTEST_READY_EXPRESSION,
            "returnByValue": True,
            "awaitPromise": True,
        },
        session_id=session_id,
    )
    response, messages = await client.recv_until_id(eval_id, timeout=remaining)
    seen_messages.extend(messages)
    navigation_observed = navigation_observed or _has_navigation_evidence(
        messages, identity
    )
    value = _remote_object_value(response)
    if not isinstance(value, dict):
        raise RuntimeError(f"reftest readiness script failed for {url}: {response}")
    if not navigation_observed:
        raise RuntimeError(
            f"reftest observed no navigation event for frame={identity.frame_id} "
            f"loader={identity.loader_id}"
        )
    href = value.get("href")
    if not isinstance(href, str) or (
        _normalized_navigation_url(href) != _normalized_navigation_url(url)
    ):
        raise RuntimeError(
            f"reftest committed to unexpected URL {href!r}; expected {url!r}"
        )
    return value, seen_messages


async def _navigate_and_capture_reftest(
    *,
    client: RawCdpClient,
    session_id: str,
    url: str,
    timeout_seconds: float,
    viewport: Viewport,
) -> tuple[CapturedScreenshot, list[dict[str, Any]]]:
    seen_messages: list[dict[str, Any]] = []
    nav_id = await client.send("Page.navigate", {"url": url}, session_id=session_id)
    nav_response, nav_seen = await client.recv_until_id(nav_id, timeout=timeout_seconds)
    seen_messages.extend(nav_seen)
    identity = _navigation_identity(
        nav_response,
        session_id=session_id,
        expected_url=url,
    )

    ready, ready_seen = await _wait_for_reftest_ready(
        client,
        session_id,
        url,
        timeout_seconds,
        identity,
        _has_navigation_evidence(nav_seen, identity),
    )
    seen_messages.extend(ready_seen)
    actual_viewport = (ready.get("width"), ready.get("height"))
    expected_viewport = (viewport.width, viewport.height)
    if actual_viewport != expected_viewport:
        raise RuntimeError(
            f"viewport mismatch for {url}: expected {expected_viewport}, got {actual_viewport}"
        )
    actual_scale = ready.get("deviceScaleFactor")
    if not isinstance(actual_scale, (int, float)) or abs(
        float(actual_scale) - viewport.device_scale_factor
    ) > 1e-6:
        raise RuntimeError(
            f"device scale mismatch for {url}: expected "
            f"{viewport.device_scale_factor}, got {actual_scale}"
        )

    capture_id = await client.send(
        "Page.captureScreenshot",
        {
            "format": "png",
            "quality": 100,
            "fromSurface": True,
            "captureBeyondViewport": False,
        },
        session_id=session_id,
    )
    capture_response, capture_seen = await client.recv_until_id(
        capture_id,
        timeout=timeout_seconds,
    )
    seen_messages.extend(capture_seen)
    encoded = (capture_response.get("result") or {}).get("data")
    if not isinstance(encoded, str) or not encoded:
        raise RuntimeError(f"captureScreenshot returned no PNG data for {url}")
    try:
        png = base64.b64decode(encoded, validate=True)
        with Image.open(io.BytesIO(png)) as image:
            image.load()
            width, height = image.size
    except Exception as exc:
        raise RuntimeError(f"captureScreenshot returned invalid PNG data for {url}: {exc}") from exc
    expected_pixels = (
        round(viewport.width * viewport.device_scale_factor),
        round(viewport.height * viewport.device_scale_factor),
    )
    if (width, height) != expected_pixels:
        raise RuntimeError(
            f"screenshot size mismatch for {url}: expected {expected_pixels}, "
            f"got {(width, height)}"
        )
    return (
        CapturedScreenshot(
            png=png,
            sha256=hashlib.sha256(png).hexdigest(),
            width=width,
            height=height,
        ),
        seen_messages,
    )


def _image_difference(
    lhs: CapturedScreenshot,
    rhs: CapturedScreenshot,
) -> tuple[int, int, Image.Image, bool]:
    with Image.open(io.BytesIO(lhs.png)) as lhs_source:
        lhs_image = lhs_source.convert("RGB")
    with Image.open(io.BytesIO(rhs.png)) as rhs_source:
        rhs_image = rhs_source.convert("RGB")

    try:
        same_size = lhs_image.size == rhs_image.size
        if same_size:
            diff_image = ImageChops.difference(lhs_image, rhs_image)
            max_difference, different_pixels = _diff_metrics(diff_image)
            return max_difference, different_pixels, diff_image, True

        width = max(lhs_image.width, rhs_image.width)
        height = max(lhs_image.height, rhs_image.height)
        lhs_canvas = Image.new("RGB", (width, height), (0, 0, 0))
        rhs_canvas = Image.new("RGB", (width, height), (255, 255, 255))
        try:
            lhs_canvas.paste(lhs_image, (0, 0))
            rhs_canvas.paste(rhs_image, (0, 0))
            diff_image = ImageChops.difference(lhs_canvas, rhs_canvas)
            max_difference, different_pixels = _diff_metrics(diff_image)
            return max_difference, different_pixels, diff_image, False
        finally:
            lhs_canvas.close()
            rhs_canvas.close()
    finally:
        lhs_image.close()
        rhs_image.close()


def _diff_metrics(diff_image: Image.Image) -> tuple[int, int]:
    max_difference = max(upper for _lower, upper in diff_image.getextrema())
    channels = diff_image.split()
    mask = channels[0]
    generated_masks: list[Image.Image] = []
    try:
        for channel in channels[1:]:
            next_mask = ImageChops.lighter(mask, channel)
            generated_masks.append(next_mask)
            mask = next_mask
        histogram = mask.histogram()
        different_pixels = sum(histogram[1:])
    finally:
        for image in channels:
            image.close()
        for image in generated_masks:
            image.close()
    return max_difference, different_pixels


def compare_reftest_screenshots(
    lhs: CapturedScreenshot,
    rhs: CapturedScreenshot,
    fuzzy: FuzzyTolerance | None,
) -> tuple[bool, dict[str, Any], Image.Image]:
    """Compare two viewport PNGs using WPT's fuzzy reftest semantics."""

    if lhs.sha256 == rhs.sha256 and (lhs.width, lhs.height) == (rhs.width, rhs.height):
        with Image.open(io.BytesIO(lhs.png)) as image:
            diff_image = Image.new("RGB", image.size, (0, 0, 0))
        max_difference = 0
        different_pixels = 0
        same_size = True
    else:
        max_difference, different_pixels, diff_image, same_size = _image_difference(lhs, rhs)

    if not same_size:
        equal = False
    elif fuzzy is None:
        equal = max_difference == 0 and different_pixels == 0
    else:
        allowed_difference = fuzzy.max_difference
        allowed_pixels = fuzzy.total_pixels
        equal = (
            (different_pixels == 0 and allowed_pixels[0] == 0)
            or (max_difference == 0 and allowed_difference[0] == 0)
            or (
                allowed_difference[0] <= max_difference <= allowed_difference[1]
                and allowed_pixels[0] <= different_pixels <= allowed_pixels[1]
            )
        )
    metrics: dict[str, Any] = {
        "equal": equal,
        "same_size": same_size,
        "max_difference": max_difference,
        "different_pixels": different_pixels,
        "test_sha256": lhs.sha256,
        "reference_sha256": rhs.sha256,
        "test_size": [lhs.width, lhs.height],
        "reference_size": [rhs.width, rhs.height],
        "fuzzy": fuzzy.to_dict() if fuzzy is not None else None,
    }
    return equal, metrics, diff_image


def reftest_relation_passes(relation: str, *, equal: bool) -> bool:
    if relation == "==":
        return equal
    if relation == "!=":
        return not equal
    raise ValueError(f"unsupported reftest relation: {relation}")


def reftest_comparisons_pass(comparisons: list[dict[str, Any]]) -> bool:
    """Apply WPT's alternate-match and required-mismatch relationship rules."""

    matching = [item for item in comparisons if item["relation"] == "=="]
    mismatching = [item for item in comparisons if item["relation"] == "!="]
    return (
        (not matching or any(bool(item["passed"]) for item in matching))
        and all(bool(item["passed"]) for item in mismatching)
    )


def _artifact_case_directory(output_dir: Path, engine: str, case_path: str) -> Path:
    slug = re.sub(r"[^A-Za-z0-9._-]+", "-", case_path).strip("-.")
    if not slug:
        slug = "reftest"
    slug = slug[:120]
    digest = hashlib.sha256(case_path.encode("utf-8")).hexdigest()[:12]
    return output_dir / "artifacts" / engine / f"{slug}-{digest}"


def _write_reftest_failure_artifacts(
    *,
    output_dir: Path,
    engine: str,
    case_path: str,
    test_screenshot: CapturedScreenshot,
    evidence: list[_ReftestEvidence],
) -> dict[str, Any]:
    case_dir = _artifact_case_directory(output_dir, engine, case_path)
    case_dir.mkdir(parents=True, exist_ok=True)
    test_path = case_dir / "test.png"
    test_path.write_bytes(test_screenshot.png)
    references: list[dict[str, Any]] = []
    for index, item in enumerate(evidence, start=1):
        reference_path = case_dir / f"reference-{index:02d}.png"
        diff_path = case_dir / f"diff-{index:02d}.png"
        reference_path.write_bytes(item.screenshot.png)
        item.diff_image.save(diff_path, format="PNG")
        references.append(
            {
                "reference_path": item.reference.reference_path,
                "relation": item.reference.relation,
                "reference": reference_path.relative_to(output_dir).as_posix(),
                "diff": diff_path.relative_to(output_dir).as_posix(),
            }
        )
    return {
        "directory": case_dir.relative_to(output_dir).as_posix(),
        "test": test_path.relative_to(output_dir).as_posix(),
        "references": references,
    }


async def _run_one_reftest(
    *,
    client: RawCdpClient,
    session_id: str,
    case: ReftestRun,
    viewport: Viewport,
    engine: str,
    artifact_output_dir: Path | None,
    reference_cache: dict[str, CapturedScreenshot],
) -> CaseResult:
    started = time.perf_counter()
    seen_messages: list[dict[str, Any]] = []
    test_screenshot: CapturedScreenshot | None = None
    evidence: list[_ReftestEvidence] = []
    try:
        test_screenshot, trace = await _navigate_and_capture_reftest(
            client=client,
            session_id=session_id,
            url=case.url,
            timeout_seconds=case.timeout_seconds,
            viewport=viewport,
        )
        seen_messages.extend(trace)
        comparisons: list[dict[str, Any]] = []
        for reference in case.references:
            reference_screenshot = reference_cache.get(reference.url)
            if reference_screenshot is None:
                reference_screenshot, trace = await _navigate_and_capture_reftest(
                    client=client,
                    session_id=session_id,
                    url=reference.url,
                    timeout_seconds=case.timeout_seconds,
                    viewport=viewport,
                )
                seen_messages.extend(trace)
                reference_cache[reference.url] = reference_screenshot
            equal, metrics, diff_image = compare_reftest_screenshots(
                test_screenshot,
                reference_screenshot,
                reference.fuzzy,
            )
            relation_passed = reftest_relation_passes(reference.relation, equal=equal)
            comparison = {
                "reference_path": reference.reference_path,
                "reference_url": reference.url,
                "relation": reference.relation,
                "passed": relation_passed,
                **metrics,
            }
            comparisons.append(comparison)
            evidence.append(
                _ReftestEvidence(
                    reference=reference,
                    screenshot=reference_screenshot,
                    diff_image=diff_image,
                )
            )

        passed = reftest_comparisons_pass(comparisons)
        artifacts: dict[str, Any] = {}
        if not passed and artifact_output_dir is not None:
            artifacts = _write_reftest_failure_artifacts(
                output_dir=artifact_output_dir,
                engine=engine,
                case_path=case.case_path,
                test_screenshot=test_screenshot,
                evidence=evidence,
            )
        failed_comparisons = [item for item in comparisons if not item["passed"]]
        failures = [
            {
                "name": f"{item['relation']} {item['reference_path']}",
                "status": TEST_STATUS_FAIL,
                "status_name": TEST_STATUS_NAMES[TEST_STATUS_FAIL],
                "message": (
                    f"maxDifference={item['max_difference']}, "
                    f"differentPixels={item['different_pixels']}"
                ),
            }
            for item in failed_comparisons
        ]
        console_errors, js_exceptions = _count_traces(seen_messages)
        return CaseResult(
            case_path=case.case_path,
            url=case.url,
            status="pass" if passed else "fail",
            duration_ms=(time.perf_counter() - started) * 1000.0,
            subtests_total=len(comparisons),
            subtests_pass=sum(1 for item in comparisons if item["passed"]),
            subtests_fail=len(failed_comparisons),
            console_errors=console_errors,
            js_exceptions=js_exceptions,
            error=None if passed else "reftest reference relations did not match",
            failures=failures[:MAX_RECORDED_FAILURES],
            failure_names=[item["name"] for item in failures],
            test_type="reftest",
            reftest_comparisons=comparisons,
            artifacts=artifacts,
        )
    except asyncio.TimeoutError as error:
        console_errors, js_exceptions = _count_traces(seen_messages)
        return CaseResult(
            case_path=case.case_path,
            url=case.url,
            status="timeout",
            duration_ms=(time.perf_counter() - started) * 1000.0,
            console_errors=console_errors,
            js_exceptions=js_exceptions,
            error=f"reftest timed out: {error}",
            test_type="reftest",
        )
    except (RawCdpError, RuntimeError, OSError) as error:
        console_errors, js_exceptions = _count_traces(seen_messages)
        return CaseResult(
            case_path=case.case_path,
            url=case.url,
            status="error",
            duration_ms=(time.perf_counter() - started) * 1000.0,
            console_errors=console_errors,
            js_exceptions=js_exceptions,
            error=f"reftest runner failed: {error}",
            test_type="reftest",
        )
    finally:
        for item in evidence:
            item.diff_image.close()


def classify_payload(
    *,
    payload: dict | None,
    case_path: str,
    url: str,
    duration_ms: float | None,
    bridge_installed: bool,
    console_errors: int = 0,
    js_exceptions: int = 0,
    error: str | None = None,
) -> CaseResult:
    """Map a bridge payload (or its absence) to a CaseResult.

    Used by both the CDP runner and the CLI HTTP-callback runner so the
    pass/fail/timeout/harness-stalled classification stays uniform.
    """

    if payload is None or not isinstance(payload, dict):
        return CaseResult(
            case_path=case_path,
            url=url,
            status="harness-stalled" if bridge_installed else "timeout",
            duration_ms=duration_ms,
            console_errors=console_errors,
            js_exceptions=js_exceptions,
            error=error or (
                "bridge installed but testharness produced no result/completion callbacks"
                if bridge_installed
                else "testharness did not complete within timeout"
            ),
        )

    payload_source = payload.get("source")
    harness = payload.get("harness")
    tests = payload.get("tests")
    harness_status = harness.get("status") if isinstance(harness, dict) else None
    harness_message = harness.get("message") if isinstance(harness, dict) else None

    counts = {"pass": 0, "fail": 0, "timeout": 0, "notrun": 0, "other": 0}
    if isinstance(tests, list):
        for entry in tests:
            if not isinstance(entry, dict):
                counts["other"] += 1
                continue
            status = entry.get("status")
            if status == TEST_STATUS_PASS:
                counts["pass"] += 1
            elif status == TEST_STATUS_FAIL or status == TEST_STATUS_PRECONDITION_FAILED:
                counts["fail"] += 1
            elif status == TEST_STATUS_TIMEOUT:
                counts["timeout"] += 1
            elif status == TEST_STATUS_NOTRUN:
                counts["notrun"] += 1
            else:
                counts["other"] += 1

    total = sum(counts.values())
    overall = "pass"
    has_observed_failure = bool(counts["fail"] or counts["timeout"])
    if harness_status == HARNESS_STATUS_TIMEOUT:
        overall = "timeout"
        has_observed_failure = True
    elif harness_status == HARNESS_STATUS_ERROR:
        overall = "fail"
        has_observed_failure = True
    elif harness_status == HARNESS_STATUS_PRECONDITION_FAILED:
        overall = "fail"
        has_observed_failure = True
    elif counts["fail"] or counts["timeout"]:
        overall = "fail"
    elif total == 0:
        overall = "fail"

    if payload_source not in FINAL_PAYLOAD_SOURCES and not has_observed_failure:
        overall = "harness-stalled" if bridge_installed else "timeout"
        if error is None:
            source_label = (
                payload_source if isinstance(payload_source, str) else "non-final"
            )
            error = (
                f"testharness produced only {source_label} payload "
                "without final completion"
            )
    elif total == 0 and error is None:
        error = "testharness completed without reporting any subtests"
        if isinstance(harness_message, str) and harness_message:
            error = f"{error}: {harness_message}"

    return CaseResult(
        case_path=case_path,
        url=url,
        status=overall,
        duration_ms=duration_ms,
        harness_status=harness_status if isinstance(harness_status, int) else None,
        harness_message=harness_message if isinstance(harness_message, str) else None,
        subtests_total=total,
        subtests_pass=counts["pass"],
        subtests_fail=counts["fail"],
        subtests_timeout=counts["timeout"],
        subtests_notrun=counts["notrun"],
        console_errors=console_errors,
        js_exceptions=js_exceptions,
        payload_source=payload_source if isinstance(payload_source, str) else None,
        error=error,
        failures=_recorded_failures(tests),
        failure_names=_failure_names(tests),
    )


async def _run_async(
    *,
    driver: EngineDriver,
    binary_override: str | None,
    cases: list[CaseRun],
    case_timeout_seconds: float,
    launch_timeout_seconds: float,
    viewport: Viewport | None,
    artifact_output_dir: Path | None,
) -> EngineRunResult:
    handle: EngineDriverHandle | None = None
    result = EngineRunResult(
        engine=driver.name,
        binary=None,
        binary_sha256=None,
        binary_version=None,
        endpoint="",
        ready_ms=None,
    )
    try:
        handle = driver.launch(binary_override=binary_override, ready_timeout_seconds=launch_timeout_seconds)
    except Exception as error:
        result.setup_error = f"launch failed: {error}"
        return result
    result.binary = str(handle.binary) if handle.binary else None
    result.binary_sha256 = handle.binary_sha256
    result.binary_version = handle.binary_version
    result.endpoint = handle.endpoint
    result.ready_ms = handle.ready_ms

    try:
        client = await connect_raw_cdp(handle.endpoint)
    except Exception as error:
        result.setup_error = f"cdp connect failed: {error}"
        result.shutdown_info = driver.shutdown(handle)
        return result

    effective_viewport = viewport
    if effective_viewport is None and any(isinstance(case, ReftestRun) for case in cases):
        effective_viewport = LAYOUT_VIEWPORT

    try:
        page = await _attach_page(client, viewport=effective_viewport)
    except Exception as error:
        result.setup_error = f"attach failed: {error}"
        try:
            await client.websocket.close()
        except Exception:
            pass
        result.shutdown_info = driver.shutdown(handle)
        return result

    relaunch_count = 0
    max_relaunches = 10
    consecutive_relaunch_failures = 0
    reference_cache: dict[str, CapturedScreenshot] = {}

    try:
        for case_index, case in enumerate(cases):
            case_path, url, timeout_seconds = _case_parts(case, case_timeout_seconds)
            # Engine died between cases?
            if handle.process.poll() is not None:
                exit_code = handle.process.returncode
                result.cases.append(
                    CaseResult(
                        case_path=case_path,
                        url=url,
                        status="crash",
                        duration_ms=None,
                        error=f"engine process exited with code {exit_code} (pre-case)",
                        test_type=_case_test_type(case),
                    )
                )
                relaunched = await _try_relaunch(
                    driver=driver,
                    binary_override=binary_override,
                    launch_timeout_seconds=launch_timeout_seconds,
                    viewport=effective_viewport,
                )
                if relaunched is None:
                    consecutive_relaunch_failures += 1
                    if consecutive_relaunch_failures >= 3:
                        for remaining in cases[case_index + 1:]:
                            rp, ru, _ = _case_parts(remaining, case_timeout_seconds)
                            result.cases.append(
                                CaseResult(
                                    case_path=rp, url=ru, status="crash",
                                    duration_ms=None,
                                    error="engine relaunch failed 3x; aborting",
                                    test_type=_case_test_type(remaining),
                                )
                            )
                        break
                    continue
                consecutive_relaunch_failures = 0
                relaunch_count += 1
                # Swap in a fresh engine and the isolated page for its next case.
                try:
                    await client.websocket.close()
                except Exception:
                    pass
                driver.shutdown(handle)  # ensure old handle fully reaped
                handle, client, page = relaunched
                reference_cache.clear()
                continue

            try:
                if isinstance(case, ReftestRun):
                    if effective_viewport is None:
                        raise RuntimeError("reftest requires a fixed viewport")
                    case_result = await _run_one_reftest(
                        client=client,
                        session_id=page.session_id,
                        case=case,
                        viewport=effective_viewport,
                        engine=driver.name,
                        artifact_output_dir=artifact_output_dir,
                        reference_cache=reference_cache,
                    )
                else:
                    case_result = await _run_one_case(
                        client=client,
                        session_id=page.session_id,
                        case_path=case_path,
                        url=url,
                        timeout_seconds=timeout_seconds,
                    )
            except Exception as error:
                # CDP / websocket / asyncio explosion mid-case.
                if isinstance(error, _PageSessionUnusable):
                    case_result = error.case_result
                else:
                    proc_alive = handle.process.poll() is None
                    case_result = CaseResult(
                        case_path=case_path,
                        url=url,
                        status="crash" if not proc_alive else "error",
                        duration_ms=None,
                        error=f"runner exception: {type(error).__name__}: {error}",
                        test_type=_case_test_type(case),
                    )
                result.cases.append(case_result)
                if relaunch_count >= max_relaunches:
                    for remaining in cases[case_index + 1:]:
                        rp, ru, _ = _case_parts(remaining, case_timeout_seconds)
                        result.cases.append(
                            CaseResult(
                                case_path=rp, url=ru, status="crash",
                                duration_ms=None,
                                error=f"exceeded max relaunches ({max_relaunches})",
                                test_type=_case_test_type(remaining),
                            )
                        )
                    break
                # Tear down current connection + engine, relaunch fresh.
                try:
                    await client.websocket.close()
                except Exception:
                    pass
                driver.shutdown(handle)
                page = None
                relaunched = await _try_relaunch(
                    driver=driver,
                    binary_override=binary_override,
                    launch_timeout_seconds=launch_timeout_seconds,
                    viewport=effective_viewport,
                )
                if relaunched is None:
                    consecutive_relaunch_failures += 1
                    if consecutive_relaunch_failures >= 3:
                        for remaining in cases[case_index + 1:]:
                            rp, ru, _ = _case_parts(remaining, case_timeout_seconds)
                            result.cases.append(
                                CaseResult(
                                    case_path=rp, url=ru, status="crash",
                                    duration_ms=None,
                                    error="engine relaunch failed 3x after crash; aborting",
                                    test_type=_case_test_type(remaining),
                                )
                            )
                        break
                    # client/handle are dead; fabricate placeholders that will fail
                    # the poll() check next iteration -> retry relaunch path.
                    handle, client, page = await _wait_then_retry_launch(
                        driver,
                        binary_override,
                        launch_timeout_seconds,
                        result,
                        cases,
                        case_index,
                        case_timeout_seconds,
                        effective_viewport,
                    )
                    if handle is None:
                        break
                    reference_cache.clear()
                else:
                    consecutive_relaunch_failures = 0
                relaunch_count += 1
                if relaunched is not None:
                    handle, client, page = relaunched
                    reference_cache.clear()
                continue

            consecutive_relaunch_failures = 0
            result.cases.append(case_result)
            if case_index + 1 < len(cases):
                try:
                    await _close_page(client, page)
                    page = await _attach_page(client, viewport=effective_viewport)
                except Exception:
                    # The completed case keeps its result, but no later case may
                    # inherit an incompletely disposed context or target set.
                    try:
                        await client.websocket.close()
                    except Exception:
                        pass
                    driver.shutdown(handle)
                    page = None
                    relaunched = await _try_relaunch(
                        driver=driver,
                        binary_override=binary_override,
                        launch_timeout_seconds=launch_timeout_seconds,
                        viewport=effective_viewport,
                    )
                    relaunch_count += 1
                    if relaunched is None:
                        for remaining in cases[case_index + 1:]:
                            rp, ru, _ = _case_parts(
                                remaining, case_timeout_seconds
                            )
                            result.cases.append(
                                CaseResult(
                                    case_path=rp,
                                    url=ru,
                                    status="crash",
                                    duration_ms=None,
                                    error=(
                                        "fresh page setup failed after case cleanup; "
                                        "engine relaunch also failed"
                                    ),
                                    test_type=_case_test_type(remaining),
                                )
                            )
                        break
                    handle, client, page = relaunched
                    reference_cache.clear()
    finally:
        if page is not None:
            try:
                await _close_page(client, page)
            except Exception:
                pass
        try:
            await client.websocket.close()
        except Exception:
            pass
        try:
            result.shutdown_info = driver.shutdown(handle)
        except Exception:
            pass
    return result


async def _try_relaunch(
    *,
    driver: EngineDriver,
    binary_override: str | None,
    launch_timeout_seconds: float,
    viewport: Viewport | None,
) -> tuple[EngineDriverHandle, RawCdpClient, _AttachedPage] | None:
    """Relaunch engine + reconnect CDP + reattach. Returns None on failure."""

    try:
        new_handle = driver.launch(
            binary_override=binary_override,
            ready_timeout_seconds=launch_timeout_seconds,
        )
    except Exception:
        return None
    try:
        new_client = await connect_raw_cdp(new_handle.endpoint)
    except Exception:
        try:
            driver.shutdown(new_handle)
        except Exception:
            pass
        return None
    try:
        new_page = await _attach_page(new_client, viewport=viewport)
    except Exception:
        try:
            await new_client.websocket.close()
        except Exception:
            pass
        try:
            driver.shutdown(new_handle)
        except Exception:
            pass
        return None
    return new_handle, new_client, new_page


async def _wait_then_retry_launch(
    driver: EngineDriver,
    binary_override: str | None,
    launch_timeout_seconds: float,
    result: EngineRunResult,
    cases: list[CaseRun],
    case_index: int,
    case_timeout_seconds: float,
    viewport: Viewport | None,
) -> tuple[EngineDriverHandle | None, RawCdpClient | None, _AttachedPage | None]:
    await asyncio.sleep(1.0)
    relaunched = await _try_relaunch(
        driver=driver,
        binary_override=binary_override,
        launch_timeout_seconds=launch_timeout_seconds,
        viewport=viewport,
    )
    if relaunched is None:
        for remaining in cases[case_index + 1:]:
            rp, ru, _ = _case_parts(remaining, case_timeout_seconds)
            result.cases.append(
                CaseResult(
                    case_path=rp, url=ru, status="crash",
                    duration_ms=None,
                    error="engine relaunch failed after backoff; aborting",
                    test_type=_case_test_type(remaining),
                )
            )
        return None, None, None
    return relaunched


def _case_parts(case: CaseRun, default_timeout: float) -> tuple[str, str, float]:
    if isinstance(case, ReftestRun):
        return case.case_path, case.url, case.timeout_seconds
    if len(case) == 3:
        return case[0], case[1], case[2]
    return case[0], case[1], default_timeout


def _case_test_type(case: CaseRun) -> str:
    return "reftest" if isinstance(case, ReftestRun) else "testharness"


def run_engine_on_cases(
    *,
    driver: EngineDriver,
    cases: list[CaseRun],
    binary_override: str | None = None,
    case_timeout_seconds: float = 30.0,
    launch_timeout_seconds: float = 30.0,
    viewport: Viewport | None = None,
    artifact_output_dir: Path | None = None,
) -> EngineRunResult:
    """Synchronous wrapper around the async runner.

    ``cases`` is a list of ``(case_path, url)`` or
    ``(case_path, url, timeout_seconds)`` tuples where ``case_path`` is the
    WPT-relative path (used as case identity) and ``url`` is what the engine
    actually navigates to (loopback or external IPv6).
    """

    return asyncio.run(
        _run_async(
            driver=driver,
            binary_override=binary_override,
            cases=cases,
            case_timeout_seconds=case_timeout_seconds,
            launch_timeout_seconds=launch_timeout_seconds,
            viewport=viewport,
            artifact_output_dir=artifact_output_dir,
        )
    )


def case_result_to_dict(case: CaseResult) -> dict[str, Any]:
    return {
        "case_path": case.case_path,
        "url": case.url,
        "test_type": case.test_type,
        "status": case.status,
        "duration_ms": case.duration_ms,
        "harness_status": case.harness_status,
        "harness_status_name": HARNESS_STATUS_NAMES.get(case.harness_status, None) if case.harness_status is not None else None,
        "harness_message": case.harness_message,
        "subtests": {
            "total": case.subtests_total,
            "pass": case.subtests_pass,
            "fail": case.subtests_fail,
            "timeout": case.subtests_timeout,
            "notrun": case.subtests_notrun,
        },
        "console_errors": case.console_errors,
        "js_exceptions": case.js_exceptions,
        "payload_source": case.payload_source,
        "error": case.error,
        "failures": case.failures,
        "failure_names": case.failure_names,
        "reftest_comparisons": case.reftest_comparisons,
        "artifacts": case.artifacts,
    }


def engine_result_to_dict(result: EngineRunResult) -> dict[str, Any]:
    return {
        "engine": result.engine,
        "binary": result.binary,
        "binary_sha256": result.binary_sha256,
        "binary_version": result.binary_version,
        "endpoint": result.endpoint,
        "ready_ms": result.ready_ms,
        "setup_error": result.setup_error,
        "shutdown": result.shutdown_info,
        "cases": [case_result_to_dict(c) for c in result.cases],
    }


def write_engine_result(path, result: EngineRunResult) -> None:
    """Write ``result`` to ``path`` as JSON. Convenience helper for CLI."""

    from pathlib import Path as _Path

    out = _Path(path)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(engine_result_to_dict(result), indent=2, sort_keys=True), encoding="utf-8")
