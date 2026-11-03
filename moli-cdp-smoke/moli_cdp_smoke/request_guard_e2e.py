"""Load an attacker page in Moli and exercise a built binary's request guard."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import urllib.error
import urllib.request
from typing import Any
from urllib.parse import urlencode, urlsplit

from playwright.async_api import Page, async_playwright
import websockets

from .config import clear_current_process_proxy_env, clear_proxy_env, moli_binary
from .raw_cdp import (
    RawCdpClient,
    connect_raw_cdp,
    connect_raw_cdp_websocket,
    discover_page_websocket_url,
    discover_websocket_url,
)
from .serve import start_moli_serve, stop_browser, wait_for_browser_endpoint


ATTACKER_SCRIPT = """
globalThis.__requestGuardTakenOver = false;
const endpoints = JSON.parse(new URL(location.href).searchParams.get('probes') || '[]');
globalThis.__requestGuardProbes = Promise.all(endpoints.map(({url, command}) =>
    new Promise((resolve, reject) => {
        const socket = new WebSocket(url);
        let opened = false;
        const timer = setTimeout(() => {
            socket.close();
            reject(new Error(`WebSocket probe timed out: ${url}`));
        }, 10000);
        socket.onerror = () => {
            clearTimeout(timer);
            resolve({url, opened});
        };
        socket.onopen = () => {
            opened = true;
            socket.send(JSON.stringify(command));
        };
        socket.onmessage = event => {
            const response = JSON.parse(event.data);
            if (response.id !== command.id) return;
            clearTimeout(timer);
            socket.close();
            if (response.error || response.type === 'error') {
                reject(new Error(JSON.stringify(response)));
            } else {
                resolve({url, opened, response});
            }
        };
    })
));
"""


async def _serve_attacker_page(
    reader: asyncio.StreamReader, writer: asyncio.StreamWriter,
) -> None:
    try:
        await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5)
        body = (
            "<!doctype html><title>Request guard attacker fixture</title>"
            f"<script>{ATTACKER_SCRIPT}</script>"
        ).encode()
        writer.write(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n"
            + f"Content-Length: {len(body)}\r\n\r\n".encode()
            + body
        )
        await writer.drain()
    finally:
        writer.close()
        await writer.wait_closed()


def _http_status(endpoint: str, headers: dict[str, str]) -> int:
    request = urllib.request.Request(endpoint + "/json/version", headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code


async def _command(
    client: RawCdpClient, method: str, params: dict[str, Any] | None = None,
) -> dict[str, Any]:
    request_id = await client.send(method, params)
    response, _ = await client.recv_until_id(request_id)
    return response["result"]


async def _navigate_moli(client: RawCdpClient, url: str) -> None:
    await _command(client, "Page.enable")
    request_id = await client.send("Page.navigate", {"url": url})
    response, seen = await client.recv_until_id(request_id)
    assert not response["result"].get("errorText"), response
    async with asyncio.timeout(10):
        while not any(message.get("method") == "Page.loadEventFired" for message in seen):
            seen.append(await client.recv())


async def _evaluate_moli(client: RawCdpClient, expression: str) -> Any:
    result = await _command(client, "Runtime.evaluate", {
        "expression": expression,
        "awaitPromise": True,
        "returnByValue": True,
    })
    assert "exceptionDetails" not in result, result
    assert "value" in result["result"], result
    return result["result"]["value"]


async def _exercise(
    page: Page | None, origin: str, name: str, extra_args: tuple[str, ...], allowed: bool,
) -> dict[str, Any]:
    server = await start_moli_serve(layout=False, extra_args=extra_args)
    browser_client = None
    page_client = None
    try:
        endpoint = await wait_for_browser_endpoint(server)
        browser_url = await discover_websocket_url(endpoint)
        page_url = await discover_page_websocket_url(endpoint)
        page_target_id = urlsplit(page_url).path.removeprefix("/devtools/page/")
        assert len(page_target_id) == 32 and all(
            character in "0123456789ABCDEF" for character in page_target_id
        ), page_url
        native_status = await asyncio.to_thread(_http_status, endpoint, {})
        same_origin_status = await asyncio.to_thread(
            _http_status, endpoint, {"Origin": endpoint},
        )
        origin_status = await asyncio.to_thread(_http_status, endpoint, {"Origin": origin})
        rebinding_status = await asyncio.to_thread(
            _http_status, endpoint, {"Host": "rebinding.example"},
        )
        assert native_status == 200, (name, native_status)
        assert same_origin_status == 200, (name, same_origin_status)
        assert origin_status == (200 if allowed else 403), (name, origin_status)
        assert rebinding_status == 403, (name, rebinding_status)

        browser_client = await connect_raw_cdp(endpoint)
        version = await _command(browser_client, "Browser.getVersion")
        assert version.get("product"), version
        page_client = await connect_raw_cdp_websocket(page_url)
        before = await _command(page_client, "Runtime.evaluate", {
            "expression": "globalThis.__requestGuardTakenOver = false; 1 + 1",
            "returnByValue": True,
        })
        assert before["result"]["value"] == 2, before

        same_origin_websocket_statuses = []
        for websocket_url, method, params in [
            (browser_url, "Browser.getVersion", {}),
            (page_url, "Runtime.evaluate", {
                "expression": "1 + 1", "returnByValue": True,
            }),
            (endpoint.replace("http://", "ws://") + "/session", "session.status", {}),
        ]:
            async with websockets.connect(
                websocket_url, origin=endpoint, open_timeout=5,
            ) as websocket:
                await _command(RawCdpClient(websocket=websocket), method, params)
                assert websocket.response is not None
                assert websocket.response.status_code == 101
                same_origin_websocket_statuses.append(websocket.response.status_code)

        endpoints = [
            {"url": browser_url, "command": {
                "id": 1, "method": "Browser.getVersion", "params": {},
            }},
            {"url": page_url, "command": {
                "id": 1, "method": "Runtime.evaluate", "params": {
                    "expression": "globalThis.__requestGuardTakenOver = true",
                    "returnByValue": True,
                },
            }},
            {"url": endpoint.replace("http://", "ws://") + "/session", "command": {
                "id": 1, "method": "session.status", "params": {},
            }},
        ]
        attacker_url = origin + "/?" + urlencode({"probes": json.dumps(endpoints)})
        if page is None:
            await _navigate_moli(page_client, attacker_url)
            assert await _evaluate_moli(page_client, "location.origin") == origin
            probes = await _evaluate_moli(page_client, "globalThis.__requestGuardProbes")
        else:
            await page.goto(attacker_url)
            assert await page.evaluate("location.origin") == origin
            probes = await page.evaluate("globalThis.__requestGuardProbes")
        assert all(probe["opened"] == allowed for probe in probes), (name, probes)
        assert len(probes) == 3, (name, probes)
        after = await _evaluate_moli(
            page_client, "({takenOver: globalThis.__requestGuardTakenOver, nativeResult: 1 + 1})",
        )
        assert after["takenOver"] == allowed, (name, after)
        assert after["nativeResult"] == 2, (name, after)
        assert (await _command(browser_client, "Browser.getVersion")).get("product")
        return {
            "case": name,
            "nativeDiscoveryStatus": native_status,
            "sameOriginDiscoveryStatus": same_origin_status,
            "sameOriginWebSocketStatuses": same_origin_websocket_statuses,
            "originDiscoveryStatus": origin_status,
            "rebindingHostStatus": rebinding_status,
            "attackerPageOrigin": origin,
            "pageTargetId": page_target_id,
            "pageWebSocketConnectionsOpened": [probe["opened"] for probe in probes],
            "pageTakeoverSucceeded": after["takenOver"],
            "nativeClientsWorkBeforeAndAfter": True,
        }
    finally:
        for client in (page_client, browser_client):
            if client is not None:
                await client.websocket.close()
        await stop_browser(server)


async def _run_cases(page: Page | None, origin: str, attacker_engine: str) -> None:
    results = []
    for name, args, allowed in [
        ("default", (), False),
        ("explicit_origin", (f"--remote-allow-origins={origin}",), True),
        ("wildcard", ("--remote-allow-origins=*",), True),
    ]:
        results.append(await _exercise(page, origin, name, args, allowed))
    print(json.dumps({
        "binary": str(moli_binary()), "attackerEngine": attacker_engine, "results": results,
    }, indent=2))


async def main(attacker_engine: str) -> None:
    clear_current_process_proxy_env()
    fixture = await asyncio.start_server(_serve_attacker_page, "127.0.0.1", 0)
    async with fixture:
        origin = f"http://127.0.0.1:{fixture.sockets[0].getsockname()[1]}"
        if attacker_engine == "moli":
            await _run_cases(None, origin, attacker_engine)
            return
        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(
                executable_path=os.environ.get("CHROMIUM_BIN"),
                args=["--no-sandbox"],
                env=clear_proxy_env(os.environ),
            )
            try:
                page = await browser.new_page()
                await _run_cases(page, origin, attacker_engine)
            finally:
                await browser.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--attacker-engine", choices=("moli", "chromium"), default="moli")
    asyncio.run(main(parser.parse_args().attacker_engine))
