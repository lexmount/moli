from __future__ import annotations

import argparse
import asyncio
import json
import random
import re
import subprocess
import time
import urllib.parse
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Iterable

from .artifacts import write_json
from .config import REPO_ROOT, chrome_binary, moli_binary
from .raw_cdp import (
    RawCdpCommandError,
    RawCdpError,
    RawCdpTimeoutError,
    RecordedCdpMessage,
    RoutedRawCdpClient,
    connect_routed_raw_cdp,
)
from .sampling import snapshot_resources
from .target_serve import start_target_serve, stop_target_serve
from .top_sites import TOP_SITES_LIST_PATH, parse_top_sites_list


DEFAULT_SEED = 20260809
DEFAULT_COUNT = 50
DEFAULT_PINNED_DOMAINS = ("csdn.net", "zol.com.cn")
DEFAULT_SEED_FILE = TOP_SITES_LIST_PATH
TRACE_METHODS = {
    "Page.frameStartedNavigating",
    "Page.frameStartedLoading",
    "Page.frameNavigated",
    "Page.domContentEventFired",
    "Page.loadEventFired",
    "Page.lifecycleEvent",
    "Page.frameStoppedLoading",
}
NETWORK_EVENT_METHODS = {
    "Network.requestWillBeSent",
    "Network.responseReceived",
    "Network.loadingFinished",
    "Network.loadingFailed",
}
NAVIGATION_RESOURCE_FIELDS = (
    "process_count",
    "pss_bytes",
    "pss_process_count",
    "rss_bytes",
    "rss_process_count",
    "thread_count",
    "fd_count",
)


@dataclass(frozen=True)
class NavigationIdentity:
    frame_id: str
    loader_id: str


@dataclass(frozen=True)
class NavigationProgress:
    kind: str
    record: RecordedCdpMessage
    identity: NavigationIdentity


@dataclass(frozen=True)
class NavigationTimeouts:
    response: float
    dcl: float
    load: float
    postcheck: float


def parse_top_100_domains(markdown: str) -> list[str]:
    in_top_100 = False
    domains: list[str] = []
    for line in markdown.splitlines():
        if line.strip() == "## Top 100":
            in_top_100 = True
            continue
        if in_top_100 and line.startswith("## "):
            break
        if not in_top_100:
            continue
        match = re.match(r"^\s*\d+\.\s+`([^`]+)`", line)
        if match is not None:
            domains.append(match.group(1).strip())
    if not domains:
        raise ValueError("seed document does not contain any domains under `## Top 100`")
    return domains


def normalize_public_url(value: str) -> str:
    value = value.strip()
    if not value:
        raise ValueError("navigation URL cannot be empty")
    if "://" not in value and not value.startswith("data:"):
        value = f"https://{value}"
    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme not in {"http", "https", "data"}:
        raise ValueError(f"unsupported navigation URL scheme: {parsed.scheme}")
    return value


def select_seed_urls(
    domains: Iterable[str],
    *,
    seed: int,
    count: int,
    pinned_domains: Iterable[str] = DEFAULT_PINNED_DOMAINS,
) -> list[str]:
    if count <= 0:
        raise ValueError("navigation count must be positive")
    normalized_domains = list(dict.fromkeys(domain.strip() for domain in domains if domain.strip()))
    pinned = list(dict.fromkeys(domain.strip() for domain in pinned_domains if domain.strip()))
    selected_pinned = pinned[:count]
    remaining = [domain for domain in normalized_domains if domain not in set(selected_pinned)]
    random.Random(seed).shuffle(remaining)
    selected = (selected_pinned + remaining)[:count]
    if len(selected) < count:
        raise ValueError(f"requested {count} URLs but seed set contains only {len(selected)}")
    return [normalize_public_url(domain) for domain in selected]


def make_recovery_url(marker: str) -> str:
    html = (
        "<!doctype html><meta charset=utf-8>"
        f"<title>{marker}</title><body data-recovery-marker=\"{marker}\">{marker}</body>"
    )
    return "data:text/html;charset=utf-8," + urllib.parse.quote(html, safe="")


def _is_exact_lifecycle_event(
    payload: dict[str, Any],
    *,
    session_id: str,
    identity: NavigationIdentity,
    name: str,
) -> bool:
    if payload.get("sessionId") != session_id or payload.get("method") != "Page.lifecycleEvent":
        return False
    params = payload.get("params")
    return (
        isinstance(params, dict)
        and params.get("name") == name
        and params.get("frameId") == identity.frame_id
        and params.get("loaderId") == identity.loader_id
    )


def find_exact_lifecycle_record(
    records: Iterable[RecordedCdpMessage],
    *,
    session_id: str,
    identity: NavigationIdentity,
    name: str,
) -> RecordedCdpMessage | None:
    return next(
        (
            record
            for record in records
            if _is_exact_lifecycle_event(
                record.payload,
                session_id=session_id,
                identity=identity,
                name=name,
            )
        ),
        None,
    )


def _successor_navigation_identity(
    payload: dict[str, Any],
    *,
    session_id: str,
    identity: NavigationIdentity,
) -> NavigationIdentity | None:
    if payload.get("sessionId") != session_id or payload.get("method") != "Page.frameStartedNavigating":
        return None
    params = payload.get("params")
    if not isinstance(params, dict) or params.get("frameId") != identity.frame_id:
        return None
    loader_id = params.get("loaderId")
    if not isinstance(loader_id, str) or not loader_id or loader_id == identity.loader_id:
        return None
    return NavigationIdentity(frame_id=identity.frame_id, loader_id=loader_id)


def find_navigation_progress_record(
    records: Iterable[RecordedCdpMessage],
    *,
    session_id: str,
    identity: NavigationIdentity,
    lifecycle_name: str,
) -> NavigationProgress | None:
    for record in records:
        if _is_exact_lifecycle_event(
            record.payload,
            session_id=session_id,
            identity=identity,
            name=lifecycle_name,
        ):
            return NavigationProgress(
                kind="lifecycle",
                record=record,
                identity=identity,
            )
        successor = _successor_navigation_identity(
            record.payload,
            session_id=session_id,
            identity=identity,
        )
        if successor is not None:
            return NavigationProgress(
                kind="successor",
                record=record,
                identity=successor,
            )
    return None


def navigation_order_violations(
    response: RecordedCdpMessage,
    dcl: RecordedCdpMessage,
    load: RecordedCdpMessage | None,
) -> list[str]:
    violations = []
    if response.sequence >= dcl.sequence:
        violations.append("DOMContentLoaded was observable before Page.navigate response")
    if load is not None:
        if response.sequence >= load.sequence:
            violations.append("load was observable before Page.navigate response")
        if dcl.sequence >= load.sequence:
            violations.append("load was observable before DOMContentLoaded")
    return violations


def _record_elapsed_ms(record: RecordedCdpMessage, started: float) -> float:
    return round((record.received_monotonic - started) * 1000.0, 3)


def _compact_trace_record(record: RecordedCdpMessage, started: float) -> dict[str, Any] | None:
    payload = record.payload
    method = payload.get("method")
    if method not in TRACE_METHODS and "id" not in payload:
        return None
    compact: dict[str, Any] = {
        "sequence": record.sequence,
        "elapsed_ms": _record_elapsed_ms(record, started),
    }
    if isinstance(method, str):
        compact["method"] = method
        params = payload.get("params")
        if isinstance(params, dict):
            if method == "Page.lifecycleEvent":
                compact.update(
                    {
                        "name": params.get("name"),
                        "frame_id": params.get("frameId"),
                        "loader_id": params.get("loaderId"),
                    }
                )
            elif method == "Page.frameNavigated":
                frame = params.get("frame")
                if isinstance(frame, dict):
                    compact.update(
                        {
                            "frame_id": frame.get("id"),
                            "loader_id": frame.get("loaderId"),
                            "url": frame.get("url"),
                        }
                    )
            else:
                for source, destination in (
                    ("frameId", "frame_id"),
                    ("loaderId", "loader_id"),
                    ("url", "url"),
                ):
                    if source in params:
                        compact[destination] = params[source]
    else:
        compact["response_id"] = payload.get("id")
        if "error" in payload:
            compact["error"] = payload["error"]
    return compact


def _compact_trace(
    client: RoutedRawCdpClient,
    *,
    after_sequence: int,
    started: float,
) -> list[dict[str, Any]]:
    return [
        compact
        for record in client.messages_since(after_sequence)
        if (compact := _compact_trace_record(record, started)) is not None
    ]


def summarize_network_activity(
    records: Iterable[RecordedCdpMessage],
    *,
    session_id: str,
    started: float,
) -> dict[str, Any]:
    requests: dict[str, dict[str, Any]] = {}
    event_counts = {method: 0 for method in sorted(NETWORK_EVENT_METHODS)}
    for record in records:
        payload = record.payload
        method = payload.get("method")
        if payload.get("sessionId") != session_id or method not in NETWORK_EVENT_METHODS:
            continue
        event_counts[str(method)] += 1
        params = payload.get("params")
        if not isinstance(params, dict):
            continue
        request_id = params.get("requestId")
        if not isinstance(request_id, str) or not request_id:
            continue
        entry = requests.setdefault(
            request_id,
            {
                "request_id": request_id,
                "url": None,
                "type": None,
                "frame_id": None,
                "loader_id": None,
                "initiator_type": None,
                "started_sequence": None,
                "started_ms": None,
                "redirect_count": 0,
                "response_status": None,
                "response_mime_type": None,
                "terminal": None,
                "terminal_sequence": None,
                "terminal_ms": None,
                "error_text": None,
                "canceled": None,
            },
        )
        if method == "Network.requestWillBeSent":
            if entry["started_sequence"] is not None:
                entry["redirect_count"] += 1
            request = params.get("request")
            initiator = params.get("initiator")
            entry.update(
                {
                    "url": request.get("url") if isinstance(request, dict) else None,
                    "type": params.get("type"),
                    "frame_id": params.get("frameId"),
                    "loader_id": params.get("loaderId"),
                    "initiator_type": (
                        initiator.get("type") if isinstance(initiator, dict) else None
                    ),
                    "started_sequence": record.sequence,
                    "started_ms": _record_elapsed_ms(record, started),
                    "terminal": None,
                    "terminal_sequence": None,
                    "terminal_ms": None,
                    "error_text": None,
                    "canceled": None,
                }
            )
        elif method == "Network.responseReceived":
            response = params.get("response")
            entry["type"] = params.get("type", entry["type"])
            entry["frame_id"] = params.get("frameId", entry["frame_id"])
            entry["loader_id"] = params.get("loaderId", entry["loader_id"])
            if isinstance(response, dict):
                entry["url"] = response.get("url", entry["url"])
                entry["response_status"] = response.get("status")
                entry["response_mime_type"] = response.get("mimeType")
        elif method == "Network.loadingFinished":
            entry.update(
                {
                    "terminal": "finished",
                    "terminal_sequence": record.sequence,
                    "terminal_ms": _record_elapsed_ms(record, started),
                }
            )
        elif method == "Network.loadingFailed":
            entry.update(
                {
                    "type": params.get("type", entry["type"]),
                    "terminal": "failed",
                    "terminal_sequence": record.sequence,
                    "terminal_ms": _record_elapsed_ms(record, started),
                    "error_text": params.get("errorText"),
                    "canceled": params.get("canceled"),
                }
            )

    ordered = sorted(
        requests.values(),
        key=lambda entry: (
            entry["started_sequence"] is None,
            entry["started_sequence"] or entry["terminal_sequence"] or 0,
        ),
    )
    inflight = [entry for entry in ordered if entry["terminal"] is None]
    failed = [entry for entry in ordered if entry["terminal"] == "failed"]
    return {
        "event_counts": event_counts,
        "request_count": len(ordered),
        "finished_count": sum(entry["terminal"] == "finished" for entry in ordered),
        "failed_count": len(failed),
        "inflight_count": len(inflight),
        "requests": ordered,
    }


def network_event_order_violations(
    records: Iterable[RecordedCdpMessage],
    *,
    session_id: str,
) -> list[dict[str, Any]]:
    timelines: dict[str, dict[str, Any]] = {}
    for record in records:
        payload = record.payload
        method = payload.get("method")
        if payload.get("sessionId") != session_id or method not in NETWORK_EVENT_METHODS:
            continue
        params = payload.get("params")
        if not isinstance(params, dict):
            continue
        request_id = params.get("requestId")
        if not isinstance(request_id, str) or not request_id:
            continue
        timeline = timelines.setdefault(
            request_id,
            {
                "request_id": request_id,
                "url": None,
                "type": None,
                "start_sequence": None,
                "response_sequence": None,
                "terminals": [],
            },
        )
        if method == "Network.requestWillBeSent":
            request = params.get("request")
            if isinstance(request, dict):
                timeline["url"] = request.get("url", timeline["url"])
            timeline["type"] = params.get("type", timeline["type"])
            if timeline["start_sequence"] is None:
                timeline["start_sequence"] = record.sequence
        elif method == "Network.responseReceived":
            timeline["type"] = params.get("type", timeline["type"])
            if timeline["response_sequence"] is None:
                timeline["response_sequence"] = record.sequence
        else:
            timeline["terminals"].append(
                {"method": method, "sequence": record.sequence}
            )

    violations: list[dict[str, Any]] = []
    for timeline in timelines.values():
        terminals = timeline.pop("terminals")
        first_start = timeline["start_sequence"]
        first_response = timeline["response_sequence"]
        first_terminal = min(
            terminals,
            key=lambda terminal: terminal["sequence"],
            default=None,
        )

        def report(kind: str, **details: Any) -> None:
            violations.append({**timeline, "kind": kind, **details})

        if first_response is not None and (
            first_start is None or first_response < first_start
        ):
            report("response_before_start", sequence=first_response)
        if first_terminal is not None and (
            first_start is None or first_terminal["sequence"] < first_start
        ):
            report(
                "terminal_before_start",
                sequence=first_terminal["sequence"],
                terminal_method=first_terminal["method"],
            )
        first_successful_terminal = min(
            (
                terminal
                for terminal in terminals
                if terminal["method"] == "Network.loadingFinished"
            ),
            key=lambda terminal: terminal["sequence"],
            default=None,
        )
        if first_successful_terminal is not None and (
            first_response is None
            or first_successful_terminal["sequence"] < first_response
        ):
            report(
                "successful_terminal_before_response",
                sequence=first_successful_terminal["sequence"],
                terminal_method=first_successful_terminal["method"],
            )
        if len(terminals) > 1:
            report(
                "duplicate_terminal",
                terminal_sequences=[terminal["sequence"] for terminal in terminals],
                terminal_methods=[terminal["method"] for terminal in terminals],
            )
    return violations


def _error_detail(error: BaseException) -> dict[str, Any]:
    detail: dict[str, Any] = {
        "type": type(error).__name__,
        "message": str(error),
    }
    if isinstance(error, RawCdpCommandError):
        detail["cdp_error"] = error.error
    if isinstance(error, RawCdpTimeoutError):
        detail["method"] = error.method
        detail["message_id"] = error.message_id
    return detail


def _command_response_record(
    records: Iterable[RecordedCdpMessage], message_id: int
) -> RecordedCdpMessage | None:
    return next((record for record in records if record.payload.get("id") == message_id), None)


async def _wait_for_navigation_progress(
    client: RoutedRawCdpClient,
    *,
    after_sequence: int,
    session_id: str,
    identity: NavigationIdentity,
    lifecycle_name: str,
    timeout: float,
) -> NavigationProgress:
    existing = find_navigation_progress_record(
        client.messages_since(after_sequence),
        session_id=session_id,
        identity=identity,
        lifecycle_name=lifecycle_name,
    )
    if existing is not None:
        return existing

    lifecycle_task = asyncio.create_task(
        client.wait_for_event(
            "Page.lifecycleEvent",
            after_sequence=after_sequence,
            session_id=session_id,
            predicate=lambda payload: _is_exact_lifecycle_event(
                payload,
                session_id=session_id,
                identity=identity,
                name=lifecycle_name,
            ),
            timeout=timeout + 1.0,
        )
    )
    successor_task = asyncio.create_task(
        client.wait_for_event(
            "Page.frameStartedNavigating",
            after_sequence=after_sequence,
            session_id=session_id,
            predicate=lambda payload: _successor_navigation_identity(
                payload,
                session_id=session_id,
                identity=identity,
            )
            is not None,
            timeout=timeout + 1.0,
        )
    )
    tasks = {lifecycle_task, successor_task}
    try:
        done, _ = await asyncio.wait(
            tasks,
            timeout=timeout,
            return_when=asyncio.FIRST_COMPLETED,
        )
        if not done:
            raise RawCdpTimeoutError(
                f"timed out waiting for {lifecycle_name} or a successor navigation",
                method="Page.lifecycleEvent",
                messages=[
                    record.json_value() for record in client.messages_since(after_sequence)
                ],
            )

        recorded = find_navigation_progress_record(
            client.messages_since(after_sequence),
            session_id=session_id,
            identity=identity,
            lifecycle_name=lifecycle_name,
        )
        if recorded is not None:
            return recorded

        progress: list[NavigationProgress] = []
        for task in done:
            record = task.result()
            if task is lifecycle_task:
                progress.append(
                    NavigationProgress(
                        kind="lifecycle",
                        record=record,
                        identity=identity,
                    )
                )
                continue
            successor = _successor_navigation_identity(
                record.payload,
                session_id=session_id,
                identity=identity,
            )
            if successor is not None:
                progress.append(
                    NavigationProgress(
                        kind="successor",
                        record=record,
                        identity=successor,
                    )
                )
        if not progress:
            raise RawCdpError("navigation progress waiter completed without a matching record")
        return min(progress, key=lambda item: item.record.sequence)
    finally:
        for task in tasks:
            if not task.done():
                task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)


def _runtime_value(response: dict[str, Any]) -> Any:
    result = response.get("result")
    if not isinstance(result, dict):
        return None
    remote = result.get("result")
    if not isinstance(remote, dict):
        return None
    return remote.get("value")


async def navigate_once(
    client: RoutedRawCdpClient,
    *,
    session_id: str,
    url: str,
    timeouts: NavigationTimeouts,
    require_load: bool = True,
    network_diagnostics: bool = False,
) -> dict[str, Any]:
    started = time.perf_counter()
    start_sequence = client.current_sequence
    row: dict[str, Any] = {
        "url": url,
        "ok": False,
        "failure_stage": None,
        "error": None,
        "navigate_result": None,
        "identity": None,
        "effective_identity": None,
        "supersessions": [],
        "response_ms": None,
        "dcl_ms": None,
        "load_ms": None,
        "order_violations": [],
        "document": None,
        "dom_root": None,
        "content_kind": None,
        "network_diagnostics": None,
    }

    def capture_network_diagnostics() -> None:
        if network_diagnostics:
            row["network_diagnostics"] = summarize_network_activity(
                client.messages_since(start_sequence),
                session_id=session_id,
                started=started,
            )

    def fail(stage: str, error: BaseException | str) -> dict[str, Any]:
        row["failure_stage"] = stage
        row["error"] = _error_detail(error) if isinstance(error, BaseException) else {"message": error}
        row["trace"] = _compact_trace(
            client,
            after_sequence=start_sequence,
            started=started,
        )
        row["elapsed_ms"] = round((time.perf_counter() - started) * 1000.0, 3)
        capture_network_diagnostics()
        return row

    try:
        navigation = await client.command(
            "Page.navigate",
            {"url": url},
            session_id=session_id,
            timeout=timeouts.response,
        )
    except (RawCdpError, TimeoutError) as error:
        return fail("navigate_response", error)

    response_record = _command_response_record(navigation.messages, navigation.message_id)
    if response_record is None:
        return fail("navigate_response", "Page.navigate response was not retained in the CDP trace")
    row["response_ms"] = _record_elapsed_ms(response_record, started)
    navigate_result = navigation.response.get("result")
    if not isinstance(navigate_result, dict):
        return fail("navigate_response", f"Page.navigate returned no result object: {navigation.response}")
    row["navigate_result"] = navigate_result
    frame_id = navigate_result.get("frameId")
    loader_id = navigate_result.get("loaderId")
    if not isinstance(frame_id, str) or not frame_id or not isinstance(loader_id, str) or not loader_id:
        return fail(
            "navigate_identity",
            f"cross-document Page.navigate returned no exact frameId/loaderId: {navigate_result}",
        )
    identity = NavigationIdentity(frame_id=frame_id, loader_id=loader_id)
    row["identity"] = asdict(identity)
    row["effective_identity"] = asdict(identity)

    effective_identity = identity
    progress_after_sequence = start_sequence
    lifecycle_name = "DOMContentLoaded"
    dcl_record: RecordedCdpMessage | None = None
    load_record: RecordedCdpMessage | None = None
    loop = asyncio.get_running_loop()
    deadline = loop.time() + timeouts.dcl
    load_budget_started = False
    while True:
        remaining = deadline - loop.time()
        failure_stage = "load" if load_budget_started else "dom_content_loaded"
        if remaining <= 0:
            return fail(
                failure_stage,
                f"timed out waiting for {lifecycle_name} or a successor navigation",
            )
        try:
            progress = await _wait_for_navigation_progress(
                client,
                after_sequence=progress_after_sequence,
                session_id=session_id,
                identity=effective_identity,
                lifecycle_name=lifecycle_name,
                timeout=remaining,
            )
        except (RawCdpError, TimeoutError) as error:
            return fail(failure_stage, error)

        progress_after_sequence = progress.record.sequence
        if progress.kind == "successor":
            params = progress.record.payload.get("params")
            row["supersessions"].append(
                {
                    "from_identity": asdict(effective_identity),
                    "to_identity": asdict(progress.identity),
                    "url": params.get("url") if isinstance(params, dict) else None,
                    "during": "load" if load_budget_started else "dom_content_loaded",
                    "sequence": progress.record.sequence,
                    "elapsed_ms": _record_elapsed_ms(progress.record, started),
                }
            )
            effective_identity = progress.identity
            row["effective_identity"] = asdict(effective_identity)
            row["dcl_ms"] = None
            dcl_record = None
            lifecycle_name = "DOMContentLoaded"
            continue

        if lifecycle_name == "DOMContentLoaded":
            dcl_record = progress.record
            row["dcl_ms"] = _record_elapsed_ms(dcl_record, started)
            if not require_load:
                break
            lifecycle_name = "load"
            if not load_budget_started:
                load_budget_started = True
                deadline = loop.time() + timeouts.load
            continue

        load_record = progress.record
        row["load_ms"] = _record_elapsed_ms(load_record, started)
        break

    if dcl_record is None:
        return fail("dom_content_loaded", "navigation completed without DOMContentLoaded")

    order_violations = navigation_order_violations(response_record, dcl_record, load_record)
    row["order_violations"] = order_violations
    if order_violations:
        return fail("event_order", "; ".join(order_violations))

    expression = (
        "(() => ({"
        "url:String(location.href),"
        "title:document.title,"
        "readyState:document.readyState,"
        "bodyTextLength:document.body?document.body.innerText.length:0,"
        "htmlLength:document.documentElement?document.documentElement.outerHTML.length:0"
        "}))()"
    )
    try:
        evaluation = await client.command(
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True},
            session_id=session_id,
            timeout=timeouts.postcheck,
        )
    except (RawCdpError, TimeoutError) as error:
        return fail("runtime_evaluate", error)
    document = _runtime_value(evaluation.response)
    if not isinstance(document, dict):
        return fail("runtime_evaluate", f"Runtime.evaluate returned no document value: {evaluation.response}")
    row["document"] = document
    expected_ready_states = {"complete"} if require_load else {"interactive", "complete"}
    if document.get("readyState") not in expected_ready_states:
        return fail(
            "document_ready_state",
            f"lifecycle completed but readyState was {document.get('readyState')!r}",
        )

    try:
        dom = await client.command(
            "DOM.getDocument",
            {"depth": 1},
            session_id=session_id,
            timeout=timeouts.postcheck,
        )
    except (RawCdpError, TimeoutError) as error:
        return fail("dom_get_document", error)
    dom_result = dom.response.get("result")
    root = dom_result.get("root") if isinstance(dom_result, dict) else None
    node_id = root.get("nodeId") if isinstance(root, dict) else None
    if not isinstance(node_id, int) or node_id <= 0:
        return fail("dom_get_document", f"DOM.getDocument returned no root node: {dom.response}")
    row["dom_root"] = {"node_id": node_id, "backend_node_id": root.get("backendNodeId")}

    error_text = navigate_result.get("errorText") if not row["supersessions"] else None
    final_url = document.get("url")
    row["content_kind"] = (
        "network_error_document"
        if isinstance(error_text, str)
        or (isinstance(final_url, str) and final_url.startswith("chrome-error://"))
        else "document"
    )
    row["ok"] = True
    row["trace"] = _compact_trace(
        client,
        after_sequence=start_sequence,
        started=started,
    )
    row["elapsed_ms"] = round((time.perf_counter() - started) * 1000.0, 3)
    capture_network_diagnostics()
    return row


async def _create_page_session(
    client: RoutedRawCdpClient,
    timeout: float,
    *,
    network_diagnostics: bool,
) -> tuple[str, str]:
    target = await client.command(
        "Target.createTarget",
        {"url": "about:blank"},
        timeout=timeout,
    )
    target_id = target.response.get("result", {}).get("targetId")
    if not isinstance(target_id, str) or not target_id:
        raise RuntimeError(f"Target.createTarget returned no targetId: {target.response}")
    attached = await client.command(
        "Target.attachToTarget",
        {"targetId": target_id, "flatten": True},
        timeout=timeout,
    )
    session_id = attached.response.get("result", {}).get("sessionId")
    if not isinstance(session_id, str) or not session_id:
        raise RuntimeError(f"Target.attachToTarget returned no sessionId: {attached.response}")
    for method, params in (
        ("Page.enable", None),
        ("Runtime.enable", None),
        ("DOM.enable", None),
        ("Page.setLifecycleEventsEnabled", {"enabled": True}),
    ):
        await client.command(method, params, session_id=session_id, timeout=timeout)
    if network_diagnostics:
        await client.command("Network.enable", session_id=session_id, timeout=timeout)
    return target_id, session_id


def _git_output(*args: str) -> str | None:
    result = subprocess.run(
        ["git", *args],
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        check=False,
        text=True,
    )
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def repository_state() -> dict[str, Any]:
    status = _git_output("status", "--porcelain")
    return {
        "commit": _git_output("rev-parse", "HEAD"),
        "dirty": None if status is None else bool(status),
    }


def capture_navigation_resource_sample(
    root_pid: int,
    *,
    index: int,
    url: str | None,
) -> dict[str, Any]:
    sample: dict[str, Any] = {
        "index": index,
        "url": url,
        "captured_at": datetime.now(UTC).isoformat(),
        "error": None,
    }
    try:
        observed = snapshot_resources(root_pid, include_lifetime_cpu=False)
    except Exception as error:
        sample["error"] = f"{type(error).__name__}: {error}"
        return sample
    sample.update({field: observed.get(field) for field in NAVIGATION_RESOURCE_FIELDS})
    return sample


def _average(values: Iterable[float | int]) -> float | None:
    values = list(values)
    return sum(values) / len(values) if values else None


def _linear_slope(points: list[tuple[int, float]]) -> float | None:
    if len(points) < 2:
        return None
    mean_x = sum(point[0] for point in points) / len(points)
    mean_y = sum(point[1] for point in points) / len(points)
    denominator = sum((point[0] - mean_x) ** 2 for point in points)
    if denominator == 0:
        return None
    return sum(
        (point[0] - mean_x) * (point[1] - mean_y) for point in points
    ) / denominator


def _resource_metric_summary(
    samples: list[dict[str, Any]],
    metric: str,
) -> dict[str, Any]:
    initial = next(
        (
            sample.get(metric)
            for sample in samples
            if sample.get("index") == 0
            and isinstance(sample.get(metric), (int, float))
        ),
        None,
    )
    points = [
        (int(sample["index"]), float(sample[metric]))
        for sample in samples
        if isinstance(sample.get("index"), int)
        and sample["index"] > 0
        and isinstance(sample.get(metric), (int, float))
    ]
    values = [value for _, value in points]
    window_size = min(10, len(values))
    first_window = values[:window_size]
    last_window = values[-window_size:] if window_size else []
    first_window_average = _average(first_window)
    last_window_average = _average(last_window)
    warm_start = min(len(points), max(1, len(points) // 10))
    warm_slope = _linear_slope(points[warm_start:])
    return {
        "observed_samples": len(points),
        "initial": initial,
        "first": values[0] if values else None,
        "final": values[-1] if values else None,
        "minimum": min(values) if values else None,
        "peak": max(values) if values else None,
        "window_size": window_size,
        "first_window_average": first_window_average,
        "last_window_average": last_window_average,
        "first_to_last_window_delta": (
            last_window_average - first_window_average
            if first_window_average is not None and last_window_average is not None
            else None
        ),
        "warm_slope_per_navigation": warm_slope,
        "warm_slope_per_100_navigations": (
            warm_slope * 100.0 if warm_slope is not None else None
        ),
    }


def summarize_navigation_resource_samples(
    samples: list[dict[str, Any]],
    periodic_resources: dict[str, Any],
) -> dict[str, Any]:
    navigation_samples = [
        sample
        for sample in samples
        if isinstance(sample.get("index"), int) and sample["index"] > 0
    ]
    quarters = []
    for offset in range(4):
        start = len(navigation_samples) * offset // 4
        end = len(navigation_samples) * (offset + 1) // 4
        quarter_samples = navigation_samples[start:end]

        def quarter_metric(metric: str) -> dict[str, Any]:
            values = [
                float(sample[metric])
                for sample in quarter_samples
                if isinstance(sample.get(metric), (int, float))
            ]
            return {
                "observed_samples": len(values),
                "average": _average(values),
                "final": values[-1] if values else None,
                "peak": max(values) if values else None,
            }

        quarters.append(
            {
                "quarter": offset + 1,
                "start_index": quarter_samples[0]["index"] if quarter_samples else None,
                "end_index": quarter_samples[-1]["index"] if quarter_samples else None,
                "sample_count": len(quarter_samples),
                "rss_bytes": quarter_metric("rss_bytes"),
                "pss_bytes": quarter_metric("pss_bytes"),
                "fd_count": quarter_metric("fd_count"),
            }
        )

    return {
        "sample_count": len(navigation_samples),
        "initial_sample_present": any(sample.get("index") == 0 for sample in samples),
        "sample_errors": sum(bool(sample.get("error")) for sample in samples),
        "rss_bytes": _resource_metric_summary(samples, "rss_bytes"),
        "pss_bytes": _resource_metric_summary(samples, "pss_bytes"),
        "fd_count": _resource_metric_summary(samples, "fd_count"),
        "thread_count": _resource_metric_summary(samples, "thread_count"),
        "process_count": _resource_metric_summary(samples, "process_count"),
        "periodic": {
            "sample_count": periodic_resources.get("sample_count"),
            "peak_rss_bytes": periodic_resources.get("peak_rss_bytes"),
            "peak_pss_bytes": periodic_resources.get("peak_pss_bytes"),
            "peak_fd_count": periodic_resources.get("peak_fd_count"),
            "peak_thread_count": periodic_resources.get("peak_thread_count"),
            "observer_error": periodic_resources.get("observer_error"),
            "late_sample_count": periodic_resources.get("late_sample_count"),
        },
        "quarters": quarters,
    }


def selected_engines(engine: str) -> tuple[str, ...]:
    if engine == "both":
        return ("moli", "chromium")
    if engine in {"moli", "chromium"}:
        return (engine,)
    raise ValueError(f"unsupported engine selection: {engine}")


async def run_engine(
    *,
    target: str,
    binary: Path,
    urls: list[str],
    timeouts: NavigationTimeouts,
    startup_timeout: float,
    recovery_timeout: float,
    network_diagnostics: bool,
    navigation_resource_samples: bool,
    periodic_resource_samples: bool = False,
) -> dict[str, Any]:
    started_at = datetime.now(UTC).isoformat()
    serve = start_target_serve(target, binary, startup_timeout)
    client: RoutedRawCdpClient | None = None
    session_id: str | None = None
    rows: list[dict[str, Any]] = []
    network_order_violations: list[dict[str, Any]] = []
    resource_samples: list[dict[str, Any]] = []
    aborted_after_index: int | None = None
    try:
        client = await connect_routed_raw_cdp(serve.endpoint)
        target_id, session_id = await _create_page_session(
            client,
            startup_timeout,
            network_diagnostics=network_diagnostics,
        )
        if navigation_resource_samples:
            resource_samples.append(
                capture_navigation_resource_sample(
                    serve.process.pid,
                    index=0,
                    url=None,
                )
            )
        for index, url in enumerate(urls, 1):
            row = await navigate_once(
                client,
                session_id=session_id,
                url=url,
                timeouts=timeouts,
                network_diagnostics=network_diagnostics,
            )
            row.update({"index": index, "target_id": target_id})
            if not row["ok"]:
                marker = f"moli-navigate-recovery-{index}-{time.time_ns()}"
                recovery_timeouts = NavigationTimeouts(
                    response=recovery_timeout,
                    dcl=recovery_timeout,
                    load=recovery_timeout,
                    postcheck=recovery_timeout,
                )
                recovery = await navigate_once(
                    client,
                    session_id=session_id,
                    url=make_recovery_url(marker),
                    timeouts=recovery_timeouts,
                    network_diagnostics=network_diagnostics,
                )
                document = recovery.get("document")
                marker_ok = isinstance(document, dict) and document.get("title") == marker
                recovery["marker"] = marker
                recovery["marker_ok"] = marker_ok
                recovery["ok"] = bool(recovery["ok"] and marker_ok)
                row["recovery"] = recovery
                if not recovery["ok"]:
                    aborted_after_index = index
            rows.append(row)
            if navigation_resource_samples:
                resource_samples.append(
                    capture_navigation_resource_sample(
                        serve.process.pid,
                        index=index,
                        url=url,
                    )
                )
            print(
                json.dumps(
                    {
                        "target": target,
                        "index": index,
                        "url": url,
                        "ok": row["ok"],
                        "failure_stage": row["failure_stage"],
                        "response_ms": row["response_ms"],
                        "dcl_ms": row["dcl_ms"],
                        "load_ms": row["load_ms"],
                        "content_kind": row["content_kind"],
                        "supersessions": len(row["supersessions"]),
                        "network_inflight": (
                            row.get("network_diagnostics") or {}
                        ).get("inflight_count"),
                        "recovery_ok": row.get("recovery", {}).get("ok"),
                    },
                    ensure_ascii=False,
                ),
                flush=True,
            )
            if aborted_after_index is not None:
                break
    finally:
        if client is not None:
            if network_diagnostics and session_id is not None:
                network_order_violations = network_event_order_violations(
                    client.messages_since(0),
                    session_id=session_id,
                )
            await client.close()
        serve_result = stop_target_serve(
            serve,
            include_resource_samples=periodic_resource_samples,
        )

    summary = {
        "planned": len(urls),
        "attempted": len(rows),
        "observable_passes": sum(bool(row["ok"]) for row in rows),
        "failures": sum(not row["ok"] for row in rows),
        "document_passes": sum(row.get("content_kind") == "document" for row in rows),
        "network_error_documents": sum(
            row.get("content_kind") == "network_error_document" for row in rows
        ),
        "recovery_attempts": sum("recovery" in row for row in rows),
        "recovery_passes": sum(row.get("recovery", {}).get("ok") is True for row in rows),
        "recovery_failures": sum(row.get("recovery", {}).get("ok") is False for row in rows),
        "order_violations": sum(bool(row.get("order_violations")) for row in rows),
        "network_order_violations": len(network_order_violations),
        "superseded_passes": sum(
            bool(row["ok"] and row.get("supersessions")) for row in rows
        ),
        "aborted_after_index": aborted_after_index,
    }
    return {
        "target": target,
        "binary": str(binary),
        "started_at": started_at,
        "finished_at": datetime.now(UTC).isoformat(),
        "ready_ms": serve.ready_ms,
        "summary": summary,
        "network_order_violations": network_order_violations,
        "navigation_resources": (
            {
                "samples": resource_samples,
                "summary": summarize_navigation_resource_samples(
                    resource_samples,
                    serve_result.get("resources", {}),
                ),
            }
            if navigation_resource_samples
            else None
        ),
        "rows": rows,
        "process": serve_result,
    }


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Fuzz sequential Page.navigate calls on one CDP target and verify recovery.",
    )
    parser.add_argument("--url", action="append", help="Explicit URL; repeat to define exact order.")
    parser.add_argument("--seed-file", type=Path, default=DEFAULT_SEED_FILE)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument("--count", type=int, default=DEFAULT_COUNT)
    parser.add_argument("--rounds", type=int, default=1)
    parser.add_argument("--pinned-domain", action="append")
    parser.add_argument("--moli-bin")
    parser.add_argument("--chrome-bin")
    parser.add_argument(
        "--engine",
        choices=("moli", "chromium", "both"),
        default="moli",
        help="Browser engine to run; `both` runs the same URL order sequentially.",
    )
    parser.add_argument("--full-resources", action="store_true")
    parser.add_argument(
        "--network-diagnostics",
        action="store_true",
        help=(
            "Enable Network events, record compact request terminal-state summaries, "
            "and audit exact request event order; off by default to keep benchmark "
            "overhead comparable."
        ),
    )
    parser.add_argument(
        "--navigation-resource-samples",
        action="store_true",
        help=(
            "Capture a process-tree RSS/PSS/FD checkpoint before the first "
            "navigation and after every navigation, then report windows, "
            "quarters, and a warm memory slope."
        ),
    )
    parser.add_argument(
        "--periodic-resource-samples",
        action="store_true",
        help=(
            "Retain the process-tree RSS/PSS/CPU time series in the output. "
            "The sampler always runs, but raw points are omitted by default "
            "because they can make long-run JSON files large."
        ),
    )
    parser.add_argument("--startup-timeout", type=float, default=20.0)
    parser.add_argument("--response-timeout", type=float, default=15.0)
    parser.add_argument("--dcl-timeout", type=float, default=15.0)
    parser.add_argument("--load-timeout", type=float, default=20.0)
    parser.add_argument("--postcheck-timeout", type=float, default=5.0)
    parser.add_argument("--recovery-timeout", type=float, default=8.0)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    for name in (
        "startup_timeout",
        "response_timeout",
        "dcl_timeout",
        "load_timeout",
        "postcheck_timeout",
        "recovery_timeout",
    ):
        if getattr(args, name) <= 0:
            parser.error(f"--{name.replace('_', '-')} must be positive")
    if args.rounds <= 0:
        parser.error("--rounds must be positive")
    if args.count <= 0:
        parser.error("--count must be positive")
    return args


def _selected_urls(args: argparse.Namespace) -> list[str]:
    if args.url:
        base = [normalize_public_url(url) for url in args.url]
    else:
        domains = [target for _, target in parse_top_sites_list(args.seed_file)]
        base = select_seed_urls(
            domains,
            seed=args.seed,
            count=args.count,
            pinned_domains=args.pinned_domain or DEFAULT_PINNED_DOMAINS,
        )
    return base * args.rounds


async def _run(args: argparse.Namespace) -> tuple[dict[str, Any], int]:
    urls = _selected_urls(args)
    timeouts = NavigationTimeouts(
        response=args.response_timeout,
        dcl=args.dcl_timeout,
        load=args.load_timeout,
        postcheck=args.postcheck_timeout,
    )
    engines: list[tuple[str, Path]] = []
    for engine in selected_engines(args.engine):
        if engine == "moli":
            engines.append(
                (
                    "moli-full-cdp" if args.full_resources else "moli-cdp",
                    moli_binary(args.moli_bin),
                )
            )
            continue
        chrome = chrome_binary(args.chrome_bin)
        if chrome is None:
            raise RuntimeError("Chromium binary not found; pass --chrome-bin or set CHROME_BIN")
        engines.append(("chrome-cdp", chrome))

    results = []
    for target, binary in engines:
        results.append(
            await run_engine(
                target=target,
                binary=binary,
                urls=urls,
                timeouts=timeouts,
                startup_timeout=args.startup_timeout,
                recovery_timeout=args.recovery_timeout,
                network_diagnostics=args.network_diagnostics,
                navigation_resource_samples=args.navigation_resource_samples,
                periodic_resource_samples=args.periodic_resource_samples,
            )
        )
    payload = {
        "schema_version": 5,
        "repository": repository_state(),
        "engine_selection": args.engine,
        "seed": args.seed,
        "rounds": args.rounds,
        "urls": urls,
        "timeouts": asdict(timeouts),
        "network_diagnostics": args.network_diagnostics,
        "navigation_resource_samples": args.navigation_resource_samples,
        "periodic_resource_samples": args.periodic_resource_samples,
        "results": results,
    }
    failed = any(
        result["summary"]["failures"]
        or result["summary"]["network_order_violations"]
        for result in results
    )
    return payload, 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    payload, exit_code = asyncio.run(_run(args))
    output = args.output
    if output is None:
        stamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
        output = REPO_ROOT / "moli-benchmark" / "results" / f"sequential-navigate-{stamp}.json"
    write_json(output, payload)
    print(json.dumps({"output": str(output), "exit_code": exit_code}), flush=True)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
