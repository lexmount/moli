from __future__ import annotations

import asyncio
import json
from contextlib import asynccontextmanager
from dataclasses import dataclass
from typing import Any, AsyncIterator, Awaitable, Callable
from urllib.parse import urlsplit

from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..helpers import attach_cdp_event_collector
from ..progress import await_with_progress
from ..raw_cdp import RawCdpClient, RawCdpError, connect_raw_cdp
from ..state import SmokeState


SemanticScenario = Callable[[SmokeState], Awaitable[dict[str, Any]]]
RawSemanticScenario = Callable[[str, str], Awaitable[dict[str, Any]]]


@dataclass(frozen=True)
class RawSemanticContract:
    name: str
    contract: str
    source: str
    commands: list[str]
    scenario: RawSemanticScenario


async def run_target_semantics_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    for item in _raw_semantic_contracts():
        try:
            observed = await await_with_progress(
                f"scenario/raw/target-semantics/{item.name}",
                item.scenario(endpoint, fixture),
            )
        except Exception as error:
            results.append(
                {
                    "name": item.name,
                    "ok": False,
                    "contract": item.contract,
                    "source": item.source,
                    "commands": item.commands,
                    "errorType": type(error).__name__,
                    "error": str(error),
                }
            )
        else:
            record_contract(
                results,
                item.name,
                contract=item.contract,
                source=item.source,
                commands=item.commands,
                observed=observed,
            )


def _raw_semantic_contracts() -> tuple[RawSemanticContract, ...]:
    source = "Chromium behavior and CDP Target domain"
    return (
        RawSemanticContract(
            "raw_cdp_contract_created_target_initial_navigation",
            "Target.createTarget with a real URL starts and commits that navigation without a follow-up Page.navigate, and the attached session observes the loaded document.",
            source,
            [
                "Target.createBrowserContext",
                "Target.setDiscoverTargets",
                "Target.createTarget",
                "Target.attachToTarget",
                "Runtime.evaluate",
            ],
            _created_target_initial_navigation,
        ),
        RawSemanticContract(
            "raw_cdp_contract_created_target_debugger_wait_lifecycle",
            "A real-URL target created under waitForDebuggerOnStart does not expose an internal about:blank load; after Runtime.runIfWaitingForDebugger, the requested main-frame load includes its child-frame document.",
            "Chromium behavior and CDP Target/Page lifecycle ordering",
            [
                "Target.setAutoAttach",
                "Target.createTarget",
                "Page.enable",
                "Page.setLifecycleEventsEnabled",
                "Runtime.enable",
                "Runtime.runIfWaitingForDebugger",
                "Page.getFrameTree",
                "Runtime.evaluate",
            ],
            _created_target_debugger_wait_lifecycle,
        ),
        RawSemanticContract(
            "raw_cdp_contract_noopener_popup_devtools_attribution",
            "An anchor-created implicit-noopener popup retains its creator target and frame as DevTools attribution while canAccessOpener and window.opener remain false; after the creator closes, openerId disappears but openerFrameId persists.",
            "Chromium behavior, WebContentsImpl popup creation, and FrameTreeNode DevTools opener attribution",
            [
                "Target.createBrowserContext",
                "Target.setDiscoverTargets",
                "Target.createTarget",
                "Target.attachToTarget",
                "Page.getFrameTree",
                "Runtime.evaluate HTMLElement.click",
                "Target.getTargetInfo",
                "Target.closeTarget",
            ],
            _noopener_popup_devtools_attribution,
        ),
        RawSemanticContract(
            "raw_cdp_contract_target_multi_attach_independence",
            "Each flattened attachment has a distinct session and detaching one session leaves the other attachment usable.",
            source,
            [
                "Target.createTarget",
                "Target.attachToTarget x2",
                "Runtime.evaluate x2",
                "Target.detachFromTarget",
                "Runtime.evaluate",
            ],
            _target_multi_attach_independence,
        ),
        RawSemanticContract(
            "raw_cdp_contract_target_close_lifecycle",
            "Closing an attached target succeeds, detaches its session, emits targetDestroyed exactly once, and leaves the browser connection usable.",
            source,
            [
                "Target.setDiscoverTargets",
                "Target.createTarget",
                "Target.attachToTarget",
                "Target.closeTarget",
                "Target.getTargets",
            ],
            _target_close_lifecycle,
        ),
        RawSemanticContract(
            "raw_cdp_contract_browser_context_disposal_isolation",
            "Disposing one browser context destroys all of its targets and sessions as one operation while a target in another context remains usable.",
            source,
            [
                "Target.createBrowserContext x2",
                "Target.createTarget x3",
                "Target.attachToTarget x3",
                "Target.disposeBrowserContext",
                "Runtime.evaluate",
            ],
            _browser_context_disposal_isolation,
        ),
    )


async def run_browser_semantics_group(state: SmokeState) -> None:
    for name, contract, source, commands, scenario in _semantic_scenarios():
        try:
            observed = await await_with_progress(
                f"scenario/page/browser-semantics/{name}",
                scenario(state),
            )
        except Exception as error:
            state.results.append(
                {
                    "name": name,
                    "ok": False,
                    "contract": contract,
                    "source": source,
                    "commands": commands,
                    "errorType": type(error).__name__,
                    "error": str(error),
                }
            )
        else:
            record_contract(
                state.results,
                name,
                contract=contract,
                source=source,
                commands=commands,
                observed=observed,
            )


def _semantic_scenarios() -> tuple[
    tuple[str, str, str, list[str], SemanticScenario], ...
]:
    return (
        (
            "cdp_contract_top_level_session_storage_namespaces",
            "Same-origin top-level targets share localStorage but have independent sessionStorage; a popup receives a creation-time sessionStorage copy and then diverges.",
            "HTML Web Storage and Chromium behavior",
            ["Runtime.evaluate", "Target.createTarget via window.open"],
            _top_level_storage_namespaces,
        ),
        (
            "cdp_contract_same_document_history_identity",
            "Same-document traversal preserves the JavaScript realm and node identity.",
            "HTML History and CDP Page domain Chromium behavior",
            [
                "Runtime.evaluate history.pushState/replaceState",
                "Page.getNavigationHistory",
                "Page.navigateToHistoryEntry",
            ],
            _same_document_history_identity,
        ),
        (
            "cdp_contract_same_document_history_entry_seeding",
            "A normal navigation from a new page retains the initial about:blank history entry, and push/replace operations expose Chromium's URL sequence and current index.",
            "HTML History and CDP Page domain Chromium behavior",
            ["Runtime.evaluate history.pushState/replaceState", "Page.getNavigationHistory"],
            _same_document_history_entry_seeding,
        ),
        (
            "cdp_contract_reset_navigation_history",
            "Page.resetNavigationHistory removes every non-current entry and retains the traversed current URL at index zero.",
            "CDP Page domain Chromium behavior",
            [
                "Runtime.evaluate history.pushState/replaceState",
                "Page.navigateToHistoryEntry",
                "Page.resetNavigationHistory",
                "Page.getNavigationHistory",
            ],
            _reset_navigation_history,
        ),
        (
            "cdp_contract_frame_tree_metadata_order_and_detach",
            "Frame tree order follows document order; child metadata contains the owning parent, name, committed URL, and loader; removing an outer frame detaches its subtree.",
            "Chromium FrameTree/Page.frame* behavior",
            ["Page.enable", "Page.getFrameTree", "Runtime.evaluate remove iframe"],
            _frame_tree_metadata_order_and_detach,
        ),
        (
            "cdp_contract_shadow_frame_window_named_access",
            "An iframe in a connected shadow tree has a contentWindow but does not contribute its name to Window named properties.",
            "Chromium FrameTree::ScopedChild behavior and HTML Window named access",
            ["Runtime.evaluate"],
            _shadow_frame_window_named_access,
        ),
        (
            "cdp_contract_dom_search_mutation_and_focus",
            "DOM search returns resolvable nodes, attribute commands emit matching mutation events, and DOM.focus updates document.activeElement.",
            "Chromium behavior and CDP DOM domain",
            [
                "DOM.performSearch/getSearchResults/discardSearchResults",
                "DOM.setAttributeValue/removeAttribute",
                "DOM.focus",
            ],
            _dom_search_mutation_and_focus,
        ),
        (
            "cdp_contract_autofill_trigger_card_semantics",
            "Autofill.trigger fills a detected credit-card form through live control values, preserves value attributes and focus, marks controls :autofill, and synchronously emits Chromium's trusted input/change sequence; an unrelated ordinary field remains a successful no-op.",
            "Chromium AutofillHandler behavior and CDP Autofill domain",
            ["DOM.getDocument/querySelector/describeNode", "Autofill.trigger"],
            _autofill_trigger_card_semantics,
        ),
        (
            "cdp_contract_dom_mutation_events_and_edit_commands",
            "Runtime and DOM editing commands emit session-local DOM mutation events before their matching response, and replacement/move commands return the inserted frontend node id.",
            "Chromium behavior and CDP DOM domain",
            [
                "Runtime.evaluate",
                "DOM.requestChildNodes",
                "DOM.setAttributesAsText",
                "DOM.setNodeValue",
                "DOM.setNodeName",
                "DOM.moveTo",
                "DOM.setOuterHTML",
            ],
            _dom_mutation_events_and_edit_commands,
        ),
        (
            "cdp_contract_dom_storage_commands_and_events",
            "DOMStorage mutations are observable through getDOMStorageItems and emit added, updated, removed, and cleared events in operation order.",
            "Chromium behavior and CDP DOMStorage domain",
            [
                "DOMStorage.enable",
                "DOMStorage.setDOMStorageItem",
                "DOMStorage.getDOMStorageItems",
                "DOMStorage.removeDOMStorageItem",
                "DOMStorage.clear",
            ],
            _dom_storage_commands_and_events,
        ),
        (
            "cdp_contract_resource_tree_entries",
            "Page.getResourceTree reports the committed document's external stylesheet, nested imported stylesheet, and script with Chromium resource types and MIME types.",
            "Chromium behavior and CDP Page domain",
            ["Page.getResourceTree"],
            _resource_tree_entries,
        ),
        (
            "cdp_contract_resource_source_search",
            "Page.searchInResource searches original HTML, CSS, and JavaScript bytes rather than live DOM mutations.",
            "Chromium behavior and CDP Page domain",
            ["Page.getResourceTree", "Page.searchInResource"],
            _resource_source_search,
        ),
        (
            "cdp_contract_xml_runtime_document_shape",
            "An application/xml navigation exposes Chromium's XMLDocument runtime shape and preserves access to the source XML nodes.",
            "Chromium behavior",
            ["Page.navigate", "Runtime.evaluate"],
            _xml_runtime_document_shape,
        ),
        (
            "cdp_contract_xml_resource_tree_mime",
            "Page.getResourceTree reports application/xml for an application/xml main resource.",
            "Chromium behavior and CDP Page domain",
            ["Page.navigate", "Page.getResourceTree"],
            _xml_resource_tree_mime,
        ),
        (
            "cdp_contract_isolated_world_navigation_restore",
            "A named isolated world installed through a new-document script is recreated with a new execution context after cross-document navigation and remains separate from the main world.",
            "Chromium behavior, Puppeteer FrameManager, and CDP Page/Runtime domains",
            [
                "Runtime.enable",
                "Page.addScriptToEvaluateOnNewDocument(worldName)",
                "Page.createIsolatedWorld",
                "Page.navigate",
                "Runtime.evaluate",
            ],
            _isolated_world_navigation_restore,
        ),
        (
            "cdp_contract_multi_session_child_frame_realm_routing",
            "Enabling Runtime on an auxiliary target session does not block another attached client from evaluating in a child-frame realm.",
            "Chromium multi-session Runtime and frame-realm behavior",
            [
                "Target.attachToTarget through Playwright",
                "Runtime.enable on auxiliary CDPSession",
                "Page.navigate to iframe document",
                "Runtime.evaluate in child frame through Playwright",
            ],
            _multi_session_child_frame_realm_routing,
        ),
        (
            "cdp_contract_event_source_runtime_and_network",
            "EventSource delivers the SSE event and the matching Network.responseReceived entry is classified as EventSource.",
            "HTML EventSource and Chromium CDP Network behavior",
            ["Network.enable", "Runtime.evaluate new EventSource"],
            _event_source_runtime_and_network,
        ),
        (
            "cdp_contract_http_cache_event_and_reuse",
            "A fresh cacheable script is fetched once, reused on the next navigation, and reported through Network.requestServedFromCache.",
            "HTTP cache semantics and Chromium CDP Network behavior",
            ["Network.enable", "Page.navigate", "Page.reload", "Network.requestServedFromCache"],
            _http_cache_event_and_reuse,
        ),
        (
            "cdp_contract_cache_storage_url_normalization",
            "CacheStorage resolves relative request URLs to absolute keys, preserves the supplied key fragment, and matches and deletes the same entry across equivalent URLs with different fragments.",
            "Service Worker Cache API and Chromium behavior",
            ["Runtime.evaluate caches.open/put/keys/match/delete"],
            _cache_storage_url_normalization,
        ),
        (
            "cdp_contract_computed_style_map_updates",
            "computedStyleMap returns typed values for script-set properties and a fresh map reflects later style mutations.",
            "CSS Typed OM and Chromium behavior",
            ["Runtime.evaluate Element.computedStyleMap"],
            _computed_style_map_updates,
        ),
        (
            "cdp_contract_view_transition_script_lifecycle",
            "A view transition runs its update callback, resolves ready/updateCallbackDone/finished, and exposes the updated DOM without requiring paint output.",
            "View Transitions and Chromium behavior",
            ["Runtime.evaluate document.startViewTransition"],
            _view_transition_script_lifecycle,
        ),
    )


@asynccontextmanager
async def _isolated_page(state: SmokeState) -> AsyncIterator[tuple[Any, Any, Any]]:
    context = await await_with_progress(
        "playwright/browser-semantics/isolated-context-new",
        state.browser.new_context(),
    )
    try:
        page = await await_with_progress(
            "playwright/browser-semantics/isolated-page-new",
            context.new_page(),
        )
        cdp = await await_with_progress(
            "playwright/browser-semantics/isolated-cdp-session-new",
            context.new_cdp_session(page),
        )
        yield context, page, cdp
    finally:
        await await_with_progress(
            "playwright/browser-semantics/isolated-context-close",
            context.close(),
        )


async def _multi_session_child_frame_realm_routing(
    state: SmokeState,
) -> dict[str, Any]:
    observed: list[str] = []
    for attempt in range(3):
        async with _isolated_page(state) as (_, page, cdp):
            await cdp.send("Runtime.enable")
            await page.goto(
                f"{state.fixture}/iframe?multi-session={attempt}",
                wait_until="domcontentloaded",
                timeout=10_000,
            )
            await wait_until(
                lambda: any("/child" in frame.url for frame in page.frames),
                "multi-session child frame",
                timeout_ms=5_000,
            )
            child = next(
                (frame for frame in page.frames if "/child" in frame.url),
                None,
            )
            if child is None:
                raise SmokeError("multi-session child frame disappeared before evaluation")
            evaluation = asyncio.create_task(
                child.evaluate("() => document.body.textContent.trim()")
            )
            try:
                text = await asyncio.wait_for(
                    asyncio.shield(evaluation),
                    timeout=5,
                )
            except TimeoutError as error:
                try:
                    await cdp.send("Runtime.disable")
                except Exception:
                    pass
                try:
                    await asyncio.wait_for(evaluation, timeout=2)
                except (Exception, asyncio.CancelledError):
                    evaluation.cancel()
                    await asyncio.gather(evaluation, return_exceptions=True)
                raise SmokeError(
                    "child-frame evaluation did not complete while another target "
                    f"session had Runtime enabled (attempt {attempt + 1})"
                ) from error
            assert_equal(
                text,
                "child body text",
                "multi-session child-frame realm evaluation",
            )
            observed.append(text)
    return {"attempts": len(observed), "values": observed}


def _require(condition: bool, label: str) -> None:
    if not condition:
        raise SmokeError(label)


def _find_dom_node_by_attribute(
    node: dict[str, Any],
    name: str,
    value: str,
) -> dict[str, Any] | None:
    attributes = node.get("attributes", [])
    if any(
        attributes[index] == name and attributes[index + 1] == value
        for index in range(0, len(attributes) - 1, 2)
    ):
        return node
    for child in [*node.get("children", []), *node.get("shadowRoots", [])]:
        found = _find_dom_node_by_attribute(child, name, value)
        if found is not None:
            return found
    content_document = node.get("contentDocument")
    if isinstance(content_document, dict):
        return _find_dom_node_by_attribute(content_document, name, value)
    return None


async def _raw_command(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    message_id = await client.send(method, params, session_id=session_id)
    response, seen = await client.recv_until_id(message_id)
    return response.get("result", {}), seen


async def _raw_wait_for_event(
    client: RawCdpClient,
    method: str,
    predicate: Callable[[dict[str, Any]], bool],
    seen: list[dict[str, Any]],
    *,
    timeout: float = 10.0,
) -> dict[str, Any]:
    for message in seen:
        if message.get("method") == method and predicate(message.get("params", {})):
            return message
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for {method}; seen={seen[-20:]}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("method") == method and predicate(message.get("params", {})):
            return message


async def _ignore_raw_error(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any],
) -> None:
    try:
        await _raw_command(client, method, params)
    except Exception:
        pass


async def _created_target_initial_navigation(
    endpoint: str,
    fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_id: str | None = None
    stage = "create browser context"
    try:
        context_result, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context_result["browserContextId"]
        await _raw_command(client, "Target.setDiscoverTargets", {"discover": True})

        target_url = f"{fixture}/history-a?created=1"
        stage = "create target"
        create_result, create_seen = await _raw_command(
            client,
            "Target.createTarget",
            {"url": target_url, "browserContextId": browser_context_id},
        )
        target_id = create_result["targetId"]
        stage = "wait for committed target URL"
        target_event = await _raw_wait_for_event(
            client,
            "Target.targetInfoChanged",
            lambda params: params.get("targetInfo", {}).get("targetId") == target_id
            and params.get("targetInfo", {}).get("url") == target_url,
            create_seen,
        )
        stage = "attach target session"
        attach, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        stage = "evaluate loaded target"
        value: dict[str, Any] | None = None

        async def loaded_document_observed() -> bool:
            nonlocal value
            evaluation, _ = await _raw_command(
                client,
                "Runtime.evaluate",
                {
                    "expression": "({url: location.href, text: document.querySelector('main')?.textContent, readyState: document.readyState})",
                    "returnByValue": True,
                },
                session_id=attach["sessionId"],
            )
            candidate = evaluation["result"].get("value")
            value = candidate if isinstance(candidate, dict) else None
            return value is not None and value.get("readyState") == "complete"

        await wait_until(loaded_document_observed, "created target document complete")
        assert_equal(
            value,
            {"url": target_url, "text": "history a", "readyState": "complete"},
            "created target loaded document",
        )
        return {
            "targetInfoUrl": target_event["params"]["targetInfo"]["url"],
            "document": value,
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        if target_id is not None:
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()


async def _created_target_debugger_wait_lifecycle(
    endpoint: str,
    fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_id: str | None = None
    stage = "create browser context"
    try:
        context_result, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context_result["browserContextId"]
        await _raw_command(
            client,
            "Target.setAutoAttach",
            {
                "autoAttach": True,
                "waitForDebuggerOnStart": True,
                "flatten": True,
            },
        )

        target_url = f"{fixture}/semantic-frames?debugger-wait=1"
        stage = "create waiting target"
        create_result, create_seen = await _raw_command(
            client,
            "Target.createTarget",
            {"url": target_url, "browserContextId": browser_context_id},
        )
        target_id = create_result["targetId"]
        attached = await _raw_wait_for_event(
            client,
            "Target.attachedToTarget",
            lambda params: params.get("targetInfo", {}).get("targetId") == target_id,
            create_seen,
        )
        assert_equal(
            attached["params"].get("waitingForDebugger"),
            True,
            "debugger wait flag",
        )
        assert_equal(
            attached["params"].get("targetInfo", {}).get("url"),
            target_url,
            "waiting target URL",
        )
        session_id = attached["params"]["sessionId"]

        stage = "enable page lifecycle"
        await _raw_command(client, "Page.enable", session_id=session_id)
        _, lifecycle_seen = await _raw_command(
            client,
            "Page.setLifecycleEventsEnabled",
            {"enabled": True},
            session_id=session_id,
        )
        premature_main_loads = [
            message
            for message in lifecycle_seen
            if message.get("method") == "Page.lifecycleEvent"
            and message.get("sessionId") == session_id
            and message.get("params", {}).get("frameId") == target_id
            and message.get("params", {}).get("name") == "load"
        ]
        _require(
            not premature_main_loads,
            f"internal initial document produced a pre-resume main-frame load: {premature_main_loads}",
        )

        stage = "resume requested document"
        await _raw_command(client, "Runtime.enable", session_id=session_id)
        _, resume_seen = await _raw_command(
            client,
            "Runtime.runIfWaitingForDebugger",
            session_id=session_id,
        )
        main_load = await _raw_wait_for_event(
            client,
            "Page.lifecycleEvent",
            lambda params: params.get("frameId") == target_id
            and params.get("name") == "load",
            resume_seen,
        )

        stage = "read committed frame tree"
        frame_tree, _ = await _raw_command(
            client,
            "Page.getFrameTree",
            session_id=session_id,
        )
        root = frame_tree.get("frameTree", {})
        assert_equal(root.get("frame", {}).get("url"), target_url, "main frame URL")
        child_urls = [
            child.get("frame", {}).get("url")
            for child in root.get("childFrames", [])
        ]
        assert_equal(
            child_urls,
            [
                f"{fixture}/semantic-frame-child?child=first&nested=1",
                f"{fixture}/semantic-frame-child?child=second",
            ],
            "child frame URLs after main load",
        )
        evaluation, _ = await _raw_command(
            client,
            "Runtime.evaluate",
            {
                "expression": (
                    "Array.from(window.frames, frame => "
                    "frame.document.querySelector('main')?.textContent)"
                ),
                "returnByValue": True,
            },
            session_id=session_id,
        )
        child_texts = evaluation.get("result", {}).get("value")
        assert_equal(
            child_texts,
            ["child first", "child second"],
            "child frame documents",
        )
        return {
            "preResumeMainLoadCount": len(premature_main_loads),
            "mainLoadLoaderId": main_load["params"].get("loaderId"),
            "childUrls": child_urls,
            "childTexts": child_texts,
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        if target_id is not None:
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()


async def _noopener_popup_devtools_attribution(
    endpoint: str,
    fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    source_target_id: str | None = None
    popup_target_id: str | None = None
    stage = "create browser context"
    try:
        context_result, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context_result["browserContextId"]
        await _raw_command(client, "Target.setDiscoverTargets", {"discover": True})

        source_url = f"{fixture}/plain?popup-source=implicit-noopener"
        stage = "create popup source target"
        source_result, _ = await _raw_command(
            client,
            "Target.createTarget",
            {"url": source_url, "browserContextId": browser_context_id},
        )
        source_target_id = source_result["targetId"]
        attached, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": source_target_id, "flatten": True},
        )
        source_session_id = attached["sessionId"]

        source_document: dict[str, Any] | None = None

        async def source_document_loaded() -> bool:
            nonlocal source_document
            evaluation, _ = await _raw_command(
                client,
                "Runtime.evaluate",
                {
                    "expression": "({url: location.href, readyState: document.readyState})",
                    "returnByValue": True,
                },
                session_id=source_session_id,
            )
            candidate = evaluation["result"].get("value")
            source_document = candidate if isinstance(candidate, dict) else None
            return source_document == {"url": source_url, "readyState": "complete"}

        stage = "wait for popup source document"
        await wait_until(source_document_loaded, "popup source document complete")
        frame_tree, _ = await _raw_command(
            client,
            "Page.getFrameTree",
            session_id=source_session_id,
        )
        source_frame_id = frame_tree["frameTree"]["frame"]["id"]

        popup_url = f"{fixture}/plain?popup=implicit-noopener-attribution"
        stage = "activate implicit-noopener anchor"
        click_result, click_seen = await _raw_command(
            client,
            "Runtime.evaluate",
            {
                "expression": (
                    "(() => {"
                    "const anchor = document.createElement('a');"
                    f"anchor.href = {json.dumps(popup_url)};"
                    "anchor.target = '_blank';"
                    "document.body.append(anchor);"
                    "anchor.click();"
                    "return 'clicked';"
                    "})()"
                ),
                "returnByValue": True,
                "userGesture": True,
            },
            session_id=source_session_id,
        )
        assert_equal(
            click_result["result"].get("value"),
            "clicked",
            "implicit-noopener anchor activation",
        )

        stage = "wait for attributed popup target"
        popup_event = await _raw_wait_for_event(
            client,
            "Target.targetCreated",
            lambda params: params.get("targetInfo", {}).get("openerId")
            == source_target_id,
            click_seen,
        )
        created_info = popup_event["params"]["targetInfo"]
        popup_target_id = created_info["targetId"]
        assert_equal(
            created_info.get("openerFrameId"),
            source_frame_id,
            "implicit-noopener creator frame attribution",
        )
        assert_equal(
            created_info.get("canAccessOpener"),
            False,
            "implicit-noopener script access flag",
        )

        committed_info: dict[str, Any] | None = None

        async def popup_navigation_committed() -> bool:
            nonlocal committed_info
            result, _ = await _raw_command(
                client,
                "Target.getTargetInfo",
                {"targetId": popup_target_id},
            )
            candidate = result.get("targetInfo")
            committed_info = candidate if isinstance(candidate, dict) else None
            return committed_info is not None and committed_info.get("url") == popup_url

        stage = "wait for popup navigation"
        await wait_until(popup_navigation_committed, "implicit-noopener popup navigation")
        if committed_info is None:
            raise SmokeError("popup navigation completed without targetInfo")
        assert_equal(
            committed_info.get("openerId"),
            source_target_id,
            "committed popup creator target attribution",
        )
        assert_equal(
            committed_info.get("openerFrameId"),
            source_frame_id,
            "committed popup creator frame attribution",
        )
        assert_equal(
            committed_info.get("canAccessOpener"),
            False,
            "committed popup script access flag",
        )

        popup_attach, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": popup_target_id, "flatten": True},
        )
        opener_value, _ = await _raw_command(
            client,
            "Runtime.evaluate",
            {"expression": "window.opener === null", "returnByValue": True},
            session_id=popup_attach["sessionId"],
        )
        assert_equal(
            opener_value["result"].get("value"),
            True,
            "implicit-noopener window.opener",
        )

        stage = "close popup creator target"
        close_result, _ = await _raw_command(
            client,
            "Target.closeTarget",
            {"targetId": source_target_id},
        )
        assert_equal(close_result.get("success"), True, "popup creator close result")

        detached_info: dict[str, Any] | None = None

        async def live_opener_removed() -> bool:
            nonlocal detached_info
            result, _ = await _raw_command(
                client,
                "Target.getTargetInfo",
                {"targetId": popup_target_id},
            )
            candidate = result.get("targetInfo")
            detached_info = candidate if isinstance(candidate, dict) else None
            return detached_info is not None and "openerId" not in detached_info

        stage = "wait for live opener removal"
        await wait_until(live_opener_removed, "popup live opener removal")
        if detached_info is None:
            raise SmokeError("popup live opener removal completed without targetInfo")
        assert_equal(
            detached_info.get("openerFrameId"),
            source_frame_id,
            "closed creator frame attribution",
        )
        assert_equal(
            detached_info.get("canAccessOpener"),
            False,
            "closed creator script access flag",
        )

        return {
            "created": {
                "openerIdMatchesSource": created_info.get("openerId")
                == source_target_id,
                "openerFrameIdMatchesSourceFrame": created_info.get("openerFrameId")
                == source_frame_id,
                "canAccessOpener": created_info.get("canAccessOpener"),
            },
            "windowOpenerIsNull": opener_value["result"].get("value"),
            "afterCreatorClose": {
                "hasOpenerId": "openerId" in detached_info,
                "openerFrameIdPreserved": detached_info.get("openerFrameId")
                == source_frame_id,
                "canAccessOpener": detached_info.get("canAccessOpener"),
            },
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        for target_id in (popup_target_id, source_target_id):
            if target_id is not None:
                await _ignore_raw_error(
                    client,
                    "Target.closeTarget",
                    {"targetId": target_id},
                )
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()


async def _target_close_lifecycle(
    endpoint: str,
    _fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_id: str | None = None
    stage = "create browser context"
    try:
        context_result, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context_result["browserContextId"]
        await _raw_command(client, "Target.setDiscoverTargets", {"discover": True})
        create_result, create_seen = await _raw_command(
            client,
            "Target.createTarget",
            {"url": "about:blank", "browserContextId": browser_context_id},
        )
        target_id = create_result["targetId"]
        attach_result, attach_seen = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        session_id = attach_result["sessionId"]

        stage = "close target"
        close_result, close_seen = await _raw_command(
            client,
            "Target.closeTarget",
            {"targetId": target_id},
        )
        assert_equal(close_result.get("success"), True, "Target.closeTarget success")
        seen = create_seen + attach_seen + close_seen
        stage = "wait for detached session"
        await _raw_wait_for_event(
            client,
            "Target.detachedFromTarget",
            lambda params: params.get("sessionId") == session_id,
            seen,
        )
        stage = "wait for targetDestroyed"
        await _raw_wait_for_event(
            client,
            "Target.targetDestroyed",
            lambda params: params.get("targetId") == target_id,
            seen,
        )
        relevant_methods = [
            message["method"]
            for message in seen
            if message.get("method") in {
                "Target.detachedFromTarget",
                "Target.targetDestroyed",
            }
            and (
                message.get("params", {}).get("sessionId") == session_id
                or message.get("params", {}).get("targetId") == target_id
            )
        ]
        assert_equal(
            relevant_methods.count("Target.detachedFromTarget"),
            1,
            "target close detached event count",
        )
        assert_equal(
            relevant_methods.count("Target.targetDestroyed"),
            1,
            "target close destroyed event count",
        )
        stage = "verify browser connection"
        targets, _ = await _raw_command(client, "Target.getTargets")
        closed_target_present = any(
            info.get("targetId") == create_result["targetId"]
            for info in targets.get("targetInfos", [])
        )
        assert_equal(closed_target_present, False, "closed target in Target.getTargets")
        target_id = None
        return {
            "eventMethods": relevant_methods,
            "closedTargetPresent": closed_target_present,
            "browserConnectionUsable": True,
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        if target_id is not None:
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()


async def _browser_context_disposal_isolation(
    endpoint: str,
    fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    first_context: str | None = None
    second_context: str | None = None
    target_ids: list[str] = []
    stage = "create browser contexts"
    try:
        first_context = (
            await _raw_command(client, "Target.createBrowserContext")
        )[0]["browserContextId"]
        second_context = (
            await _raw_command(client, "Target.createBrowserContext")
        )[0]["browserContextId"]
        await _raw_command(client, "Target.setDiscoverTargets", {"discover": True})

        sessions: list[str] = []
        create_seen: list[dict[str, Any]] = []
        for context_id, suffix in (
            (first_context, "disposed-a"),
            (first_context, "disposed-b"),
            (second_context, "survivor"),
        ):
            created, seen = await _raw_command(
                client,
                "Target.createTarget",
                {
                    "url": f"{fixture}/plain?context={suffix}",
                    "browserContextId": context_id,
                },
            )
            target_ids.append(created["targetId"])
            create_seen.extend(seen)
            attached, seen = await _raw_command(
                client,
                "Target.attachToTarget",
                {"targetId": created["targetId"], "flatten": True},
            )
            sessions.append(attached["sessionId"])
            create_seen.extend(seen)

        disposed_target_ids = target_ids[:2]
        survivor_target_id = target_ids[2]
        disposed_session = sessions[0]
        survivor_session = sessions[2]
        stage = "dispose first browser context"
        _, dispose_seen = await _raw_command(
            client,
            "Target.disposeBrowserContext",
            {"browserContextId": first_context},
        )
        first_context = None
        seen = create_seen + dispose_seen
        for disposed_target_id in disposed_target_ids:
            await _raw_wait_for_event(
                client,
                "Target.targetDestroyed",
                lambda params, expected=disposed_target_id: params.get("targetId")
                == expected,
                seen,
            )

        stage = "verify disposed session rejection"
        try:
            await _raw_command(
                client,
                "Runtime.evaluate",
                {"expression": "1", "returnByValue": True},
                session_id=disposed_session,
            )
        except RawCdpError as error:
            disposed_error = str(error)
        else:
            raise SmokeError("disposed target session remained usable")

        stage = "verify second browser context"
        survivor, _ = await _raw_command(
            client,
            "Runtime.evaluate",
            {"expression": "location.search", "returnByValue": True},
            session_id=survivor_session,
        )
        assert_equal(
            survivor["result"].get("value"),
            "?context=survivor",
            "other browser context target",
        )
        target_ids = [survivor_target_id]
        return {
            "destroyedTargetIds": disposed_target_ids,
            "disposedSessionError": disposed_error,
            "survivorTargetId": survivor_target_id,
            "survivorValue": survivor["result"].get("value"),
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        for target_id in target_ids:
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        for context_id in (first_context, second_context):
            if context_id is not None:
                await _ignore_raw_error(
                    client,
                    "Target.disposeBrowserContext",
                    {"browserContextId": context_id},
                )
        await client.websocket.close()


async def _target_multi_attach_independence(
    endpoint: str,
    fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_id: str | None = None
    stage = "create browser context"
    try:
        context_result, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context_result["browserContextId"]
        stage = "enable target discovery"
        await _raw_command(client, "Target.setDiscoverTargets", {"discover": True})

        target_url = f"{fixture}/history-a?created=1"
        stage = "create target"
        create_result, create_seen = await _raw_command(
            client,
            "Target.createTarget",
            {"url": target_url, "browserContextId": browser_context_id},
        )
        target_id = create_result["targetId"]
        stage = "wait for committed target URL"
        target_event = await _raw_wait_for_event(
            client,
            "Target.targetInfoChanged",
            lambda params: params.get("targetInfo", {}).get("targetId") == target_id
            and params.get("targetInfo", {}).get("url") == target_url,
            create_seen,
        )

        stage = "attach first session"
        first_attach, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        stage = "attach second session"
        second_attach, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        first_session = first_attach["sessionId"]
        second_session = second_attach["sessionId"]
        _require(first_session != second_session, "flattened attachments reused one session id")

        stage = "evaluate first session"
        first_eval, _ = await _raw_command(
            client,
            "Runtime.evaluate",
            {"expression": "location.href", "returnByValue": True},
            session_id=first_session,
        )
        stage = "evaluate second session"
        second_eval, _ = await _raw_command(
            client,
            "Runtime.evaluate",
            {"expression": "document.querySelector('main').textContent", "returnByValue": True},
            session_id=second_session,
        )
        assert_equal(
            first_eval["result"].get("value"),
            target_url,
            "created target committed URL",
        )
        assert_equal(
            second_eval["result"].get("value"),
            "history a",
            "second target attachment evaluation",
        )

        stage = "detach first session"
        await _raw_command(client, "Target.detachFromTarget", {"sessionId": first_session})
        stage = "evaluate surviving session"
        surviving_eval, _ = await _raw_command(
            client,
            "Runtime.evaluate",
            {"expression": "6 * 7", "returnByValue": True},
            session_id=second_session,
        )
        assert_equal(
            surviving_eval["result"].get("value"),
            42,
            "second attachment after first detach",
        )

        return {
            "createdUrl": target_event["params"]["targetInfo"]["url"],
            "sessionIdsDistinct": first_session != second_session,
            "survivingSessionValue": surviving_eval["result"].get("value"),
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        if target_id is not None:
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()


async def _top_level_storage_namespaces(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (context, first, _cdp):
        await first.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        await first.evaluate(
            """() => {
              localStorage.clear();
              sessionStorage.clear();
              localStorage.setItem('semantic-local', 'shared');
              sessionStorage.setItem('semantic-session', 'first');
            }"""
        )

        second = await context.new_page()
        await second.goto(f"{state.fixture}/plain?top=second", wait_until="load", timeout=10_000)
        second_initial = await second.evaluate(
            """() => ({
              local: localStorage.getItem('semantic-local'),
              session: sessionStorage.getItem('semantic-session'),
            })"""
        )
        assert_equal(
            second_initial,
            {"local": "shared", "session": None},
            "independent top-level storage",
        )
        await second.evaluate("sessionStorage.setItem('semantic-session', 'second')")
        await second.reload(wait_until="load", timeout=10_000)
        second_after_reload = await second.evaluate(
            "sessionStorage.getItem('semantic-session')"
        )
        assert_equal(
            second_after_reload,
            "second",
            "second top-level sessionStorage after reload",
        )

        async with first.expect_popup(timeout=10_000) as popup_info:
            await first.evaluate("url => window.open(url, '_blank')", f"{state.fixture}/plain?popup=1")
        popup = await popup_info.value
        await popup.wait_for_load_state("load", timeout=10_000)
        popup_initial = await popup.evaluate(
            """() => ({
              local: localStorage.getItem('semantic-local'),
              session: sessionStorage.getItem('semantic-session'),
            })"""
        )
        assert_equal(
            popup_initial,
            {"local": "shared", "session": "first"},
            "popup storage snapshot",
        )
        await popup.evaluate("sessionStorage.setItem('semantic-session', 'popup')")
        await popup.reload(wait_until="load", timeout=10_000)
        popup_after_reload = await popup.evaluate(
            "sessionStorage.getItem('semantic-session')"
        )
        assert_equal(
            popup_after_reload,
            "popup",
            "popup sessionStorage after reload",
        )
        first_after_popup = await first.evaluate("sessionStorage.getItem('semantic-session')")
        second_after_popup = await second.evaluate("sessionStorage.getItem('semantic-session')")
        assert_equal(first_after_popup, "first", "opener sessionStorage after popup mutation")
        assert_equal(second_after_popup, "second", "second top-level sessionStorage")
        return {
            "secondInitial": second_initial,
            "popupInitial": popup_initial,
            "afterPopupMutation": {
                "opener": first_after_popup,
                "secondTopLevel": second_after_popup,
                "popupAfterReload": popup_after_reload,
            },
        }


async def _same_document_history_identity(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        base_url = f"{state.fixture}/plain#base"
        await page.goto(base_url, wait_until="load", timeout=10_000)
        element = await page.query_selector("main")
        _require(element is not None, "missing main element for history identity contract")
        await page.evaluate(
            """() => {
              globalThis.__semanticRealmMarker = { value: 17 };
              document.querySelector('main').__semanticNodeMarker = 23;
              history.pushState({ step: 1 }, '', '#one');
              history.replaceState({ step: 2 }, '', '#two');
              history.pushState({ step: 3 }, '', '#three');
            }"""
        )
        before = await cdp.send("Page.getNavigationHistory")
        target_entry = next(
            (entry for entry in before["entries"] if entry["url"].endswith("#two")),
            None,
        )
        _require(target_entry is not None, f"missing #two history entry: {before}")

        await cdp.send("Page.navigateToHistoryEntry", {"entryId": target_entry["id"]})
        await page.wait_for_url(f"{state.fixture}/plain#two", timeout=10_000)
        realm_marker = await page.evaluate("globalThis.__semanticRealmMarker.value")
        node_marker = await element.evaluate("node => node.__semanticNodeMarker")
        assert_equal(realm_marker, 17, "same-document realm marker")
        assert_equal(node_marker, 23, "same-document element identity marker")
        return {
            "entryUrls": [entry["url"] for entry in before["entries"]],
            "realmMarker": realm_marker,
            "nodeMarker": node_marker,
        }


async def _same_document_history_entry_seeding(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        base_url = f"{state.fixture}/plain#base"
        await page.goto(base_url, wait_until="load", timeout=10_000)
        await page.evaluate(
            """() => {
              history.pushState({ step: 1 }, '', '#one');
              history.replaceState({ step: 2 }, '', '#two');
              history.pushState({ step: 3 }, '', '#three');
            }"""
        )
        history = await cdp.send("Page.getNavigationHistory")
        expected_urls = [
            "about:blank",
            base_url,
            f"{state.fixture}/plain#two",
            f"{state.fixture}/plain#three",
        ]
        actual_urls = [entry["url"] for entry in history["entries"]]
        assert_equal(actual_urls, expected_urls, "same-document history entry URLs")
        assert_equal(history["currentIndex"], 3, "same-document history current index")
        assert_equal(
            [entry["userTypedURL"] for entry in history["entries"]],
            ["about:blank", base_url, base_url, base_url],
            "same-document history userTypedURL metadata",
        )
        assert_equal(
            [entry["transitionType"] for entry in history["entries"]],
            ["auto_toplevel", "typed", "link", "link"],
            "same-document history transition metadata",
        )
        return {
            "currentIndex": history["currentIndex"],
            "entries": history["entries"],
        }


async def _reset_navigation_history(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        await page.goto(f"{state.fixture}/plain#base", wait_until="load", timeout=10_000)
        await page.evaluate(
            """() => {
              history.pushState({ step: 1 }, '', '#one');
              history.replaceState({ step: 2 }, '', '#two');
              history.pushState({ step: 3 }, '', '#three');
            }"""
        )
        before = await cdp.send("Page.getNavigationHistory")
        target_entry = next(
            entry for entry in before["entries"] if entry["url"].endswith("#two")
        )
        await cdp.send("Page.navigateToHistoryEntry", {"entryId": target_entry["id"]})
        expected_url = f"{state.fixture}/plain#two"
        await page.wait_for_url(expected_url, timeout=10_000)
        await cdp.send("Page.resetNavigationHistory")
        after = await cdp.send("Page.getNavigationHistory")
        assert_equal(after["currentIndex"], 0, "reset history current index")
        assert_equal(len(after["entries"]), 1, "reset history entry count")
        assert_equal(after["entries"][0]["url"], expected_url, "reset retained URL")
        assert_equal(
            after["entries"][0]["id"],
            target_entry["id"],
            "reset retained entry identity",
        )
        assert_equal(
            {
                "userTypedURL": after["entries"][0]["userTypedURL"],
                "title": after["entries"][0]["title"],
                "transitionType": after["entries"][0]["transitionType"],
            },
            {
                "userTypedURL": f"{state.fixture}/plain#base",
                "title": "",
                "transitionType": "link",
            },
            "reset retained entry metadata",
        )
        return {"before": before, "after": after}


async def _frame_tree_metadata_order_and_detach(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        events = attach_cdp_event_collector(
            cdp,
            ["Page.frameAttached", "Page.frameNavigated", "Page.frameDetached"],
        )
        await cdp.send("Page.enable")
        await page.goto(f"{state.fixture}/semantic-frames", wait_until="load", timeout=10_000)
        tree = (await cdp.send("Page.getFrameTree"))["frameTree"]
        root = tree["frame"]
        children = tree.get("childFrames", [])
        assert_equal(len(children), 2, "top-level semantic frame count")
        first_tree, second_tree = children
        first = first_tree["frame"]
        second = second_tree["frame"]
        assert_equal([first.get("name"), second.get("name")], ["first-frame", "second-frame"], "frame document order")
        assert_equal(first.get("parentId"), root["id"], "first frame parentId")
        assert_equal(second.get("parentId"), root["id"], "second frame parentId")
        assert_equal(first.get("url"), f"{state.fixture}/semantic-frame-child?child=first&nested=1", "first frame URL")
        assert_equal(second.get("url"), f"{state.fixture}/semantic-frame-child?child=second", "second frame URL")
        _require(bool(first.get("loaderId")), f"first frame missing loaderId: {first}")
        _require(bool(second.get("loaderId")), f"second frame missing loaderId: {second}")

        nested_trees = first_tree.get("childFrames", [])
        assert_equal(len(nested_trees), 1, "nested semantic frame count")
        nested = nested_trees[0]["frame"]
        assert_equal(nested.get("name"), "nested-frame", "nested frame name")
        assert_equal(nested.get("parentId"), first["id"], "nested frame parentId")
        assert_equal(nested.get("url"), f"{state.fixture}/semantic-frame-grandchild", "nested frame URL")
        _require(bool(nested.get("loaderId")), f"nested frame missing loaderId: {nested}")

        await page.evaluate("document.querySelector('#first').remove()")
        await wait_until(
            lambda: all(
                any(
                    event["method"] == "Page.frameDetached"
                    and event["params"].get("frameId") == frame_id
                    for event in events
                )
                for frame_id in (first["id"], nested["id"])
            ),
            "outer and nested Page.frameDetached events",
        )
        detach_ids = [
            event["params"]["frameId"]
            for event in events
            if event["method"] == "Page.frameDetached"
            and event["params"].get("frameId") in {first["id"], nested["id"]}
        ]
        return {
            "rootFrameId": root["id"],
            "children": [
                {"id": first["id"], "name": first["name"], "url": first["url"], "loaderId": first["loaderId"]},
                {"id": second["id"], "name": second["name"], "url": second["url"], "loaderId": second["loaderId"]},
            ],
            "nested": {"id": nested["id"], "parentId": nested["parentId"], "url": nested["url"]},
            "detachedFrameIds": detach_ids,
        }


async def _shadow_frame_window_named_access(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, _cdp):
        await page.goto(f"{state.fixture}/semantic-shadow-frame", wait_until="load", timeout=10_000)
        await page.wait_for_function(
            """() => {
              const frame = document.querySelector('#host').shadowRoot.querySelector('iframe');
              return frame.contentDocument && frame.contentDocument.readyState === 'complete';
            }""",
            timeout=10_000,
        )
        observed = await page.evaluate(
            """() => {
              const frame = document.querySelector('#host').shadowRoot.querySelector('iframe');
              return {
                hasContentWindow: !!frame.contentWindow,
                namedPropertyPresent: 'shadowNamed' in window,
                namedPropertyType: typeof window.shadowNamed,
              };
            }"""
        )
        assert_equal(observed["hasContentWindow"], True, "shadow iframe contentWindow")
        assert_equal(observed["namedPropertyPresent"], False, "shadow iframe Window named property")
        assert_equal(observed["namedPropertyType"], "undefined", "shadow iframe named property type")
        return observed


async def _dom_search_mutation_and_focus(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        events = attach_cdp_event_collector(cdp, ["DOM.attributeModified", "DOM.attributeRemoved"])
        await cdp.send("DOM.enable")
        await page.goto(f"{state.fixture}/semantic-dom", wait_until="load", timeout=10_000)
        root_id = (await cdp.send("DOM.getDocument", {"depth": -1}))["root"]["nodeId"]
        search = await cdp.send("DOM.performSearch", {"query": "semantic search needle"})
        _require(search["resultCount"] > 0, f"DOM.performSearch returned no results: {search}")
        search_results = await cdp.send(
            "DOM.getSearchResults",
            {"searchId": search["searchId"], "fromIndex": 0, "toIndex": search["resultCount"]},
        )
        _require(all(node_id > 0 for node_id in search_results["nodeIds"]), f"invalid search node ids: {search_results}")
        await cdp.send("DOM.discardSearchResults", {"searchId": search["searchId"]})

        target_id = (await cdp.send("DOM.querySelector", {"nodeId": root_id, "selector": "#search-target"}))["nodeId"]
        await cdp.send("DOM.setAttributeValue", {"nodeId": target_id, "name": "data-semantic", "value": "updated"})
        await wait_until(
            lambda: any(
                event["method"] == "DOM.attributeModified"
                and event["params"].get("nodeId") == target_id
                and event["params"].get("name") == "data-semantic"
                and event["params"].get("value") == "updated"
                for event in events
            ),
            "DOM.attributeModified",
        )
        await cdp.send("DOM.removeAttribute", {"nodeId": target_id, "name": "data-semantic"})
        await wait_until(
            lambda: any(
                event["method"] == "DOM.attributeRemoved"
                and event["params"].get("nodeId") == target_id
                and event["params"].get("name") == "data-semantic"
                for event in events
            ),
            "DOM.attributeRemoved",
        )

        focus_id = (await cdp.send("DOM.querySelector", {"nodeId": root_id, "selector": "#focus-target"}))["nodeId"]
        await cdp.send("DOM.focus", {"nodeId": focus_id})
        active_id = await page.evaluate("document.activeElement.id")
        assert_equal(active_id, "focus-target", "DOM.focus active element")
        return {
            "searchResultCount": search["resultCount"],
            "searchNodeIds": search_results["nodeIds"],
            "mutationEvents": [event["method"] for event in events],
            "activeElementId": active_id,
        }


async def _autofill_trigger_card_semantics(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        await page.goto(
            f"{state.fixture}/semantic-autofill-card",
            wait_until="load",
            timeout=10_000,
        )
        root_id = (await cdp.send("DOM.getDocument", {"depth": -1}))["root"]["nodeId"]

        async def backend_node_id(selector: str) -> int:
            node_id = (
                await cdp.send(
                    "DOM.querySelector",
                    {"nodeId": root_id, "selector": selector},
                )
            )["nodeId"]
            _require(node_id > 0, f"Autofill anchor not found for {selector}")
            return (
                await cdp.send(
                    "DOM.describeNode",
                    {"nodeId": node_id},
                )
            )["node"]["backendNodeId"]

        card = {
            "number": "4444444444444448",
            "name": "T2B Tester",
            "expiryMonth": "12",
            "expiryYear": "2030",
            "cvc": "123",
        }
        card_field_id = await backend_node_id("#CREDIT_CARD_NUMBER")
        await cdp.send("Autofill.trigger", {"fieldId": card_field_id, "card": card})
        observed = await page.evaluate(
            """() => ({
              values: Object.fromEntries(
                [...document.querySelectorAll('#card-form input')].map(
                  control => [control.id, control.value]
                )
              ),
              valueAttributes: Object.fromEntries(
                [...document.querySelectorAll('#card-form input')].map(
                  control => [control.id, control.getAttribute('value')]
                )
              ),
              autofilled: [...document.querySelectorAll('#card-form input')].map(
                control => [control.id, control.matches(':autofill')]
              ),
              activeElementId: document.activeElement && document.activeElement.id,
              events: __autofillEvents,
            })"""
        )
        expected_values = {
            "CREDIT_CARD_NUMBER": "4444444444444448",
            "CREDIT_CARD_NAME_FULL": "T2B Tester",
            "CREDIT_CARD_EXP_MONTH": "12",
            "CREDIT_CARD_EXP_4_DIGIT_YEAR": "2030",
            "CREDIT_CARD_VERIFICATION_CODE": "123",
        }
        assert_equal(observed["values"], expected_values, "Autofill live values")
        assert_equal(
            observed["valueAttributes"],
            {field: None for field in expected_values},
            "Autofill value attributes",
        )
        assert_equal(
            observed["autofilled"],
            [[field, True] for field in expected_values],
            "Autofill pseudo-class state",
        )
        assert_equal(observed["activeElementId"], "", "Autofill focus preservation")
        expected_events: list[dict[str, Any]] = []
        for field in expected_values:
            expected_events.extend(
                [
                    {
                        "type": "input",
                        "id": field,
                        "trusted": True,
                        "bubbles": True,
                        "composed": True,
                    },
                    {
                        "type": "change",
                        "id": field,
                        "trusted": True,
                        "bubbles": True,
                        "composed": False,
                    },
                ]
            )
        assert_equal(observed["events"], expected_events, "Autofill event sequence")

        ordinary_field_id = await backend_node_id("#ordinary-field")
        await cdp.send("Autofill.trigger", {"fieldId": ordinary_field_id, "card": card})
        ordinary = await page.evaluate(
            """() => ({
              value: document.querySelector('#ordinary-field').value,
              autofilled: document.querySelector('#ordinary-field').matches(':autofill'),
              eventCount: __autofillEvents.length,
            })"""
        )
        assert_equal(
            ordinary,
            {"value": "", "autofilled": False, "eventCount": len(expected_events)},
            "ordinary Autofill anchor no-op",
        )
        return {**observed, "ordinaryField": ordinary}


async def _dom_mutation_events_and_edit_commands(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        progress_prefix = "command/page/browser-semantics/dom-mutation-edit"

        async def step(label: str, awaitable: Awaitable[Any]) -> Any:
            return await await_with_progress(f"{progress_prefix}/{label}", awaitable)

        await step("DOM.enable", cdp.send("DOM.enable"))
        html = """<!doctype html><body>
              <div id="unrequested"><span>old</span></div>
              <p id="attrs" data-old="one">attribute target</p>
              <p id="value">old text</p>
              <section id="rename">rename target</section>
              <div id="move">move target</div>
              <div id="destination"></div>
              <div id="outer">outer target</div>
            </body>"""
        await step(
            "Page.setContent",
            page.set_content(html),
        )
        root_id = (
            await step(
                "DOM.getDocument-shallow",
                cdp.send("DOM.getDocument", {"depth": 0}),
            )
        )["root"]["nodeId"]
        unrequested_id = (
            await step(
                "DOM.querySelector-unrequested",
                cdp.send(
                    "DOM.querySelector",
                    {"nodeId": root_id, "selector": "#unrequested"},
                ),
            )
        )["nodeId"]

        event_methods = [
            "DOM.attributeModified",
            "DOM.attributeRemoved",
            "DOM.characterDataModified",
            "DOM.childNodeCountUpdated",
            "DOM.childNodeInserted",
            "DOM.childNodeRemoved",
        ]
        wire_order: list[dict[str, Any]] = []
        for method in event_methods:
            cdp.on(
                method,
                lambda params, method=method: wire_order.append(
                    {"kind": "event", "method": method, "params": params}
                ),
            )

        async def send_and_require_events(
            label: str,
            method: str,
            params: dict[str, Any],
            expected_methods: list[str],
        ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
            start = len(wire_order)
            result = await step(label, cdp.send(method, params))
            wire_order.append({"kind": "response", "method": method})
            command_order = wire_order[start:]
            observed_methods = [
                item["method"] for item in command_order if item["kind"] == "event"
            ]
            for expected_method in expected_methods:
                _require(
                    expected_method in observed_methods,
                    f"{method} response overtook {expected_method}: {command_order}",
                )
            return result, command_order

        _, shallow_runtime_order = await send_and_require_events(
            "Runtime.evaluate-shallow-mutation",
            "Runtime.evaluate",
            {
                "expression": "document.querySelector('#unrequested').append(document.createElement('i'))"
            },
            ["DOM.childNodeCountUpdated"],
        )

        document = (
            await step(
                "DOM.getDocument-deep",
                cdp.send("DOM.getDocument", {"depth": -1}),
            )
        )["root"]

        def node_for_selector(selector: str) -> int:
            node = _find_dom_node_by_attribute(document, "id", selector.removeprefix("#"))
            _require(node is not None, f"missing DOM node for {selector}: {document}")
            node_id = node.get("nodeId")
            _require(isinstance(node_id, int) and node_id > 0, f"invalid node id for {selector}: {node}")
            return node_id

        attrs_id = node_for_selector("#attrs")
        _, attributes_order = await send_and_require_events(
            "DOM.setAttributesAsText",
            "DOM.setAttributesAsText",
            {
                "nodeId": attrs_id,
                "text": 'id="attrs" data-new="two" title="updated"',
                "name": "data-old",
            },
            ["DOM.attributeModified", "DOM.attributeRemoved"],
        )

        value_node = _find_dom_node_by_attribute(document, "id", "value")
        _require(value_node is not None, f"missing value node: {document}")
        value_children = value_node.get("children", [])
        _require(len(value_children) == 1, f"unexpected value children: {value_children}")
        text_node_id = value_children[0].get("nodeId")
        _require(isinstance(text_node_id, int) and text_node_id > 0, f"invalid text node: {value_children[0]}")
        _, value_order = await send_and_require_events(
            "DOM.setNodeValue-change",
            "DOM.setNodeValue",
            {"nodeId": text_node_id, "value": "updated text"},
            ["DOM.characterDataModified"],
        )
        same_value_result = await step(
            "DOM.setNodeValue-same",
            cdp.send(
                "DOM.setNodeValue",
                {"nodeId": text_node_id, "value": "updated text"},
            ),
        )
        assert_equal(same_value_result, {}, "DOM.setNodeValue same-value result")

        rename_id = node_for_selector("#rename")
        rename_result, rename_order = await send_and_require_events(
            "DOM.setNodeName-element",
            "DOM.setNodeName",
            {"nodeId": rename_id, "name": "article"},
            ["DOM.childNodeRemoved", "DOM.childNodeInserted"],
        )
        rename_insert = next(
            item
            for item in rename_order
            if item.get("method") == "DOM.childNodeInserted"
        )
        assert_equal(
            rename_result["nodeId"],
            rename_insert["params"]["node"]["nodeId"],
            "DOM.setNodeName returned inserted node id",
        )

        move_id = node_for_selector("#move")
        destination_id = node_for_selector("#destination")
        move_result, move_order = await send_and_require_events(
            "DOM.moveTo",
            "DOM.moveTo",
            {"nodeId": move_id, "targetNodeId": destination_id},
            ["DOM.childNodeRemoved", "DOM.childNodeInserted"],
        )
        move_insert = next(
            item
            for item in move_order
            if item.get("method") == "DOM.childNodeInserted"
        )
        assert_equal(
            move_result["nodeId"],
            move_insert["params"]["node"]["nodeId"],
            "DOM.moveTo returned inserted node id",
        )

        outer_id = node_for_selector("#outer")
        _, outer_order = await send_and_require_events(
            "DOM.setOuterHTML",
            "DOM.setOuterHTML",
            {
                "nodeId": outer_id,
                "outerHTML": '<aside id="outer-replacement">replacement</aside>',
            },
            ["DOM.childNodeRemoved", "DOM.childNodeInserted"],
        )
        assert_equal(
            await step(
                "Runtime.evaluate-outer-replacement",
                page.evaluate("document.querySelector('#outer-replacement')?.localName"),
            ),
            "aside",
            "DOM.setOuterHTML replacement",
        )

        pi_object = (
            await step(
                "Runtime.evaluate-create-processing-instruction",
                cdp.send(
                    "Runtime.evaluate",
                    {
                        "expression": "(() => { const pi = document.createProcessingInstruction('old-target', 'data'); document.insertBefore(pi, document.firstChild); return pi; })()"
                    },
                ),
            )
        )["result"]["objectId"]
        pi_node_id = (
            await step(
                "DOM.requestNode-processing-instruction",
                cdp.send("DOM.requestNode", {"objectId": pi_object}),
            )
        )["nodeId"]
        pi_rename_result, pi_rename_order = await send_and_require_events(
            "DOM.setNodeName-processing-instruction",
            "DOM.setNodeName",
            {"nodeId": pi_node_id, "name": "xml"},
            ["DOM.childNodeRemoved", "DOM.childNodeInserted"],
        )
        pi_rename_insert = next(
            item
            for item in pi_rename_order
            if item.get("method") == "DOM.childNodeInserted"
        )
        assert_equal(
            pi_rename_result["nodeId"],
            pi_rename_insert["params"]["node"]["nodeId"],
            "DOM.setNodeName processing-instruction node id",
        )
        assert_equal(
            await step(
                "Runtime.evaluate-processing-instruction-target",
                page.evaluate("document.firstChild.target"),
            ),
            "xml",
            "DOM.setNodeName processing-instruction xml target",
        )

        return {
            "runtimeOrder": [item["method"] for item in shallow_runtime_order],
            "attributesOrder": [item["method"] for item in attributes_order],
            "valueOrder": [item["method"] for item in value_order],
            "renameOrder": [item["method"] for item in rename_order],
            "moveOrder": [item["method"] for item in move_order],
            "outerOrder": [item["method"] for item in outer_order],
            "piRenameOrder": [item["method"] for item in pi_rename_order],
        }


async def _dom_storage_commands_and_events(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        await page.evaluate("localStorage.clear()")
        events = attach_cdp_event_collector(
            cdp,
            [
                "DOMStorage.domStorageItemAdded",
                "DOMStorage.domStorageItemUpdated",
                "DOMStorage.domStorageItemRemoved",
                "DOMStorage.domStorageItemsCleared",
            ],
        )
        await cdp.send("DOMStorage.enable")
        storage_id = {
            "securityOrigin": await page.evaluate("location.origin"),
            "isLocalStorage": True,
        }
        await cdp.send(
            "DOMStorage.setDOMStorageItem",
            {"storageId": storage_id, "key": "semantic-key", "value": "one"},
        )
        await cdp.send(
            "DOMStorage.setDOMStorageItem",
            {"storageId": storage_id, "key": "semantic-key", "value": "two"},
        )
        items = (await cdp.send("DOMStorage.getDOMStorageItems", {"storageId": storage_id}))["entries"]
        assert_equal(items, [["semantic-key", "two"]], "DOMStorage updated entries")
        await cdp.send(
            "DOMStorage.removeDOMStorageItem",
            {"storageId": storage_id, "key": "semantic-key"},
        )
        await cdp.send(
            "DOMStorage.setDOMStorageItem",
            {"storageId": storage_id, "key": "clear-key", "value": "clear-value"},
        )
        await cdp.send("DOMStorage.clear", {"storageId": storage_id})
        expected_methods = [
            "DOMStorage.domStorageItemAdded",
            "DOMStorage.domStorageItemUpdated",
            "DOMStorage.domStorageItemRemoved",
            "DOMStorage.domStorageItemAdded",
            "DOMStorage.domStorageItemsCleared",
        ]
        await wait_until(
            lambda: [event["method"] for event in events] == expected_methods,
            "ordered DOMStorage mutation events",
        )
        final_items = (await cdp.send("DOMStorage.getDOMStorageItems", {"storageId": storage_id}))["entries"]
        assert_equal(final_items, [], "DOMStorage entries after clear")
        return {
            "storageId": storage_id,
            "updatedEntries": items,
            "eventMethods": [event["method"] for event in events],
            "finalEntries": final_items,
        }


async def _resource_tree_entries(state: SmokeState) -> dict[str, Any]:
    state.fixture_server.reset_request_count("/semantic-resource.css")
    state.fixture_server.reset_request_count("/semantic-resource-import.css")
    async with _isolated_page(state) as (_context, page, cdp):
        document_url = f"{state.fixture}/semantic-resource-page"
        await cdp.send("Page.enable")
        await page.goto(document_url, wait_until="load", timeout=10_000)
        tree = (await cdp.send("Page.getResourceTree"))["frameTree"]
        frame = tree["frame"]
        resources = tree.get("resources", [])
        by_url = {resource["url"]: resource for resource in resources}
        expected = {
            f"{state.fixture}/semantic-resource.css": ("Stylesheet", "text/css"),
            f"{state.fixture}/semantic-resource-import.css": ("Stylesheet", "text/css"),
            f"{state.fixture}/semantic-resource.js": ("Script", "application/javascript"),
        }
        for url, (resource_type, mime_type) in expected.items():
            resource = by_url.get(url)
            _require(
                resource is not None,
                "missing resource tree entry for "
                f"{url}: resources={resources}, requests={{"
                f"'stylesheet': {state.fixture_server.request_count('/semantic-resource.css')}, "
                f"'import': {state.fixture_server.request_count('/semantic-resource-import.css')}"
                "}",
            )
            assert_equal(resource.get("type"), resource_type, f"resource type for {url}")
            assert_equal(resource.get("mimeType"), mime_type, f"resource MIME type for {url}")

        return {
            "frame": {"id": frame["id"], "url": frame["url"]},
            "resources": [
                {"url": url, "type": by_url[url]["type"], "mimeType": by_url[url]["mimeType"]}
                for url in expected
            ],
            "serverRequestCounts": {
                "stylesheet": state.fixture_server.request_count("/semantic-resource.css"),
                "import": state.fixture_server.request_count("/semantic-resource-import.css"),
            },
        }


async def _resource_source_search(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        document_url = f"{state.fixture}/semantic-resource-page"
        stylesheet_url = f"{state.fixture}/semantic-resource.css"
        script_url = f"{state.fixture}/semantic-resource.js"
        await cdp.send("Page.enable")
        await page.goto(document_url, wait_until="load", timeout=10_000)
        frame = (await cdp.send("Page.getResourceTree"))["frameTree"]["frame"]

        script_matches = (
            await cdp.send(
                "Page.searchInResource",
                {
                    "frameId": frame["id"],
                    "url": script_url,
                    "query": "__semanticResourceScriptToken",
                },
            )
        )["result"]
        assert_equal(len(script_matches), 1, "script source search match count")
        _require(
            "__semanticResourceScriptToken" in script_matches[0]["lineContent"],
            f"script source search line content: {script_matches}",
        )
        case_insensitive_matches = (
            await cdp.send(
                "Page.searchInResource",
                {
                    "frameId": frame["id"],
                    "url": script_url,
                    "query": "__semanticresourcescripttoken",
                    "caseSensitive": False,
                },
            )
        )["result"]
        assert_equal(
            len(case_insensitive_matches),
            1,
            "case-insensitive script source search match count",
        )
        regex_matches = (
            await cdp.send(
                "Page.searchInResource",
                {
                    "frameId": frame["id"],
                    "url": script_url,
                    "query": "semanticResourceScript(Token|Missing)",
                    "caseSensitive": True,
                    "isRegex": True,
                },
            )
        )["result"]
        assert_equal(len(regex_matches), 1, "regex script source search match count")
        stylesheet_matches = (
            await cdp.send(
                "Page.searchInResource",
                {
                    "frameId": frame["id"],
                    "url": stylesheet_url,
                    "query": "rgb(1, 2, 3)",
                },
            )
        )["result"]
        assert_equal(len(stylesheet_matches), 1, "stylesheet source search match count")
        document_matches = (
            await cdp.send(
                "Page.searchInResource",
                {
                    "frameId": frame["id"],
                    "url": document_url,
                    "query": "semantic original document token",
                },
            )
        )["result"]
        assert_equal(len(document_matches), 1, "document source search match count")
        await page.evaluate(
            "document.body.append(Object.assign(document.createElement('p'), {textContent: 'semantic-live-only-token'}))"
        )
        live_matches = (
            await cdp.send(
                "Page.searchInResource",
                {"frameId": frame["id"], "url": document_url, "query": "semantic-live-only-token"},
            )
        )["result"]
        assert_equal(live_matches, [], "resource search excludes live DOM-only text")
        return {
            "frame": {"id": frame["id"], "url": frame["url"]},
            "matches": {
                "script": script_matches,
                "caseInsensitive": case_insensitive_matches,
                "regex": regex_matches,
                "stylesheet": stylesheet_matches,
                "document": document_matches,
                "liveDomOnly": live_matches,
            },
        }


async def _xml_runtime_document_shape(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, _cdp):
        document_url = f"{state.fixture}/semantic-document.xml"
        await page.goto(document_url, wait_until="load", timeout=10_000)
        runtime = await page.evaluate(
            """() => ({
              contentType: document.contentType,
              constructorName: document.constructor.name,
              rootName: document.documentElement.nodeName,
              rootNamespace: document.documentElement.namespaceURI,
              sourceParentId: document.querySelector('semantic-root')?.parentElement?.id,
              sourceDisplay:
                getComputedStyle(document.querySelector('#webkit-xml-viewer-source-xml')).display,
              childText: document.querySelector('semantic-child')?.textContent,
              prettyPrintIncludesSource:
                document.querySelector('.pretty-print')?.textContent.includes('semantic-root') === true,
            })"""
        )
        assert_equal(
            runtime,
            {
                "contentType": "application/xml",
                "constructorName": "XMLDocument",
                "rootName": "html",
                "rootNamespace": "http://www.w3.org/1999/xhtml",
                "sourceParentId": "webkit-xml-viewer-source-xml",
                "sourceDisplay": "none",
                "childText": "xml-ready",
                "prettyPrintIncludesSource": True,
            },
            "Chromium XML runtime document shape",
        )
        return runtime


async def _xml_resource_tree_mime(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        document_url = f"{state.fixture}/semantic-document.xml"
        await cdp.send("Page.enable")
        await page.goto(document_url, wait_until="load", timeout=10_000)
        frame = (await cdp.send("Page.getResourceTree"))["frameTree"]["frame"]
        assert_equal(frame.get("mimeType"), "application/xml", "XML frame MIME type")
        assert_equal(frame.get("url"), document_url, "XML frame URL")
        return {"frameUrl": frame.get("url"), "frameMimeType": frame.get("mimeType")}


async def _isolated_world_navigation_restore(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        events = attach_cdp_event_collector(cdp, ["Runtime.executionContextCreated"])
        await cdp.send("Runtime.enable")
        await cdp.send("Page.enable")
        await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        frame_id = (await cdp.send("Page.getFrameTree"))["frameTree"]["frame"]["id"]
        await cdp.send(
            "Page.addScriptToEvaluateOnNewDocument",
            {"source": "//# sourceURL=semantic-utility.js", "worldName": "semantic-utility"},
        )
        created = await cdp.send(
            "Page.createIsolatedWorld",
            {"frameId": frame_id, "worldName": "semantic-utility", "grantUniveralAccess": True},
        )
        first_context_id = created["executionContextId"]
        first_context = next(
            event["params"]["context"]
            for event in reversed(events)
            if event["params"].get("context", {}).get("name") == "semantic-utility"
        )
        await cdp.send(
            "Runtime.evaluate",
            {
                "contextId": first_context_id,
                "expression": "globalThis.__semanticIsolated = 'first'; location.href",
                "returnByValue": True,
            },
        )
        assert_equal(
            await page.evaluate("globalThis.__semanticIsolated"),
            None,
            "isolated world value absent from main world",
        )

        event_offset = len(events)
        await page.goto(f"{state.fixture}/history-a?isolated=1", wait_until="load", timeout=10_000)
        try:
            await wait_until(
                lambda: any(
                    event["params"].get("context", {}).get("name") == "semantic-utility"
                    for event in events[event_offset:]
                ),
                "recreated semantic-utility execution context",
            )
        except SmokeError as error:
            contexts = [event["params"].get("context", {}) for event in events]
            raise SmokeError(f"{error}; observed execution contexts={contexts}") from error
        recreated = next(
            event["params"]["context"]
            for event in reversed(events[event_offset:])
            if event["params"].get("context", {}).get("name") == "semantic-utility"
        )
        second_context_id = recreated["id"]
        _require(
            recreated.get("uniqueId") != first_context.get("uniqueId"),
            f"isolated world navigation reused uniqueId: first={first_context}, second={recreated}",
        )
        second_eval = await cdp.send(
            "Runtime.evaluate",
            {
                "contextId": second_context_id,
                "expression": "({url: location.href, marker: globalThis.__semanticIsolated})",
                "returnByValue": True,
            },
        )
        second_value = second_eval["result"]["value"]
        assert_equal(second_value["url"], f"{state.fixture}/history-a?isolated=1", "recreated isolated world URL")
        assert_equal(second_value.get("marker"), None, "recreated isolated world has fresh global")
        assert_equal(
            await page.evaluate("globalThis.__semanticIsolated"),
            None,
            "recreated isolated world remains separate",
        )
        return {
            "firstExecutionContextId": first_context_id,
            "secondExecutionContextId": second_context_id,
            "numericIdReused": first_context_id == second_context_id,
            "uniqueIds": [first_context.get("uniqueId"), recreated.get("uniqueId")],
            "secondWorldValue": second_value,
        }


async def _event_source_runtime_and_network(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, cdp):
        events = attach_cdp_event_collector(
            cdp,
            ["Network.requestWillBeSent", "Network.responseReceived", "Network.loadingFinished"],
        )
        await cdp.send("Network.enable")
        await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        event_source_url = f"{state.fixture}/semantic-event-source"
        runtime_value = await page.evaluate(
            """url => new Promise((resolve, reject) => {
              const source = new EventSource(url);
              const timer = setTimeout(() => {
                source.close();
                reject(new Error('EventSource timeout'));
              }, 5000);
              source.addEventListener('semantic', event => {
                clearTimeout(timer);
                source.close();
                resolve({data: event.data, readyState: source.readyState});
              });
              source.onerror = () => {
                clearTimeout(timer);
                source.close();
                reject(new Error('EventSource error'));
              };
            })""",
            event_source_url,
        )
        assert_equal(runtime_value, {"data": "event-source-ready", "readyState": 2}, "EventSource runtime result")
        await wait_until(
            lambda: any(
                event["method"] == "Network.responseReceived"
                and event["params"].get("response", {}).get("url") == event_source_url
                for event in events
            ),
            "EventSource Network.responseReceived",
        )
        response_event = next(
            event
            for event in events
            if event["method"] == "Network.responseReceived"
            and event["params"].get("response", {}).get("url") == event_source_url
        )
        assert_equal(response_event["params"].get("type"), "EventSource", "EventSource Network resource type")
        request_id = response_event["params"]["requestId"]
        await wait_until(
            lambda: any(
                event["method"] == "Network.loadingFinished"
                and event["params"].get("requestId") == request_id
                for event in events
            ),
            "EventSource Network.loadingFinished",
        )
        event_methods = [
            event["method"]
            for event in events
            if event["params"].get("requestId") == request_id
        ]
        assert_equal(
            event_methods,
            [
                "Network.requestWillBeSent",
                "Network.responseReceived",
                "Network.loadingFinished",
            ],
            "EventSource Network event order",
        )
        return {
            "runtime": runtime_value,
            "requestId": request_id,
            "resourceType": response_event["params"]["type"],
            "eventMethods": event_methods,
        }


async def _http_cache_event_and_reuse(state: SmokeState) -> dict[str, Any]:
    cache_route = "/semantic-cache.js"
    state.fixture_server.reset_request_count(cache_route)
    async with _isolated_page(state) as (_context, page, cdp):
        events = attach_cdp_event_collector(
            cdp,
            [
                "Network.requestWillBeSent",
                "Network.responseReceived",
                "Network.requestServedFromCache",
                "Network.loadingFinished",
            ],
        )
        await cdp.send("Network.enable")
        await page.goto(f"{state.fixture}/semantic-cache-page", wait_until="load", timeout=10_000)
        first_value = await page.evaluate("globalThis.__semanticCacheRequest")
        await page.reload(wait_until="load", timeout=10_000)
        second_value = await page.evaluate("globalThis.__semanticCacheRequest")
        cache_url = f"{state.fixture}{cache_route}"
        request_ids = [
            event["params"]["requestId"]
            for event in events
            if event["method"] == "Network.requestWillBeSent"
            and event["params"].get("request", {}).get("url") == cache_url
        ]
        assert_equal(len(request_ids), 2, "cacheable script request event count")
        await wait_until(
            lambda: any(
                event["method"] == "Network.requestServedFromCache"
                and event["params"].get("requestId") == request_ids[-1]
                for event in events
            ),
            "second script Network.requestServedFromCache",
        )
        server_requests = state.fixture_server.request_count(cache_route)
        assert_equal(server_requests, 1, "cacheable script server request count")
        assert_equal(first_value, 1, "first cacheable script body")
        assert_equal(second_value, 1, "cached script body")
        return {
            "requestIds": request_ids,
            "serverRequestCount": server_requests,
            "scriptValues": [first_value, second_value],
            "servedFromCacheRequestId": request_ids[-1],
        }


async def _cache_storage_url_normalization(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, _cdp):
        await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        observed = await page.evaluate(
            """async () => {
              const cacheName = 'semantic-cache-storage';
              await caches.delete(cacheName);
              const cache = await caches.open(cacheName);
              await cache.put(
                '/semantic-cache-storage-entry#put-fragment',
                new Response('cache-storage-body', {
                  headers: {'content-type': 'text/plain'},
                })
              );
              const keys = await cache.keys();
              const matched = await cache.match(
                new URL('/semantic-cache-storage-entry#match-fragment', location.href).href
              );
              const matchedBody = matched ? await matched.text() : null;
              const deleted = await cache.delete('/semantic-cache-storage-entry#delete-fragment');
              const remaining = await cache.keys();
              await caches.delete(cacheName);
              return {
                keyUrls: keys.map(request => request.url),
                matchedBody,
                deleted,
                remainingCount: remaining.length,
              };
            }"""
        )
        assert_equal(
            observed,
            {
                "keyUrls": [
                    f"{state.fixture}/semantic-cache-storage-entry#put-fragment"
                ],
                "matchedBody": "cache-storage-body",
                "deleted": True,
                "remainingCount": 0,
            },
            "CacheStorage normalized request URLs",
        )
        return observed


async def _computed_style_map_updates(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, _cdp):
        await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        observed = await page.evaluate(
            """() => {
              const element = document.querySelector('main');
              element.style.display = 'block';
              element.style.color = 'rgb(1, 2, 3)';
              element.style.opacity = '0.25';
              const first = element.computedStyleMap();
              const initial = {
                display: first.get('display').toString(),
                color: first.get('color').toString(),
                opacity: first.get('opacity').toString(),
              };
              element.style.color = 'rgb(4, 5, 6)';
              const updated = element.computedStyleMap();
              return {
                initial,
                updatedColor: updated.get('color').toString(),
                valueConstructors: {
                  display: first.get('display').constructor.name,
                  color: first.get('color').constructor.name,
                  opacity: first.get('opacity').constructor.name,
                },
              };
            }"""
        )
        assert_equal(
            observed["initial"],
            {"display": "block", "color": "rgb(1, 2, 3)", "opacity": "0.25"},
            "computedStyleMap initial values",
        )
        assert_equal(
            observed["updatedColor"],
            "rgb(4, 5, 6)",
            "computedStyleMap updated color",
        )
        assert_equal(
            observed["valueConstructors"],
            {
                "display": "CSSKeywordValue",
                "color": "CSSStyleValue",
                "opacity": "CSSUnitValue",
            },
            "computedStyleMap value constructors",
        )
        return observed


async def _view_transition_script_lifecycle(state: SmokeState) -> dict[str, Any]:
    async with _isolated_page(state) as (_context, page, _cdp):
        await page.goto(f"{state.fixture}/plain", wait_until="load", timeout=10_000)
        observed = await page.evaluate(
            """async () => {
              if (typeof document.startViewTransition !== 'function') {
                throw new Error('document.startViewTransition is unavailable');
              }
              const order = [];
              const transition = document.startViewTransition(() => {
                order.push('callback');
                document.querySelector('main').textContent = 'transition updated';
              });
              const update = transition.updateCallbackDone.then(() => {
                order.push('updateCallbackDone');
                return 'updateCallbackDone';
              });
              const ready = transition.ready.then(() => {
                order.push('ready');
                return 'ready';
              });
              const finished = transition.finished.then(() => {
                order.push('finished');
                return 'finished';
              });
              const promises = await Promise.all([update, ready, finished]);
              return {
                order,
                promises,
                text: document.querySelector('main').textContent,
              };
            }"""
        )
        assert_equal(
            observed["promises"],
            ["updateCallbackDone", "ready", "finished"],
            "View Transition promise results",
        )
        assert_equal(observed["text"], "transition updated", "View Transition DOM update")
        assert_equal(observed["order"][0], "callback", "View Transition callback order")
        assert_equal(observed["order"][-1], "finished", "View Transition terminal order")
        assert_equal(
            set(observed["order"]),
            {"callback", "updateCallbackDone", "ready", "finished"},
            "View Transition lifecycle labels",
        )
        return observed
