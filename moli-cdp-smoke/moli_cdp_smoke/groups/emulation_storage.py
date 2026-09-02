from __future__ import annotations

import json
import time
import urllib.parse
import urllib.request
from typing import Any

from ..assertions import SmokeError, assert_equal, record
from ..png_image import decode_png


async def run_emulation_storage_group(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    await run_playwright_screenshot_clip_surface(browser, fixture, results)
    await run_geolocation_override_smoke(browser, fixture, results)
    await run_storage_and_cookie_isolation_smoke(browser, fixture, results)
    await run_indexeddb_baseline_smoke(browser, fixture, results)
    await run_browser_context_profile_smoke(browser, fixture, results)


async def run_playwright_screenshot_clip_surface(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context(viewport={"width": 640, "height": 360}, device_scale_factor=2)
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        initial = decode_png(await page.screenshot())
        assert_equal(
            (initial.width, initial.height),
            (1280, 720),
            "Playwright viewport screenshot applies live DPR",
        )

        await page.set_viewport_size({"width": 320, "height": 240})
        resized = decode_png(await page.screenshot())
        assert_equal(
            (resized.width, resized.height),
            (640, 480),
            "Playwright resized viewport screenshot applies live DPR",
        )
        record(
            results,
            "playwright_screenshot_clip_surface",
            {"initial": [1280, 720], "resized": [640, 480], "deviceScaleFactor": 2},
        )
    finally:
        await context.close()


async def run_geolocation_override_smoke(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    context = await browser.new_context(permissions=["geolocation"])
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        primary = await context.new_cdp_session(page)
        attached = await context.new_cdp_session(page)
        try:
            await primary.send(
                "Emulation.setGeolocationOverride",
                {"latitude": 48.85, "longitude": 2.35, "accuracy": 5},
            )
            assert_equal(
                await _read_geolocation(page),
                {"ok": True, "latitude": 48.85, "longitude": 2.35, "accuracy": 5},
                "CDP geolocation position override",
            )

            await page.reload(wait_until="load", timeout=10_000)
            assert_equal(
                await _read_geolocation(page),
                {"ok": True, "latitude": 48.85, "longitude": 2.35, "accuracy": 5},
                "CDP geolocation override across navigation",
            )

            await primary.send("Emulation.setGeolocationOverride", {})
            unavailable = await _read_geolocation(page)
            assert_equal(unavailable.get("ok"), False, "CDP explicit unavailable result")
            assert_equal(unavailable.get("code"), 2, "CDP explicit unavailable error code")

            await attached.send(
                "Emulation.setGeolocationOverride",
                {"latitude": 35, "longitude": 139, "accuracy": 3},
            )
            assert_equal(
                await _read_geolocation(page),
                {"ok": True, "latitude": 35, "longitude": 139, "accuracy": 3},
                "CDP attached session geolocation override",
            )

            await primary.send("Emulation.clearGeolocationOverride")
            cleared = await _read_geolocation(page)
            assert_equal(cleared.get("ok"), False, "CDP geolocation clear restores provider")
            record(results, "geolocation_override_set_unavailable_clear")
        finally:
            await attached.detach()
            await primary.detach()
    finally:
        await context.close()


async def _read_geolocation(page: Any) -> dict[str, Any]:
    return await page.evaluate(
        """() => new Promise(resolve => {
          navigator.geolocation.getCurrentPosition(
            position => resolve({
              ok: true,
              latitude: position.coords.latitude,
              longitude: position.coords.longitude,
              accuracy: position.coords.accuracy,
            }),
            error => resolve({ok: false, code: error.code, message: error.message}),
            {timeout: 300, maximumAge: 0}
          );
        })"""
    )


async def run_storage_and_cookie_isolation_smoke(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    context_a = await browser.new_context()
    context_b = await browser.new_context()
    try:
        page_a = await context_a.new_page()
        await page_a.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page_a.evaluate(
            """() => {
              localStorage.clear();
              sessionStorage.clear();
              localStorage.setItem('local-smoke', 'local-value');
              sessionStorage.setItem('session-smoke', 'session-value');
            }"""
        )
        await page_a.reload(wait_until="load", timeout=10_000)
        storage_after_reload = await page_a.evaluate(
            """() => ({
              local: localStorage.getItem('local-smoke'),
              session: sessionStorage.getItem('session-smoke'),
            })"""
        )
        assert_equal(storage_after_reload, {"local": "local-value", "session": "session-value"}, "storage persists across reload")

        await context_a.add_cookies([{"name": "isolatedCookie", "value": "a", "url": fixture}])
        page_b = await context_b.new_page()
        await page_b.goto(f"{fixture}/echo-cookie", wait_until="load", timeout=10_000)
        cookie_echo_b = await page_b.text_content("body")
        if "isolatedCookie=a" in cookie_echo_b:
            raise SmokeError(f"cookie leaked across browser contexts: {cookie_echo_b}")
        record(results, "storage_cookie_isolation_smoke")
    finally:
        await context_a.close()
        await context_b.close()


async def run_indexeddb_baseline_smoke(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        value = await page.evaluate(
            """async () => {
              const db = await new Promise((resolve, reject) => {
                const request = indexedDB.open('smoke-db', 1);
                request.onupgradeneeded = () => request.result.createObjectStore('store');
                request.onerror = () => reject(request.error);
                request.onsuccess = () => resolve(request.result);
              });
              await new Promise((resolve, reject) => {
                const tx = db.transaction('store', 'readwrite');
                tx.objectStore('store').put('indexed-value', 'key');
                tx.oncomplete = resolve;
                tx.onerror = () => reject(tx.error);
              });
              return await new Promise((resolve, reject) => {
                const tx = db.transaction('store', 'readonly');
                const request = tx.objectStore('store').get('key');
                request.onsuccess = () => resolve(request.result);
                request.onerror = () => reject(request.error);
              });
            }"""
        )
        assert_equal(value, "indexed-value", "IndexedDB put/get baseline")
        record(results, "indexeddb_baseline_smoke")
    finally:
        await context.close()


async def run_browser_context_profile_smoke(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    profile_user_agent = "MoliProfileSmoke/1.0"
    profile_context = await browser.new_context(
        user_agent=profile_user_agent,
        locale="zh-CN",
        timezone_id="Asia/Shanghai",
        extra_http_headers={"x-moli-profile-smoke": "context-extra-header"},
    )
    try:
        profile_page = await profile_context.new_page()
        profile_referer = f"{fixture}/profile-referer"
        token = f"profile-{int(time.time() * 1000)}"
        await profile_page.goto(
            f"{fixture}/profile-headers?token={urllib.parse.quote(token)}",
            wait_until="load",
            timeout=10_000,
            referer=profile_referer,
        )
        headers = json.loads(
            urllib.request.urlopen(f"{fixture}/profile-result?token={urllib.parse.quote(token)}", timeout=5).read().decode()
        )
        if not headers:
            raise SmokeError(f"profile fixture did not capture request for {token}")
        assert_equal(headers.get("userAgent"), profile_user_agent, "profile context User-Agent header")
        if "zh-cn" not in str(headers.get("acceptLanguage") or "").lower():
            raise SmokeError(f"profile context Accept-Language header missing zh-CN: {headers.get('acceptLanguage')}")
        assert_equal(headers.get("extraHeader"), "context-extra-header", "profile context extra HTTP header")
        assert_equal(headers.get("referer"), profile_referer, "profile context goto referer header")
        runtime = await profile_page.evaluate(
            """() => ({
              userAgent: navigator.userAgent,
              language: navigator.language,
              languages: navigator.languages,
              timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
            })"""
        )
        assert_equal(runtime.get("userAgent"), profile_user_agent, "profile context navigator.userAgent")
        assert_equal(runtime.get("language"), "zh-CN", "profile context navigator.language")
        assert_equal(runtime.get("languages", [None])[0], "zh-CN", "profile context navigator.languages[0]")
        assert_equal(runtime.get("timeZone"), "Asia/Shanghai", "profile context timezone")
        record(results, "browser_context_profile_overrides")
    finally:
        await profile_context.close()
