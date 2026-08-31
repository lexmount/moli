from __future__ import annotations

import base64
from typing import Any

from ..assertions import SmokeError, assert_equal, record_contract
from ..config import REPO_ROOT


async def run_font_face_payload_group(
    browser: Any,
    fixture: str,
    results: list[dict[str, Any]],
) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        valid_woff2 = base64.b64encode(
            (REPO_ROOT / "moli-layout/tests/fixtures/moli-ahem.woff2").read_bytes()
        ).decode("ascii")
        observed = await page.evaluate(
            """async validWoff2 => {
              const invalidFaces = [
                new FontFace('InvalidCss', 'garbage'),
                new FontFace(
                  'InvalidDataUrl',
                  'url("data:font/woff2;base64,d09GMmdhcmJhZ2U=")'
                ),
                new FontFace(
                  'InvalidBuffer',
                  new TextEncoder().encode('wOF2garbage')
                ),
              ];
              const invalidInitial = invalidFaces.map(face => face.status);
              const invalidLoaded = invalidFaces.map(face => face.loaded);
              const invalidSamePromise = invalidFaces.map(
                (face, index) => face.load() === invalidLoaded[index]
              );
              const invalidOutcomes = await Promise.all(invalidFaces.map(
                async (face, index) => {
                  try {
                    await invalidLoaded[index];
                    return 'resolved';
                  } catch (error) {
                    return error.name;
                  }
                }
              ));

              const bytes = Uint8Array.from(
                atob(validWoff2),
                character => character.charCodeAt(0)
              );
              const padded = new Uint8Array(bytes.length + 5);
              padded.set(bytes, 3);
              const offsetView = new Uint8Array(padded.buffer, 3, bytes.length);
              const validFaces = [
                new FontFace(
                  'ValidDataUrl',
                  `url("data:font/woff2;base64,${validWoff2}")`
                ),
                new FontFace('ValidOffsetBuffer', offsetView),
              ];
              const validInitial = validFaces.map(face => face.status);
              const validLoaded = validFaces.map(face => face.loaded);
              const validSamePromise = validFaces.map(
                (face, index) => face.load() === validLoaded[index]
              );
              const validOutcomes = await Promise.all(validFaces.map(
                async (face, index) => {
                  try {
                    return await validLoaded[index] === face
                      ? 'resolved-self'
                      : 'resolved-other';
                  } catch (error) {
                    return error.name;
                  }
                }
              ));

              return {
                invalidInitial,
                invalidSamePromise,
                invalidOutcomes,
                invalidFinal: invalidFaces.map(face => face.status),
                validInitial,
                validSamePromise,
                validOutcomes,
                validFinal: validFaces.map(face => face.status),
              };
            }""",
            valid_woff2,
        )

        assert_equal(
            observed.get("invalidInitial", [None, None, None])[0],
            "error",
            "malformed FontFace CSS source is synchronously invalid",
        )
        assert_equal(
            observed.get("invalidInitial", [None, None, None])[2],
            "error",
            "malformed FontFace BufferSource is synchronously invalid",
        )
        if observed.get("invalidInitial", [None, None])[1] not in {"unloaded", "error"}:
            raise SmokeError(
                "malformed data-URL FontFace has an invalid initial status: "
                f"{observed.get('invalidInitial')!r}"
            )
        assert_equal(
            observed.get("invalidSamePromise"),
            [True, True, True],
            "FontFace.load reuses the loaded promise for malformed payloads",
        )
        assert_equal(
            observed.get("invalidOutcomes"),
            ["SyntaxError", "NetworkError", "SyntaxError"],
            "malformed FontFace payload rejection classes",
        )
        assert_equal(
            observed.get("invalidFinal"),
            ["error", "error", "error"],
            "malformed FontFace payload terminal statuses",
        )
        assert_equal(
            observed.get("validInitial", [None, None])[1],
            "loaded",
            "valid offset BufferSource is parsed synchronously",
        )
        if observed.get("validInitial", [None])[0] not in {"unloaded", "loaded"}:
            raise SmokeError(
                "valid data-URL FontFace has an invalid initial status: "
                f"{observed.get('validInitial')!r}"
            )
        assert_equal(
            observed.get("validSamePromise"),
            [True, True],
            "FontFace.load reuses the loaded promise for valid payloads",
        )
        assert_equal(
            observed.get("validOutcomes"),
            ["resolved-self", "resolved-self"],
            "valid FontFace payloads resolve with their FontFace",
        )
        assert_equal(
            observed.get("validFinal"),
            ["loaded", "loaded"],
            "valid FontFace payload terminal statuses",
        )
        record_contract(
            results,
            "font_face_payload_validation",
            contract=(
                "Malformed CSS, decoded data-URL, and BufferSource font payloads "
                "reject with Chromium-compatible DOMException classes while valid "
                "data and an offset ArrayBufferView remain accepted."
            ),
            source="Debian Chromium 145 executable oracle and CSS Font Loading API",
            commands=["Runtime.evaluate"],
            observed=observed,
        )
    finally:
        await context.close()


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
