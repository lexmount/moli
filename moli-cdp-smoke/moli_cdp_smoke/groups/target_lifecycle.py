from __future__ import annotations

import asyncio
import json
import sys
import urllib.request
from pathlib import Path
from typing import Any

from ..assertions import SmokeError, assert_equal, record, wait_until
from ..navigation import navigate as navigate_document
from ..raw_cdp import RawCdpClient, connect_raw_cdp
from ..serve import BrowserProcess
from .browser_endpoint import browser_endpoint_lifecycle


def _navigation_gate_control(fixture: str, action: str) -> dict[str, Any]:
    with urllib.request.urlopen(f"{fixture}/navigation-suspension-gate/{action}", timeout=2) as response:
        return json.load(response)


def process_descriptors(pid: int) -> dict[int, str]:
    # Inspect only the managed server, not this client or other smoke workers.
    # A missing/inaccessible managed process is a failure, not a skipped check.
    descriptors: dict[int, str] = {}
    for path in Path(f"/proc/{pid}/fd").iterdir():
        try:
            descriptors[int(path.name)] = str(path.readlink())
        except FileNotFoundError:
            # A transient socket can close between directory listing/readlink.
            continue
    return descriptors


def process_resources(pid: int, profile_dir: str | None = None) -> dict[str, int] | None:
    if not sys.platform.startswith("linux"):
        return None
    descriptors = process_descriptors(pid)
    return {
        "fds": len(descriptors),
        "profileFileFds": sum(value.startswith(profile_dir.rstrip("/") + "/")
                              for value in descriptors.values()) if profile_dir else 0,
        "eventpoll": sum(value == "anon_inode:[eventpoll]" for value in descriptors.values()),
        "eventfd": sum(value == "anon_inode:[eventfd]" for value in descriptors.values()),
        "threads": len(list(Path(f"/proc/{pid}/task").iterdir())),
        "maxFd": max(descriptors, default=-1),
    }


def assert_resources_bounded(baseline: dict[str, int], current: dict[str, int]) -> None:
    # Permit a fixed amount of in-flight teardown/transport bookkeeping, never
    # a per-iteration allowance. The old +2 FD/page leak fails at the first batch.
    for key, allowance in (("fds", 8), ("eventpoll", 2), ("eventfd", 2), ("threads", 2)):
        initial, observed = baseline[key], current[key]
        if key == "fds":
            # Chrome initializes profile database services after startup. These
            # regular files are recorded separately from target/transport FDs.
            # Moli supplies no profile path, so its all-FD budget is unchanged.
            initial -= baseline.get("profileFileFds", 0)
            observed -= current.get("profileFileFds", 0)
        if observed > initial + allowance:
            raise SmokeError(f"target teardown leaked {key}: baseline={baseline}, current={current}")


async def wait_for_resources_bounded(
    pid: int, profile_dir: str | None, baseline: dict[str, int],
) -> dict[str, int]:
    # Target.targetDestroyed retires the target registry entry. Its IO resources
    # can still be closing asynchronously; require reclamation within the same
    # ten-second bound used by target commands, without relaxing any budget.
    current: dict[str, int] | None = None
    violation: SmokeError | None = None

    def reclaimed() -> bool:
        nonlocal current, violation
        current = process_resources(pid, profile_dir)
        if current is None:
            raise RuntimeError("managed process resource sampling became unavailable")
        try:
            assert_resources_bounded(baseline, current)
        except SmokeError as error:
            violation = error
            return False
        return True

    try:
        await wait_until(reclaimed, "target IO resource reclamation")
    except SmokeError as error:
        if violation is not None:
            raise violation from error
        raise
    assert current is not None
    return current


class LifecycleProbe:
    def __init__(self, client: RawCdpClient) -> None:
        self.client = client
        self.destroyed: set[str] = set()

    def observe(self, message: dict[str, Any]) -> None:
        if message.get("method") == "Target.targetDestroyed":
            self.destroyed.add(message["params"]["targetId"])

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
        await navigate_document(self.client, session, {"url": url}, on_message=self.observe)
        value = await self.call("Runtime.evaluate", {
            "expression": "document.querySelector('main')?.textContent", "returnByValue": True,
        }, session)
        assert_equal(value.get("result", {}).get("value"), "plain ok", "real HTTP document after churn")
        await self.close(target)

    async def close_during_navigation(
        self, fixture: str, mode: str, release_before_close: bool, background: bool,
        replacement: bool,
    ) -> dict[str, Any]:
        params: dict[str, Any] = {"url": "about:blank", "background": background}
        if mode == "context":
            params.update(await self.call("Target.createBrowserContext"))
        target = (await self.call("Target.createTarget", params))["targetId"]
        session = await self.attach(target)
        await self.call("Page.enable", session=session)
        if replacement:
            # Retain the second-navigation regression as well as the first
            # navigation out of the initial empty Document.
            await navigate_document(self.client, session, {
                "url": "data:text/html,<script>window.name='retiring-page'</script>",
            }, on_message=self.observe)
        await asyncio.to_thread(_navigation_gate_control, fixture, "reset")
        navigation = await self.client.send("Page.navigate", {
            "url": f"{fixture}/navigation-suspension-gated-document",
        }, session_id=session)

        async def request_seen() -> bool:
            status = await asyncio.to_thread(_navigation_gate_control, fixture, "status")
            return status["requestSeen"] is True

        responses: dict[int, dict[str, Any]] = {}
        recent: list[dict[str, Any]] = []
        try:
            await wait_until(request_seen, "retiring target navigation reached HTTP gate")
            if release_before_close:
                await asyncio.to_thread(_navigation_gate_control, fixture, "release")
            if mode == "target":
                close = await self.client.send("Target.closeTarget", {"targetId": target})
            elif mode == "page":
                close = await self.client.send("Page.close", session_id=session)
            else:
                close = await self.client.send("Target.disposeBrowserContext", {
                    "browserContextId": params["browserContextId"],
                })
            try:
                async with asyncio.timeout(10):
                    while not ({navigation, close} <= responses.keys() and target in self.destroyed):
                        message = await self.client.recv()
                        self.observe(message)
                        recent.append(message)
                        del recent[:-20]
                        if message.get("method") in ("Inspector.targetCrashed", "Target.targetCrashed"):
                            raise SmokeError(f"target crashed during {mode} navigation close: {message}")
                        if message.get("id") in (navigation, close):
                            responses[message["id"]] = message
                        if message.get("id") == close and (
                            "error" in message or not isinstance(message.get("result"), dict)
                        ):
                            raise SmokeError(f"{mode} navigation close failed: {message}")
            except TimeoutError as error:
                raise SmokeError(
                    f"{mode} navigation close timed out: target={target} session={session} "
                    f"release_before_close={release_before_close} responses={responses} recent={recent}"
                ) from error
            terminal = responses[close]
            if mode == "target":
                assert_equal(terminal["result"].get("success"), True, "in-flight Target.closeTarget accepted")
            navigation_terminal = responses[navigation]
            if "error" in navigation_terminal:
                assert_equal(navigation_terminal["error"].get("code"), -32000,
                             "navigation terminates with a target/frame cancellation error")
            elif not isinstance(navigation_terminal.get("result", {}).get("frameId"), str):
                raise SmokeError(f"navigation returned no terminal frame identity: {navigation_terminal}")
            # Chromium can acknowledge cancellation with a loader-only reply.
            # The required terminal is paired with this exact target's close
            # receipt/destruction, not an engine-specific errorText string.
            self.destroyed.remove(target)
        finally:
            await asyncio.to_thread(_navigation_gate_control, fixture, "release")

        async def response_completed() -> bool:
            status = await asyncio.to_thread(_navigation_gate_control, fixture, "status")
            return status["responseCompleted"] is True

        await wait_until(response_completed, "retired navigation fixture handler completed")
        targets = (await self.call("Target.getTargets"))["targetInfos"]
        assert_equal(any(item["targetId"] == target for item in targets), False,
                     "in-flight target stays retired after response release")
        return {"targetId": target, "sessionId": session,
                "navigation": responses[navigation], "close": responses[close]}


async def run_target_lifecycle_group(
    endpoint: str, fixture: str, results: list[dict[str, Any]], serve: BrowserProcess | None,
) -> None:
    if serve is not None:
        await browser_endpoint_lifecycle(endpoint, results)
    pid = serve.process.pid if serve is not None else None
    profile_dir = serve.data_dir if serve is not None and serve.engine == "chromium" else None
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

        for replacement in (False, True):
            for mode in ("target", "page", "context"):
                # Hold the first navigation before response/Document commit.
                # Chromium can acknowledge closeTarget during its initial
                # process swap without destroying the Target. Page.close can
                # also lose its active-page route during a later swap.
                # Browser-level replacement routes still race response release.
                release_modes = (False, True) if replacement and mode != "page" else (False,)
                for release_before_close in release_modes:
                    for background in (False, True):
                        observed = await probe.close_during_navigation(
                            fixture, mode, release_before_close, background, replacement,
                        )
                        sentinel = await probe.call("Runtime.evaluate", {
                            "expression": "globalThis.lifecycleSentinel", "returnByValue": True,
                        }, peer_session)
                        assert_equal(sentinel.get("result", {}).get("value"), 42,
                                     "peer survives closing an in-flight navigation")
                        await probe.navigate(f"{fixture}/plain?after-inflight-{mode}")
                        record(results, "target_lifecycle_inflight_navigation_close", {
                            "mode": mode, "releaseBeforeClose": release_before_close,
                            "background": background, "replacement": replacement, "observed": observed,
                        })

        modes = (("target", 800), ("page", 800), ("detach", 128), ("context", 64))
        # Warm all closure routes and the shared network machinery once, then
        # hold the same default BrowserContext and a live peer for every batch.
        for mode, _ in modes:
            for index in range(4):
                await probe.cycle(mode, index)
        await probe.navigate(f"{fixture}/plain?lifecycle-warmup")
        baseline = process_resources(pid, profile_dir) if pid is not None else None
        record(results, "target_lifecycle_baseline", {
            "pid": pid, "resources": baseline,
            "descriptors": process_descriptors(pid) if baseline is not None else None,
            "fdBudget": "non-profile-fds" if profile_dir else "all-fds",
            "fdSampling": ("external-endpoint-without-owned-pid" if pid is None else
                           "linux-proc" if baseline is not None else "unavailable-on-this-platform"),
        })
        for mode, count in modes:
            for index in range(count):
                await probe.cycle(mode, index)
                if (index + 1) % 100 == 0 or index + 1 == count:
                    current = process_resources(pid, profile_dir) if pid is not None else None
                    # Persist the observation before asserting, so a failed
                    # run retains the resource slope and exact failing batch.
                    record(results, "target_lifecycle_sample", {
                        "mode": mode, "closed": index + 1, "resources": current,
                    })
                    print(f"[moli-cdp-smoke] target-lifecycle {mode} {index + 1}/{count} {current}",
                          file=sys.stderr, flush=True)
                    if baseline is not None and current is not None:
                        try:
                            assert_resources_bounded(baseline, current)
                        except SmokeError:
                            record(results, "target_lifecycle_reclaim_pending", {
                                "mode": mode, "closed": index + 1, "resources": current,
                                "descriptors": process_descriptors(pid),
                            })
                            try:
                                current = await wait_for_resources_bounded(pid, profile_dir, baseline)
                            except SmokeError:
                                record(results, "target_lifecycle_reclaim_failed", {
                                    "mode": mode, "closed": index + 1,
                                    "resources": process_resources(pid, profile_dir),
                                    "descriptors": process_descriptors(pid),
                                })
                                raise
                            record(results, "target_lifecycle_reclaimed", {
                                "mode": mode, "closed": index + 1, "resources": current,
                            })

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
