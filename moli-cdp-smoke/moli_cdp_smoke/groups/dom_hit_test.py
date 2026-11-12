from __future__ import annotations

import asyncio
from typing import Any

from playwright.async_api import Page, TimeoutError as PlaywrightTimeoutError, async_playwright

from ..assertions import SmokeError, assert_equal, record
from ..diagnostics import new_cdp_session
from ..navigation import navigate as navigate_document
from ..raw_cdp import RawCdpClient, connect_raw_cdp, discover_browser_version


async def _response_for_id(
    client: RawCdpClient,
    message_id: int,
    *,
    timeout: float = 5.0,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    seen: list[dict[str, Any]] = []
    deadline = asyncio.get_running_loop().time() + timeout
    while True:
        remaining = deadline - asyncio.get_running_loop().time()
        if remaining <= 0:
            raise SmokeError(
                f"timed out waiting for raw CDP response id={message_id}: {seen[-20:]}"
            )
        message = await asyncio.wait_for(client.recv(), timeout=remaining)
        seen.append(message)
        if message.get("id") == message_id:
            return message, seen


async def _success(
    client: RawCdpClient,
    method: str,
    params: dict[str, Any] | None = None,
    *,
    session_id: str | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    message_id = await client.send(method, params, session_id=session_id)
    response, seen = await _response_for_id(client, message_id)
    if "error" in response:
        raise SmokeError(f"{method} failed: {response['error']!r}")
    result = response.get("result")
    if not isinstance(result, dict):
        raise SmokeError(f"{method} returned no result object: {response!r}")
    return result, seen


def _attribute_map(node: dict[str, Any]) -> dict[str, str]:
    attributes = node.get("attributes") or []
    return {
        str(attributes[index]): str(attributes[index + 1])
        for index in range(0, len(attributes) - 1, 2)
    }


async def run_dom_hit_test_group(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    version = await discover_browser_version(endpoint)
    is_moli = isinstance(version.get("Moli-Version"), str)
    client = await connect_raw_cdp(endpoint)
    target_id: str | None = None
    try:
        created, _ = await _success(client, "Target.createTarget", {"url": "about:blank"})
        target_id = created.get("targetId")
        if not isinstance(target_id, str) or not target_id:
            raise SmokeError(f"Target.createTarget returned no targetId: {created!r}")

        attached, _ = await _success(
            client,
            "Target.attachToTarget",
            {"targetId": target_id, "flatten": True},
        )
        session_id = attached.get("sessionId")
        if not isinstance(session_id, str) or not session_id:
            raise SmokeError(f"Target.attachToTarget returned no sessionId: {attached!r}")

        await _success(client, "Page.enable", session_id=session_id)
        await navigate_document(client, session_id,
                                {"url": f"{fixture}/chromium-cdp-hit-test-page"})
        await _success(client, "Page.captureScreenshot", session_id=session_id)

        command_id = await client.send(
            "DOM.getNodeForLocation",
            {
                "x": 10,
                "y": 10,
                "includeUserAgentShadowDOM": True,
                "ignorePointerEventsNone": True,
            },
            session_id=session_id,
        )
        response, _ = await _response_for_id(client, command_id)
        assert_equal(response.get("sessionId"), session_id, "hit-test response session")

        if "error" in response:
            raise SmokeError(f"DOM.getNodeForLocation failed: {response!r}")
        hit = response.get("result")
        if not isinstance(hit, dict):
            raise SmokeError(f"DOM.getNodeForLocation returned no hit-test result: {response!r}")
        backend_node_id = hit.get("backendNodeId")
        frame_id = hit.get("frameId")
        if not isinstance(backend_node_id, int) or backend_node_id <= 0:
            raise SmokeError(f"DOM.getNodeForLocation returned an invalid backendNodeId: {hit!r}")
        if not isinstance(frame_id, str) or not frame_id:
            raise SmokeError(f"DOM.getNodeForLocation returned an invalid frameId: {hit!r}")

        described, _ = await _success(
            client,
            "DOM.describeNode",
            {"backendNodeId": backend_node_id},
            session_id=session_id,
        )
        node = described.get("node")
        if not isinstance(node, dict):
            raise SmokeError(f"could not describe the hit node: {described!r}")
        hit_node_id = _attribute_map(node).get("id")
        assert_equal(hit_node_id, "hit-overlay", "option-aware hit node")
        record(
            results,
            "dom_get_node_for_location_layout_hit_test",
            {
                "supported": True,
                "engine": "moli" if is_moli else "chromium",
                "backendNodeIdPresent": True,
                "frameIdPresent": True,
                "nodeName": node.get("nodeName"),
                "nodeIdAttribute": hit_node_id,
            },
        )
    finally:
        if target_id is not None:
            try:
                await _success(client, "Target.closeTarget", {"targetId": target_id})
            except Exception:
                pass
        await client.websocket.close()

    await _run_hover_hit_tests(endpoint, fixture, results, is_moli=is_moli)


async def _point_target(page: Page) -> dict[str, Any]:
    dom = await page.evaluate("""() => ({
      first: document.elementFromPoint(90, 80)?.id,
      all: document.elementsFromPoint(90, 80).map(node => node.id)
    })""")
    session = await new_cdp_session(page.context, page)
    try:
        hit = await session.send("DOM.getNodeForLocation", {"x": 90, "y": 80})
        described = await session.send("DOM.describeNode", {"backendNodeId": hit["backendNodeId"]})
        return {**dom, "cdp": _attribute_map(described["node"]).get("id")}
    finally:
        await session.detach()


async def _assert_hover_blocked(page: Page) -> None:
    try:
        await page.locator("#front").hover(timeout=500)
    except PlaywrightTimeoutError:
        return
    raise SmokeError("hover must reject a target that does not receive pointer events")


async def _navigate_hover_case(page: Page, fixture: str, case: str, *, is_moli: bool) -> None:
    await page.goto(f"{fixture}/hover-hit-test/{case}", wait_until="load")
    if not is_moli:
        # Chromium's input targeting consumes compositor state. As in the
        # layout-policy group, wait for a rendered frame before calibrating it.
        await page.bring_to_front()
        await page.screenshot()


async def _run_hover_hit_tests(
    endpoint: str,
    fixture: str,
    results: list[dict[str, Any]],
    *,
    is_moli: bool,
) -> None:
    # https://github.com/lexmount/moli/issues/2069#issuecomment-6726238838
    # Fetch complete documents and keep Moli cold: no screenshot, force action,
    # or injected DOM may hide a stale first-demand geometry or hit-test bug.
    async with async_playwright() as playwright:
        browser = await playwright.chromium.connect_over_cdp(endpoint)
        page = await browser.contexts[0].new_page()
        page.set_default_timeout(5_000)
        try:
            for case in ("stacked", "transform", "opacity", "fixed-pass-through", "label"):
                await _navigate_hover_case(page, fixture, case, is_moli=is_moli)
                point = await _point_target(page)
                assert_equal(point["first"], "front", f"{case}: DOM hit target")
                assert_equal(point["all"][0], "front", f"{case}: front-to-back hit stack")
                assert_equal(point["cdp"], "front", f"{case}: CDP hit target")
                if case in ("stacked", "transform", "opacity"):
                    assert_equal(
                        point["all"].index("front") < point["all"].index("back"),
                        True,
                        f"{case}: paint order overrides DOM order and nested z-index",
                    )
                await page.mouse.move(700, 500)
                await page.locator("#front").hover()
                assert_equal(
                    await page.locator("#front").evaluate("node => getComputedStyle(node).backgroundColor"),
                    "rgb(0, 0, 255)",
                    f"{case}: hover updates CSS",
                )
                if case == "label":
                    assert_equal(
                        await page.locator(".figcaption").evaluate("node => getComputedStyle(node).display"),
                        "block",
                        "transparent input hovers its label wrapper",
                    )
                    await page.locator("#front").check()
                    assert_equal(await page.locator("#front").is_checked(), True, "transparent radio checked")
                else:
                    await page.locator("#front").click()
                events = await page.evaluate("() => hitEvents")
                for event_type in ("mousemove", "click"):
                    assert_equal(
                        any(event["type"] == event_type and event["target"] == "front" and event["trusted"]
                            for event in events),
                        True,
                        f"{case}: trusted {event_type} uses the verified hit target: {events!r}",
                    )
                record(results, f"hover_hit_test_{case}", {"point": point, "events": events})

            await _navigate_hover_case(page, fixture, "fixed", is_moli=is_moli)
            point = await _point_target(page)
            assert_equal(point["first"], "cover", "fixed overlay DOM hit")
            assert_equal(point["cdp"], "cover", "fixed overlay CDP hit")
            await _assert_hover_blocked(page)
            await page.locator("#cover").hover()
            assert_equal(
                await page.evaluate("() => hitEvents.some(event => event.type === 'mousemove' && event.target === 'cover' && event.trusted)"),
                True,
                "fixed overlay receives the trusted hover",
            )
            record(results, "hover_hit_test_fixed_overlay", {"point": point})

            await _navigate_hover_case(page, fixture, "label-passive", is_moli=is_moli)
            point = await _point_target(page)
            assert_equal(point["first"], "wrapper", "pointer-events:none input exposes its label")
            assert_equal(point["cdp"], "wrapper", "CDP agrees on the label ancestor")
            # Chromium also rejects direct hover on this input. Accepting every
            # ancestor hit would bypass pointer-events and real occlusion.
            await _assert_hover_blocked(page)
            await page.locator("#wrapper").click()
            assert_equal(await page.locator("#front").is_checked(), True, "label click activates its input")
            record(results, "hover_hit_test_label_ancestor", {"point": point})

            await _navigate_hover_case(page, fixture, "figure", is_moli=is_moli)
            await page.locator(".figure").hover()
            assert_equal(
                await page.locator(".figcaption").evaluate("node => getComputedStyle(node).display"),
                "block",
                "navigated page hover reveals its caption",
            )
            record(results, "hover_hit_test_navigated_figure")
        finally:
            await page.close()
            await browser.close()
