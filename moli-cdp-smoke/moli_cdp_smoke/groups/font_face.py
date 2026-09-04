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

