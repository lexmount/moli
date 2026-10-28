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
        # Only inspect the tree after observing lifecycle events: getTree
        # installs observers and would otherwise mask the missing-subscription bug.
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
