from __future__ import annotations

from typing import Any
from urllib.parse import urlsplit
from ..diagnostics import new_cdp_session
from . import SmokeState
from ..config import is_moli_browser
from ..assertions import SmokeError, assert_equal, wait_until
from ..fixture import FixtureServer
from ..helpers import capture_layout

from .chromium_cdp_support import _navigate_with_cdp_until_dom_ready, _send_cdp_expect_optional_error


async def _verify_chromium_page_layout_metrics_sample(state: SmokeState) -> None:
    await _navigate_with_cdp_until_dom_ready(state, f"{state.fixture}/chromium-cdp-layout-page")
    await capture_layout(state.page)
    initial = await state.cdp.send("Page.getLayoutMetrics")
    content = initial.get("cssContentSize") or {}
    viewport = initial.get("cssLayoutViewport") or {}
    visual = initial.get("cssVisualViewport") or {}
    if content.get("width", 0) <= 0 or content.get("height", 0) <= 0:
        raise SmokeError(f"Page.getLayoutMetrics should expose content size: {initial}")
    if viewport.get("clientWidth", 0) <= 0 or viewport.get("clientHeight", 0) <= 0:
        raise SmokeError(f"Page.getLayoutMetrics should expose layout viewport size: {initial}")
    if visual.get("clientWidth", 0) <= 0 or visual.get("clientHeight", 0) <= 0:
        raise SmokeError(f"Page.getLayoutMetrics should expose visual viewport size: {initial}")

    await state.cdp.send("Runtime.evaluate", {"expression": "window.scrollTo(100, 100)"})
    after_scroll = await state.cdp.send("Page.getLayoutMetrics")
    scrolled_visual = after_scroll.get("cssVisualViewport") or {}
    if scrolled_visual.get("pageX", 0) < 0 or scrolled_visual.get("pageY", 0) < 0:
        raise SmokeError(f"Page.getLayoutMetrics scroll coordinates should be non-negative: {after_scroll}")
    state.record("chromium_page_layout_metrics_sample")

async def _verify_chromium_idle_override_sample(state: SmokeState) -> None:
    primary = await new_cdp_session(state.context, state.page)
    peer = await new_cdp_session(state.context, state.page)
    same_site_fixture = FixtureServer()
    same_site_fixture.start()
    split = urlsplit(state.fixture)
    primary_origin = f"{split.scheme}://{split.hostname}:{split.port}"
    alternate_origin = f"{split.scheme}://localhost:{split.port}"
    same_site_origin = same_site_fixture.url
    primary_url = f"{primary_origin}/chromium-cdp-idle-page?first"
    alternate_url = f"{alternate_origin}/chromium-cdp-idle-page?cross-origin"
    target_info = await primary.send("Target.getTargetInfo")
    browser_context_id = target_info.get("targetInfo", {}).get("browserContextId")

    async def grant(origin: str) -> None:
        params = {"permissions": ["idleDetection"], "origin": origin}
        if browser_context_id:
            params["browserContextId"] = browser_context_id
        await primary.send(
            "Browser.grantPermissions",
            params,
        )

    async def runtime_value(cdp: Any, expression: str, *, await_promise: bool = False) -> Any:
        result = await cdp.send(
            "Runtime.evaluate",
            {
                "expression": expression,
                "returnByValue": True,
                "awaitPromise": await_promise,
            },
        )
        exception = result.get("exceptionDetails")
        if exception:
            raise SmokeError(f"Idle override Runtime.evaluate failed: {result}")
        return result.get("result", {}).get("value")

    try:
        await grant(primary_origin)
        await grant(alternate_origin)
        await grant(same_site_origin)
        await _navigate_with_cdp_until_dom_ready(state, primary_url)

        initial = await runtime_value(
            primary,
            """
            (async () => {
              globalThis.__idleEvents = [];
              globalThis.__idleDetector = new IdleDetector();
              __idleDetector.addEventListener('change', () => {
                __idleEvents.push(`${__idleDetector.userState}/${__idleDetector.screenState}`);
              });
              const before = [__idleDetector.userState, __idleDetector.screenState];
              await __idleDetector.start();
              return {before, state: [__idleDetector.userState, __idleDetector.screenState], events: __idleEvents};
            })()
            """,
            await_promise=True,
        )
        assert_equal(
            initial,
            {
                "before": [None, None],
                "state": ["active", "unlocked"],
                "events": ["active/unlocked"],
            },
            "IdleDetector initial state",
        )

        await primary.send(
            "Emulation.setIdleOverride",
            {"isUserActive": False, "isScreenUnlocked": False},
        )
        assert_equal(
            await runtime_value(
                primary,
                "({state:[__idleDetector.userState,__idleDetector.screenState],events:__idleEvents})",
            ),
            {
                "state": ["idle", "locked"],
                "events": ["active/unlocked", "idle/locked"],
            },
            "IdleDetector setIdleOverride state",
        )

        await peer.send(
            "Emulation.setIdleOverride",
            {"isUserActive": True, "isScreenUnlocked": False, "ignoredExtra": True},
        )
        await peer.detach()
        peer = None
        assert_equal(
            await runtime_value(
                primary,
                "[__idleDetector.userState,__idleDetector.screenState,__idleEvents.length]",
            ),
            ["active", "locked", 3],
            "Idle override last writer and detach persistence",
        )

        child = next((frame for frame in state.page.frames if frame != state.page.main_frame), None)
        if child is None:
            raise SmokeError("Idle override fixture should create a child frame")
        child_state = await child.evaluate(
            """
            async () => {
              const detector = new IdleDetector();
              await detector.start();
              return [detector.userState, detector.screenState];
            }
            """
        )
        assert_equal(
            child_state,
            ["active", "unlocked"],
            "top-level idle override should not affect child frame",
        )

        await primary.send("Emulation.clearIdleOverride")
        assert_equal(
            await runtime_value(
                primary,
                "[__idleDetector.userState,__idleDetector.screenState,__idleEvents.length]",
            ),
            ["active", "unlocked", 4],
            "clearIdleOverride should restore actual state",
        )
        await primary.send(
            "Emulation.setIdleOverride",
            {"isUserActive": False, "isScreenUnlocked": False},
        )

        await runtime_value(primary, "history.pushState({},'',location.pathname+'?same-document')")
        assert_equal(
            await runtime_value(primary, "[__idleDetector.userState,__idleDetector.screenState]"),
            ["idle", "locked"],
            "same-document navigation should preserve idle override",
        )

        same_origin_url = f"{primary_origin}/chromium-cdp-idle-page?same-origin"
        await _navigate_with_cdp_until_dom_ready(state, same_origin_url)
        assert_equal(
            await runtime_value(
                primary,
                "(async()=>{const d=new IdleDetector();await d.start();return [d.userState,d.screenState]})()",
                await_promise=True,
            ),
            ["idle", "locked"] if is_moli_browser() else ["active", "unlocked"],
            "same-origin cross-document navigation should preserve idle override",
        )

        await _navigate_with_cdp_until_dom_ready(
            state,
            f"{same_site_origin}/chromium-cdp-idle-page?same-site-different-origin",
        )
        assert_equal(
            await runtime_value(
                primary,
                "(async()=>{const d=new IdleDetector();await d.start();return [d.userState,d.screenState]})()",
                await_promise=True,
            ),
            ["idle", "locked"] if is_moli_browser() else ["active", "unlocked"],
            "same-site cross-origin navigation should preserve idle override",
        )

        other_page = await state.context.new_page()
        try:
            await other_page.goto(f"{primary_origin}/chromium-cdp-idle-page?other-target")
            assert_equal(
                await other_page.evaluate(
                    "async()=>{const d=new IdleDetector();await d.start();return [d.userState,d.screenState]}"
                ),
                ["active", "unlocked"],
                "idle override should not cross target boundaries",
            )
        finally:
            await other_page.close()

        await _navigate_with_cdp_until_dom_ready(state, alternate_url)
        assert_equal(
            await runtime_value(
                primary,
                "(async()=>{const d=new IdleDetector();await d.start();return [d.userState,d.screenState]})()",
                await_promise=True,
            ),
            ["active", "unlocked"],
            "cross-origin navigation should clear idle override",
        )

        invalid_cases = [
            {},
            {"isUserActive": True},
            {"isScreenUnlocked": True},
            {"isUserActive": None, "isScreenUnlocked": True},
            {"isUserActive": "true", "isScreenUnlocked": True},
        ]
        for params in invalid_cases:
            error = await _send_cdp_expect_optional_error(
                primary,
                "Emulation.setIdleOverride",
                params,
            )
            if error is None or "Invalid" not in error["message"]:
                raise SmokeError(
                    f"Emulation.setIdleOverride should reject invalid params {params}: {error}"
                )
        await primary.send("Emulation.clearIdleOverride", {"ignoredExtra": True})
        state.record("chromium_idle_override_sample")
    finally:
        if peer is not None:
            await peer.detach()
        await primary.detach()
        same_site_fixture.stop()

async def _verify_chromium_page_get_app_manifest_sample(state: SmokeState) -> None:
    none_url = f"{state.fixture}/chromium-app-manifest-none/path/page"
    await _navigate_with_cdp_until_dom_ready(state, none_url)
    implicit = await state.cdp.send("Page.getAppManifest")
    expected_scope = f"{state.fixture}/chromium-app-manifest-none/path/"
    expected_implicit_manifest = {
        "display": "kUndefined",
        "id": none_url,
        "orientation": "DEFAULT",
        "preferRelatedApplications": False,
        "scope": expected_scope,
        "startUrl": none_url,
    }
    assert_equal(implicit.get("url"), "", "implicit manifest URL")
    assert_equal(implicit.get("errors"), [], "implicit manifest errors")
    assert_equal(implicit.get("data"), "", "implicit manifest data")
    assert_equal(implicit.get("parsed"), {"scope": expected_scope}, "implicit parsed manifest")
    assert_equal(implicit.get("manifest"), expected_implicit_manifest, "implicit manifest")

    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-app-manifest-valid/page"
    )
    valid_manifest_route = "/chromium-app-manifests/app.webmanifest"
    state.fixture_server.reset_request_count(valid_manifest_route)
    manifest_network_start = len(state.subresource_events)
    explicit = await state.cdp.send("Page.getAppManifest")
    manifest = explicit.get("manifest") or {}
    expected_manifest_url = f"{state.fixture}/chromium-app-manifests/app.webmanifest"
    expected_manifest_scope = f"{state.fixture}/chromium-app-manifests/"
    await _assert_manifest_network_lifecycle(
        state.subresource_events,
        manifest_network_start,
        expected_manifest_url,
    )
    assert_equal(explicit.get("url"), expected_manifest_url, "explicit manifest URL")
    assert_equal(explicit.get("errors"), [], "explicit manifest errors")
    if not isinstance(explicit.get("data"), str) or '"Manifest Name"' not in explicit["data"]:
        raise SmokeError(f"Page.getAppManifest should preserve raw manifest data: {explicit}")
    assert_equal(manifest.get("name"), "Manifest Name", "manifest name")
    assert_equal(manifest.get("description"), "Manifest Description", "manifest description")
    assert_equal(
        manifest.get("id"), f"{state.fixture}/identity?x=1", "resolved manifest id"
    )
    assert_equal(
        manifest.get("startUrl"),
        f"{state.fixture}/chromium-app-manifests/start?x=2#fragment",
        "resolved manifest start URL",
    )
    assert_equal(manifest.get("scope"), expected_manifest_scope, "resolved manifest scope")
    assert_equal(manifest.get("display"), "kStandalone", "manifest display")
    assert_equal(
        manifest.get("displayOverrides"),
        ["kFullscreen", "kBrowser"],
        "manifest display overrides",
    )
    assert_equal(manifest.get("orientation"), "PORTRAIT_PRIMARY", "manifest orientation")
    assert_equal(
        manifest.get("backgroundColor"),
        "rgba(17,34,51,0.5019607843137255)",
        "manifest background color",
    )
    assert_equal(manifest.get("themeColor"), "rgba(255,0,0,1)", "manifest theme color")
    if not manifest.get("icons") or not manifest.get("shortcuts"):
        raise SmokeError(f"Page.getAppManifest should expose icons and shortcuts: {explicit}")
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        1,
        "first successful manifest request count",
    )

    matched = await state.cdp.send("Page.getAppManifest", {"manifestId": manifest["id"]})
    assert_equal(matched.get("manifest", {}).get("id"), manifest["id"], "matching manifest id")
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        1,
        "successful manifest result should be document-cached",
    )
    mismatch = await _send_cdp_expect_optional_error(
        state.cdp,
        "Page.getAppManifest",
        {"manifestId": manifest["id"] + "-mismatch"},
    )
    if not mismatch or "does not match the input" not in mismatch["message"]:
        raise SmokeError(f"Page.getAppManifest should reject a mismatched manifestId: {mismatch}")
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        1,
        "manifestId validation should reuse the document cache",
    )

    transient_link_change_network_start = len(state.subresource_events)
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "(() => {"
                "const link = document.querySelector('link[rel~=manifest]');"
                "const href = link.getAttribute('href');"
                "link.setAttribute('href', '/chromium-app-manifests/invalid.webmanifest');"
                "link.setAttribute('href', href);"
                "})()"
            ),
            "returnByValue": True,
        },
    )
    restored_link = await state.cdp.send("Page.getAppManifest")
    assert_equal(
        restored_link.get("manifest", {}).get("id"),
        manifest["id"],
        "manifest after a transient href change",
    )
    await _assert_manifest_network_lifecycle(
        state.subresource_events,
        transient_link_change_network_start,
        expected_manifest_url,
    )
    valid_requests_after_link_change = state.fixture_server.request_count(
        valid_manifest_route
    )

    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "document.querySelector('link[rel~=manifest]')"
                ".setAttribute('crossorigin', 'use-credentials')"
            ),
            "returnByValue": True,
        },
    )
    credentials_changed = await state.cdp.send("Page.getAppManifest")
    assert_equal(
        credentials_changed.get("manifest", {}).get("id"),
        manifest["id"],
        "manifest after crossorigin change",
    )
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        valid_requests_after_link_change,
        "crossorigin change should preserve Chromium's manifest cache",
    )

    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "document.querySelector('link[rel~=manifest]')"
                ".setAttribute('rel', 'alternate')"
            ),
            "returnByValue": True,
        },
    )
    removed = await state.cdp.send("Page.getAppManifest")
    assert_equal(removed.get("url"), "", "manifest after rel removal")
    assert_equal(
        state.fixture_server.request_count(valid_manifest_route),
        valid_requests_after_link_change,
        "rel removal should not fetch or return cached manifest",
    )

    invalid_manifest_route = "/chromium-app-manifests/invalid.webmanifest"
    state.fixture_server.reset_request_count(invalid_manifest_route)
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "const link = document.querySelector('link[rel~=alternate]');"
                "link.setAttribute('rel', 'manifest');"
                f"link.setAttribute('href', '{invalid_manifest_route}')"
            ),
            "returnByValue": True,
        },
    )
    invalid_after_href_change = await state.cdp.send("Page.getAppManifest")
    if not any(
        error.get("critical") == 1
        for error in invalid_after_href_change.get("errors") or []
    ):
        raise SmokeError(
            "href change should replace the cached manifest with the newly fetched parse result: "
            f"{invalid_after_href_change}"
        )
    assert_equal(
        state.fixture_server.request_count(invalid_manifest_route),
        1,
        "href change should fetch the new manifest",
    )
    await state.cdp.send("Page.getAppManifest")
    assert_equal(
        state.fixture_server.request_count(invalid_manifest_route),
        2,
        "critical manifest parse failures should not be cached",
    )

    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-app-manifest-invalid/page"
    )
    invalid = await state.cdp.send("Page.getAppManifest")
    if "data" in invalid:
        raise SmokeError(f"a critically invalid manifest should omit data: {invalid}")
    if not any(error.get("critical") == 1 for error in invalid.get("errors") or []):
        raise SmokeError(f"an invalid manifest should report a critical parse error: {invalid}")

    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-app-manifest-missing/page"
    )
    missing = await state.cdp.send("Page.getAppManifest")
    assert_equal(
        missing.get("url"),
        f"{state.fixture}/chromium-app-manifests/missing.webmanifest",
        "missing manifest URL",
    )
    assert_equal(missing.get("data"), "", "missing manifest data")
    assert_equal(
        missing.get("manifest", {}).get("startUrl"),
        f"{state.fixture}/chromium-app-manifest-missing/page",
        "missing manifest default start URL",
    )

    await _navigate_with_cdp_until_dom_ready(
        state, f"{state.fixture}/chromium-app-manifest-redirect/page"
    )
    redirected = await state.cdp.send("Page.getAppManifest")
    assert_equal(
        redirected.get("url"),
        f"{state.fixture}/chromium-app-manifest-final/final.webmanifest",
        "redirected manifest final URL",
    )
    assert_equal(
        redirected.get("manifest", {}).get("startUrl"),
        f"{state.fixture}/chromium-app-manifest-final/start",
        "redirected manifest resolution base",
    )

    dynamic_url = f"{state.fixture}/chromium-app-manifest-dynamic/page"
    await _navigate_with_cdp_until_dom_ready(state, dynamic_url)
    before = await state.cdp.send("Page.getAppManifest")
    assert_equal(before.get("url"), "", "dynamic manifest before insertion")
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": "document.head.insertAdjacentHTML('beforeend', '<link rel=manifest href=/chromium-app-manifests/app.webmanifest>')",
            "returnByValue": True,
        },
    )
    after = await state.cdp.send("Page.getAppManifest")
    assert_equal(after.get("url"), expected_manifest_url, "dynamic manifest after insertion")

    await _navigate_with_cdp_until_dom_ready(state, dynamic_url)
    data_manifest_url = (
        "data:application/manifest+json,"
        "%7B%22start_url%22%3A%22relative-start%22%2C%22scope%22%3A%22.%2F%22%2C"
        "%22icons%22%3A%5B%7B%22src%22%3A%22icon.png%22%7D%5D%7D"
    )
    await state.cdp.send(
        "Runtime.evaluate",
        {
            "expression": (
                "document.head.insertAdjacentHTML('beforeend', "
                f"'<link rel=manifest href={data_manifest_url}>')"
            ),
            "returnByValue": True,
        },
    )
    embedded = await state.cdp.send("Page.getAppManifest")
    embedded_manifest = embedded.get("manifest") or {}
    expected_embedded_base = f"{state.fixture}/chromium-app-manifest-dynamic/"
    assert_equal(embedded.get("url"), data_manifest_url, "data manifest URL")
    assert_equal(
        embedded_manifest.get("startUrl"),
        expected_embedded_base + "relative-start" if is_moli_browser() else dynamic_url,
        "data manifest document-relative start URL",
    )
    assert_equal(
        embedded_manifest.get("scope"),
        expected_embedded_base,
        "data manifest document-relative scope",
    )
    assert_equal(
        (embedded_manifest.get("icons") or [{}])[0].get("url"),
        expected_embedded_base + "icon.png" if is_moli_browser() else None,
        "data manifest document-relative icon URL",
    )
    state.record("chromium_page_get_app_manifest_sample")

async def _assert_manifest_network_lifecycle(
    events: list[dict[str, Any]],
    start: int,
    expected_url: str,
) -> None:
    def matching_request() -> dict[str, Any] | None:
        return next(
            (
                event
                for event in events[start:]
                if event.get("method") == "Network.requestWillBeSent"
                and event.get("params", {}).get("request", {}).get("url") == expected_url
            ),
            None,
        )

    def has_terminal() -> bool:
        request = matching_request()
        if request is None:
            return False
        request_id = request.get("params", {}).get("requestId")
        return any(
            event.get("method") in {"Network.loadingFinished", "Network.loadingFailed"}
            and event.get("params", {}).get("requestId") == request_id
            for event in events[start:]
        )

    await wait_until(has_terminal, "Page.getAppManifest Network lifecycle")
    request = matching_request()
    if request is None:
        raise SmokeError(f"manifest request event is missing for {expected_url}")
    request_params = request.get("params", {})
    assert_equal(request_params.get("type"), "Manifest", "manifest request resource type")
    request_id = request_params.get("requestId")
    matching = [
        event
        for event in events[start:]
        if event.get("params", {}).get("requestId") == request_id
    ]
    methods = [event.get("method") for event in matching]
    if "Network.loadingFailed" in methods:
        raise SmokeError(f"manifest request should not fail: {matching}")
    required = [
        "Network.requestWillBeSent",
        "Network.responseReceived",
        "Network.loadingFinished",
    ]
    if any(method not in methods for method in required):
        raise SmokeError(f"manifest request lifecycle is incomplete: {matching}")
    indexes = [methods.index(method) for method in required]
    if indexes != sorted(indexes):
        raise SmokeError(f"manifest request lifecycle is out of order: {matching}")
    response = matching[methods.index("Network.responseReceived")]
    assert_equal(
        response.get("params", {}).get("type"),
        "Manifest",
        "manifest response resource type",
    )


async def run_page_manifest_and_layout(state: SmokeState) -> None:
    await _verify_chromium_page_get_app_manifest_sample(state)
    await _verify_chromium_page_layout_metrics_sample(state)


async def run_idle_override(state: SmokeState) -> None:
    await _verify_chromium_idle_override_sample(state)
