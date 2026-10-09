from __future__ import annotations

import asyncio
from typing import Any

from ..assertions import SmokeError, assert_equal
from ..raw_cdp import RawCdpClient, connect_raw_cdp
from ..state import SmokeState


class _Session:
    def __init__(self, client: RawCdpClient, session_id: str):
        self.client = client
        self.session_id = session_id
        self.events: list[dict[str, Any]] = []

    @classmethod
    async def open(cls, endpoint: str, target_id: str) -> _Session:
        client = await connect_raw_cdp(endpoint)
        try:
            command_id = await client.send("Target.attachToTarget", {"targetId": target_id, "flatten": True})
            response, _ = await client.recv_until_id(command_id)
            return cls(client, response["result"]["sessionId"])
        except BaseException:
            await client.websocket.close()
            raise

    async def send(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        command_id = await self.client.send(method, params, session_id=self.session_id)
        response, seen = await self.client.recv_until_id(command_id)
        self.events.extend(e for e in seen if "method" in e)
        return response["result"]

    async def completed_request(self, url: str) -> str:
        async with asyncio.timeout(5):
            while True:
                requests = [e["params"]["requestId"] for e in self.events
                            if e["method"] == "Network.requestWillBeSent" and e["params"]["request"]["url"] == url]
                if requests and any(e["method"] == "Network.loadingFinished" and e["params"]["requestId"] == requests[0]
                                    for e in self.events):
                    return requests[0]
                self.events.append(await self.client.recv())

    async def detach(self) -> None:
        await self.client.websocket.close()


async def run_network_durable_group(state: SmokeState) -> None:
    await _budget_case(state, "total", 12_000, 32_000, [8_000, 8_000, 8_000], {2})
    await _budget_case(state, "oversize", 12_000, 10_000, [18_000, 4_000], {1})


async def _budget_case(
    state: SmokeState,
    name: str,
    small_total: int,
    small_resource: int,
    sizes: list[int],
    small_retained: set[int],
) -> None:
    page = state.page
    await page.goto(f"{state.fixture}/plain", wait_until="load")
    target_id = (await state.cdp.send("Target.getTargetInfo"))["targetInfo"]["targetId"]
    # Independent WebSockets also isolate Chromium's root-session collectors.
    large = await _Session.open(state.endpoint, target_id)
    small = None
    try:
        small = await _Session.open(state.endpoint, target_id)
        for label, session, total, resource in (
            ("large", large, 128_000, 32_000),
            ("small", small, small_total, small_resource),
        ):
            await session.send("Network.enable", {
                "enableDurableMessages": True,
                "maxTotalBufferSize": total,
                "maxResourceBufferSize": resource,
            })

        request_ids: dict[str, list[str]] = {"large": [], "small": []}
        for index, size in enumerate(sizes):
            url = f"{state.fixture}/api-durable-body?size={size}&case={name}&index={index}"
            length = await page.evaluate("url => fetch(url).then(r => r.text()).then(s => s.length)", url)
            assert_equal(length, size, "durable budget does not truncate the page consumer")

            for label, session in (("large", large), ("small", small)):
                request_ids[label].append(await session.completed_request(url))

        # Retire the renderer's ordinary cache to observe durable retention.
        await page.goto("data:text/html,<p>new document</p>", wait_until="load")
        for index, size in enumerate(sizes):
            body = await large.send("Network.getResponseBody", {"requestId": request_ids["large"][index]})
            assert_equal(body, {"body": "x" * size, "base64Encoded": False}, f"{name} large session retains body {index}")
            if index in small_retained:
                body = await small.send("Network.getResponseBody", {"requestId": request_ids["small"][index]})
                assert_equal(body, {"body": "x" * size, "base64Encoded": False}, f"{name} small session retains body {index}")
            else:
                await _missing(small, request_ids["small"][index])
        state.record(f"network_durable_{name}_session_budget", {
            "sizes": sizes, "smallRetained": sorted(small_retained), "largeRetained": list(range(len(sizes))),
        })

        # Chromium 145 permits another Network.enable on the same handler.
        await large.send("Network.enable", {
            "enableDurableMessages": True, "maxTotalBufferSize": 128_000,
            "maxResourceBufferSize": 32_000,
        })
        await small.send("Network.disable")
        await small.send("Network.enable", {
            "enableDurableMessages": True, "maxTotalBufferSize": small_total,
            "maxResourceBufferSize": small_resource,
        })
        await small.detach()
        small = None
        body = await large.send("Network.getResponseBody", {"requestId": request_ids["large"][0]})
        assert_equal(body["body"], "x" * sizes[0], "peer disable/re-enable/detach preserves large session")
        state.record(f"network_durable_{name}_peer_revocation")
    finally:
        if small is not None:
            await small.detach()
        await large.detach()


async def _missing(session: Any, request_id: str) -> None:
    try:
        await session.send("Network.getResponseBody", {"requestId": request_id})
    except Exception as error:
        if "No resource with given identifier" in str(error) or "evicted from inspector cache" in str(error):
            return
        raise
    raise SmokeError(f"session unexpectedly retained body {request_id}")
