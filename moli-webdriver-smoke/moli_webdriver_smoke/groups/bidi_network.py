from __future__ import annotations

import asyncio
import json
from typing import Any

import websockets

from ..assertions import assert_equal, assert_true, record
from ..client import ClassicClient, classic_value
from ..config import WebDriverTarget
from ..scenarios import record_failure
from ..selenium_options import create_selenium_options


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


async def _run_background_iframe(
    target: WebDriverTarget,
    fixture: str,
    results: list[dict[str, Any]],
    name: str,
    isolated: bool,
    operation: str,
) -> None:
    # The same public-wire commands run on Moli and ChromeDriver. In
    # particular, do not attach a CDP session to the background Page.
    options = create_selenium_options(target)
    options.set_capability("webSocketUrl", True)
    client = ClassicClient(target.endpoint)
    created = classic_value(client.post("/session", {"capabilities": {"alwaysMatch": options.to_capabilities()}}))
    capabilities = created["capabilities"]
    scenario_error: Exception | None = None
    try:
        async with websockets.connect(capabilities["webSocketUrl"], max_size=2**24) as websocket:
            command_id = 0
            events: list[dict[str, Any]] = []

            async def command(method: str, params: dict[str, Any], error: str | None = None) -> dict[str, Any]:
                nonlocal command_id
                command_id += 1
                await websocket.send(json.dumps({"id": command_id, "method": method, "params": params}))
                try:
                    async with asyncio.timeout(10):
                        while True:
                            response = json.loads(await websocket.recv())
                            if response.get("id") == command_id:
                                break
                            events.append(response)
                except TimeoutError as timeout:
                    raise RuntimeError(f"{name}: timed out on {method} (id={command_id})") from timeout
                assert_equal(response["type"], "error" if error else "success", f"{name}: {method}: {response!r}")
                if error:
                    assert_equal(response["error"], error, f"{name}: {method} error")
                    return response
                return response["result"]

            async def evaluate(context: str, expression: str) -> dict[str, Any]:
                result = await command("script.evaluate", {
                    "expression": expression, "target": {"context": context}, "awaitPromise": True,
                })
                assert_equal(result["type"], "success", f"{name}: script completion: {result!r}")
                return result["result"]

            async def load_frame(context: str, url: str, *, existing: bool = False) -> None:
                frame = "document.querySelector('iframe')" if existing else "document.createElement('iframe')"
                append = "" if existing else "document.body.appendChild(frame);"
                result = await evaluate(context, (
                    "new Promise((resolve, reject) => {"
                    f"const frame = {frame};"
                    "frame.onload = () => resolve(true); frame.onerror = reject;"
                    f"frame.src = {json.dumps(url)}; {append}"
                    "})"
                ))
                assert_equal(result, {"type": "boolean", "value": True}, f"{name}: iframe load")

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

            def matching_events() -> list[dict[str, Any]]:
                return [event for event in events if (
                    event.get("params", {}).get("request", {}).get("url") == url
                    or event.get("params", {}).get("url") == url
                )]

            expected_methods = {
                "network.beforeRequestSent", "network.responseCompleted",
                "browsingContext.domContentLoaded", "browsingContext.load",
            }
            try:
                async with asyncio.timeout(10):
                    while not expected_methods.issubset({event["method"] for event in matching_events()}):
                        events.append(json.loads(await websocket.recv()))
            except TimeoutError as timeout:
                raise RuntimeError(f"{name}: missing iframe events: {matching_events()!r}") from timeout

            observed = matching_events()
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
