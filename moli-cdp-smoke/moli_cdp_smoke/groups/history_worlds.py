from __future__ import annotations

import asyncio
from contextlib import suppress
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..raw_cdp import connect_raw_cdp


async def run_history_worlds_group(
    endpoint: str, fixture: str, results: list[dict[str, Any]]
) -> None:
    """Exercise PR #818's world boundaries through a real CDP connection."""
    client = await connect_raw_cdp(endpoint)
    browser_context: str | None = None
    session: str | None = None

    async def command(method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        message_id = await client.send(method, params, session_id=session)
        response, _ = await client.recv_until_id(message_id, timeout=10)
        if "error" in response:
            raise SmokeError(f"{method}: {response['error']!r}")
        return response["result"]

    async def evaluate(source: str, context: int | None = None) -> Any:
        params: dict[str, Any] = {"expression": source, "returnByValue": True}
        if context is not None:
            params["contextId"] = context
        response = await command("Runtime.evaluate", params)
        if "exceptionDetails" in response:
            raise SmokeError(f"history world evaluation: {response['exceptionDetails']!r}")
        return response["result"].get("value")

    try:
        browser_context = (await command("Target.createBrowserContext"))["browserContextId"]
        target = (await command("Target.createTarget", {
            "url": "about:blank", "browserContextId": browser_context,
        }))["targetId"]
        session = (await command("Target.attachToTarget", {
            "targetId": target, "flatten": True,
        }))["sessionId"]
        await command("Runtime.enable")
        await command("Page.enable")
        navigate_id = await client.send("Page.navigate", {"url": f"{fixture}/plain"}, session_id=session)
        navigation = None
        loaded = False
        async with asyncio.timeout(10):
            while navigation is None or not loaded:
                message = await client.recv()
                if message.get("id") == navigate_id:
                    if "error" in message:
                        raise SmokeError(f"Page.navigate: {message['error']!r}")
                    navigation = message["result"]
                if message.get("sessionId") == session and message.get("method") == "Page.loadEventFired":
                    loaded = True
        if navigation.get("errorText"):
            raise SmokeError(f"history world navigation: {navigation!r}")
        frame = (await command("Page.getFrameTree"))["frameTree"]["frame"]["id"]
        await evaluate("""
            history.replaceState({value: 1}, '', '#one');
            navigation.updateCurrentEntry({state: {value: 7}});
            navigation.currentEntry.expando = 'page';
            navigation.currentEntry.getState = () => 'page-override';
            globalThis.pageChanges = 0;
            navigation.addEventListener('currententrychange', () => pageChanges++);
        """)
        isolated = (await command("Page.createIsolatedWorld", {
            "frameId": frame, "worldName": "history-native-regression",
        }))["executionContextId"]
        wrappers = await evaluate("""
            [navigation.currentEntry instanceof NavigationHistoryEntry,
             navigation.currentEntry.expando === undefined,
             navigation.currentEntry.getState().value,
             navigation.entries().includes(navigation.currentEntry),
             Object.getPrototypeOf(history.state) === Object.prototype]
        """, isolated)
        assert_equal(wrappers, [True, True, 7, True, True], "Navigation entry world wrappers")

        changes = await evaluate("""
            globalThis.changes = 0;
            const previous = navigation.currentEntry;
            globalThis.changeViews = [];
            navigation.addEventListener('currententrychange', function (event) {
                changes++;
                changeViews.push([this === navigation, event.target === navigation,
                    event instanceof NavigationCurrentEntryChangeEvent,
                    event.from instanceof NavigationHistoryEntry, event.from === previous]);
            });
            history.pushState({value: 2}, '', '#two');
            [changes, changeViews]
        """, isolated)
        assert_equal(changes, [1, [[True] * 5]], "one currententrychange in isolated world")
        assert_equal(await evaluate("pageChanges"), 1, "one currententrychange in page world")

        cancellation = await evaluate("""
            (() => {
                const before = location.href;
                const length = history.length;
                let seen = 0;
                navigation.addEventListener('navigate', function (event) {
                    seen++;
                    event.preventDefault();
                }, {once: true});
                history.pushState({}, '', '#must-not-commit');
                return [seen, location.href === before, history.length === length];
            })()
        """, isolated)
        assert_equal(cancellation, [1, True, True], "isolated navigate listener cancels commit")

        errors = await evaluate("""
            ['pushState', 'replaceState'].map(method => {
                try { history[method]({}, '', 'https://other.invalid/'); }
                catch (error) { return [error.name, error instanceof DOMException,
                    Object.getPrototypeOf(error) === DOMException.prototype]; }
                return ['did not throw'];
            })
        """, isolated)
        assert_equal(errors, [["SecurityError", True, True]] * 2, "History error realm")

        await evaluate("history.pushState({value: 3, map: new Map([['x', 2n]])}, '', '#three')")
        state = await evaluate("""
            [history.state.value, history.state.map instanceof Map,
             history.state.map.get('x') === 2n, history.state === history.state,
             changes]
        """, isolated)
        assert_equal(state, [3, True, True, True, 2], "native History updates reach existing worlds")
        record(results, "raw_cdp_history_world_boundaries", {
            "wrappers": wrappers, "changes": changes, "cancellation": cancellation,
            "errors": errors, "state": state,
        })
    finally:
        session = None
        if browser_context is not None:
            with suppress(Exception):
                await command("Target.disposeBrowserContext", {"browserContextId": browser_context})
        await client.websocket.close()
