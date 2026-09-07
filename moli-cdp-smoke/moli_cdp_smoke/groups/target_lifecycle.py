from __future__ import annotations

import asyncio
import sys
from pathlib import Path
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..raw_cdp import RawCdpClient, connect_raw_cdp
from ..serve import MoliServe


def process_resources(pid: int) -> dict[str, int] | None:
    if not sys.platform.startswith("linux"):
        return None
    # Inspect only the managed server, not this client or other smoke workers.
    # A missing/inaccessible managed process is a failure, not a skipped check.
    descriptors: dict[int, str] = {}
    for path in Path(f"/proc/{pid}/fd").iterdir():
        try:
            descriptors[int(path.name)] = str(path.readlink())
        except FileNotFoundError:
            # A transient socket can close between directory listing/readlink.
            continue
    return {
        "fds": len(descriptors),
        "eventpoll": sum(value == "anon_inode:[eventpoll]" for value in descriptors.values()),
        "eventfd": sum(value == "anon_inode:[eventfd]" for value in descriptors.values()),
        "threads": len(list(Path(f"/proc/{pid}/task").iterdir())),
        "maxFd": max(descriptors, default=-1),
    }


def assert_resources_bounded(baseline: dict[str, int], current: dict[str, int]) -> None:
    # Permit a fixed amount of in-flight teardown/transport bookkeeping, never
    # a per-iteration allowance. The old +2 FD/page leak fails at the first batch.
    for key, allowance in (("fds", 8), ("eventpoll", 2), ("eventfd", 2), ("threads", 2)):
        if current[key] > baseline[key] + allowance:
            raise SmokeError(f"target teardown leaked {key}: baseline={baseline}, current={current}")


class LifecycleProbe:
    def __init__(self, client: RawCdpClient) -> None:
        self.client = client
        self.destroyed: set[str] = set()
        self.loaded: set[str] = set()

    def observe(self, message: dict[str, Any]) -> None:
        if message.get("method") == "Target.targetDestroyed":
            self.destroyed.add(message["params"]["targetId"])
        elif message.get("method") == "Page.loadEventFired":
            self.loaded.add(message["sessionId"])

    async def call(self, method: str, params: dict[str, Any] | None = None,
                   session: str | None = None) -> dict[str, Any]:
        response, seen = await self.client.recv_until_id(
            await self.client.send(method, params, session_id=session), timeout=10,
        )
        for message in seen:
            self.observe(message)
        return response["result"]

    async def wait_for(self, identities: set[str], identity: str, label: str) -> None:
        async def receive() -> None:
            while identity not in identities:
                self.observe(await self.client.recv())
            identities.remove(identity)
        try:
            await asyncio.wait_for(receive(), timeout=10)
        except TimeoutError as error:
            raise SmokeError(f"missing {label} for exact identity {identity}") from error

    async def attach(self, target: str) -> str:
        return (await self.call("Target.attachToTarget", {
            "targetId": target, "flatten": True,
        }))["sessionId"]

    async def close(self, target: str) -> None:
        result = await self.call("Target.closeTarget", {"targetId": target})
        assert_equal(result.get("success"), True, "Target.closeTarget accepted")
        await self.wait_for(self.destroyed, target, "Target.targetDestroyed")

    async def cycle(self, mode: str, index: int) -> None:
        params: dict[str, Any] = {"url": "about:blank", "background": bool(index % 2)}
        if mode == "context":
            params.update(await self.call("Target.createBrowserContext"))
        target = (await self.call("Target.createTarget", params))["targetId"]
        if mode == "context":
            await self.call("Target.disposeBrowserContext", {
                "browserContextId": params["browserContextId"],
            })
            await self.wait_for(self.destroyed, target, "disposed context target")
        elif mode == "page":
            await self.call("Page.close", session=await self.attach(target))
            await self.wait_for(self.destroyed, target, "Page.close target destruction")
        else:
            if mode == "detach":
                await self.call("Target.detachFromTarget", {"sessionId": await self.attach(target)})
            await self.close(target)

    async def navigate(self, url: str) -> None:
        target = (await self.call("Target.createTarget", {"url": "about:blank"}))["targetId"]
        session = await self.attach(target)
        await self.call("Page.enable", session=session)
        response = await self.call("Page.navigate", {"url": url}, session)
        if response.get("errorText"):
            raise SmokeError(f"navigation after target churn failed: {response}")
        await self.wait_for(self.loaded, session, "post-churn Page.loadEventFired")
        value = await self.call("Runtime.evaluate", {
            "expression": "document.querySelector('main')?.textContent", "returnByValue": True,
        }, session)
        assert_equal(value.get("result", {}).get("value"), "plain ok", "real HTTP document after churn")
        await self.close(target)


async def run_target_lifecycle_group(
    endpoint: str, fixture: str, results: list[dict[str, Any]], serve: MoliServe | None,
) -> None:
    pid = serve.process.pid if serve is not None else None
    client = await connect_raw_cdp(endpoint)
    probe = LifecycleProbe(client)
    try:
        await probe.call("Target.setDiscoverTargets", {"discover": True})
        for target in (await probe.call("Target.getTargets"))["targetInfos"]:
            if target["type"] == "page":
                await probe.close(target["targetId"])
        peer = (await probe.call("Target.createTarget", {"url": "about:blank"}))["targetId"]
        peer_session = await probe.attach(peer)
        await probe.call("Runtime.evaluate", {"expression": "globalThis.lifecycleSentinel = 42"}, peer_session)

        modes = (("target", 800), ("page", 800), ("detach", 128), ("context", 64))
        # Warm all closure routes and the shared network machinery once, then
        # hold the same default BrowserContext and a live peer for every batch.
        for mode, _ in modes:
            for index in range(4):
                await probe.cycle(mode, index)
        await probe.navigate(f"{fixture}/plain?lifecycle-warmup")
        baseline = process_resources(pid) if pid is not None else None
        record(results, "target_lifecycle_baseline", {
            "pid": pid, "resources": baseline,
            "fdSampling": ("external-endpoint-without-owned-pid" if pid is None else
                           "linux-proc" if baseline is not None else "unavailable-on-this-platform"),
        })
        for mode, count in modes:
            for index in range(count):
                await probe.cycle(mode, index)
                if (index + 1) % 100 == 0 or index + 1 == count:
                    current = process_resources(pid) if pid is not None else None
                    # Persist the observation before asserting, so a failed
                    # run retains the resource slope and exact failing batch.
                    record(results, "target_lifecycle_sample", {
                        "mode": mode, "closed": index + 1, "resources": current,
                    })
                    print(f"[moli-cdp-smoke] target-lifecycle {mode} {index + 1}/{count} {current}",
                          file=sys.stderr, flush=True)
                    if baseline is not None and current is not None:
                        assert_resources_bounded(baseline, current)

            await probe.navigate(f"{fixture}/plain?after-{mode}-{count}")
            sentinel = await probe.call("Runtime.evaluate", {
                "expression": "globalThis.lifecycleSentinel", "returnByValue": True,
            }, peer_session)
            assert_equal(sentinel.get("result", {}).get("value"), 42, "peer renderer survives teardown")
            targets = (await probe.call("Target.getTargets"))["targetInfos"]
            assert_equal([item["targetId"] for item in targets if item["type"] == "page"],
                         [peer], "no closed Page remains registered")
            record(results, f"target_lifecycle_{mode}", {"closed": count, "navigation": "ok", "peer": "alive"})
        await probe.close(peer)
    finally:
        await client.websocket.close()
