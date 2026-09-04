from __future__ import annotations

import argparse
import asyncio
import json
import os
import shutil
import sys
import tempfile
import traceback
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Awaitable, Callable, Iterable

from playwright.async_api import async_playwright

from .assertions import record
from .config import clear_current_process_proxy_env
from .fixture import FixtureServer
from .groups.action_window import run_action_window_group
from .groups.agent_browser import run_agent_browser_group
from .groups.agent_episode import run_agent_episode_group
from .groups.browser_semantics import run_browser_semantics_group
from .groups.classic_scrollbar import run_classic_scrollbar_group
from .groups.cdp_use import run_cdp_use_group
from .groups.chrome_remote_interface import run_chrome_remote_interface_group
from .groups.chromium_cdp import run_chromium_cdp_group, run_computed_style_group
from .groups.core import run_core_group
from .groups.dom_input import run_dom_input_group
from .groups.dom_hit_test import run_dom_hit_test_group
from .groups.dom_parser_mutations import run_dom_parser_mutations_group
from .groups.dom_shadow_outer_html import run_dom_shadow_outer_html_group
from .groups.dom_snapshot import run_dom_snapshot_group
from .groups.dom_whitespace import run_dom_whitespace_group
from .groups.document_content import run_document_content_group
from .groups.emulation_storage import run_emulation_storage_group
from .groups.error_document import run_error_document_group
from .groups.fetch_runtime_teardown import run_fetch_runtime_teardown_group
from .groups.inspector_routing import run_inspector_routing_group
from .groups.iframe_input import run_iframe_input_group
from .groups.layout_screenshot import run_layout_screenshot_group
from .groups.multi_client import run_multi_client_group
from .groups.multi_context import run_multi_context_group
from .groups.multi_page import run_multi_page_group
from .groups.network import (
    run_download_group,
    run_network_body_cache_group,
    run_page_network_group,
    run_websocket_group,
)
from .groups.navigation_outcomes import run_navigation_outcomes_group
from .groups.pdf import run_pdf_group
from .groups.playwright_compat import run_playwright_compat_group
from .groups.protocol import run_raw_protocol_group
from .groups.protocol_regressions import (
    run_debugger_breakpoints_group,
    run_file_chooser_group,
    run_runtime_exception_group,
)
from .groups.proxy_auth import run_proxy_auth_group
from .groups.puppeteer import run_puppeteer_group
from .groups.stagehand import run_stagehand_group
from .groups.target_semantics import run_target_semantics_group
from .groups.tracing import run_raw_tracing_group, run_tracing_group
from .groups.url_policy import run_url_policy_group
from .groups.media_error import run_media_error_group
from .groups.workers import run_workers_group
from .groups.xhr_sync_semantics import run_xhr_sync_semantics_group
from .helpers import attach_cdp_event_collector
from .progress import await_with_progress
from .serve import (
    MoliServe,
    start_moli_serve,
    stop_moli_serve,
    wait_for_cdp_server,
    wait_for_moli_endpoint,
)
from .state import SmokeState


clear_current_process_proxy_env()


RawGroupRunner = Callable[[str, str, list[dict[str, Any]]], Awaitable[None]]
ExternalGroupRunner = Callable[[str, str, list[dict[str, Any]]], Awaitable[None]]
PageGroupRunner = Callable[[SmokeState], Awaitable[None]]
BrowserGroupRunner = Callable[[Any, str, list[dict[str, Any]]], Awaitable[None]]


@dataclass(frozen=True)
class SmokeGroup:
    name: str
    description: str
    phase: str
    runner: RawGroupRunner | ExternalGroupRunner | PageGroupRunner | BrowserGroupRunner


async def _await_group(group: SmokeGroup, awaitable: Awaitable[None]) -> None:
    await await_with_progress(
        f"group/{group.phase}/{group.name}",
        awaitable,
    )


RAW_GROUPS: tuple[SmokeGroup, ...] = (
    SmokeGroup(
        "debugger-breakpoints",
        "Raw Debugger breakpoint commands dispatched while the page is normally running.",
        "raw",
        run_debugger_breakpoints_group,
    ),
    SmokeGroup(
        "runtime-exception",
        "Raw asynchronous Runtime.exceptionThrown delivery without a follow-up command.",
        "raw",
        run_runtime_exception_group,
    ),
    SmokeGroup(
        "file-chooser",
        "Raw intercepted file-input activation and Page.fileChooserOpened delivery.",
        "raw",
        run_file_chooser_group,
    ),
    SmokeGroup(
        "inspector-routing",
        "Chromium-calibrated Page/Worker active-JS interrupt, nested Main receiver, per-session FIFO, and non-V8 IO boundaries.",
        "raw",
        run_inspector_routing_group,
    ),
    SmokeGroup(
        "action-window",
        "Moli raw-CDP wheel batching, overflow-container axes, screenshot flush/reset, and exact-Document retirement contracts.",
        "raw",
        run_action_window_group,
    ),
    SmokeGroup(
        "url-policy",
        "Hosted file URL rejection across raw Page.navigate and Runtime fetch/XHR without lifecycle, interception, or transport leakage.",
        "raw",
        run_url_policy_group,
    ),
    SmokeGroup(
        "agent-episode",
        "RL-shaped awaitPromise observe/fill/click navigation and error-Document contract.",
        "raw",
        run_agent_episode_group,
    ),
    SmokeGroup(
        "dom-parser-mutations",
        "Raw Chromium-calibrated parser-tail DOM mutation and DCL binding refresh order.",
        "raw",
        run_dom_parser_mutations_group,
    ),
    SmokeGroup(
        "tracing-wire",
        "Raw Chromium-calibrated Tracing start-ack and end response/data/completion wire order.",
        "raw",
        run_raw_tracing_group,
    ),
    SmokeGroup(
        "dom-hit-test",
        "Raw DOM.getNodeForLocation capability boundary for Moli and Chromium.",
        "raw",
        run_dom_hit_test_group,
    ),
    SmokeGroup(
        "layout-screenshot",
        "Raw viewport PNG, stable Shadow/iframe TreeScopes, generation-gated 1 FPS JPEG screencast, ACK, mutation, and default-Mock boundaries.",
        "raw",
        run_layout_screenshot_group,
    ),
    SmokeGroup(
        "pdf",
        "Raw Page.printToPDF base64, IO stream, pagination, orientation, and validation contracts.",
        "raw",
        run_pdf_group,
    ),
    SmokeGroup(
        "target-semantics",
        "Cross-engine raw CDP TargetHandler, stable Tab/Page ownership, auto-attach, activation, visibility, attachment, and close contracts.",
        "raw",
        run_target_semantics_group,
    ),
    SmokeGroup(
        "multi-client",
        "Concurrent 2/3/7-client browser/page WebSocket routing, isolation, "
        "and per-client ordering.",
        "raw",
        run_multi_client_group,
    ),
    SmokeGroup(
        "protocol",
        "Raw CDP command/reply flows that avoid Playwright helper follow-up commands.",
        "raw",
        run_raw_protocol_group,
    ),
)

PAGE_GROUPS: tuple[SmokeGroup, ...] = (
    SmokeGroup(
        "iframe-input",
        "Cross-engine transformed single and nested iframe hover, click, wheel, and target-Document coordinate routing.",
        "page",
        run_iframe_input_group,
    ),
    SmokeGroup(
        "classic-scrollbar",
        "Cross-engine classic scrollbar layout, viewport overflow policy, paint-ordered controls, and raw CDP input.",
        "page",
        run_classic_scrollbar_group,
    ),
    SmokeGroup(
        "xhr-sync-semantics",
        "Cross-engine Chromium/WPT synchronous XHR events, restrictions, failures, reset, and CDP Network projection.",
        "page",
        run_xhr_sync_semantics_group,
    ),
    SmokeGroup(
        "browser-semantics",
        "Cross-engine target, history, frame, storage, DOM, resource, runtime, and network contracts.",
        "page",
        run_browser_semantics_group,
    ),
    SmokeGroup(
        "core",
        "Navigation, frames, wait-for-function, cookies, redirects, and history.",
        "page",
        run_core_group,
    ),
    SmokeGroup(
        "network",
        "Document/fetch/XHR routing, Network events, parser resources, and response bodies.",
        "page",
        run_page_network_group,
    ),
    SmokeGroup(
        "proxy-auth",
        "Chromium-calibrated HTTP proxy auth and HTTPS CONNECT failure contracts.",
        "page",
        run_proxy_auth_group,
    ),
    SmokeGroup(
        "chromium-cdp",
        "Chromium inspector-protocol derived CDP samples for Page, Runtime, Input, Performance, Profiler, and DOM.",
        "page",
        run_chromium_cdp_group,
    ),
    SmokeGroup(
        "error-document",
        "Chromium-calibrated failed-navigation error Document identity, lifecycle, replacement, recovery, and target isolation.",
        "page",
        run_error_document_group,
    ),
    SmokeGroup(
        "navigation-outcomes",
        "Chromium-calibrated direct Page.navigate downloads, redirects, binary/no-content responses, HTTP errors, and transport failures.",
        "page",
        run_navigation_outcomes_group,
    ),
    SmokeGroup(
        "computed-style",
        "Cross-engine CSSOM and CSS.getComputedStyleForNode breadth, stability, and mutation contracts.",
        "page",
        run_computed_style_group,
    ),
    SmokeGroup(
        "tracing",
        "Chromium-calibrated Tracing ownership, event ordering, and JSON stream contracts.",
        "page",
        run_tracing_group,
    ),
    SmokeGroup(
        "document-content",
        "Page.setDocumentContent replacement identity, parser pause, child-frame, and error-atomicity workflows.",
        "page",
        run_document_content_group,
    ),
    SmokeGroup(
        "dom-snapshot",
        "Chromium-aligned DOM identity and DOMSnapshot behavior across document.open replacement.",
        "page",
        run_dom_snapshot_group,
    ),
    SmokeGroup(
        "dom-whitespace",
        "Chromium-aligned Inspector DOM whitespace projection using an ldm0.top-derived fixture.",
        "page",
        run_dom_whitespace_group,
    ),
    SmokeGroup(
        "dom-shadow-outer-html",
        "Chromium-aligned DOM.getOuterHTML author-shadow inclusion across node references.",
        "page",
        run_dom_shadow_outer_html_group,
    ),
    SmokeGroup(
        "playwright-compat",
        "Playwright upstream derived route and CDPSession compatibility samples.",
        "page",
        run_playwright_compat_group,
    ),
    SmokeGroup(
        "workers",
        "Worker postMessage, worker fetch routing, and worker XHR routing.",
        "page",
        run_workers_group,
    ),
    SmokeGroup(
        "websocket",
        "WebSocket runtime and Network.webSocket* event coverage.",
        "page",
        run_websocket_group,
    ),
    SmokeGroup(
        "dom-input",
        "setContent, file chooser, locator/input, DOM handles, and touch dispatch boundaries.",
        "page",
        run_dom_input_group,
    ),
    SmokeGroup(
        "download",
        "Download event, artifact, and cancellation flows.",
        "page",
        run_download_group,
    ),
    SmokeGroup(
        "network-body-cache",
        "Cross-engine Network.getResponseBody retention and inspector-cache eviction contracts.",
        "page",
        run_network_body_cache_group,
    ),
    SmokeGroup(
        "fetch-runtime-teardown",
        "CDP-driven BrowserContext teardown with an in-flight fetch callback.",
        "page",
        run_fetch_runtime_teardown_group,
    ),
)

BROWSER_GROUPS: tuple[SmokeGroup, ...] = (
    SmokeGroup(
        "media-error",
        "HTMLMediaElement MediaError publication, identity, and reset lifecycle.",
        "browser",
        run_media_error_group,
    ),
    SmokeGroup(
        "emulation-storage",
        "Viewport, storage, profile, and detailed locale/timezone override contracts.",
        "browser",
        run_emulation_storage_group,
    ),
    SmokeGroup(
        "multi-context",
        "Cross-context owner-state routing and target isolation workflows.",
        "browser",
        run_multi_context_group,
    ),
    SmokeGroup(
        "multi-page",
        "Multi-target churn, navigation, interception, popup, session, worker, "
        "screenshot, and teardown regression matrix.",
        "browser",
        run_multi_page_group,
    ),
)

MANAGED_EXTERNAL_GROUPS: tuple[SmokeGroup, ...] = (
    SmokeGroup(
        "puppeteer",
        "Puppeteer over CDP workflows using the smoke project's pinned puppeteer-core module.",
        "external",
        run_puppeteer_group,
    ),
)


OPTIONAL_EXTERNAL_GROUPS: tuple[SmokeGroup, ...] = (
    SmokeGroup(
        "chrome-remote-interface",
        "Optional chrome-remote-interface browser/page session goal-path workflows.",
        "external",
        run_chrome_remote_interface_group,
    ),
    SmokeGroup(
        "cdp-use",
        "Optional cdp-use browser/page session goal-path workflows.",
        "external",
        run_cdp_use_group,
    ),
    SmokeGroup(
        "stagehand",
        "Optional Stagehand deterministic goal-path workflows without LLM operations.",
        "external",
        run_stagehand_group,
    ),
    SmokeGroup(
        "agent-browser",
        "Optional agent-browser CLI goal-path workflows with an isolated daemon session.",
        "external",
        run_agent_browser_group,
    ),
)


DEFAULT_GROUPS: tuple[SmokeGroup, ...] = (
    RAW_GROUPS + PAGE_GROUPS + BROWSER_GROUPS + MANAGED_EXTERNAL_GROUPS
)
ALL_GROUPS: tuple[SmokeGroup, ...] = DEFAULT_GROUPS + OPTIONAL_EXTERNAL_GROUPS
DEFAULT_GROUP_NAMES: tuple[str, ...] = tuple(group.name for group in DEFAULT_GROUPS)
DEFAULT_GROUP_NAME_SET = frozenset(DEFAULT_GROUP_NAMES)
GROUPS_BY_NAME: dict[str, SmokeGroup] = {group.name: group for group in ALL_GROUPS}


@dataclass(frozen=True)
class SmokeSelection:
    groups: tuple[SmokeGroup, ...]

    @property
    def raw_groups(self) -> tuple[SmokeGroup, ...]:
        return tuple(group for group in self.groups if group.phase == "raw")

    @property
    def page_groups(self) -> tuple[SmokeGroup, ...]:
        return tuple(group for group in self.groups if group.phase == "page")

    @property
    def external_groups(self) -> tuple[SmokeGroup, ...]:
        return tuple(group for group in self.groups if group.phase == "external")

    @property
    def browser_groups(self) -> tuple[SmokeGroup, ...]:
        return tuple(group for group in self.groups if group.phase == "browser")

    @property
    def needs_playwright(self) -> bool:
        return bool(self.page_groups or self.browser_groups)


def _split_group_names(raw_names: Iterable[str]) -> list[str]:
    names: list[str] = []
    for raw_name in raw_names:
        names.extend(name.strip() for name in raw_name.split(",") if name.strip())
    return names


def resolve_group_selection(raw_names: Iterable[str] = ()) -> SmokeSelection:
    names = _split_group_names(raw_names)
    if not names:
        env_names = os.environ.get("MOLI_SMOKE_GROUPS", "")
        names = _split_group_names([env_names]) if env_names else list(DEFAULT_GROUP_NAMES)
    unknown = [name for name in names if name not in GROUPS_BY_NAME]
    if unknown:
        available = ", ".join(group.name for group in ALL_GROUPS)
        raise RuntimeError(
            f"unknown smoke group(s): {', '.join(unknown)}; available groups: {available}"
        )
    selected: list[SmokeGroup] = []
    seen: set[str] = set()
    for name in names:
        if name in seen:
            continue
        selected.append(GROUPS_BY_NAME[name])
        seen.add(name)
    return SmokeSelection(tuple(selected))


def group_listing() -> list[dict[str, Any]]:
    return [
        {
            "name": group.name,
            "phase": group.phase,
            "default": group.name in DEFAULT_GROUP_NAME_SET,
            "description": group.description,
        }
        for group in ALL_GROUPS
    ]


async def run_smoke(
    endpoint: str,
    fixture_server: FixtureServer,
    selection: SmokeSelection,
    results: list[dict[str, Any]] | None = None,
) -> list[dict[str, Any]]:
    if not selection.needs_playwright:
        return results if results is not None else []
    if results is None:
        results = []
    temp_dir = Path(tempfile.mkdtemp(prefix="moli-pw-smoke-"))
    playwright = None
    try:
        playwright = await await_with_progress(
            "playwright/start", async_playwright().start()
        )
        browser = await await_with_progress(
            "playwright/connect-over-cdp",
            playwright.chromium.connect_over_cdp(endpoint, timeout=10_000),
        )
        try:
            record(results, "connect_over_cdp", {"browserContexts": len(browser.contexts)})

            context = await await_with_progress(
                "playwright/browser-new-context",
                browser.new_context(accept_downloads=True),
            )
            record(results, "browser_new_context")

            page = await await_with_progress(
                "playwright/context-new-page",
                context.new_page(),
            )
            cdp = await await_with_progress(
                "playwright/context-new-cdp-session",
                context.new_cdp_session(page),
            )
            websocket_events = attach_cdp_event_collector(
                cdp,
                [
                    "Network.webSocketCreated",
                    "Network.webSocketWillSendHandshakeRequest",
                    "Network.webSocketHandshakeResponseReceived",
                    "Network.webSocketFrameSent",
                    "Network.webSocketFrameReceived",
                    "Network.webSocketClosed",
                ],
            )
            subresource_events = attach_cdp_event_collector(
                cdp,
                [
                    "Network.requestWillBeSent",
                    "Network.responseReceived",
                    "Network.dataReceived",
                    "Network.loadingFinished",
                    "Network.loadingFailed",
                ],
            )
            await await_with_progress(
                "playwright/network-enable",
                cdp.send("Network.enable"),
            )

            state = SmokeState(
                endpoint=endpoint,
                browser=browser,
                context=context,
                page=page,
                cdp=cdp,
                fixture=fixture_server.url,
                fixture_server=fixture_server,
                temp_dir=temp_dir,
                results=results,
                subresource_events=subresource_events,
                websocket_events=websocket_events,
            )

            for group in selection.page_groups:
                await _await_group(
                    group,
                    group.runner(state),  # type: ignore[misc]
                )

            await await_with_progress("playwright/context-close", context.close())
            for group in selection.browser_groups:
                await _await_group(
                    group,
                    group.runner(browser, fixture_server.url, results),  # type: ignore[misc]
                )
            return results
        finally:
            await await_with_progress("playwright/browser-close", browser.close())
    finally:
        if playwright is not None:
            await await_with_progress("playwright/stop", playwright.stop())
        shutil.rmtree(temp_dir, ignore_errors=True)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run one isolated CDP smoke group against moli."
    )
    parser.add_argument(
        "--group",
        action="append",
        default=[],
        help=(
            "Run exactly one named group. The supervisor is responsible for "
            "selecting and scheduling multiple groups."
        ),
    )
    parser.add_argument(
        "--list-groups",
        action="store_true",
        help="List available groups as JSON and exit.",
    )
    parser.add_argument(
        "--endpoint",
        help="Use an existing HTTP CDP endpoint instead of starting moli serve.",
    )
    parser.add_argument(
        "--result",
        type=Path,
        help="Atomically write the worker result JSON to this path.",
    )
    return parser.parse_args(argv)


def _emit_worker_payload(
    payload: dict[str, Any],
    result_path: Path | None,
    *,
    failed: bool,
) -> None:
    rendered = json.dumps(payload, indent=2, ensure_ascii=False) + "\n"
    if result_path is None:
        print(rendered, file=sys.stderr if failed else sys.stdout, end="")
        return
    result_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = result_path.with_name(
        f".{result_path.name}.tmp-{os.getpid()}"
    )
    temporary.write_text(rendered, encoding="utf-8")
    os.replace(temporary, result_path)


async def async_main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.list_groups:
        print(json.dumps({"groups": group_listing()}, indent=2, ensure_ascii=False))
        return 0
    selection = resolve_group_selection(args.group)
    if len(selection.groups) != 1:
        raise RuntimeError(
            "moli-cdp-smoke-worker requires exactly one --group; "
            "use moli-cdp-smoke to schedule the suite"
        )
    selected_group = selection.groups[0]
    fixture = FixtureServer()
    fixture_started = False
    serve: MoliServe | None = None
    results: list[dict[str, Any]] = []
    endpoint: str | None = None
    failures: list[str] = []
    try:
        fixture.start()
        fixture_started = True
        if args.endpoint:
            endpoint = args.endpoint.rstrip("/")
        else:
            port_env = os.environ.get("MOLI_CDP_PORT")
            port = int(port_env) if port_env else 0
            if port < 0 or port > 65535 or (port == 0 and port_env is not None):
                raise RuntimeError(f"invalid MOLI_CDP_PORT: {port_env}")
            serve = await start_moli_serve(port)
            endpoint = await wait_for_moli_endpoint(serve)
        if endpoint is None:
            raise RuntimeError("CDP endpoint was not initialized")
        if serve is None:
            await wait_for_cdp_server(endpoint, serve)
        for current_group in selection.raw_groups:
            await _await_group(
                current_group,
                current_group.runner(endpoint, fixture.url, results),  # type: ignore[misc]
            )
        await run_smoke(endpoint, fixture, selection, results)
        for current_group in selection.external_groups:
            await _await_group(
                current_group,
                current_group.runner(endpoint, fixture.url, results),  # type: ignore[misc]
            )
    except Exception as error:
        failures.append("".join(traceback.format_exception(error)))
    finally:
        try:
            await stop_moli_serve(serve)
        except Exception as error:
            failures.append("".join(traceback.format_exception(error)))
        if fixture_started:
            try:
                fixture.stop()
            except Exception as error:
                failures.append("".join(traceback.format_exception(error)))

    ok = not failures and not any(result.get("ok") is False for result in results)
    payload: dict[str, Any] = {
        "ok": ok,
        "group": selected_group.name,
        "endpoint": endpoint,
        "fixture": fixture.url,
        "results": results,
    }
    if failures:
        payload["error"] = "\n".join(failures)
        if serve is not None and serve.logs:
            payload["moliLogTail"] = serve.logs[-5_000:]
    _emit_worker_payload(payload, args.result, failed=not ok)
    return 0 if ok else 1


def main(argv: list[str] | None = None) -> None:
    raise SystemExit(asyncio.run(async_main(argv)))
