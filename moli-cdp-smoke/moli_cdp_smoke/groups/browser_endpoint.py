from __future__ import annotations

from typing import Any
from urllib.parse import urlsplit
from uuid import UUID, uuid4

import websockets
from websockets.exceptions import InvalidStatus

from ..assertions import SmokeError, assert_equal, record
from ..raw_cdp import connect_raw_cdp, discover_websocket_url
from ..serve import start_browser, stop_browser, wait_for_browser_endpoint


def _browser_path(websocket_url: str) -> str:
    path = urlsplit(websocket_url).path
    prefix = "/devtools/browser/"
    if not path.startswith(prefix):
        raise SmokeError(f"unexpected browser endpoint: {websocket_url}")
    assert_equal(UUID(path.removeprefix(prefix)).version, 4, "browser endpoint UUID version")
    return path


async def _reject(endpoint: str, path: str) -> None:
    websocket_url = f"ws://{urlsplit(endpoint).netloc}{path}"
    try:
        async with websockets.connect(websocket_url, proxy=None, open_timeout=5):
            raise SmokeError(f"unexpectedly accepted browser endpoint: {websocket_url}")
    except InvalidStatus as error:
        assert_equal(error.response.status_code, 404, "unknown browser endpoint status")


async def browser_endpoint_discovery(endpoint: str, results: list[dict[str, Any]]) -> None:
    websocket_url = await discover_websocket_url(endpoint)
    path = _browser_path(websocket_url)
    assert_equal(await discover_websocket_url(endpoint), websocket_url, "stable discovery")
    await _reject(endpoint, "/devtools/browser/moli-browser")
    await _reject(endpoint, f"/devtools/browser/{uuid4()}")

    server = await start_browser()
    try:
        other_endpoint = await wait_for_browser_endpoint(server)
        other_path = _browser_path(await discover_websocket_url(other_endpoint))
        if other_path == path:
            raise SmokeError("independent browsers reused the endpoint UUID")
        await _reject(endpoint, other_path)
        await _reject(other_endpoint, path)
        port = urlsplit(other_endpoint).port
        assert port is not None
    finally:
        await stop_browser(server)

    server = await start_browser(port)
    try:
        restarted_endpoint = await wait_for_browser_endpoint(server)
        restarted_path = _browser_path(await discover_websocket_url(restarted_endpoint))
        if restarted_path in (path, other_path):
            raise SmokeError("restarted browser reused an endpoint UUID")
        await _reject(restarted_endpoint, other_path)
        client = await connect_raw_cdp(restarted_endpoint)
        try:
            request_id = await client.send("Browser.getVersion")
            response, _ = await client.recv_until_id(request_id)
            if "error" in response or not response.get("result", {}).get("product"):
                raise SmokeError(f"discovered browser endpoint is unusable: {response}")
        finally:
            await client.websocket.close()
    finally:
        await stop_browser(server)
    record(results, "browser_endpoint_uuid_discovery_and_restart", {
        "stableWithinInstance": True,
        "distinctBetweenInstances": True,
        "staleEndpointRejectedAfterRestart": True,
        "unknownEndpointStatus": 404,
    })
