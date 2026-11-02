from __future__ import annotations

import asyncio
import base64
import importlib.metadata
import inspect
import json
import sys
import time
import urllib.parse
import urllib.request
from typing import Any, Callable

from cdp_use import CDPClient


EXPECTED_VERSION = "1.4.5"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def read_json(url: str) -> dict[str, Any]:
    with urllib.request.urlopen(url, timeout=5) as response:
        payload = json.load(response)
    require(isinstance(payload, dict), f"invalid JSON object from {url}: {payload!r}")
    return payload


async def wait_for(
    probe: Callable[[], Any],
    label: str,
    *,
    timeout_seconds: float = 7,
) -> Any:
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            value = probe()
            if inspect.isawaitable(value):
                value = await value
            if value:
                return value
        except Exception as error:  # Navigation can replace the realm between probes.
            last_error = error
        await asyncio.sleep(0.025)
    suffix = f"; last error: {last_error}" if last_error else ""
    raise RuntimeError(f"timed out waiting for {label}{suffix}")


async def run(endpoint: str, fixture: str) -> dict[str, Any]:
    version = importlib.metadata.version("cdp-use")
    require(version == EXPECTED_VERSION, f"cdp-use {version} is installed; expected {EXPECTED_VERSION}")
    discovery = await asyncio.to_thread(
        read_json, f"{endpoint.rstrip('/')}/json/version"
    )
    browser_websocket = discovery.get("webSocketDebuggerUrl")
    require(
        isinstance(browser_websocket, str) and bool(browser_websocket),
        "discovery payload has no browser websocket",
    )
    is_moli = isinstance(discovery.get("Moli-Version"), str)
    client = CDPClient(browser_websocket)
    results: list[dict[str, Any]] = []
    browser_context_id: str | None = None
    target_ids: list[str] = []

    def record(name: str, **data: Any) -> None:
        results.append({"name": name, "ok": True, **data})

    async def send(
        method: str,
        params: dict[str, Any] | None = None,
        session_id: str | None = None,
    ) -> dict[str, Any]:
        return await client.send_raw(method, params or {}, session_id)

    async def evaluate(session_id: str, expression: str) -> Any:
        response = await send(
            "Runtime.evaluate",
            {"expression": expression, "awaitPromise": True, "returnByValue": True},
            session_id,
        )
        if response.get("exceptionDetails"):
            raise RuntimeError(
                f"Runtime.evaluate failed: {response['exceptionDetails']!r}"
            )
        return response.get("result", {}).get("value")

    async def wait_ready(session_id: str, expected_path: str) -> None:
        expression = (
            'document.readyState === "complete" && location.pathname === '
            + json.dumps(expected_path)
        )
        await wait_for(
            lambda: evaluate(session_id, expression), f"document {expected_path}"
        )

    async def navigate(session_id: str, url: str) -> None:
        response = await send("Page.navigate", {"url": url}, session_id)
        require(not response.get("errorText"), f"Page.navigate failed: {response!r}")
        await wait_ready(session_id, urllib.parse.urlparse(url).path)

    async def attach(target_id: str) -> str:
        response = await send(
            "Target.attachToTarget", {"targetId": target_id, "flatten": True}
        )
        session_id = response.get("sessionId")
        require(isinstance(session_id, str) and bool(session_id), "attach returned no sessionId")
        await send("Page.enable", session_id=session_id)
        await send("Runtime.enable", session_id=session_id)
        await send("Network.enable", session_id=session_id)
        return session_id

    async def create_page(url: str) -> tuple[str, str]:
        response = await send(
            "Target.createTarget",
            {"url": url, "browserContextId": browser_context_id},
        )
        target_id = response.get("targetId")
        require(isinstance(target_id, str) and bool(target_id), "createTarget returned no targetId")
        target_ids.append(target_id)
        session_id = await attach(target_id)
        await wait_ready(session_id, urllib.parse.urlparse(url).path)
        return target_id, session_id

    await client.start()
    try:
        live_version = await send("Browser.getVersion")
        require(
            live_version.get("product") == discovery.get("Browser"),
            "live Browser.getVersion identity mismatch",
        )
        context = await send("Target.createBrowserContext")
        browser_context_id = context.get("browserContextId")
        require(
            isinstance(browser_context_id, str) and bool(browser_context_id),
            "createBrowserContext returned no id",
        )

        first_url = f"{fixture}/plain?client=cdp-use-first"
        first_target, first_session = await create_page(first_url)
        targets = await send("Target.getTargets")
        require(
            any(info.get("targetId") == first_target for info in targets["targetInfos"]),
            "browser session cannot observe the page target",
        )
        record(
            "cdp_use_browser_page_session_binding",
            clientVersion=version,
            product=live_version.get("product"),
        )

        await evaluate(
            first_session,
            '''localStorage.setItem("external-shared", "cdp-use-local");
               sessionStorage.setItem("external-private", "cdp-use-first");
               globalThis.__externalPageMarker = "first";''',
        )
        _, second_session = await create_page(
            f"{fixture}/plain?client=cdp-use-second"
        )
        second_storage = await evaluate(
            second_session,
            '''({
              local: localStorage.getItem("external-shared"),
              session: sessionStorage.getItem("external-private"),
              marker: globalThis.__externalPageMarker || null,
            })''',
        )
        require(
            second_storage == {
                "local": "cdp-use-local",
                "session": None,
                "marker": None,
            },
            f"second page storage/realm mismatch: {second_storage!r}",
        )
        await evaluate(
            second_session,
            '''sessionStorage.setItem("external-private", "cdp-use-second");
               globalThis.__externalPageMarker = "second";''',
        )
        first_storage = await evaluate(
            first_session,
            '''({
              local: localStorage.getItem("external-shared"),
              session: sessionStorage.getItem("external-private"),
              marker: globalThis.__externalPageMarker,
            })''',
        )
        require(
            first_storage
            == {
                "local": "cdp-use-local",
                "session": "cdp-use-first",
                "marker": "first",
            },
            f"first page storage/realm changed: {first_storage!r}",
        )
        record("cdp_use_multi_page_storage_isolation")

        await navigate(first_session, f"{fixture}/history-a?client=cdp-use")
        await navigate(first_session, f"{fixture}/history-b?client=cdp-use")
        history = await send("Page.getNavigationHistory", session_id=first_session)
        expected_history_urls = [
            first_url,
            f"{fixture}/history-a?client=cdp-use",
            f"{fixture}/history-b?client=cdp-use",
        ]
        history_urls = [entry["url"] for entry in history["entries"]]
        require(
            history_urls == expected_history_urls,
            f"direct-target history mismatch: {history['entries']!r}",
        )
        require(
            history["currentIndex"] == 2,
            f"direct-target currentIndex was {history['currentIndex']}",
        )
        require(
            history["entries"][0]["transitionType"] == "auto_toplevel",
            "direct-target initial transition was "
            f"{history['entries'][0]['transitionType']!r}",
        )
        history_a = next(
            (
                entry
                for entry in history["entries"]
                if urllib.parse.urlparse(entry["url"]).path == "/history-a"
            ),
            None,
        )
        require(history_a is not None, f"history-a entry missing: {history['entries']!r}")
        await send(
            "Page.navigateToHistoryEntry",
            {"entryId": history_a["id"]},
            first_session,
        )
        await wait_ready(first_session, "/history-a")
        record("cdp_use_navigation_history_workflow", entryUrls=history_urls)

        await navigate(first_session, f"{fixture}/iframe?client=cdp-use")

        async def find_frame_tree() -> dict[str, Any] | None:
            tree = (await send("Page.getFrameTree", session_id=first_session))["frameTree"]
            return tree if len(tree.get("childFrames", [])) == 1 else None

        frame_tree = await wait_for(find_frame_tree, "child frame tree")
        child_frame = frame_tree["childFrames"][0]["frame"]
        require(
            child_frame.get("parentId") == frame_tree["frame"]["id"],
            f"wrong child parentId: {child_frame!r}",
        )
        world = await send(
            "Page.createIsolatedWorld",
            {
                "frameId": child_frame["id"],
                "worldName": "cdp-use-external-smoke",
            },
            first_session,
        )
        child_response = await send(
            "Runtime.evaluate",
            {
                "expression": "document.body.textContent.trim()",
                "contextId": world["executionContextId"],
                "returnByValue": True,
            },
            first_session,
        )
        child_text = child_response.get("result", {}).get("value")
        require(
            "child body text" in str(child_text),
            f"wrong child frame text: {child_text!r}",
        )
        record("cdp_use_frame_tree_isolated_world", childFrameId=child_frame["id"])

        await navigate(first_session, f"{fixture}/plain?client=cdp-use-fetch")
        route_url = f"{fixture}/external-client-cdp-use"
        network_state = {"request": False, "response": False, "finished": False}
        route_request_id: str | None = None
        fulfill_future = asyncio.get_running_loop().create_future()
        fulfill_tasks: set[asyncio.Task[None]] = set()

        def request_will_be_sent(params: dict[str, Any], session_id: str | None) -> None:
            nonlocal route_request_id
            if session_id == first_session and params.get("request", {}).get("url") == route_url:
                route_request_id = params.get("requestId")
                network_state["request"] = True

        def response_received(params: dict[str, Any], session_id: str | None) -> None:
            if session_id == first_session and params.get("response", {}).get("url") == route_url:
                network_state["response"] = True

        def loading_finished(params: dict[str, Any], session_id: str | None) -> None:
            if session_id == first_session and params.get("requestId") == route_request_id:
                network_state["finished"] = True

        async def fulfill_request(params: dict[str, Any], session_id: str | None) -> None:
            if session_id != first_session or params.get("request", {}).get("url") != route_url:
                return
            try:
                await send(
                    "Fetch.fulfillRequest",
                    {
                        "requestId": params["requestId"],
                        "responseCode": 200,
                        "responseHeaders": [
                            {"name": "Content-Type", "value": "application/json"}
                        ],
                        "body": base64.b64encode(b'{"source":"cdp-use"}').decode("ascii"),
                    },
                    first_session,
                )
                if not fulfill_future.done():
                    fulfill_future.set_result(session_id)
            except Exception as error:
                if not fulfill_future.done():
                    fulfill_future.set_exception(error)

        def request_paused(params: dict[str, Any], session_id: str | None) -> None:
            # cdp-use awaits async handlers inside its receive loop. Schedule the
            # reply so that loop remains free to receive fulfillRequest's response.
            task = asyncio.create_task(fulfill_request(params, session_id))
            fulfill_tasks.add(task)
            task.add_done_callback(fulfill_tasks.discard)

        client.register.Network.requestWillBeSent(request_will_be_sent)
        client.register.Network.responseReceived(response_received)
        client.register.Network.loadingFinished(loading_finished)
        client.register.Fetch.requestPaused(request_paused)
        try:
            await send(
                "Fetch.enable",
                {
                    "patterns": [
                        {
                            "urlPattern": "*external-client-cdp-use*",
                            "requestStage": "Request",
                        }
                    ]
                },
                first_session,
            )
            body = await evaluate(
                first_session,
                f"fetch({json.dumps(route_url)}).then(response => response.text())",
            )
            event_session = await asyncio.wait_for(fulfill_future, timeout=5)
            require(event_session == first_session, f"Fetch event routed to {event_session!r}")
            require(body == '{"source":"cdp-use"}', f"wrong fulfilled body: {body!r}")
            await wait_for(
                lambda: all(network_state.values()),
                "fulfilled request Network lifecycle",
            )
        finally:
            await send("Fetch.disable", session_id=first_session)
            if fulfill_tasks:
                await asyncio.gather(*fulfill_tasks, return_exceptions=True)
        record("cdp_use_fetch_fulfill_network_lifecycle")

        await navigate(first_session, f"{fixture}/plain?client=cdp-use-position")
        point = await evaluate(
            first_session,
            '''(() => {
              document.body.innerHTML = '<button id="position">position</button>';
              globalThis.__externalPositionClicks = 0;
              document.querySelector('#position').addEventListener('click', () => __externalPositionClicks += 1);
              const rect = document.querySelector('#position').getBoundingClientRect();
              return {x: rect.left + rect.width / 2, y: rect.top + rect.height / 2, width: rect.width, height: rect.height};
            })()''',
        )
        position_error: str | None = None
        try:
            coordinates = {
                "x": point.get("x") or 1,
                "y": point.get("y") or 1,
                "button": "left",
                "clickCount": 1,
            }
            await send(
                "Input.dispatchMouseEvent",
                {"type": "mousePressed", **coordinates},
                first_session,
            )
            await send(
                "Input.dispatchMouseEvent",
                {"type": "mouseReleased", **coordinates},
                first_session,
            )
        except Exception as error:
            position_error = str(error)
        click_count = await evaluate(first_session, "globalThis.__externalPositionClicks")
        if position_error is None:
            require(
                point.get("width", 0) > 0 and point.get("height", 0) > 0,
                f"position click returned an empty button rect: {point!r}",
            )
            require(click_count == 1, f"position click count was {click_count}")
            position_boundary = "layout-supported"
        elif is_moli:
            require(
                any(
                    marker in position_error.lower()
                    for marker in ("not supported", "unsupported", "layout hit testing")
                ),
                f"Moli position click did not return an explicit capability error: {position_error!r}",
            )
            require(click_count == 0, f"unsupported position click mutated the DOM: {click_count}")
            position_boundary = "explicit-client-failure"
        else:
            raise RuntimeError(f"Chromium position click failed: {position_error}")
        record(
            "cdp_use_position_click_capability_boundary",
            boundary=position_boundary,
            supported=position_error is None,
            clickCount=click_count,
        )

        return {"ok": True, "results": results}
    finally:
        if browser_context_id:
            try:
                await send(
                    "Target.disposeBrowserContext",
                    {"browserContextId": browser_context_id},
                )
            except Exception:
                for target_id in target_ids:
                    try:
                        await send("Target.closeTarget", {"targetId": target_id})
                    except Exception:
                        pass
        await client.stop()


async def main() -> None:
    if len(sys.argv) != 3:
        raise RuntimeError("usage: cdp_use_smoke.py ENDPOINT FIXTURE")
    payload = await run(sys.argv[1], sys.argv[2])
    sys.stdout.write(json.dumps(payload, separators=(",", ":")))


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except Exception as error:
        print(f"{type(error).__name__}: {error}", file=sys.stderr)
        raise
