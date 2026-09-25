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

        await evaluate("""
            navigation.addEventListener('navigate', event => {
                globalThis.probeDestination = event.destination;
                event.signal.pageOnly = 'page';
                event.signal.addEventListener = () => { throw new Error('page override'); };
            }, {once: true});
        """)
        await evaluate("""
            globalThis.signalViews = [];
            navigation.addEventListener('navigate', event => {
                signalViews.push(event instanceof NavigateEvent,
                    event.signal instanceof AbortSignal,
                    Object.getPrototypeOf(event.signal) === AbortSignal.prototype,
                    event.signal.pageOnly === undefined);
                try { event.signal.addEventListener('abort', () => {}); signalViews.push(true); }
                catch (_) { signalViews.push(false); }
            }, {once: true});
        """, isolated)
        await evaluate("history.pushState({}, '', '#signal-view')")
        signal_views = await evaluate("signalViews", isolated)
        assert_equal(signal_views, [True] * 5, "NavigateEvent signal world wrapper")

        await evaluate("""
            globalThis.syntheticViews = [];
            for (const type of ['probe', 'navigate']) {
                navigation.addEventListener(type, function (event) {
                    syntheticViews.push([event instanceof Event, event.expando === undefined,
                        event.target === navigation, event.currentTarget === navigation,
                        this === navigation, event instanceof CustomEvent,
                        event instanceof NavigateEvent, event.detail ?? null]);
                    event.preventDefault();
                }, {once: true});
            }
        """)
        original_identity = await evaluate("""
            (() => {
                const facts = [];
                for (const original of [new CustomEvent('probe', {detail: 7, cancelable: true}),
                                        new Event('navigate', {cancelable: true})]) {
                    original.expando = 'isolated';
                    navigation.addEventListener(original.type, event => {
                        facts.push(event === original, event.defaultPrevented,
                            event.target === navigation, event.composedPath()[0] === navigation);
                    }, {once: true});
                    facts.push(navigation.dispatchEvent(original), original.defaultPrevented,
                        original.currentTarget === null, original.composedPath().length === 0);
                }
                return facts;
            })()
        """, isolated)
        assert_equal(original_identity, [True, True, True, True, False, True, True, True] * 2,
                     "synthetic event identity and shared cancellation")
        synthetic_views = await evaluate("syntheticViews")
        assert_equal(synthetic_views, [
            [True, True, True, True, True, True, False, 7],
            [True, True, True, True, True, False, False, None],
        ], "synthetic event interface and expando isolation")

        await evaluate("""
            globalThis.probeController = new AbortController();
            globalThis.probeData = new FormData();
            probeData.set('value', 'page');
            const file = new File(['bytes'], 'probe.txt', {type: 'text/plain', lastModified: 7});
            file.pageOnly = true;
            probeData.set('file', file);
            const source = document.createElement('button');
            source.id = 'world-probe-source';
            document.body.appendChild(source);
            globalThis.platformProbe = new NavigateEvent('platform-probe', {
                destination: probeDestination, signal: probeController.signal,
                formData: probeData, sourceElement: source,
            });
            navigation.addEventListener('platform-probe', event => {
                event.signal.pageOnly = true;
                event.signal.addEventListener = () => { throw new Error('page signal override'); };
                event.formData.pageOnly = true;
                event.formData.get = () => 'page override';
                event.sourceElement.pageOnly = true;
                event.sourceElement.getAttribute = () => 'page override';
            }, {once: true});
        """)
        await evaluate("""
            globalThis.platformViews = [];
            navigation.addEventListener('platform-probe', event => {
                globalThis.probeSignal = event.signal;
                globalThis.probeFormData = event.formData;
                const file = event.formData.get('file');
                platformViews.push(event instanceof NavigateEvent,
                    event.signal instanceof AbortSignal, event.signal.pageOnly === undefined,
                    event.formData instanceof FormData, event.formData.pageOnly === undefined,
                    event.formData.get('value') === 'page',
                    event.sourceElement instanceof HTMLButtonElement,
                    event.sourceElement === document.getElementById('world-probe-source'),
                    event.sourceElement.pageOnly === undefined,
                    event.sourceElement.getAttribute('id') === 'world-probe-source',
                    file instanceof File, file.pageOnly === undefined,
                    file === event.formData.get('file'), file.name === 'probe.txt',
                    file.size === 5, file.type === 'text/plain', file.lastModified === 7);
                event.formData.set('value', 'isolated');
            }, {once: true});
        """, isolated)
        await evaluate("navigation.dispatchEvent(platformProbe)")
        # Chrome 154 crashes when aborting this synthetic NavigateEvent's signal.
        # Shared abort state and delivery are covered by history_worlds.rs.
        platform_views = await evaluate("platformViews", isolated)
        assert_equal(platform_views, [True] * 17, "nested platform object world wrappers")
        assert_equal(await evaluate("FormData.prototype.get.call(probeData, 'value')"),
                     "isolated", "FormData writes reach the original object")
        await evaluate("FormData.prototype.set.call(probeData, 'value', 'updated')")
        assert_equal(await evaluate("probeFormData.get('value')", isolated),
                     "updated", "FormData views remain live")
        event_fields = []
        for source, receiver in [(isolated, None), (None, isolated)]:
            await evaluate("""
                globalThis.eventFieldObservations = [];
                navigation.addEventListener('backing-fields-probe', event => {
                    eventFieldObservations.push([event instanceof CustomEvent,
                        event.detail, event.type, event.target === navigation]);
                    event.preventDefault();
                }, {once: true});
                navigation.addEventListener('backing-toggle-probe', event => {
                    eventFieldObservations.push([event instanceof ToggleEvent,
                        event.oldState, event.newState, event.source === null,
                        event.target === navigation, event.expando === undefined]);
                }, {once: true});
            """, receiver)
            source_facts = await evaluate("""
                (() => {
                    let getterCalls = 0;
                    const original = new CustomEvent('backing-fields-probe', {detail: 7, cancelable: true});
                    Object.defineProperty(original, 'detail', {
                        get() { getterCalls++; return 99; }, configurable: true,
                    });
                    let sameObject = false;
                    navigation.addEventListener('backing-fields-probe', event => {
                        sameObject = event === original;
                    }, {once: true});
                    const dispatched = navigation.dispatchEvent(original);
                    const toggle = new ToggleEvent('backing-toggle-probe', {oldState: 'closed', newState: 'open'});
                    toggle.expando = 'source';
                    return [dispatched, sameObject, getterCalls, original.defaultPrevented,
                        navigation.dispatchEvent(toggle)];
                })()
            """, source)
            observed = await evaluate("eventFieldObservations", receiver)
            assert_equal(source_facts, [False, True, 0, True, True],
                         "internal fields preserve source identity and do not invoke shadow getters")
            assert_equal(observed, [[True, 7, "backing-fields-probe", True],
                                    [True, "closed", "open", True, True, True]],
                         "CustomEvent and ToggleEvent use internal fields in the receiving world")
            event_fields.append({"source": source_facts, "receiver": observed})

        dispatch_path = await evaluate("""
            (() => {
                const host = document.createElement('div');
                document.body.appendChild(host);
                const shadow = host.attachShadow({mode: 'open'});
                const one = document.createElement('button');
                const two = document.createElement('button');
                shadow.append(one, two);
                let calls = 0, getterCalls = 0;
                host.addEventListener('path-probe', () => calls++);
                const event = new Event('path-probe', {bubbles: true, composed: true});
                Object.defineProperty(event, 'relatedTarget', {get() { getterCalls++; return two; }});
                one.dispatchEvent(event);
                one.dispatchEvent(new FocusEvent('path-probe', {bubbles: true, composed: true, relatedTarget: two}));
                host.remove();
                return [calls, getterCalls];
            })()
        """)
        assert_equal(dispatch_path, [1, 0], "dispatch uses internal relatedTarget, not author expandos")

        record(results, "raw_cdp_history_world_boundaries", {
            "event_fields": event_fields, "dispatch_path": dispatch_path,
            "wrappers": wrappers, "changes": changes, "cancellation": cancellation,
            "errors": errors, "state": state,
            "signal_views": signal_views, "original_identity": original_identity,
            "synthetic_views": synthetic_views, "platform_views": platform_views,
        })
    finally:
        session = None
        if browser_context is not None:
            with suppress(Exception):
                await command("Target.disposeBrowserContext", {"browserContextId": browser_context})
        await client.websocket.close()
