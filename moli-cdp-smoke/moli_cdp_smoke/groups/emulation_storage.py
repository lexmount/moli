from __future__ import annotations

import json
import time
import urllib.parse
import urllib.request
from typing import Any, Awaitable

from ..assertions import SmokeError, assert_equal, record, record_contract
from ..png_image import decode_png


async def run_emulation_storage_group(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    await run_playwright_screenshot_clip_surface(browser, fixture, results)
    await run_geolocation_override_smoke(browser, fixture, results)
    await run_locale_timezone_runtime_surface_smoke(browser, fixture, results)
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


async def run_locale_timezone_runtime_surface_smoke(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        baseline = await _read_locale_timezone_runtime(page)
        cdp = await context.new_cdp_session(page)
        try:
            await cdp.send("Emulation.setLocaleOverride", {"locale": "fr-FR"})
            await cdp.send(
                "Emulation.setTimezoneOverride",
                {"timezoneId": "Europe/Paris"},
            )
            runtime = await _read_locale_timezone_runtime(page)
            assert_equal(
                runtime.get("navigatorLanguage"),
                baseline.get("navigatorLanguage"),
                "locale override does not impersonate Accept-Language",
            )
            assert_equal(
                runtime.get("intlLocales"),
                {
                    "Collator": "fr-FR",
                    "DateTimeFormat": "fr-FR",
                    "ListFormat": "fr-FR",
                    "NumberFormat": "fr-FR",
                    "PluralRules": "fr-FR",
                    "RelativeTimeFormat": "fr-FR",
                    "Segmenter": "fr-FR",
                },
                "locale override drives default Intl constructors",
            )
            assert_equal(runtime.get("timezone"), "Europe/Paris", "Intl timezone override")
            assert_equal(runtime.get("frenchDecimal"), True, "Intl locale formatting override")
            assert_equal(
                runtime.get("winter"),
                [-60, 2024, 0, 1, 1, 1, 0, 0, 0],
                "winter Date timezone override",
            )
            assert_equal(
                runtime.get("summer"),
                [-120, 2024, 6, 1, 1, 2],
                "summer Date timezone override",
            )
            assert_equal(
                runtime.get("dateBoundary"),
                [2024, 0, 1, 1, 0, 30],
                "timezone override crosses the local calendar boundary",
            )
            assert_equal(
                runtime.get("localeDateStrings"),
                ["01/01/2024 01:00:00", "01/01/2024", "01:00:00"],
                "Date locale methods consume both overrides",
            )
            assert_equal(
                runtime.get("explicitOptions"),
                ["en-US", "UTC"],
                "explicit Intl options win over emulation defaults",
            )
            assert_equal(
                runtime.get("invalidOptions"),
                "TypeError",
                "Intl option validation survives default injection",
            )
            date_strings = runtime.get("dateStrings")
            if not isinstance(date_strings, list) or len(date_strings) != 3:
                raise SmokeError(f"Date string override returned an invalid shape: {date_strings!r}")
            expected_prefixes = (
                "Mon Jan 01 2024 01:00:00 GMT+0100",
                "Mon Jan 01 2024",
                "01:00:00 GMT+0100",
            )
            for value, prefix in zip(date_strings, expected_prefixes, strict=True):
                if not isinstance(value, str) or not value.startswith(prefix):
                    raise SmokeError(
                        f"Date string override: expected prefix {prefix!r}, got {value!r}"
                    )

            invalid_timezone_error = await _expect_protocol_error(
                cdp.send(
                    "Emulation.setTimezoneOverride",
                    {"timezoneId": "Mars/Olympus"},
                ),
                "invalid timezone override",
            )
            if "invalid timezone" not in invalid_timezone_error.lower():
                raise SmokeError(
                    "invalid timezone returned an unexpected protocol error: "
                    f"{invalid_timezone_error}"
                )
            assert_equal(
                await _read_locale_timezone_runtime(page),
                runtime,
                "rejected timezone override leaves the active state unchanged",
            )

            await cdp.send("Emulation.setLocaleOverride", {"locale": ""})
            locale_cleared = await _read_locale_timezone_runtime(page)
            assert_equal(
                locale_cleared.get("intlLocales"),
                baseline.get("intlLocales"),
                "locale override can be cleared independently",
            )
            assert_equal(
                [locale_cleared.get("timezone"), locale_cleared.get("winter")],
                ["Europe/Paris", runtime.get("winter")],
                "clearing locale preserves timezone Date semantics",
            )

            await cdp.send("Emulation.setLocaleOverride", {"locale": "fr-FR"})
            await cdp.send("Emulation.setTimezoneOverride", {"timezoneId": ""})
            timezone_cleared = await _read_locale_timezone_runtime(page)
            assert_equal(
                timezone_cleared.get("intlLocales"),
                runtime.get("intlLocales"),
                "clearing timezone preserves locale defaults",
            )
            assert_equal(
                [timezone_cleared.get("timezone"), timezone_cleared.get("winter")],
                [baseline.get("timezone"), baseline.get("winter")],
                "timezone override can be cleared independently",
            )

            await cdp.send(
                "Emulation.setTimezoneOverride",
                {"timezoneId": "Europe/Paris"},
            )
            await page.reload(wait_until="load", timeout=10_000)
            assert_equal(
                await _read_locale_timezone_runtime(page),
                runtime,
                "locale/timezone overrides survive navigation",
            )
            await cdp.send("Emulation.setLocaleOverride", {"locale": ""})
            await cdp.send("Emulation.setTimezoneOverride", {"timezoneId": ""})
            assert_equal(
                await _read_locale_timezone_runtime(page),
                baseline,
                "clearing locale/timezone overrides restores native defaults",
            )
            record_contract(
                results,
                "locale_timezone_runtime_surfaces",
                contract=(
                    "Locale and timezone overrides drive default Intl and local Date "
                    "surfaces, survive navigation, remain independently clearable, and "
                    "reject an invalid timezone without changing the prior state."
                ),
                source="Debian Chromium 145 CDP executable oracle",
                commands=[
                    "Emulation.setLocaleOverride",
                    "Emulation.setTimezoneOverride",
                    "Runtime.evaluate",
                    "Page.reload",
                ],
                observed={
                    "locale": "fr-FR",
                    "timezone": "Europe/Paris",
                    "winterOffsetMinutes": -60,
                    "summerOffsetMinutes": -120,
                    "invalidTimezoneRejected": True,
                },
            )
        finally:
            await cdp.detach()
    finally:
        await context.close()


async def _read_locale_timezone_runtime(page: Any) -> dict[str, Any]:
    return await page.evaluate(
        """() => {
          const winter = new Date('2024-01-01T00:00:00Z');
          const summer = new Date('2024-07-01T00:00:00Z');
          const dateBoundary = new Date('2023-12-31T23:30:00Z');
          const intlConstructors = [
            'Collator',
            'DateTimeFormat',
            'ListFormat',
            'NumberFormat',
            'PluralRules',
            'RelativeTimeFormat',
            'Segmenter',
          ];
          return {
            navigatorLanguage: navigator.language,
            intlLocales: Object.fromEntries(intlConstructors.map(name => [
              name,
              new Intl[name]().resolvedOptions().locale,
            ])),
            timezone: new Intl.DateTimeFormat().resolvedOptions().timeZone,
            frenchDecimal: new Intl.NumberFormat().format(1.5).endsWith(',5'),
            winter: [
              winter.getTimezoneOffset(),
              winter.getFullYear(),
              winter.getMonth(),
              winter.getDate(),
              winter.getDay(),
              winter.getHours(),
              winter.getMinutes(),
              winter.getSeconds(),
              winter.getMilliseconds(),
            ],
            summer: [
              summer.getTimezoneOffset(),
              summer.getFullYear(),
              summer.getMonth(),
              summer.getDate(),
              summer.getDay(),
              summer.getHours(),
            ],
            dateBoundary: [
              dateBoundary.getFullYear(),
              dateBoundary.getMonth(),
              dateBoundary.getDate(),
              dateBoundary.getDay(),
              dateBoundary.getHours(),
              dateBoundary.getMinutes(),
            ],
            dateStrings: [
              winter.toString(),
              winter.toDateString(),
              winter.toTimeString(),
            ],
            localeDateStrings: [
              winter.toLocaleString(),
              winter.toLocaleDateString(),
              winter.toLocaleTimeString(),
            ],
            explicitOptions: [
              new Intl.NumberFormat('en-US').resolvedOptions().locale,
              new Intl.DateTimeFormat('en-US', {timeZone: 'UTC'})
                .resolvedOptions().timeZone,
            ],
            invalidOptions: (() => {
              try {
                new Intl.DateTimeFormat(undefined, null);
                return 'accepted';
              } catch (error) {
                return error.name;
              }
            })(),
          };
        }"""
    )


async def _expect_protocol_error(awaitable: Awaitable[Any], label: str) -> str:
    try:
        await awaitable
    except Exception as error:  # Playwright exposes protocol failures as Error.
        return str(error)
    raise SmokeError(f"{label}: expected a protocol error")


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
