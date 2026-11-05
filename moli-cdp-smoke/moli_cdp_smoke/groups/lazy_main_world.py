"""Raw CDP activation contracts; the same cases run against Chromium.

Do not enable Runtime or use Playwright to set up the scriptless document:
those operations themselves demand a Window realm.
"""
from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import quote

from ..assertions import SmokeError, assert_equal, record
from ..raw_cdp import RawCdpClient, connect_raw_cdp


@dataclass
class _Wire:
    client: RawCdpClient
    messages: list[dict[str, Any]] = field(default_factory=list)

    async def call(self, method: str, params: dict[str, Any] | None = None,
                   session: str | None = None) -> dict[str, Any]:
        request = await self.client.send(method, params, session_id=session)
        response, messages = await self.client.recv_until_id(request)
        self.messages.extend(messages)
        result = response.get("result")
        if "error" in response or not isinstance(result, dict):
            raise SmokeError(f"{method}: expected a CDP result object: {response!r}")
        if "exceptionDetails" in result:
            raise SmokeError(f"{method}: {result}")
        return result

    async def load(self, session: str, html: str) -> str:
        return await self.load_url(session, "data:text/html," + quote(html))

    async def load_url(self, session: str, url: str) -> str:
        await self.call("Page.setLifecycleEventsEnabled", {"enabled": True}, session)
        first = len(self.messages)
        navigation = await self.call("Page.navigate", {"url": url}, session)
        if navigation.get("errorText") or not navigation.get("loaderId"):
            raise SmokeError(f"navigation failed: {navigation}")
        async with asyncio.timeout(15):
            while not any(
                m.get("sessionId") == session and m.get("method") == "Page.lifecycleEvent"
                and m.get("params", {}).get("name") == "load"
                and m["params"].get("frameId") == navigation["frameId"]
                and m["params"].get("loaderId") == navigation["loaderId"]
                for m in self.messages[first:]
            ):
                self.messages.append(await self.client.recv())
        return navigation["frameId"]

    async def evaluate(self, session: str, expression: str, **params: Any) -> Any:
        result = await self.call("Runtime.evaluate", {
            "expression": expression, "returnByValue": True, **params,
        }, session)
        return result.get("result", {}).get("value")


async def run_lazy_main_world_group(endpoint: str, _fixture: str,
                                    results: list[dict[str, Any]]) -> None:
    for activation in ("evaluate", "runtime-enable", "isolated-world", "preload",
                       "deferred-script", "inline-load", "native-io", "navigate-close"):
        client = await connect_raw_cdp(endpoint)
        wire = _Wire(client)
        context = None
        try:
            context = (await wire.call("Target.createBrowserContext"))["browserContextId"]
            target = (await wire.call("Target.createTarget", {
                "url": "about:blank", "browserContextId": context,
            }))["targetId"]
            session = (await wire.call("Target.attachToTarget", {
                "targetId": target, "flatten": True,
            }))["sessionId"]
            await wire.call("Page.enable", session=session)
            html = "<!doctype html><title>Lazy activation</title><style>p{color:blue}</style><p id='target'>Native content</p>"
            if activation == "preload":
                await wire.call("Page.addScriptToEvaluateOnNewDocument", {
                    "source": "globalThis.activationCount = (globalThis.activationCount || 0) + 1",
                }, session)
            elif activation == "deferred-script":
                html += "<script defer src='data:text/javascript,globalThis.activationCount=1'></script>"
            elif activation == "inline-load":
                html = html.replace("<p id=", "<body onload='globalThis.activationCount=1'><p id=")
            frame = await wire.load(session, html)
            # DOM and Page queries must work before any explicit script request.
            root = (await wire.call("DOM.getDocument", {"depth": 1}, session))["root"]["nodeId"]
            node = (await wire.call("DOM.querySelector", {"nodeId": root, "selector": "#target"}, session))["nodeId"]
            outer = (await wire.call("DOM.getOuterHTML", {"nodeId": node}, session))["outerHTML"]
            if "Native content" not in outer:
                raise SmokeError(f"native DOM query returned {outer!r}")
            if activation == "native-io":
                await wire.call("Emulation.setTouchEmulationEnabled", {
                    "enabled": True, "maxTouchPoints": 7,
                }, session)
                await wire.call("Emulation.setEmulatedMedia", {
                    "media": "print",
                    "features": [{"name": "prefers-color-scheme", "value": "dark"}],
                }, session)
                await wire.call("Performance.enable", session=session)
                metrics = {m["name"]: m["value"] for m in
                           (await wire.call("Performance.getMetrics", session=session))["metrics"]}
                if metrics.get("Documents", 0) < 1 or metrics.get("Nodes", 0) < 4:
                    raise SmokeError(f"native Document metrics disappeared before Runtime: {metrics}")
                await wire.call("Emulation.setScriptExecutionDisabled", {"value": True}, session)
                await wire.call("Emulation.setScriptExecutionDisabled", {"value": False}, session)
                assert_equal(await wire.call("Runtime.terminateExecution", session=session), {},
                             "terminating a scriptless idle page completes")
            if activation == "navigate-close":
                await wire.load(session, html.replace("Native content", "Replacement"))
                assert_equal((await wire.call("Target.closeTarget", {"targetId": target}))["success"], True,
                             "scriptless target closes after navigation")
            else:
                if activation == "runtime-enable":
                    await wire.call("Runtime.enable", session=session)
                elif activation == "isolated-world":
                    world = await wire.call("Page.createIsolatedWorld", {
                        "frameId": frame, "worldName": "lazy-activation-world",
                    }, session)
                    assert_equal(await wire.evaluate(session, "globalThis.isolatedMarker = 7; document.title",
                                                     contextId=world["executionContextId"]), "Lazy activation",
                                 "isolated world can read the current native Document")
                    assert_equal(await wire.evaluate(session, "typeof isolatedMarker"), "undefined",
                                 "main and isolated globals remain separate")
                observed = await wire.evaluate(session, "({title:document.title, state:document.readyState, text:document.querySelector('#target').textContent, color:getComputedStyle(document.querySelector('#target')).color})")
                assert_equal(observed, {"title": "Lazy activation", "state": "complete",
                                        "text": "Native content", "color": "rgb(0, 0, 255)"},
                             "first main-world evaluation sees completed native Document")
                if activation == "native-io":
                    assert_equal(await wire.evaluate(session, "({touch:navigator.maxTouchPoints, print:matchMedia('print').matches, dark:matchMedia('(prefers-color-scheme: dark)').matches})"),
                                 {"touch": 7, "print": True, "dark": True},
                                 "native emulation survives the first main-world activation")
                if activation in ("preload", "deferred-script", "inline-load"):
                    assert_equal(await wire.evaluate(session, "activationCount"), 1,
                                 "initial script runs exactly once")
                if activation in ("evaluate", "runtime-enable"):
                    await wire.call("Runtime.enable", session=session)
                    contexts = [m["params"]["context"] for m in wire.messages
                                if m.get("sessionId") == session and m.get("method") == "Runtime.executionContextCreated"]
                    defaults = [c for c in contexts if c.get("auxData", {}).get("isDefault")]
                    assert_equal(len(defaults), 1, "main execution context is published once")
                    assert_equal(await wire.evaluate(session, "new Promise((resolve, reject) => { const s=document.createElement('script'); s.src='data:text/javascript,globalThis.delayedMarker=9'; s.onload=()=>resolve(delayedMarker); s.onerror=reject; document.body.append(s); })", awaitPromise=True),
                                 9, "late external script executes in the same initialized main world")
            record(results, f"lazy_main_world_{activation}", {"nativeDomBeforeRuntime": True})
        finally:
            if context is not None:
                await wire.call("Target.disposeBrowserContext", {"browserContextId": context})
            await client.websocket.close()
