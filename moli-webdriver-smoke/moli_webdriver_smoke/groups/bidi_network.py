from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from typing import Any
from urllib.parse import urlencode

import websockets

from ..assertions import assert_equal, assert_true, record
from ..client import ClassicClient, classic_value
from ..config import WebDriverTarget
from ..scenarios import record_failure
from ..selenium_options import create_selenium_options


_IFRAME_EVENTS = {
    "network.beforeRequestSent", "network.responseCompleted",
    "browsingContext.domContentLoaded", "browsingContext.load",
}


class _BidiNetworkConnection:
    def __init__(self, websocket: Any, name: str) -> None:
        self.websocket = websocket
        self.name = name
        self.command_id = 0
        self.events: list[dict[str, Any]] = []

    async def command(self, method: str, params: dict[str, Any], error: str | None = None) -> dict[str, Any]:
        self.command_id += 1
        await self.websocket.send(json.dumps({"id": self.command_id, "method": method, "params": params}))
        try:
            async with asyncio.timeout(10):
                while True:
                    response = json.loads(await self.websocket.recv())
                    if response.get("id") == self.command_id:
                        break
                    self.events.append(response)
        except TimeoutError as timeout:
            raise RuntimeError(f"{self.name}: timed out on {method} (id={self.command_id})") from timeout
        assert_equal(response["type"], "error" if error else "success", f"{self.name}: {method}: {response!r}")
        if error:
            assert_equal(response["error"], error, f"{self.name}: {method} error")
            return response
        return response["result"]

    async def evaluate(self, context: str, expression: str) -> dict[str, Any]:
        result = await self.command("script.evaluate", {
            "expression": expression, "target": {"context": context}, "awaitPromise": True,
        })
        assert_equal(result["type"], "success", f"{self.name}: script completion: {result!r}")
        return result["result"]

    async def load_frame(self, context: str, url: str, *, existing: bool = False) -> None:
        frame = "document.querySelector('iframe')" if existing else "document.createElement('iframe')"
        append = "" if existing else "document.body.appendChild(frame);"
        result = await self.evaluate(context, (
            "new Promise((resolve, reject) => {"
            f"const frame = {frame};"
            "frame.onload = () => resolve(true); frame.onerror = reject;"
            f"frame.src = {json.dumps(url)}; {append}"
            "})"
        ))
        assert_equal(result, {"type": "boolean", "value": True}, f"{self.name}: iframe load")

    async def wait_events(self, url: str, expected_methods: set[str]) -> list[dict[str, Any]]:
        def matching_events() -> list[dict[str, Any]]:
            return [event for event in self.events if (
                event.get("params", {}).get("request", {}).get("url") == url
                or event.get("params", {}).get("url") == url
            )]

        try:
            async with asyncio.timeout(10):
                while not expected_methods.issubset({event["method"] for event in matching_events()}):
                    self.events.append(json.loads(await self.websocket.recv()))
        except TimeoutError as timeout:
            raise RuntimeError(f"{self.name}: missing iframe events: {matching_events()!r}") from timeout
        return matching_events()


@asynccontextmanager
async def _bidi_connection(
    target: WebDriverTarget, name: str, *, initial_url: str | None = None,
) -> AsyncIterator[tuple[_BidiNetworkConnection, dict[str, Any]]]:
    # Use the same public-wire commands on Moli and ChromeDriver, without
    # attaching a CDP session to either Page.
    options = create_selenium_options(target)
    options.set_capability("webSocketUrl", True)
    client = ClassicClient(target.endpoint)
    created = classic_value(client.post("/session", {"capabilities": {"alwaysMatch": options.to_capabilities()}}))
    capabilities = created["capabilities"]
    scenario_error: Exception | None = None
    try:
        if initial_url is not None:
            classic_value(client.post(f"/session/{created['sessionId']}/url", {"url": initial_url}))
        async with websockets.connect(capabilities["webSocketUrl"], max_size=2**24) as websocket:
            yield _BidiNetworkConnection(websocket, name), capabilities
    except Exception as error:
        scenario_error = error
    finally:
        try:
            client.delete(f"/session/{created['sessionId']}")
        except Exception as error:
            if scenario_error is None:
                scenario_error = error
    if scenario_error is not None:
        raise scenario_error


async def run_bidi_network_group(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    continue_on_failure: bool = False,
) -> None:
    for isolated in (False, True):
        for operation in ("insert", "navigate", "nested"):
            name = f"bidi_background_iframe_{'isolated' if isolated else 'shared'}_{operation}"
            try:
                await _run_background_iframe(target, fixture, results, name, isolated, operation)
            except Exception as error:
                if not continue_on_failure:
                    raise
                record_failure(results, "bidi-network", name, error)
    for isolated in (False, True):
        name = f"bidi_iframe_collectors_{'isolated' if isolated else 'shared'}"
        try:
            await _run_iframe_collectors(target, fixture, results, name, isolated)
        except Exception as error:
            if not continue_on_failure:
                raise
            record_failure(results, "bidi-network", name, error)
    for network, user_context in ((False, False), (True, False), (False, True)):
        name = f"bidi_{'user_context' if user_context else 'global'}_iframe_without_get_tree_{'network' if network else 'lifecycle'}"
        try:
            await _run_global_subscription(target, fixture, results, name, network, user_context)
        except Exception as error:
            if not continue_on_failure:
                raise
            record_failure(results, "bidi-network", name, error)
    try:
        await _run_existing_context_replay(target, fixture, results)
    except Exception as error:
        if not continue_on_failure:
            raise
        record_failure(results, "bidi-network", "bidi_existing_context_replay", error)
    for name, scenario in (
        ("bidi_existing_popup_opener_replay", _run_existing_popup_opener_replay),
    ):
        try:
            await scenario(target, fixture, results, name)
        except Exception as error:
            if not continue_on_failure:
                raise
            record_failure(results, "bidi-network", name, error)
    for operation in ("navigation", "nested_detach", "document_navigation", "transient"):
        name = f"bidi_frame_identity_{operation}"
        try:
            await _run_frame_identity(target, fixture, results, name, operation)
        except Exception as error:
            if not continue_on_failure:
                raise
            record_failure(results, "bidi-network", name, error)


async def _run_background_iframe(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    name: str,
    isolated: bool,
    operation: str,
) -> None:
    async with _bidi_connection(target, name) as (bidi, capabilities):
        command = bidi.command
        load_frame = bidi.load_frame
        events = bidi.events
        foreground = (await command("browsingContext.getTree", {}))["contexts"][0]["context"]
        await command("browsingContext.navigate", {
            "context": foreground, "url": f"{fixture}/webdriver/frame", "wait": "complete",
        })
        create: dict[str, Any] = {"type": "tab", "background": True}
        if isolated:
            create["userContext"] = (await command("browser.createUserContext", {}))["userContext"]
        background = (await command("browsingContext.create", create))["context"]
        await command("browsingContext.navigate", {
            "context": background, "url": f"{fixture}/webdriver/frame", "wait": "complete",
        })
        await command("browsingContext.activate", {"context": foreground})
        parent = background
        if operation != "insert":
            await load_frame(background, f"{fixture}/webdriver/frame?setup={operation}")
            if operation == "nested":
                tree = (await command("browsingContext.getTree", {"root": background}))["contexts"][0]
                parent = tree["children"][0]["context"]

        await command("session.subscribe", {
            "events": ["network.beforeRequestSent", "network.responseCompleted", "browsingContext.domContentLoaded", "browsingContext.load"],
            "contexts": [foreground, background],
        })
        collectors = {}
        for label, context in (("foreground", foreground), ("background", background)):
            collectors[label] = (await command("network.addDataCollector", {
                "dataTypes": ["response"], "maxEncodedDataSize": 10000, "contexts": [context],
            }))["collector"]

        url = f"{fixture}/webdriver/network-data?case={name}"
        events.clear()
        await load_frame(parent, url, existing=operation == "navigate")
        tree = (await command("browsingContext.getTree", {"root": background}))["contexts"][0]

        def find_child(node: dict[str, Any]) -> str | None:
            if node["url"] == url:
                return node["context"]
            for child in node.get("children") or []:
                found = find_child(child)
                if found:
                    return found
            return None

        child = find_child(tree)
        assert_true(bool(child), f"{name}: loaded iframe must be in the background context tree")

        observed = await bidi.wait_events(url, _IFRAME_EVENTS)
        for event in observed:
            assert_equal(event["params"]["context"], child, f"{name}: {event['method']} context")
        methods = [event["method"] for event in observed]
        assert_true(methods.index("network.beforeRequestSent") < methods.index("network.responseCompleted"), f"{name}: request/response order")
        assert_true(methods.index("browsingContext.domContentLoaded") < methods.index("browsingContext.load"), f"{name}: lifecycle order")
        completed = next(event for event in observed if event["method"] == "network.responseCompleted")
        request = completed["params"]["request"]["request"]
        data = await command("network.getData", {
            "request": request, "dataType": "response", "collector": collectors["background"],
        })
        assert_equal(data["bytes"], {"type": "string", "value": "webdriver network body"}, f"{name}: background collector body")
        await command("network.getData", {
            "request": request, "dataType": "response", "collector": collectors["foreground"],
        }, error="no such network data")
        record(results, name, {
            "browserName": capabilities["browserName"], "browserVersion": capabilities["browserVersion"],
            "iframeEvents": sorted({event["method"] for event in observed}),
            "responseBody": data["bytes"], "foregroundCollectorError": "no such network data",
        })


async def _run_iframe_collectors(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str, isolated: bool,
) -> None:
    async with _bidi_connection(target, name) as (bidi, capabilities):
        contexts = []
        collectors = []
        for _ in range(2):
            create: dict[str, Any] = {"type": "tab", "background": True}
            if isolated:
                create["userContext"] = (await bidi.command("browser.createUserContext", {}))["userContext"]
            context = (await bidi.command("browsingContext.create", create))["context"]
            contexts.append(context)
            await bidi.command("browsingContext.navigate", {
                "context": context, "url": f"{fixture}/webdriver/frame", "wait": "complete",
            })
            collectors.append((await bidi.command("network.addDataCollector", {
                "dataTypes": ["response"], "maxEncodedDataSize": 10000, "contexts": [context],
            }))["collector"])
        shared_collector = (await bidi.command("network.addDataCollector", {
            "dataTypes": ["response"], "maxEncodedDataSize": 10000, "contexts": contexts,
        }))["collector"]
        await bidi.command("session.subscribe", {"events": sorted(_IFRAME_EVENTS), "contexts": contexts})
        requests = []
        bodies = ["iframe response from context A", "iframe response from context B"]

        async def read_body(index: int, collector: str | None = None) -> dict[str, Any]:
            data = await bidi.command("network.getData", {
                "request": requests[index], "dataType": "response", "collector": collector or collectors[index],
            })
            assert_equal(data["bytes"], {"type": "string", "value": bodies[index]}, f"{name}: collector {index} body")
            return data["bytes"]

        for index, context in enumerate(contexts):
            url = f"{fixture}/webdriver/network-data?{urlencode({'case': name, 'body': bodies[index]})}"
            await bidi.load_frame(context, url)
            observed = await bidi.wait_events(url, _IFRAME_EVENTS)
            completed = next(event for event in observed if event["method"] == "network.responseCompleted")
            requests.append(completed["params"]["request"]["request"])
            await read_body(index)
        # Re-read A after B has completed: checking only each immediate result
        # would miss B overwriting the connection-wide collector entry for A.
        retained = [await read_body(index) for index in range(2)]
        shared_retained = [await read_body(index, shared_collector) for index in range(2)]
        assert_true(requests[0] != requests[1], f"{name}: iframe request IDs must be distinct")
        for index in range(2):
            await bidi.command("network.getData", {
                "request": requests[index], "dataType": "response", "collector": collectors[1 - index],
            }, error="no such network data")
        record(results, name, {
            "browserName": capabilities["browserName"], "browserVersion": capabilities["browserVersion"],
            "distinctRequests": True, "retainedBodies": retained, "sharedCollectorBodies": shared_retained,
            "otherCollectorError": "no such network data",
        })


async def _run_global_subscription(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str,
    network: bool, user_context: bool,
) -> None:
    # Classic prepares the existing Page before the BiDi connection. The very
    # first BiDi command is a global subscription, without getTree or create.
    async with _bidi_connection(target, name, initial_url=f"{fixture}/webdriver/frame") as (bidi, capabilities):
        methods = _IFRAME_EVENTS if network else {"browsingContext.domContentLoaded", "browsingContext.load"}
        params: dict[str, Any] = {"events": sorted(methods)}
        if user_context:
            params["userContexts"] = ["default"]
        await bidi.command("session.subscribe", params)
        realms = (await bidi.command("script.getRealms", {"type": "window"}))["realms"]
        assert_equal(len(realms), 1, f"{name}: initial window realm")
        context = realms[0]["context"]
        url = f"{fixture}/webdriver/network-data?case={name}"
        await bidi.load_frame(context, url)
        observed = await bidi.wait_events(url, methods)
        # In the previous implementation getTree initialized observation and
        # masked this bug, so inspect the tree only after lifecycle events arrive.
        tree = (await bidi.command("browsingContext.getTree", {"root": context}))["contexts"][0]
        child = next(child["context"] for child in tree["children"] if child["url"] == url)
        for event in observed:
            assert_equal(event["params"]["context"], child, f"{name}: {event['method']} context")
        ordered = [event["method"] for event in observed]
        assert_true(ordered.index("browsingContext.domContentLoaded") < ordered.index("browsingContext.load"), f"{name}: lifecycle order")
        record(results, name, {
            "browserName": capabilities["browserName"], "browserVersion": capabilities["browserVersion"],
            "iframeEvents": sorted({event["method"] for event in observed}), "subscribedBeforeGetTree": True,
        })


async def _run_existing_context_replay(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]],
) -> None:
    name = "bidi_existing_context_replay"
    async with _bidi_connection(target, name, initial_url=f"{fixture}/webdriver/basic") as (bidi, capabilities):
        await bidi.command("session.subscribe", {"events": ["browsingContext.contextCreated"]})
        # A following command drains the initial notifications without using
        # getTree to initialize Frame observation before the subscription.
        await bidi.command("script.getRealms", {"type": "window"})
        created = [event["params"] for event in bidi.events if event.get("method") == "browsingContext.contextCreated"]
        roots = [context for context in created if context["parent"] is None]
        assert_equal(len(roots), 1, f"{name}: existing top-level context")
        root = roots[0]
        children = [context for context in created if context["parent"] == root["context"]]
        assert_equal(len(children), 1, f"{name}: existing iframe")
        assert_equal(children[0]["url"], f"{fixture}/webdriver/frame", f"{name}: iframe URL")
        assert_equal(children[0]["clientWindow"], root["clientWindow"], f"{name}: window ownership")
        assert_true(created.index(root) < created.index(children[0]), f"{name}: parent published first")
        assert_true(all(context["children"] is None for context in created), f"{name}: shallow context notifications")
        await bidi.command("session.subscribe", {"events": ["browsingContext.contextCreated"], "userContexts": ["default"]})
        await bidi.command("script.getRealms", {"type": "window"})
        assert_equal(sum(event.get("method") == "browsingContext.contextCreated" for event in bidi.events), 2, f"{name}: overlapping subscription does not replay known contexts")
        tree = (await bidi.command("browsingContext.getTree", {"root": root["context"]}))["contexts"][0]
        assert_equal(tree["children"][0]["context"], children[0]["context"], f"{name}: replay and live tree share identity")
        record(results, name, {"browserName": capabilities["browserName"], "browserVersion": capabilities["browserVersion"], "initialContexts": 2, "duplicateReplay": False})


async def _wait_context_event(bidi: _BidiNetworkConnection, method: str, context: str) -> dict[str, Any]:
    try:
        async with asyncio.timeout(10):
            while True:
                matching = next((event for event in bidi.events if event.get("method") == method and event.get("params", {}).get("context") == context), None)
                if matching is not None:
                    return matching
                bidi.events.append(json.loads(await bidi.websocket.recv()))
    except TimeoutError as error:
        raise RuntimeError(f"{bidi.name}: missing {method} for {context}: {bidi.events!r}") from error


async def _run_existing_popup_opener_replay(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str,
) -> None:
    async with _bidi_connection(target, name, initial_url=f"{fixture}/webdriver/frame") as (bidi, capabilities):
        opener = (await bidi.command("script.getRealms", {"type": "window"}))["realms"][0]["context"]
        opened = await bidi.command("script.evaluate", {
            "expression": "window.open('about:blank', '_blank'); true",
            "target": {"context": opener}, "awaitPromise": True, "userActivation": True,
        })
        assert_equal(opened["type"], "success", f"{name}: popup creation")
        contexts = (await bidi.command("browsingContext.getTree", {}))["contexts"]
        assert_equal(len(contexts), 2, f"{name}: popup created before subscribing")
        popup = next(context["context"] for context in contexts if context["context"] != opener)
        url = f"{fixture}/webdriver/frame?popup={name}"
        await bidi.command("browsingContext.navigate", {"context": popup, "url": url, "wait": "complete"})
        await bidi.command("session.subscribe", {"events": ["browsingContext.contextCreated"]})
        await bidi.command("script.getRealms", {"type": "window"})
        created = [event["params"] for event in bidi.events if event.get("method") == "browsingContext.contextCreated"]
        assert_equal(len(created), 2, f"{name}: each existing context replayed once")
        by_id = {context["context"]: context for context in created}
        assert_equal(by_id[opener]["originalOpener"], None, f"{name}: original tab has no opener")
        assert_equal(by_id[popup]["originalOpener"], opener, f"{name}: replay retains popup opener")
        assert_equal(by_id[popup]["parent"], None, f"{name}: popup is a top-level context")
        assert_equal(by_id[popup]["url"], url, f"{name}: replay retains current popup URL")
        tree = (await bidi.command("browsingContext.getTree", {"root": popup}))["contexts"][0]
        assert_equal(tree["originalOpener"], by_id[popup]["originalOpener"], f"{name}: replay agrees with getTree")
        record(results, name, {
            "browserName": capabilities["browserName"], "browserVersion": capabilities["browserVersion"],
            "openedBeforeSubscription": True, "popupOpenerRetained": True,
        })


async def _run_frame_identity(
    target: WebDriverTarget, fixture: str, results: list[dict[str, Any]], name: str, operation: str,
) -> None:
    async with _bidi_connection(target, name, initial_url=f"{fixture}/webdriver/frame") as (bidi, capabilities):
        await bidi.command("session.subscribe", {"events": sorted(_IFRAME_EVENTS | {"browsingContext.contextCreated", "browsingContext.contextDestroyed"})})
        context = (await bidi.command("script.getRealms", {"type": "window"}))["realms"][0]["context"]
        if operation == "transient":
            result = await bidi.evaluate(context, "{ const frame = document.createElement('iframe'); document.body.appendChild(frame); frame.remove(); } true")
            assert_equal(result, {"type": "boolean", "value": True}, f"{name}: transient frame script")
            try:
                async with asyncio.timeout(10):
                    while True:
                        created = next((event for event in bidi.events if event.get("method") == "browsingContext.contextCreated" and event.get("params", {}).get("parent") == context), None)
                        if created is not None:
                            break
                        bidi.events.append(json.loads(await bidi.websocket.recv()))
            except TimeoutError as error:
                raise RuntimeError(f"{name}: missing transient contextCreated: {bidi.events!r}") from error
            child = created["params"]["context"]
            destroyed = await _wait_context_event(bidi, "browsingContext.contextDestroyed", child)
            assert_true(bidi.events.index(created) < bidi.events.index(destroyed), f"{name}: create precedes destroy")
            assert_equal(destroyed["params"]["parent"], context, f"{name}: transient parent")
            assert_equal(destroyed["params"]["url"], "about:blank", f"{name}: transient URL")
        else:
            collector = (await bidi.command("network.addDataCollector", {"dataTypes": ["response"], "maxEncodedDataSize": 10000}))["collector"]
            first = f"{fixture}/webdriver/frame?case={name}"
            await bidi.load_frame(context, first)
            await bidi.wait_events(first, _IFRAME_EVENTS)
            tree = (await bidi.command("browsingContext.getTree", {"root": context}))["contexts"][0]
            child = tree["children"][0]["context"]
            created = await _wait_context_event(bidi, "browsingContext.contextCreated", child)
            assert_equal(created["params"]["parent"], context, f"{name}: iframe parent")
            root_created = await _wait_context_event(bidi, "browsingContext.contextCreated", context)
            assert_equal(created["params"]["clientWindow"], root_created["params"]["clientWindow"], f"{name}: client window")
            await bidi.command("session.subscribe", {"events": ["browsingContext.contextCreated"]})
            await bidi.command("script.getRealms", {"type": "window"})
            assert_equal(sum(event.get("method") == "browsingContext.contextCreated" and event.get("params", {}).get("context") == context for event in bidi.events), 1, f"{name}: overlapping subscriptions do not replay the root twice")
            second = f"{fixture}/webdriver/network-data?body=retained-iframe-response&case={name}"
            await bidi.load_frame(child if operation in ("nested_detach", "document_navigation") else context, second, existing=operation == "navigation")
            observed = await bidi.wait_events(second, _IFRAME_EVENTS)
            tree = (await bidi.command("browsingContext.getTree", {"root": context}))["contexts"][0]
            if operation == "navigation":
                assert_equal(tree["children"][0]["context"], child, f"{name}: navigation preserves frame ID")
                assert_equal(sum(event.get("method") == "browsingContext.contextCreated" and event.get("params", {}).get("context") == child for event in bidi.events), 1, f"{name}: navigation does not recreate context")
            else:
                nested = tree["children"][0]["children"][0]["context"]
                nested_created = await _wait_context_event(bidi, "browsingContext.contextCreated", nested)
                assert_equal(nested_created["params"]["parent"], child, f"{name}: nested parent")
            completed = next(event for event in observed if event["method"] == "network.responseCompleted")
            request = completed["params"]["request"]["request"]
            if operation == "document_navigation":
                replacement = f"{fixture}/webdriver/frame?replacement={name}"
                await bidi.command("browsingContext.navigate", {"context": context, "url": replacement, "wait": "complete"})
                await bidi.wait_events(replacement, {"browsingContext.domContentLoaded", "browsingContext.load"})
            else:
                await bidi.evaluate(context, "document.querySelector('iframe').remove(); true")
            destroyed = await _wait_context_event(bidi, "browsingContext.contextDestroyed", child)
            assert_equal(destroyed["params"]["url"], second if operation == "navigation" else first, f"{name}: current URL on detach")
            if operation in ("nested_detach", "document_navigation"):
                nested_destroyed = next(node for node in destroyed["params"]["children"] if node["context"] == nested)
                assert_equal(nested_destroyed["url"], second, f"{name}: nested URL on detach")
                await bidi.command("session.subscribe", {"events": ["browsingContext.load"], "contexts": [nested]}, error="no such frame")
            retained = await bidi.command("network.getData", {"request": request, "dataType": "response", "collector": collector})
            assert_equal(retained["bytes"], {"type": "string", "value": "retained-iframe-response"}, f"{name}: durable response after detach")
        tree = (await bidi.command("browsingContext.getTree", {"root": context}))["contexts"][0]
        assert_equal(tree["children"], [], f"{name}: removed frames leave no live children")
        await bidi.command("session.subscribe", {"events": ["browsingContext.load"], "contexts": [child]}, error="no such frame")
        record(results, name, {"browserName": capabilities["browserName"], "browserVersion": capabilities["browserVersion"], "operation": operation, "frameIdentityAndLifetime": True})
