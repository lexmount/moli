from __future__ import annotations

import asyncio
import json
import urllib.request
from dataclasses import dataclass
from typing import Any, Awaitable, Callable
from urllib.parse import quote

from ..assertions import SmokeError, assert_equal, record_contract, wait_until
from ..progress import await_with_progress
from ..raw_cdp import RawCdpClient, RawCdpError, connect_raw_cdp


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
        RawSemanticContract(
            "raw_cdp_contract_target_handler_access_and_flatten",
            "Browser-level auto-attach requires flattened sessions, while a Tab target handler cannot invoke browser-only Target commands.",
            "Chromium TargetHandler access modes and browser-level AutoAttacher contract",
            [
                "Target.attachToBrowserTarget",
                "Target.setAutoAttach",
                "Target.createTarget forTab",
                "Target.attachToTarget",
                "Target.getBrowserContexts",
                "Target.autoAttachRelated",
            ],
            _target_handler_access_and_flatten,
        ),
        RawSemanticContract(
            "raw_cdp_contract_repeated_tab_auto_attach_reconciles_filter",
            "Repeating Target.setAutoAttach on a Tab target re-runs discovery against existing child targets, attaches a Page that became newly eligible, and does not duplicate that attachment on an identical third call.",
            "Chromium DevToolsAgentHostImpl::AutoAttach and TargetAutoAttacher reconciliation",
            [
                "Target.createTarget forTab",
                "Target.attachToTarget",
                "Target.setAutoAttach x3",
                "Runtime.evaluate",
            ],
            _repeated_tab_auto_attach_reconciles_filter,
        ),
        RawSemanticContract(
            "raw_cdp_contract_create_target_foreground_activation",
            "Target.createTarget activates the new target by default, background=true preserves the current foreground target, and Target.activateTarget changes Page visibility.",
            "Chromium DevToolsProtocolTest.CreateTargetWithFocus and Target.createTarget",
            [
                "Target.createBrowserContext",
                "Target.createTarget x3",
                "Target.attachToTarget x3",
                "Runtime.evaluate document.visibilityState",
                "Target.activateTarget",
            ],
            _create_target_foreground_activation,
        ),
        RawSemanticContract(
            "raw_cdp_contract_tab_session_stays_with_its_tab",
            "A Tab target session and its auto-attached Page child remain bound to that tab when another tab becomes foreground.",
            "Chromium DevTools TabTarget and child AgentHost ownership",
            [
                "Target.createTarget forTab x2",
                "Target.attachToTarget",
                "Target.setAutoAttach",
                "Target.getTargetInfo",
                "Target.activateTarget",
                "Runtime.evaluate",
            ],
            _tab_session_stays_with_its_tab,
        ),
        RawSemanticContract(
            "raw_cdp_contract_screencast_visibility_follows_foreground",
            "A foreground Page screencast becomes hidden when another target is activated and visible again when that target closes.",
            "Chromium WebContents visibility and Page screencast lifecycle",
            [
                "Target.createBrowserContext",
                "Target.createTarget x2",
                "Target.attachToTarget",
                "Page.startScreencast",
                "Target.closeTarget",
                "Page.stopScreencast",
            ],
            _screencast_visibility_follows_foreground,
        ),
        RawSemanticContract(
            "raw_cdp_contract_closed_default_target_stays_closed",
            "The default Page returned by remote-debugging discovery is a real target: closing it removes it from subsequent discovery instead of recreating a fabricated descriptor.",
            "Chromium DevToolsAgentHost discovery and /json/close lifecycle",
            ["GET /json/list", "GET /json/close/{targetId}", "GET /json/list"],
            _closed_default_target_stays_closed,
        ),
    )


def _require(condition: bool, label: str) -> None:
    if not condition:
        raise SmokeError(label)

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


async def _raw_command_error(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
    timeout: float = 10.0,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    message_id = await client.send(method, params, session_id=session_id)
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out waiting for failing {method} response id={message_id}; "
                f"seen={seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") != message_id:
            continue
        error = message.get("error")
        if not isinstance(error, dict):
            raise SmokeError(f"{method} unexpectedly succeeded: {message}")
        return error, seen


def _read_json_list_url(url: str) -> list[dict[str, Any]]:
    with urllib.request.urlopen(url, timeout=2) as response:
        value = json.loads(response.read().decode("utf-8"))
    if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
        raise SmokeError(f"discovery returned a non-target list from {url}: {value!r}")
    return value


async def _discovered_targets(endpoint: str) -> list[dict[str, Any]]:
    return await asyncio.to_thread(
        _read_json_list_url,
        f"{endpoint.rstrip('/')}/json/list",
    )


def _read_text_url(url: str) -> str:
    with urllib.request.urlopen(url, timeout=2) as response:
        return response.read().decode("utf-8")


async def _raw_wait_for_event(
    client: RawCdpClient,
    method: str,
    predicate: Callable[[dict[str, Any]], bool],
    seen: list[dict[str, Any]],
    *,
    session_id: str | None = None,
    timeout: float = 10.0,
) -> dict[str, Any]:
    def matches(message: dict[str, Any]) -> bool:
        return (
            message.get("method") == method
            and (session_id is None or message.get("sessionId") == session_id)
            and predicate(message.get("params", {}))
        )

    for message in seen:
        if matches(message):
            return message
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out waiting for {method}; seen={seen[-20:]}")
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if matches(message):
            return message


async def _ignore_raw_error(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any],
    *,
    session_id: str | None = None,
) -> None:
    try:
        await _raw_command(client, method, params, session_id=session_id)
    except Exception:
        pass


async def _target_handler_access_and_flatten(
    endpoint: str,
    _fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_id: str | None = None
    browser_session_id: str | None = None
    stage = "attach browser target"
    try:
        browser_attach, _ = await _raw_command(client, "Target.attachToBrowserTarget")
        browser_session_id = browser_attach["sessionId"]

        flatten_errors: list[dict[str, Any]] = []
        for params in (
            {"autoAttach": True, "waitForDebuggerOnStart": False},
            {
                "autoAttach": True,
                "waitForDebuggerOnStart": False,
                "flatten": False,
            },
        ):
            stage = "reject non-flattened browser auto-attach"
            error, _ = await _raw_command_error(
                client,
                "Target.setAutoAttach",
                params,
                session_id=browser_session_id,
            )
            assert_equal(error.get("code"), -32602, "browser auto-attach error code")
            assert_equal(
                error.get("message"),
                "Only flatten protocol is supported with browser level auto-attach",
                "browser auto-attach error message",
            )
            flatten_errors.append(error)

        stage = "enable flattened browser auto-attach"
        await _raw_command(
            client,
            "Target.setAutoAttach",
            {
                "autoAttach": True,
                "waitForDebuggerOnStart": False,
                "flatten": True,
                "filter": [
                    {"type": "page", "exclude": True},
                    {"type": "tab", "exclude": True},
                    {},
                ],
            },
            session_id=browser_session_id,
        )

        context, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context["browserContextId"]
        created, _ = await _raw_command(
            client,
            "Target.createTarget",
            {
                "url": "about:blank",
                "browserContextId": browser_context_id,
                "forTab": True,
            },
        )
        target_id = created["targetId"]
        tab_attach, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        tab_session_id = tab_attach["sessionId"]

        stage = "reject browser-only command on Tab handler"
        access_error, _ = await _raw_command_error(
            client,
            "Target.getBrowserContexts",
            session_id=tab_session_id,
        )
        assert_equal(access_error.get("code"), -32000, "Tab handler access error code")
        assert_equal(access_error.get("message"), "Not allowed", "Tab handler access error")

        stage = "reject browser-only related auto-attach on Tab handler"
        related_error, _ = await _raw_command_error(
            client,
            "Target.autoAttachRelated",
            {
                "targetId": target_id,
                "waitForDebuggerOnStart": False,
            },
            session_id=tab_session_id,
        )
        assert_equal(
            related_error,
            {
                "code": -32000,
                "message": "Target.autoAttachRelated is only supported on the Browser target",
            },
            "Tab related auto-attach access error",
        )
        return {
            "flattenErrorCodes": [error["code"] for error in flatten_errors],
            "tabAccessError": access_error,
            "tabAutoAttachRelatedError": related_error,
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        if browser_session_id is not None:
            await _ignore_raw_error(
                client,
                "Target.setAutoAttach",
                {
                    "autoAttach": False,
                    "waitForDebuggerOnStart": False,
                    "flatten": True,
                },
                session_id=browser_session_id,
            )
        if target_id is not None:
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()

async def _repeated_tab_auto_attach_reconciles_filter(
    endpoint: str,
    _fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_id: str | None = None
    stage = "create Tab target"
    try:
        context, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context["browserContextId"]
        created, _ = await _raw_command(
            client,
            "Target.createTarget",
            {
                "url": "about:blank",
                "browserContextId": browser_context_id,
                "forTab": True,
            },
        )
        target_id = created["targetId"]
        attached_tab, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        tab_session_id = attached_tab["sessionId"]

        stage = "exclude existing child Page"
        _, excluded_seen = await _raw_command(
            client,
            "Target.setAutoAttach",
            {
                "autoAttach": True,
                "waitForDebuggerOnStart": False,
                "flatten": True,
                "filter": [{"type": "page", "exclude": True}],
            },
            session_id=tab_session_id,
        )
        excluded_attachments = [
            message
            for message in excluded_seen
            if message.get("method") == "Target.attachedToTarget"
        ]
        assert_equal(excluded_attachments, [], "excluded existing Page attachments")

        stage = "reconcile newly eligible existing child Page"
        _, eligible_seen = await _raw_command(
            client,
            "Target.setAutoAttach",
            {
                "autoAttach": True,
                "waitForDebuggerOnStart": False,
                "flatten": True,
                "filter": [{"type": "page"}],
            },
            session_id=tab_session_id,
        )
        eligible_attachments = [
            message
            for message in eligible_seen
            if message.get("method") == "Target.attachedToTarget"
            and message.get("sessionId") == tab_session_id
            and message.get("params", {}).get("targetInfo", {}).get("type") == "page"
        ]
        assert_equal(len(eligible_attachments), 1, "newly eligible Page attachment count")
        child_session_id = eligible_attachments[0]["params"]["sessionId"]

        stage = "repeat identical auto-attach without duplication"
        _, repeated_seen = await _raw_command(
            client,
            "Target.setAutoAttach",
            {
                "autoAttach": True,
                "waitForDebuggerOnStart": False,
                "flatten": True,
                "filter": [{"type": "page"}],
            },
            session_id=tab_session_id,
        )
        repeated_attachments = [
            message
            for message in repeated_seen
            if message.get("method") == "Target.attachedToTarget"
        ]
        assert_equal(repeated_attachments, [], "duplicate Page attachments")

        evaluation, _ = await _raw_command(
            client,
            "Runtime.evaluate",
            {"expression": "location.href", "returnByValue": True},
            session_id=child_session_id,
        )
        assert_equal(
            evaluation.get("result", {}).get("value"),
            "about:blank",
            "reconciled child Page session",
        )
        return {
            "newAttachments": len(eligible_attachments),
            "duplicateAttachments": len(repeated_attachments),
            "childSessionUrl": evaluation.get("result", {}).get("value"),
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


async def _create_target_foreground_activation(
    endpoint: str,
    _fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_ids: list[str] = []
    session_ids: list[str] = []
    stage = "create browser context"
    try:
        context, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context["browserContextId"]
        for index, background in ((1, None), (2, None), (3, True)):
            params: dict[str, Any] = {
                "url": f"about:blank#foreground-{index}",
                "browserContextId": browser_context_id,
            }
            if background is not None:
                params["background"] = background
            created, _ = await _raw_command(client, "Target.createTarget", params)
            target_ids.append(created["targetId"])

        for target_id in target_ids:
            attached, _ = await _raw_command(
                client,
                "Target.attachToTarget",
                {"targetId": target_id, "flatten": True},
            )
            session_ids.append(attached["sessionId"])

        async def visibility_states() -> list[str | None]:
            states: list[str | None] = []
            for session_id in session_ids:
                evaluation, _ = await _raw_command(
                    client,
                    "Runtime.evaluate",
                    {
                        "expression": "document.visibilityState",
                        "returnByValue": True,
                    },
                    session_id=session_id,
                )
                states.append(evaluation.get("result", {}).get("value"))
            return states

        stage = "observe default foreground and explicit background"
        initial_visibility = await visibility_states()
        assert_equal(
            initial_visibility,
            ["hidden", "visible", "hidden"],
            "created target visibility",
        )

        stage = "activate first target"
        await _raw_command(
            client,
            "Target.activateTarget",
            {"targetId": target_ids[0]},
        )
        activated_visibility: list[str | None] = []

        async def first_target_is_foreground() -> bool:
            nonlocal activated_visibility
            activated_visibility = await visibility_states()
            return activated_visibility == ["visible", "hidden", "hidden"]

        await wait_until(
            first_target_is_foreground,
            "activated target Page visibility",
        )
        return {
            "initialVisibility": initial_visibility,
            "activatedVisibility": activated_visibility,
            "explicitBackgroundTarget": target_ids[2],
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        for target_id in reversed(target_ids):
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()


async def _tab_session_stays_with_its_tab(
    endpoint: str,
    _fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    tab_target_ids: list[str] = []
    stage = "create and attach first Tab target"
    try:
        context, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context["browserContextId"]
        first, _ = await _raw_command(
            client,
            "Target.createTarget",
            {
                "url": "about:blank#stable-tab-one",
                "browserContextId": browser_context_id,
                "forTab": True,
            },
        )
        first_tab_target_id = first["targetId"]
        tab_target_ids.append(first_tab_target_id)
        attached, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": first_tab_target_id, "flatten": True},
        )
        tab_session_id = attached["sessionId"]

        stage = "auto-attach the first Tab child Page"
        _, auto_attach_seen = await _raw_command(
            client,
            "Target.setAutoAttach",
            {
                "autoAttach": True,
                "waitForDebuggerOnStart": False,
                "flatten": True,
                "filter": [{"type": "page"}],
            },
            session_id=tab_session_id,
        )
        child_attachments = [
            message
            for message in auto_attach_seen
            if message.get("method") == "Target.attachedToTarget"
            and message.get("sessionId") == tab_session_id
            and message.get("params", {}).get("targetInfo", {}).get("type") == "page"
        ]
        assert_equal(len(child_attachments), 1, "first Tab child attachment count")
        first_child_target_id = child_attachments[0]["params"]["targetInfo"]["targetId"]
        first_child_session_id = child_attachments[0]["params"]["sessionId"]

        stage = "activate a different Tab target"
        second, creation_seen = await _raw_command(
            client,
            "Target.createTarget",
            {
                "url": "about:blank#stable-tab-two",
                "browserContextId": browser_context_id,
                "forTab": True,
            },
        )
        second_tab_target_id = second["targetId"]
        tab_target_ids.append(second_tab_target_id)

        _, activation_seen = await _raw_command(
            client,
            "Target.activateTarget",
            {"targetId": second_tab_target_id},
        )
        tab_info, info_seen = await _raw_command(
            client,
            "Target.getTargetInfo",
            session_id=tab_session_id,
        )
        assert_equal(
            tab_info.get("targetInfo", {}).get("targetId"),
            first_tab_target_id,
            "attached Tab target identity",
        )
        child_url, evaluation_seen = await _raw_command(
            client,
            "Runtime.evaluate",
            {"expression": "location.href", "returnByValue": True},
            session_id=first_child_session_id,
        )
        assert_equal(
            child_url.get("result", {}).get("value"),
            "about:blank#stable-tab-one",
            "attached child Page identity",
        )
        leaked_attachments = [
            message
            for message in [
                *creation_seen,
                *activation_seen,
                *info_seen,
                *evaluation_seen,
            ]
            if message.get("method") == "Target.attachedToTarget"
            and message.get("sessionId") == tab_session_id
        ]
        assert_equal(leaked_attachments, [], "cross-Tab child attachments")
        return {
            "tabTargetIdStable": first_tab_target_id,
            "childTargetIdStable": first_child_target_id,
            "childUrlAfterOtherTabActivation": child_url.get("result", {}).get("value"),
            "crossTabAttachments": len(leaked_attachments),
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        for target_id in reversed(tab_target_ids):
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()


async def _screencast_visibility_follows_foreground(
    endpoint: str,
    _fixture: str,
) -> dict[str, Any]:
    client = await connect_raw_cdp(endpoint)
    browser_context_id: str | None = None
    target_ids: list[str] = []
    screencast_session_id: str | None = None
    stage = "create and attach foreground Page"
    try:
        context, _ = await _raw_command(client, "Target.createBrowserContext")
        browser_context_id = context["browserContextId"]
        first, _ = await _raw_command(
            client,
            "Target.createTarget",
            {
                "url": "about:blank#screencast-visible",
                "browserContextId": browser_context_id,
            },
        )
        first_target_id = first["targetId"]
        target_ids.append(first_target_id)
        attached, _ = await _raw_command(
            client,
            "Target.attachToTarget",
            {"targetId": first_target_id, "flatten": True},
        )
        screencast_session_id = attached["sessionId"]

        stage = "start visible screencast"
        _, start_seen = await _raw_command(
            client,
            "Page.startScreencast",
            {"format": "jpeg", "quality": 40, "everyNthFrame": 1000},
            session_id=screencast_session_id,
        )
        initial_event = await _raw_wait_for_event(
            client,
            "Page.screencastVisibilityChanged",
            lambda params: params.get("visible") is True,
            start_seen,
            session_id=screencast_session_id,
        )

        stage = "demote screencast Page by creating foreground target"
        second, creation_seen = await _raw_command(
            client,
            "Target.createTarget",
            {
                "url": "about:blank#screencast-new-foreground",
                "browserContextId": browser_context_id,
            },
        )
        second_target_id = second["targetId"]
        target_ids.append(second_target_id)
        hidden_event = await _raw_wait_for_event(
            client,
            "Page.screencastVisibilityChanged",
            lambda params: params.get("visible") is False,
            creation_seen,
            session_id=screencast_session_id,
        )

        stage = "close foreground target and promote screencast Page"
        _, close_seen = await _raw_command(
            client,
            "Target.closeTarget",
            {"targetId": second_target_id},
        )
        target_ids.remove(second_target_id)
        visible_event = await _raw_wait_for_event(
            client,
            "Page.screencastVisibilityChanged",
            lambda params: params.get("visible") is True,
            close_seen,
            session_id=screencast_session_id,
        )
        return {
            "initialVisible": initial_event.get("params", {}).get("visible"),
            "hiddenAfterForegroundCreation": hidden_event.get("params", {}).get("visible"),
            "visibleAfterForegroundClose": visible_event.get("params", {}).get("visible"),
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error
    finally:
        if screencast_session_id is not None:
            await _ignore_raw_error(
                client,
                "Page.stopScreencast",
                {},
                session_id=screencast_session_id,
            )
        for target_id in reversed(target_ids):
            await _ignore_raw_error(client, "Target.closeTarget", {"targetId": target_id})
        if browser_context_id is not None:
            await _ignore_raw_error(
                client,
                "Target.disposeBrowserContext",
                {"browserContextId": browser_context_id},
            )
        await client.websocket.close()


async def _closed_default_target_stays_closed(
    endpoint: str,
    _fixture: str,
) -> dict[str, Any]:
    stage = "discover default Page target"
    try:
        targets = await _discovered_targets(endpoint)
        page_targets = [target for target in targets if target.get("type") == "page"]
        assert_equal(len(page_targets), 1, "default Page target count before close")
        target_id = page_targets[0].get("id")
        if not isinstance(target_id, str) or not target_id:
            raise SmokeError(f"default Page target had no id: {page_targets[0]!r}")

        stage = "close default target through remote-debugging HTTP"
        close_text = await asyncio.to_thread(
            _read_text_url,
            f"{endpoint.rstrip('/')}/json/close/{quote(target_id, safe='')}",
        )

        remaining: list[dict[str, Any]] = []

        async def closed_target_is_absent() -> bool:
            nonlocal remaining
            remaining = await _discovered_targets(endpoint)
            return all(target.get("id") != target_id for target in remaining)

        stage = "verify closed default is not re-advertised"
        await wait_until(closed_target_is_absent, "closed default target absent from discovery")
        second_read = await _discovered_targets(endpoint)
        _require(
            all(target.get("id") != target_id for target in second_read),
            f"closed default target was recreated: {second_read}",
        )
        return {
            "closedTargetId": target_id,
            "closeResponse": close_text,
            "remainingTargetIds": [target.get("id") for target in second_read],
        }
    except Exception as error:
        raise SmokeError(f"{stage}: {type(error).__name__}: {error}") from error


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
