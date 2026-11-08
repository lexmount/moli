from __future__ import annotations

from contextlib import suppress
from typing import Any
from ..diagnostics import new_cdp_session
from . import SmokeState
from ..assertions import SmokeError, assert_equal, wait_until
from ..helpers import attach_cdp_event_collector

from .chromium_cdp_support import _events_with_method, _frame_tree_ids, _has_event, _navigate_with_cdp_until_dom_ready


async def _verify_chromium_page_frame_loading_sample(state: SmokeState) -> None:
    events = attach_cdp_event_collector(
        state.cdp,
        ["Page.frameStartedLoading", "Page.frameStoppedLoading"],
    )
    await state.cdp.send("Page.enable")
    start = len(events)
    await state.cdp.send("Page.navigate", {"url": f"{state.fixture}/chromium-cdp-lifecycle-page?frame-loading"})

    await wait_until(
        lambda: _has_event(events[start:], "Page.frameStartedLoading")
        and _has_event(events[start:], "Page.frameStoppedLoading"),
        "Chromium Page.frameStartedLoading/Page.frameStoppedLoading sample",
    )
    started = next(event for event in events[start:] if event["method"] == "Page.frameStartedLoading")
    stopped = next(event for event in events[start:] if event["method"] == "Page.frameStoppedLoading")
    started_frame_id = started["params"].get("frameId")
    stopped_frame_id = stopped["params"].get("frameId")
    frame_tree = await state.cdp.send("Page.getFrameTree")
    root_frame_id = frame_tree.get("frameTree", {}).get("frame", {}).get("id")
    if not started_frame_id or started_frame_id != stopped_frame_id or started_frame_id != root_frame_id:
        raise SmokeError(f"Page frame loading events should share frameId: {events[start:]}")
    state.record("chromium_page_frame_loading_sample")

async def _verify_chromium_page_frame_tree_sample(state: SmokeState) -> None:
    events = attach_cdp_event_collector(state.cdp, ["Page.frameNavigated"])
    await state.cdp.send("Page.enable")
    start = len(events)
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/iframe")

    # The top document reaching DOMContentLoaded does not imply that an iframe
    # navigation has committed. Chromium publishes the child frameNavigated at
    # commit, so synchronize on that protocol fact before reading the tree.
    await wait_until(
        lambda: any(
            event["method"] == "Page.frameNavigated"
            and str(event["params"].get("frame", {}).get("url", "")).endswith("/child")
            for event in events[start:]
        ),
        "child Page.frameNavigated for /child iframe",
    )
    child_navigation = next(
        event["params"]["frame"]
        for event in events[start:]
        if event["method"] == "Page.frameNavigated"
        and str(event["params"].get("frame", {}).get("url", "")).endswith("/child")
    )
    result = await state.cdp.send("Page.getFrameTree")
    root = result.get("frameTree", {})
    root_frame = root.get("frame", {})
    if not root_frame.get("id") or not str(root_frame.get("url", "")).endswith("/iframe"):
        raise SmokeError(f"Page.getFrameTree root frame mismatch: {result}")
    children = root.get("childFrames") or []
    if not children:
        raise SmokeError(f"Page.getFrameTree should expose child frames: {result}")
    child_urls = [child.get("frame", {}).get("url", "") for child in children]
    if not any(str(url).endswith("/child") for url in child_urls):
        raise SmokeError(f"Page.getFrameTree missed /child iframe: {result}")
    child_frame = next(
        child.get("frame", {})
        for child in children
        if str(child.get("frame", {}).get("url", "")).endswith("/child")
    )
    if (
        child_navigation.get("id") != child_frame.get("id")
        or child_navigation.get("parentId") != root_frame.get("id")
        or not child_navigation.get("loaderId")
        or child_navigation.get("loaderId") != child_frame.get("loaderId")
    ):
        raise SmokeError(
            "child Page.frameNavigated should match Page.getFrameTree: "
            f"event={child_navigation}, tree={result}"
        )
    state.record("chromium_page_get_frame_tree_sample")

async def _verify_chromium_child_frame_multi_session_fanout_sample(
    state: SmokeState,
) -> None:
    methods = [
        "Page.frameAttached",
        "Page.frameStartedNavigating",
        "Page.frameNavigated",
        "Page.lifecycleEvent",
        "Page.frameStoppedLoading",
    ]
    page = await state.context.new_page()
    page_only = None
    lifecycle = None

    def navigation_for_url(events: list[dict[str, Any]], url: str, label: str) -> dict[str, Any]:
        matches = [
            event["params"]["frame"]
            for event in events
            if event["method"] == "Page.frameNavigated"
            and event["params"].get("frame", {}).get("url") == url
        ]
        assert_equal(len(matches), 1, f"{label} frameNavigated count")
        return matches[0]

    def navigation_frame_id_for_url(
        events: list[dict[str, Any]], url: str
    ) -> str | None:
        for event in events:
            if event["method"] != "Page.frameNavigated":
                continue
            frame = event["params"].get("frame", {})
            if frame.get("url") == url and isinstance(frame.get("id"), str):
                return frame["id"]
        return None

    def navigation_has_stopped(events: list[dict[str, Any]], url: str) -> bool:
        frame_id = navigation_frame_id_for_url(events, url)
        return frame_id is not None and any(
            event["method"] == "Page.frameStoppedLoading"
            and event["params"].get("frameId") == frame_id
            for event in events
        )

    def exact_event_index(
        events: list[dict[str, Any]],
        method: str,
        frame_id: str,
        label: str,
        *,
        lifecycle_name: str | None = None,
    ) -> int:
        indices = []
        for index, event in enumerate(events):
            if event["method"] != method:
                continue
            params = event["params"]
            event_frame_id = (
                params.get("frame", {}).get("id")
                if method == "Page.frameNavigated"
                else params.get("frameId")
            )
            if event_frame_id != frame_id:
                continue
            if lifecycle_name is not None and params.get("name") != lifecycle_name:
                continue
            indices.append(index)
        assert_equal(len(indices), 1, f"{label} {method} count")
        return indices[0]

    def assert_page_event_sequence(
        events: list[dict[str, Any]],
        navigation: dict[str, Any],
        label: str,
    ) -> None:
        frame_id = navigation["id"]
        indices = [
            exact_event_index(events, "Page.frameAttached", frame_id, label),
            exact_event_index(events, "Page.frameStartedNavigating", frame_id, label),
            exact_event_index(events, "Page.frameNavigated", frame_id, label),
            exact_event_index(events, "Page.frameStoppedLoading", frame_id, label),
        ]
        assert_equal(indices, sorted(indices), f"{label} child frame event order")
        started = events[indices[1]]["params"]
        assert_equal(started.get("url"), navigation["url"], f"{label} started URL")
        assert_equal(
            started.get("loaderId"),
            navigation["loaderId"],
            f"{label} started loaderId",
        )

    def flattened_frames(frame_tree: dict[str, Any]) -> dict[str, dict[str, Any]]:
        frames: dict[str, dict[str, Any]] = {}
        pending = [frame_tree]
        while pending:
            current = pending.pop()
            frame = current.get("frame", {})
            frame_id = frame.get("id")
            if isinstance(frame_id, str) and frame_id:
                frames[frame_id] = frame
            pending.extend(current.get("childFrames") or [])
        return frames

    async def append_frame(frame_id: str, name: str, url: str) -> None:
        await page.evaluate(
            """({id, name, url}) => {
              const frame = document.createElement('iframe');
              frame.id = id;
              frame.name = name;
              frame.src = url;
              document.body.appendChild(frame);
            }""",
            {"id": frame_id, "name": name, "url": url},
        )

    try:
        await page.goto(f"{state.fixture}/plain?child-frame-session-fanout", wait_until="load")

        page_only = await new_cdp_session(state.context, page)
        lifecycle = await new_cdp_session(state.context, page)
        page_only_events = attach_cdp_event_collector(page_only, methods)
        lifecycle_events = attach_cdp_event_collector(lifecycle, methods)
        await page_only.send("Page.enable")
        await lifecycle.send("Page.enable")
        await lifecycle.send("Page.setLifecycleEventsEnabled", {"enabled": True})
        page_only_start = len(page_only_events)
        lifecycle_start = len(lifecycle_events)

        outer_url = f"{state.fixture}/semantic-frame-child?child=fanout&nested=1"
        nested_url = f"{state.fixture}/semantic-frame-grandchild"
        await append_frame("fanout", "fanout-frame", outer_url)
        await wait_until(
            lambda: all(
                navigation_has_stopped(events[start:], url)
                for events, start in (
                    (page_only_events, page_only_start),
                    (lifecycle_events, lifecycle_start),
                )
                for url in (outer_url, nested_url)
            ),
            "both attached sessions child Page event fan-out",
        )

        page_only_batch = page_only_events[page_only_start:]
        lifecycle_batch = lifecycle_events[lifecycle_start:]
        page_only_navigations = {
            url: navigation_for_url(page_only_batch, url, "Page-only session")
            for url in (outer_url, nested_url)
        }
        lifecycle_navigations = {
            url: navigation_for_url(lifecycle_batch, url, "lifecycle session")
            for url in (outer_url, nested_url)
        }

        for url in (outer_url, nested_url):
            assert_equal(
                lifecycle_navigations[url],
                page_only_navigations[url],
                f"multi-session child frame metadata for {url}",
            )
            assert_page_event_sequence(
                page_only_batch,
                page_only_navigations[url],
                f"Page-only session {url}",
            )
            assert_page_event_sequence(
                lifecycle_batch,
                lifecycle_navigations[url],
                f"lifecycle session {url}",
            )

        outer_navigation = page_only_navigations[outer_url]
        nested_navigation = page_only_navigations[nested_url]
        page_only_tree = (await page_only.send("Page.getFrameTree"))["frameTree"]
        lifecycle_tree = (await lifecycle.send("Page.getFrameTree"))["frameTree"]
        page_only_frames = flattened_frames(page_only_tree)
        lifecycle_frames = flattened_frames(lifecycle_tree)
        root_id = page_only_tree["frame"]["id"]
        assert_equal(lifecycle_tree["frame"]["id"], root_id, "multi-session root frame id")

        for navigation, expected_parent_id in (
            (outer_navigation, root_id),
            (nested_navigation, outer_navigation["id"]),
        ):
            frame_id = navigation["id"]
            expected = {
                key: navigation.get(key)
                for key in ("id", "parentId", "loaderId", "name", "url")
            }
            assert_equal(
                expected["parentId"],
                expected_parent_id,
                f"child frame parent for {navigation['url']}",
            )
            assert_equal(
                {key: page_only_frames[frame_id].get(key) for key in expected},
                expected,
                f"Page-only frame tree metadata for {navigation['url']}",
            )
            assert_equal(
                {key: lifecycle_frames[frame_id].get(key) for key in expected},
                expected,
                f"lifecycle frame tree metadata for {navigation['url']}",
            )

        child_frame_ids = {outer_navigation["id"], nested_navigation["id"]}
        assert_equal(
            [
                event
                for event in page_only_batch
                if event["method"] == "Page.lifecycleEvent"
                and event["params"].get("frameId") in child_frame_ids
            ],
            [],
            "Page lifecycle events remain session-local",
        )
        for navigation in lifecycle_navigations.values():
            frame_id = navigation["id"]
            label = f"lifecycle session {navigation['url']}"
            navigated_index = exact_event_index(
                lifecycle_batch, "Page.frameNavigated", frame_id, label
            )
            dom_content_loaded_index = exact_event_index(
                lifecycle_batch,
                "Page.lifecycleEvent",
                frame_id,
                label,
                lifecycle_name="DOMContentLoaded",
            )
            load_index = exact_event_index(
                lifecycle_batch,
                "Page.lifecycleEvent",
                frame_id,
                label,
                lifecycle_name="load",
            )
            stopped_index = exact_event_index(
                lifecycle_batch, "Page.frameStoppedLoading", frame_id, label
            )
            lifecycle_indices = [
                navigated_index,
                dom_content_loaded_index,
                load_index,
                stopped_index,
            ]
            assert_equal(
                lifecycle_indices,
                sorted(lifecycle_indices),
                f"child lifecycle terminal order for {navigation['url']}",
            )
            for index in (dom_content_loaded_index, load_index):
                assert_equal(
                    lifecycle_batch[index]["params"].get("loaderId"),
                    navigation["loaderId"],
                    f"child lifecycle loaderId for {navigation['url']}",
                )

        await page_only.send("Page.disable")
        page_only_disable_mark = len(page_only_events)
        lifecycle_disable_mark = len(lifecycle_events)
        after_disable_url = f"{state.fixture}/semantic-frame-child?child=after-disable"
        await append_frame("after-disable", "after-disable-frame", after_disable_url)
        await wait_until(
            lambda: navigation_has_stopped(
                lifecycle_events[lifecycle_disable_mark:], after_disable_url
            ),
            "enabled session post-disable child terminal event",
        )
        assert_equal(
            page_only_events[page_only_disable_mark:],
            [],
            "Page.disable stops later child Page events for that session",
        )
        post_disable_navigation = navigation_for_url(
            lifecycle_events[lifecycle_disable_mark:],
            after_disable_url,
            "lifecycle post-disable session",
        )
        assert_page_event_sequence(
            lifecycle_events[lifecycle_disable_mark:],
            post_disable_navigation,
            "lifecycle post-disable session",
        )
        state.record(
            "chromium_page_child_frame_multi_session_fanout_sample",
            {
                "frameIds": [outer_navigation["id"], nested_navigation["id"]],
                "pageOnlyLifecycleCount": 0,
                "postDisableFrameId": post_disable_navigation["id"],
            },
        )
    finally:
        if lifecycle is not None:
            with suppress(Exception):
                await lifecycle.detach()
        if page_only is not None:
            with suppress(Exception):
                await page_only.detach()
        with suppress(Exception):
            await page.close()

async def _verify_chromium_page_frame_attached_parent_sample(state: SmokeState) -> None:
    events = attach_cdp_event_collector(state.cdp, ["Page.frameAttached"])
    await state.cdp.send("Page.enable")
    start = len(events)
    await _navigate_with_cdp_until_dom_ready(
        state,
        f"{state.fixture}/iframe?frame-attached-parent",
    )
    await wait_until(
        lambda: _has_event(events[start:], "Page.frameAttached"),
        "Chromium Page.frameAttached parent frame sample",
    )

    frame_tree = await state.cdp.send("Page.getFrameTree")
    frame_ids = _frame_tree_ids(frame_tree.get("frameTree", {}))
    attached = _events_with_method(events[start:], "Page.frameAttached")
    for event in attached:
        params = event.get("params", {})
        frame_id = params.get("frameId")
        parent_frame_id = params.get("parentFrameId")
        if not isinstance(frame_id, str) or not frame_id:
            raise SmokeError(f"Page.frameAttached should carry a non-empty frameId: {event}")
        if not isinstance(parent_frame_id, str) or not parent_frame_id:
            raise SmokeError(
                f"Page.frameAttached should carry a non-empty parentFrameId: {event}"
            )
        if frame_id not in frame_ids or parent_frame_id not in frame_ids:
            raise SmokeError(
                "Page.frameAttached should reference frames in the committed frame tree: "
                f"event={event}, frameTree={frame_tree}"
            )
    state.record("chromium_page_frame_attached_parent_sample", {"eventCount": len(attached)})

async def _verify_chromium_page_fragment_navigation_sample(state: SmokeState) -> None:
    await state.cdp.send("Page.enable")
    base_url = f"{state.fixture}/plain"
    first = await state.cdp.send("Page.navigate", {"url": base_url})
    await state.page.wait_for_load_state("load", timeout=10_000)
    if not first.get("frameId"):
        raise SmokeError(f"Page.navigate should return frameId for normal navigation: {first}")

    fragment_url = f"{base_url}#fragment"
    second = await state.cdp.send("Page.navigate", {"url": fragment_url})
    if second.get("errorText"):
        raise SmokeError(f"Page.navigate fragment navigation should not fail: {second}")
    location_result = await state.cdp.send(
        "Runtime.evaluate",
        {"expression": "location.href", "returnByValue": True},
    )
    assert_equal(
        location_result.get("result", {}).get("value"),
        fragment_url,
        "Chromium Page.navigate fragment location sample",
    )
    if second.get("frameId") and second.get("frameId") != first.get("frameId"):
        raise SmokeError(f"Page.navigate fragment should stay in the same frame: {first} -> {second}")
    state.record("chromium_page_fragment_navigation_sample")


async def run_frame_group(state: SmokeState) -> None:
    await _verify_chromium_page_frame_loading_sample(state)
    await _verify_chromium_page_frame_tree_sample(state)
    await _verify_chromium_child_frame_multi_session_fanout_sample(state)
    await _verify_chromium_page_frame_attached_parent_sample(state)
    await _verify_chromium_page_fragment_navigation_sample(state)
