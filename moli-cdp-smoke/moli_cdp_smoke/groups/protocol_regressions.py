from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..raw_cdp import RawCdpClient, connect_raw_cdp


@dataclass(frozen=True)
class RunningPage:
    browser_context_id: str
    target_id: str
    session_id: str


async def run_debugger_breakpoints_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    client = await connect_raw_cdp(endpoint)
    page: RunningPage | None = None
    try:
        page = await _create_running_page(client, f"{fixture}/plain")
        session_id = page.session_id

        enable_id = await client.send("Debugger.enable", session_id=session_id)
        await client.recv_until_id(enable_id, timeout=5)

        source_url = "moli-running-debugger-probe.js"
        source = (
            "function moliRunningDebuggerProbe(value) {\n"
            "  const next = value + 1;\n"
            "  return next;\n"
            "}\n"
            "window.moliRunningDebuggerProbe = moliRunningDebuggerProbe;\n"
            f"//# sourceURL={source_url}"
        )
        evaluate_id = await client.send(
            "Runtime.evaluate",
            {"expression": source},
            session_id=session_id,
        )
        _evaluate, evaluate_messages = await client.recv_until_id(
            evaluate_id, timeout=5
        )
        script_parsed = _find_session_event(
            evaluate_messages,
            session_id,
            "Debugger.scriptParsed",
            url=source_url,
        )
        if script_parsed is None:
            script_parsed = await _recv_until_session_event(
                client,
                session_id,
                "Debugger.scriptParsed",
                timeout=5,
                url=source_url,
            )
        script_id = script_parsed.get("params", {}).get("scriptId")
        if not isinstance(script_id, str) or not script_id:
            raise SmokeError(f"Debugger.scriptParsed returned no scriptId: {script_parsed}")

        possible_id = await client.send(
            "Debugger.getPossibleBreakpoints",
            {
                "start": {
                    "scriptId": script_id,
                    "lineNumber": 0,
                    "columnNumber": 0,
                }
            },
            session_id=session_id,
        )
        possible, _ = await client.recv_until_id(possible_id, timeout=5)
        locations = possible.get("result", {}).get("locations")
        if not isinstance(locations, list) or not locations:
            raise SmokeError(
                "Debugger.getPossibleBreakpoints returned no locations while the page "
                f"was running: {possible}"
            )
        record(
            results,
            "raw_cdp_running_debugger_get_possible_breakpoints",
            {"locationCount": len(locations)},
        )

        set_id = await client.send(
            "Debugger.setBreakpoint",
            {
                "location": {
                    "scriptId": script_id,
                    "lineNumber": 1,
                    "columnNumber": 0,
                }
            },
            session_id=session_id,
        )
        set_response, _ = await client.recv_until_id(set_id, timeout=5)
        breakpoint_id = _required_breakpoint_id(set_response, "Debugger.setBreakpoint")
        record(results, "raw_cdp_running_debugger_set_breakpoint")

        remove_id = await client.send(
            "Debugger.removeBreakpoint",
            {"breakpointId": breakpoint_id},
            session_id=session_id,
        )
        await client.recv_until_id(remove_id, timeout=5)
        record(results, "raw_cdp_running_debugger_remove_breakpoint")

        set_by_url_id = await client.send(
            "Debugger.setBreakpointByUrl",
            {"lineNumber": 1, "urlRegex": "moli-running-debugger-.*\\.js"},
            session_id=session_id,
        )
        set_by_url, _ = await client.recv_until_id(set_by_url_id, timeout=5)
        url_breakpoint_id = _required_breakpoint_id(
            set_by_url, "Debugger.setBreakpointByUrl"
        )
        record(results, "raw_cdp_running_debugger_set_breakpoint_by_url")

        remove_url_id = await client.send(
            "Debugger.removeBreakpoint",
            {"breakpointId": url_breakpoint_id},
            session_id=session_id,
        )
        await client.recv_until_id(remove_url_id, timeout=5)

        disable_id = await client.send("Debugger.disable", session_id=session_id)
        await client.recv_until_id(disable_id, timeout=5)
    finally:
        await _dispose_page(client, page)
        await _close_client(client)


async def run_runtime_exception_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    client = await connect_raw_cdp(endpoint)
    page: RunningPage | None = None
    try:
        page = await _create_running_page(client, f"{fixture}/plain")
        session_id = page.session_id
        # Match the public CDP workflow exactly: enable Runtime on the live
        # post-navigation Inspector session immediately before scheduling the
        # asynchronous exception.
        enable_id = await client.send("Runtime.enable", session_id=session_id)
        await client.recv_until_id(enable_id, timeout=5)
        marker = "moli-smoke-async-exception"
        exceptions = await _schedule_async_exception_and_collect(
            client,
            session_id,
            marker,
            expected_session_ids={session_id},
        )
        exception_id = _runtime_exception_id(exceptions[session_id], marker)
        record(
            results,
            "raw_cdp_runtime_async_exception_thrown",
            {"exceptionId": exception_id},
        )

        peer_session_id = await _attach_to_running_page(client, page)
        disable_id = await client.send("Runtime.disable", session_id=session_id)
        await client.recv_until_id(disable_id, timeout=5)
        peer_enable_id = await client.send(
            "Runtime.enable", session_id=peer_session_id
        )
        await client.recv_until_id(peer_enable_id, timeout=5)

        enabled_peer_marker = "moli-smoke-async-exception-enabled-peer"
        peer_exceptions = await _schedule_async_exception_and_collect(
            client,
            peer_session_id,
            enabled_peer_marker,
            expected_session_ids={peer_session_id},
        )
        if session_id in peer_exceptions:
            raise SmokeError(
                "Runtime-disabled attachment received Runtime.exceptionThrown: "
                f"{peer_exceptions[session_id]}"
            )
        _runtime_exception_id(
            peer_exceptions[peer_session_id], enabled_peer_marker
        )
        record(results, "raw_cdp_runtime_exception_enabled_attachment_only")

        source_enable_id = await client.send("Runtime.enable", session_id=session_id)
        await client.recv_until_id(source_enable_id, timeout=5)
        fanout_marker = "moli-smoke-async-exception-multi-attachment"
        fanout_exceptions = await _schedule_async_exception_and_collect(
            client,
            peer_session_id,
            fanout_marker,
            expected_session_ids={session_id, peer_session_id},
        )
        for event_session_id in (session_id, peer_session_id):
            _runtime_exception_id(
                fanout_exceptions[event_session_id], fanout_marker
            )
        record(results, "raw_cdp_runtime_exception_fans_out_to_enabled_attachments")
    finally:
        await _dispose_page(client, page)
        await _close_client(client)


async def run_file_chooser_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    client = await connect_raw_cdp(endpoint)
    page: RunningPage | None = None
    try:
        page = await _create_running_page(client, f"{fixture}/plain")
        session_id = page.session_id

        install_id = await client.send(
            "Runtime.evaluate",
            {
                "expression": (
                    "document.body.insertAdjacentHTML("
                    "'beforeend', '<input id=\"moli-file-input\" type=\"file\">'"
                    "); 'installed'"
                ),
                "returnByValue": True,
            },
            session_id=session_id,
        )
        installed, _ = await client.recv_until_id(install_id, timeout=5)
        assert_equal(
            installed.get("result", {}).get("result", {}).get("value"),
            "installed",
            "file chooser fixture installation",
        )

        intercept_id = await client.send(
            "Page.setInterceptFileChooserDialog",
            {"enabled": True},
            session_id=session_id,
        )
        await client.recv_until_id(intercept_id, timeout=5)

        click_id = await client.send(
            "Runtime.evaluate",
            {
                "expression": (
                    "document.getElementById('moli-file-input').click(); 'clicked'"
                ),
                "returnByValue": True,
                "userGesture": True,
            },
            session_id=session_id,
        )
        clicked, click_messages = await client.recv_until_id(click_id, timeout=5)
        assert_equal(
            clicked.get("result", {}).get("result", {}).get("value"),
            "clicked",
            "file input activation result",
        )
        chooser = _find_session_event(
            click_messages,
            session_id,
            "Page.fileChooserOpened",
        )
        if chooser is None:
            chooser = await _recv_until_session_event(
                client,
                session_id,
                "Page.fileChooserOpened",
                timeout=6,
            )
        assert_equal(
            chooser.get("params", {}).get("mode"),
            "selectSingle",
            "Page.fileChooserOpened selection mode",
        )
        record(results, "raw_cdp_page_file_chooser_opened")
    finally:
        await _dispose_page(client, page)
        await _close_client(client)


def _required_breakpoint_id(response: dict[str, Any], command: str) -> str:
    breakpoint_id = response.get("result", {}).get("breakpointId")
    if not isinstance(breakpoint_id, str) or not breakpoint_id:
        raise SmokeError(f"{command} returned no breakpointId: {response}")
    return breakpoint_id


def _find_session_event(
    messages: list[dict[str, Any]],
    session_id: str,
    method: str,
    *,
    url: str | None = None,
) -> dict[str, Any] | None:
    for message in messages:
        if message.get("sessionId") != session_id or message.get("method") != method:
            continue
        if url is None or message.get("params", {}).get("url") == url:
            return message
    return None


async def _recv_until_session_event(
    client: RawCdpClient,
    session_id: str,
    method: str,
    *,
    timeout: float,
    url: str | None = None,
) -> dict[str, Any]:
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out waiting for {session_id} {method}; seen={seen[-20:]}"
            )
        try:
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
        except TimeoutError as error:
            raise SmokeError(
                f"timed out waiting for {session_id} {method}; seen={seen[-20:]}"
            ) from error
        seen.append(message)
        if message.get("sessionId") != session_id or message.get("method") != method:
            continue
        if url is None or message.get("params", {}).get("url") == url:
            return message


async def _create_running_page(client: RawCdpClient, url: str) -> RunningPage:
    context_id = await client.send("Target.createBrowserContext")
    context, _ = await client.recv_until_id(context_id, timeout=5)
    browser_context_id = context.get("result", {}).get("browserContextId")
    if not isinstance(browser_context_id, str) or not browser_context_id:
        raise SmokeError(f"Target.createBrowserContext returned no id: {context}")

    create_id = await client.send(
        "Target.createTarget",
        {"browserContextId": browser_context_id, "url": "about:blank"},
    )
    created, _ = await client.recv_until_id(create_id, timeout=5)
    target_id = created.get("result", {}).get("targetId")
    if not isinstance(target_id, str) or not target_id:
        raise SmokeError(f"Target.createTarget returned no id: {created}")

    attach_id = await client.send(
        "Target.attachToTarget",
        {"targetId": target_id, "flatten": True},
    )
    attached, _ = await client.recv_until_id(attach_id, timeout=5)
    session_id = attached.get("result", {}).get("sessionId")
    if not isinstance(session_id, str) or not session_id:
        raise SmokeError(f"Target.attachToTarget returned no sessionId: {attached}")

    for method in ("Runtime.enable", "Page.enable"):
        enable_id = await client.send(method, session_id=session_id)
        await client.recv_until_id(enable_id, timeout=5)

    navigate_id = await client.send(
        "Page.navigate",
        {"url": url},
        session_id=session_id,
    )
    saw_response = False
    saw_load = False
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + 10
    while not (saw_response and saw_load):
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(f"timed out creating running page; seen={seen[-20:]}")
        try:
            message = await asyncio.wait_for(client.recv(), timeout=remaining)
        except TimeoutError as error:
            raise SmokeError(
                f"timed out creating running page; seen={seen[-20:]}"
            ) from error
        seen.append(message)
        if message.get("id") == navigate_id:
            if "error" in message:
                raise SmokeError(f"Page.navigate failed: {message}")
            saw_response = True
        if (
            message.get("sessionId") == session_id
            and message.get("method") == "Page.loadEventFired"
        ):
            saw_load = True
    return RunningPage(
        browser_context_id=browser_context_id,
        target_id=target_id,
        session_id=session_id,
    )


async def _attach_to_running_page(
    client: RawCdpClient,
    page: RunningPage,
) -> str:
    attach_id = await client.send(
        "Target.attachToTarget",
        {"targetId": page.target_id, "flatten": True},
    )
    attached, _ = await client.recv_until_id(attach_id, timeout=5)
    session_id = attached.get("result", {}).get("sessionId")
    if not isinstance(session_id, str) or not session_id:
        raise SmokeError(f"Target.attachToTarget returned no peer sessionId: {attached}")
    return session_id


async def _schedule_async_exception_and_collect(
    client: RawCdpClient,
    source_session_id: str,
    marker: str,
    *,
    expected_session_ids: set[str],
) -> dict[str, dict[str, Any]]:
    evaluate_id = await client.send(
        "Runtime.evaluate",
        {
            "expression": (
                "setTimeout(function () { "
                f"throw new Error('{marker}');"
                " }, 0)"
            )
        },
        session_id=source_session_id,
    )
    evaluate, messages = await client.recv_until_id(evaluate_id, timeout=5)
    if "error" in evaluate:
        raise SmokeError(f"failed to schedule asynchronous exception: {evaluate}")

    exceptions: dict[str, dict[str, Any]] = {}

    def observe(message: dict[str, Any]) -> None:
        if message.get("method") != "Runtime.exceptionThrown":
            return
        details = message.get("params", {}).get("exceptionDetails", {})
        description = details.get("exception", {}).get("description", "")
        if marker not in description and marker not in str(details.get("text", "")):
            return
        event_session_id = message.get("sessionId")
        if isinstance(event_session_id, str):
            exceptions.setdefault(event_session_id, message)

    for message in messages:
        observe(message)

    deadline = asyncio.get_running_loop().time() + 6
    while not expected_session_ids.issubset(exceptions):
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                "timed out waiting for Runtime.exceptionThrown fan-out; "
                f"marker={marker!r} expected={sorted(expected_session_ids)!r} "
                f"received={sorted(exceptions)!r}"
            )
        try:
            observe(await asyncio.wait_for(client.recv(), timeout=remaining))
        except TimeoutError as error:
            raise SmokeError(
                "timed out waiting for Runtime.exceptionThrown fan-out; "
                f"marker={marker!r} expected={sorted(expected_session_ids)!r} "
                f"received={sorted(exceptions)!r}"
            ) from error

    quiet_deadline = asyncio.get_running_loop().time() + 0.2
    while True:
        remaining = quiet_deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            break
        try:
            observe(await asyncio.wait_for(client.recv(), timeout=remaining))
        except TimeoutError:
            break
    return exceptions


def _runtime_exception_id(exception: dict[str, Any], marker: str) -> int:
    details = exception.get("params", {}).get("exceptionDetails", {})
    exception_id = details.get("exceptionId")
    if not isinstance(exception_id, int) or exception_id <= 0:
        raise SmokeError(
            "Runtime.exceptionThrown returned no positive exceptionId: "
            f"{exception}"
        )
    description = details.get("exception", {}).get("description", "")
    if marker not in description and marker not in str(details.get("text", "")):
        raise SmokeError(
            "Runtime.exceptionThrown did not describe the thrown error: "
            f"{exception}"
        )
    return exception_id


async def _dispose_page(client: RawCdpClient, page: RunningPage | None) -> None:
    if page is None:
        return
    try:
        dispose_id = await client.send(
            "Target.disposeBrowserContext",
            {"browserContextId": page.browser_context_id},
        )
        await client.recv_until_id(dispose_id, timeout=3)
    except Exception:
        pass


async def _close_client(client: RawCdpClient) -> None:
    try:
        await asyncio.wait_for(client.websocket.close(), timeout=1)
    except Exception:
        transport = getattr(client.websocket, "transport", None)
        if transport is not None:
            transport.abort()
