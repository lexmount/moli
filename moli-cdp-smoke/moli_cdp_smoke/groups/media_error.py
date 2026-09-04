from __future__ import annotations

from typing import Any

from ..assertions import SmokeError, assert_equal, record_contract
async def run_media_error_group(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        await page.evaluate(
            """() => {
              const video = document.createElement('video');
              globalThis.__moliMediaErrorProbe = {
                video,
                errors: [],
                firstError: null,
              };
              video.addEventListener('error', () => {
                const error = video.error;
                __moliMediaErrorProbe.errors.push({
                  error,
                  code: error && error.code,
                  message: error && error.message,
                  stableDuringHandler: video.error === error,
                });
                if (!__moliMediaErrorProbe.firstError) {
                  __moliMediaErrorProbe.firstError = error;
                }
              });
              video.src = 'http://[';
              document.body.append(video);
            }"""
        )
        await page.wait_for_function(
            "globalThis.__moliMediaErrorProbe.errors.length === 1",
            timeout=10_000,
        )
        first = await page.evaluate(
            """() => {
              const probe = __moliMediaErrorProbe;
              const video = probe.video;
              const error = probe.firstError;
              const descriptor = Object.getOwnPropertyDescriptor(
                MediaError.prototype,
                'code'
              );
              let receiverError = null;
              try {
                Reflect.get(MediaError.prototype, 'code', {});
              } catch (caught) {
                receiverError = caught.name;
              }
              let constructorError = null;
              try {
                new MediaError();
              } catch (caught) {
                constructorError = caught.name;
              }
              return {
                eventCount: probe.errors.length,
                eventCode: probe.errors[0].code,
                eventMessage: probe.errors[0].message,
                stableDuringHandler: probe.errors[0].stableDuringHandler,
                instance: error instanceof MediaError,
                code: error && error.code,
                message: error && error.message,
                stableAfterHandler: video.error === error,
                networkState: video.networkState,
                readyState: video.readyState,
                constants: [
                  MediaError.MEDIA_ERR_ABORTED,
                  MediaError.MEDIA_ERR_NETWORK,
                  MediaError.MEDIA_ERR_DECODE,
                  MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED,
                ],
                descriptor: [
                  typeof descriptor.get,
                  descriptor.set === undefined,
                  descriptor.enumerable,
                  descriptor.configurable,
                ],
                receiverError,
                constructorError,
              };
            }"""
        )
        assert_equal(first.get("eventCount"), 1, "media error event count")
        assert_equal(first.get("eventCode"), 4, "MediaError is visible in the error handler")
        if not first.get("eventMessage"):
            raise SmokeError("MediaError.message is empty in the error handler")
        assert_equal(first.get("stableDuringHandler"), True, "MediaError handler identity")
        assert_equal(first.get("instance"), True, "HTMLMediaElement.error brand")
        assert_equal(first.get("code"), 4, "unsupported media source error code")
        if not first.get("message"):
            raise SmokeError("HTMLMediaElement.error.message is empty")
        assert_equal(first.get("stableAfterHandler"), True, "MediaError stable getter identity")
        assert_equal(first.get("networkState"), 3, "failed media network state")
        assert_equal(first.get("readyState"), 0, "failed media ready state")
        assert_equal(first.get("constants"), [1, 2, 3, 4], "MediaError constants")
        assert_equal(
            first.get("descriptor"),
            ["function", True, True, True],
            "MediaError.code readonly Web IDL descriptor",
        )
        assert_equal(first.get("receiverError"), "TypeError", "MediaError receiver brand check")
        assert_equal(first.get("constructorError"), "TypeError", "MediaError illegal constructor")

        cleared = await page.evaluate(
            """() => {
              const probe = __moliMediaErrorProbe;
              probe.video.load();
              return probe.video.error === null;
            }"""
        )
        assert_equal(cleared, True, "HTMLMediaElement.load clears MediaError synchronously")
        await page.wait_for_function(
            "globalThis.__moliMediaErrorProbe.errors.length === 2",
            timeout=10_000,
        )
        reloaded = await page.evaluate(
            """() => {
              const probe = __moliMediaErrorProbe;
              const second = probe.video.error;
              const result = {
                eventCount: probe.errors.length,
                code: second && second.code,
                newIdentity: second !== probe.firstError,
                stableIdentity: second === probe.video.error,
              };
              probe.video.removeAttribute('src');
              probe.video.load();
              result.clearedWithoutSource = probe.video.error === null;
              return result;
            }"""
        )
        assert_equal(
            reloaded,
            {
                "eventCount": 2,
                "code": 4,
                "newIdentity": True,
                "stableIdentity": True,
                "clearedWithoutSource": True,
            },
            "MediaError clear and repeated-failure lifecycle",
        )
        record_contract(
            results,
            "media_error_failure_lifecycle",
            contract=(
                "A terminal unsupported-source failure publishes a stable MediaError "
                "before the error event, load() clears it synchronously, and a later "
                "failure creates a fresh error object."
            ),
            source="Debian Chromium 145 executable oracle and HTML media element lifecycle",
            commands=["Runtime.evaluate"],
            observed={"first": first, "reloaded": reloaded},
        )
    finally:
        await context.close()
