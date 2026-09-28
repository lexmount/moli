"""Run deterministic DOM-reader fixtures on a real CDP browser, without screenshots.

Not part of offline unittest discovery: pass an explicit owned browser binary.
"""
from __future__ import annotations

import argparse
import asyncio
from copy import deepcopy
from pathlib import Path
import unittest

from moli_benchmark.artifacts import write_json as save
from moli_benchmark.fingerprint.browser import browser_session
from moli_benchmark.fingerprint.probe import dom_probe as survey_probe


def bot(value: str) -> str:
    return f"<div><strong>Test <span>Results:</span></strong><svg></svg><strong>{value}</strong></div>"


def card(label: str, value: str) -> str:
    return (f"<section><div><div><h3>{label}</h3></div></div>"
            f"<div><div><p>{value}</p></div></div></section>")


HASH = "a" * 32
JA3 = "771,4865-4866,0-23-35,29-23,0"
JA4 = "t13d1516h2_" + "b" * 12 + "_" + "c" * 12
TLS = ("<h3>HTTP/2 Fingerprint</h3>" + card("Akamai Hash", HASH)
       + card("JA3", JA3) + card("JA4", JA4))
CREEP = {"lies": {"totalLies": 0}, "capturedErrors": {"data": []},
         "offlineAudioContext": {"lied": False}, "svg": {"lied": False},
         "headless": {"likeHeadlessRating": 38, "headlessRating": 0, "stealthRating": 0,
                      "headless": {"webDriverIsOn": False, "opaqueToken": "not retained"}}}


def fixtures():
    for verdict in ["Normal", "Robot"]:
        yield verdict, "browserscan-bot", bot(verdict), None, {"valid": True, "verdict": verdict}
    yield "missing", "browserscan-bot", "<p>Normal</p>", None, {"valid": False, "verdict": None}
    yield "pending", "browserscan-bot", bot("Loading..."), None, {"valid": False, "verdict": None}
    yield "hidden", "browserscan-bot", f"<div hidden>{bot('Normal')}</div>", None, {"valid": False}
    yield "template", "browserscan-bot", f"<template>{bot('Normal')}</template>", None, {"valid": False}
    yield "duplicate", "browserscan-bot", bot("Normal") + bot("Robot"), None, {"valid": False}
    yield "duplicate-value", "browserscan-bot", bot("Normal").replace(
        "</div>", "<strong>Robot</strong></div>"), None, {"valid": False}
    yield "hidden-decoy", "browserscan-bot", bot("Robot") + f"<aside hidden>{bot('Normal')}</aside>", None, {
        "valid": True, "verdict": "Robot"}
    yield "explanation-decoy", "browserscan-bot", bot("") + "<p>Normal</p>", None, {"valid": False}
    yield "tls-complete", "browserscan-tls", TLS, None, {"valid": True, "fields": {
        "akamaiHash": "populated", "ja3": "populated", "ja4": "populated"}}
    # Real ad scripts append a div inside p. Putting that in HTML source instead
    # would implicitly close p during parsing, exercising a different DOM.
    yield "tls-ad-annotation", "browserscan-tls", TLS + """<script>
      for (const p of document.querySelectorAll('section p')) {
        const ad = document.createElement('div');
        ad.textContent = 'Convert Data Files'; p.append(ad);
      }
    </script>""", None, {"valid": True}
    yield "tls-hash-only-in-ad", "browserscan-tls", TLS.replace(HASH, "") + f"""<script>
      const ad = document.createElement('div'); ad.textContent = '{HASH}';
      document.querySelector('section p').append(ad);
    </script>""", None, {"valid": False}
    yield "tls-hashes-in-prose", "browserscan-tls", (
        "<h3>HTTP/2 Fingerprint</h3>" + card("Akamai Hash", "") + card("JA3", "") + card("JA4", "")
        + f"<p>{HASH} {HASH}</p><script>const example='{HASH}'</script>"), None, {"valid": False}
    yield "tls-no-ja4", "browserscan-tls", TLS.replace(JA4, "Loading..."), None, {"valid": False}
    yield "tls-malformed", "browserscan-tls", TLS.replace(JA3, HASH), None, {"valid": False}
    yield "tls-duplicate", "browserscan-tls", TLS + card("JA3", JA3), None, {"valid": False}
    yield "tls-hidden", "browserscan-tls", f"<div aria-hidden=true>{TLS}</div>", None, {"valid": False}
    yield "creep-zero", "creepjs", "<p>99% headless</p>", CREEP, {
        "valid": True, "ratingsComplete": True,
        "percentages": ["38% like headless", "0% headless", "0% stealth"],
        "metrics": {"totalLies": 0, "capturedErrorCount": 0, "audioLied": False, "svgLied": False},
        "headlessSignals": {"headless": {"webDriverIsOn": False}, "likeHeadless": None, "stealth": None}}
    yield "creep-absent", "creepjs", "<p>0% headless</p>", None, {
        "valid": False, "ratingsComplete": False, "percentages": []}
    partial = deepcopy(CREEP)
    del partial["headless"]["headlessRating"]
    yield "creep-partial", "creepjs", "", partial, {"valid": True, "ratingsComplete": False,
        "ratings": {"likeHeadless": 38, "headless": None, "stealth": 0}}
    invalid = deepcopy(CREEP)
    invalid["lies"]["totalLies"] = "0"
    invalid["headless"].update(likeHeadlessRating=101, headlessRating=False, stealthRating="0")
    yield "creep-invalid-types", "creepjs", "", invalid, {
        "valid": False, "ratingsComplete": False, "percentages": []}


async def check(args):
    assertions = unittest.TestCase()
    results = []
    # Even a supported-but-missing report must not execute the rendered-text fallback.
    probe = survey_probe("() => { throw new Error('unexpected fallback'); }")
    async with browser_session(args.binary, args.engine, args.output) as (page, _):
        for name, site, html, fingerprint, expected in fixtures():
            record = {"fixture": name, "passed": False}
            try:
                await page.set_content(html, wait_until="domcontentloaded", timeout=10000)
                await page.evaluate("""fingerprint => {
                  window.Fingerprint = fingerprint;
                  const forbidden = () => { throw new Error('layout-dependent read'); };
                  Object.defineProperty(HTMLElement.prototype, 'innerText', {get: forbidden, configurable: true});
                  Element.prototype.getBoundingClientRect = forbidden;
                  window.getComputedStyle = forbidden;
                }""", fingerprint)
                actual = await asyncio.wait_for(page.evaluate(probe, {"site": site}), 10)
                for key, value in expected.items():
                    assertions.assertEqual(actual[key], value, f"{name}: {key}")
                record["passed"] = True
            except Exception as error:
                record.update(error_type=type(error).__name__, detail=str(error))
            results.append(record)
        # Only unrelated sites may use the supplied legacy collector.
        actual = await page.evaluate(survey_probe("arg => ({fallback: arg.site})"), {"site": "other"})
        assertions.assertEqual(actual, {"fallback": "other"})
        save(args.output / "fixtures.json", results)
    failures = [item for item in results if not item["passed"]]
    print(f"{args.engine}: {len(results) - len(failures)}/{len(results)} DOM fixtures passed; fallback dispatch passed")
    if failures:
        raise SystemExit(f"Fixture failures: {failures}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--engine", choices=["moli", "chromium"], required=True)
    parser.add_argument("--output", type=Path, required=True)
    asyncio.run(check(parser.parse_args()))
