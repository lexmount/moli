"""Run deterministic DOM-reader fixtures on a real CDP browser, without screenshots.

Not part of offline unittest discovery: pass an explicit owned browser binary.
"""
from __future__ import annotations

import argparse
import asyncio
from copy import deepcopy
from contextlib import contextmanager
from html import escape
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
import unittest

from moli_benchmark.artifacts import write_json as save
from moli_benchmark.fingerprint.browser import browser_session
from moli_benchmark.fingerprint.probe import dom_probe as survey_probe
from moli_benchmark.fingerprint.probe import site_probe
from moli_benchmark.fingerprint.cases import CASES


@contextmanager
def local_origin():
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            body = b'<!doctype html><title>Local fixture</title>'
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_args):
            pass

    with ThreadingHTTPServer(('127.0.0.1', 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield f'http://127.0.0.1:{server.server_port}/fingerprint-check'
        finally:
            server.shutdown()
            thread.join(timeout=2)


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

    # All adapters reject absent results; the FP DOM shell cannot replace its API.
    for case in CASES:
        yield case.id + '-empty', case.id, '', None, {'valid': False}
    for site in ['device-browser-static', 'device-browser-behavior']:
        for is_bot in [True, False]:
            value = {'isBot': is_bot, 'details': {'isAutomatedWithCDP': False, 'hasWebdriverTrue': False},
                     'visitorId': 'do not retain'}
            html = '<pre id=jsonResult>' + escape(json.dumps(value)) + '</pre>'
            yield site + str(is_bot), site, html, None, {'valid': True, 'isBot': is_bot, 'details': value['details']}
        yield site + '-prose', site, '<p>isBot: false</p>', None, {'valid': False}
        yield site + '-malformed', site, '<pre id=jsonResult>{"isBot":"false"}</pre>', None, {'valid': False}
    yield 'proxy-errors-not-pass', 'incolumitas-proxy', '<p>WebRTC Proxy net::ERR_CERT_COMMON_NAME_INVALID</p>', None, {
        'valid': False, 'completion': 'unverified', 'certificateErrors': ['net::ERR_CERT_COMMON_NAME_INVALID']}
    newer = {'webdriverPresent': 'OK', 'behavioralClassificationScore': 0}
    detection = {'intoli': {'webdriver': 'FAIL'}, 'fpscanner': {'HEADCHR': 'OK'}}
    inco = '<pre id=new-tests>' + escape(json.dumps(newer)) + '</pre><pre id=detection-tests>' + escape(json.dumps(detection)) + '</pre>'
    yield 'inco-zero-and-fail', 'incolumitas-bot', inco, None, {
        'valid': True, 'intoli': {'webdriver': 'FAIL'}, 'behavior': {'jsonScore': 0, 'domScore': None}}
    sanny_ids = ['user-agent-result', 'webdriver-result', 'advanced-webdriver-result', 'chrome-result',
                 'permissions-result', 'plugins-length-result', 'plugins-type-result', 'languages-result',
                 'webgl-vendor', 'webgl-renderer', 'broken-image-dimensions']
    sanny = '<table>' + ''.join(f'<tr><td id={name} class=failed>test</td></tr>' for name in sanny_ids) + '</table>'
    yield 'sanny-failed-is-complete', 'sannysoft', sanny, None, {'valid': True}
    pixelscan = '<div checkervalue>No masking detected</div><div checkervalue>Automated behavior detected</div>'
    yield 'pixel-negative-verdict', 'pixelscan', pixelscan, None, {'valid': True, 'masking': False, 'automated': True}
    yield 'pixel-ambiguous', 'pixelscan', pixelscan + '<div checkervalue>Masking detected</div>', None, {'valid': False, 'masking': None}
    js_fields = {'userAgent': 'Mozilla/5.0 fixture', 'platform': 'Win32', 'hardwareConcurrency': '4', 'webdriver': 'false'}
    js = ''.join(f'<div id=js-{key}>{value}</div>' for key, value in js_fields.items())
    yield 'leaks-js', 'browserleaks-js', js, None, {'valid': True, 'fields': js_fields}
    yield 'leaks-js-pending', 'browserleaks-js', js.replace('>4<', '>Loading...<'), None, {'valid': False}
    gl = '<table><tr><td>WebGL Report Hash</td><td><span id=gl-report-hash>' + HASH + '</span>Pretty-print</td></tr></table>'
    yield 'leaks-gl', 'browserleaks-webgl', gl, None, {'valid': True, 'unavailable': False}
    yield 'leaks-gl-pending', 'browserleaks-webgl', gl.replace(HASH, 'Loading...'), None, {'valid': False}
    # Match the live site's ico() prefix, not just a toy cell containing "False".
    decorate = lambda value: ('<span class=true>✔</span> ' if value == 'True' else '<span class=false>✖</span> ') + value
    status = lambda a, b: f'<table><tr><td id=gl1-status>{decorate(a)}</td><td id=gl2-status>{decorate(b)}</td></tr></table>'
    yield 'leaks-gl-unavailable', 'browserleaks-webgl', status('False (supported, but disabled or unavailable)', 'False'), None, {
        'valid': True, 'unavailable': True, 'capabilities': {'webgl1': 'unavailable', 'webgl2': 'unsupported'}}
    yield 'leaks-gl-explanation', 'browserleaks-webgl', '<p>WebGL is disabled or unavailable</p>', None, {'valid': False}
    yield 'leaks-gl-unsupported', 'browserleaks-webgl', status('False', 'False'), None, {'valid': True, 'unavailable': True}
    yield 'leaks-gl-partial-status', 'browserleaks-webgl', status('False', 'Loading...'), None, {'valid': False}
    yield 'leaks-gl-loading-report', 'browserleaks-webgl', status('True', 'True'), None, {'valid': False}
    yield 'leaks-gl-child-decoy', 'browserleaks-webgl', status('', '').replace('✖', 'False'), None, {'valid': False}


async def check(args):
    assertions = unittest.TestCase()
    results = []
    # Even a supported-but-missing report must not execute the rendered-text fallback.
    probe = survey_probe("() => { throw new Error('unexpected fallback'); }")
    async with browser_session(args.binary, args.engine, args.output) as (page, _):
        with local_origin() as url:
            await page.goto(url, wait_until='domcontentloaded', timeout=10000)
        combined_probe = site_probe()
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
                selected_probe = probe if site in {'browserscan-bot', 'browserscan-tls', 'creepjs'} else combined_probe
                actual = await asyncio.wait_for(page.evaluate(selected_probe, {"site": site}), 10)
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
