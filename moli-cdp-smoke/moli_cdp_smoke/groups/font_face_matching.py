from __future__ import annotations

import base64
from typing import Any

from ..assertions import assert_equal, record_contract
from ..config import REPO_ROOT, is_moli_browser


FONT_MATCHING_PROBE = r"""async ({narrowData, wideData}) => {
  const bytes = data => Uint8Array.from(atob(data), c => c.charCodeAt(0));
  const load = async (query, text) => {
    try { return (await document.fonts.load(query, text)).map(face => face.family); }
    catch (error) { return error.name; }
  };
  const bad = (family, descriptors = {}) => {
    const face = new FontFace(family, 'url(data:font/ttf;base64,AA==)', descriptors);
    face.loaded.catch(() => {});
    document.fonts.add(face);
    return face;
  };
  const descriptorRanges = [];
  for (const [property, range, single, request, changed] of [
    ['weight','100 900','400','700','100'],
    ['stretch','75% 125%','100%','expanded','75%'],
    ['style','oblique 10deg 30deg','oblique 20deg','oblique 30deg','oblique 10deg'],
    ['weight','calc(100 + 200) calc(800 + 100)','700','800','100'],
    ['stretch','calc(50% + 25%) calc(100% + 25%)','100%','expanded','75%'],
    ['style','oblique calc(5deg + 5deg) calc(15deg + 15deg)','oblique 20deg','oblique 30deg','oblique 10deg'],
    ['weight','900 100','400','700','100'],
    ['stretch','125% 75%','100%','expanded','75%'],
    ['style','oblique 30deg 10deg','oblique 20deg','oblique 30deg','oblique 10deg'],
  ]) {
    const family = 'GeometryRange' + descriptorRanges.length;
    const good = new FontFace(family, bytes(wideData), {[property]:range});
    const other = new FontFace(family, bytes(narrowData), {[property]:single});
    document.fonts.add(good); document.fonts.add(other);
    const matches = await document.fonts.load(`${request} 40px ${family}`, 'AAAA');
    const target = document.createElement('span');
    target.textContent = 'AAAA'; target.style.cssText = `display:inline-block;font:${request} 40px ${family}`;
    document.body.append(target);
    const result = [matches.length, matches[0] === good, target.getBoundingClientRect().width];
    good[property] = changed;
    result.push(target.getBoundingClientRect().width);
    // Use a fresh family to distinguish stale selection from descriptor parsing.
    const freshFamily = family + 'Fresh';
    const replacement = new FontFace(freshFamily, bytes(wideData), {[property]: good[property]});
    const freshOther = new FontFace(freshFamily, bytes(narrowData), {[property]: single});
    document.fonts.add(replacement); document.fonts.add(freshOther);
    target.style.fontFamily = freshFamily;
    result.push(target.getBoundingClientRect().width);
    descriptorRanges.push(result);
  }
  const range = bad('ReviewRange', {unicodeRange: 'U+0041'});
  const text = {
    checks: ['B', ''].map(text => document.fonts.check('16px ReviewRange', text)),
    loads: [await load('16px ReviewRange', 'B'), await load('16px ReviewRange', '')],
    status: range.status,
    included: document.fonts.check('16px ReviewRange', 'A'),
  };
  const properties = [];
  const calculations = [];
  for (const [family, goodDescriptors, badDescriptors, query] of [
    ['ReviewMathWeight', {weight:'400'}, {weight:'700'}, 'calc(400 + 300) 16px ReviewMathWeight'],
    ['ReviewMathStyle', {style:'oblique 14deg'}, {style:'oblique 20deg'}, 'oblique calc(10deg + 10deg) 16px ReviewMathStyle'],
    ['ReviewMathWidth', {}, {stretch:'calc(50% + 25%)'}, '16px ReviewMathWidth'],
  ]) {
    document.fonts.add(new FontFace(family,bytes(wideData),goodDescriptors));
    bad(family,badDescriptors);
    calculations.push([document.fonts.check(query,'A'),await load(query,'A')]);
  }
  calculations.push([
    document.fonts.check('condensed 16px ReviewMathWidth', 'A'),
    await load('condensed 16px ReviewMathWidth', 'A'),
  ]);
  for (const [family, descriptors, selectedQuery] of [
    ['ReviewWeight', {weight: '700'}, 'bold 16px ReviewWeight'],
    ['ReviewStyle', {style: 'italic'}, 'italic 16px ReviewStyle'],
    ['ReviewWidth', {stretch: 'condensed'}, 'condensed 16px ReviewWidth'],
  ]) {
    const good = new FontFace(family, bytes(wideData));
    const other = bad(family, descriptors);
    document.fonts.add(good);
    properties.push([
      document.fonts.check(`16px ${family}`, 'A'),
      await load(`16px ${family}`, 'A'), other.status,
      document.fonts.check(selectedQuery, 'A'),
    ]);
  }
  const normal = new FontFace('ReviewCapabilityRange', bytes(wideData), {unicodeRange: 'U+0042'});
  document.fonts.add(normal);
  const bold = bad('ReviewCapabilityRange', {weight: '700', unicodeRange: 'U+0041'});
  const capabilityBeforeRange = [document.fonts.check('16px ReviewCapabilityRange', 'A'),
    await load('16px ReviewCapabilityRange', 'A'), bold.status];
  normal.unicodeRange = 'U+0041';
  const descriptorWrites = [document.fonts.check('16px ReviewCapabilityRange', 'A')];
  bold.weight = '400';
  descriptorWrites.push(document.fonts.check('16px ReviewCapabilityRange', 'A'));
  bold.weight = '700';
  descriptorWrites.push(document.fonts.check('16px ReviewCapabilityRange', 'A'),
    document.fonts.check('bolder 16px ReviewCapabilityRange', 'A'));

  bad('ReviewEmoji', {unicodeRange: 'U+1F600'});
  const scalars = ['😁', '😀'].map(text => document.fonts.check('16px ReviewEmoji', text));
  const first = new FontFace('ReviewFirstFamily', bytes(wideData));
  const second = new FontFace('ReviewSecondFamily', bytes(wideData));
  document.fonts.add(first); document.fonts.add(second);
  const familyOrder = await load('16px ReviewSecondFamily, ReviewFirstFamily', 'A');

  const span = document.createElement('span');
  span.textContent = 'AAAA';
  span.style.cssText = 'display:inline-block;font:40px ReviewOrder';
  document.body.append(span);
  const width = () => span.getBoundingClientRect().width;
  const narrow = new FontFace('ReviewOrder', bytes(narrowData));
  const wide = new FontFace('ReviewOrder', bytes(wideData));
  document.fonts.add(narrow);
  const order = [width()];
  document.fonts.add(wide); order.push(width());
  document.fonts.delete(narrow); document.fonts.delete(wide);
  document.fonts.add(wide); document.fonts.add(narrow); order.push(width());
  document.fonts.delete(wide); document.fonts.add(wide); order.push(width());

  const style = document.createElement('style');
  style.textContent = `@font-face {font-family:ReviewCss;src:url(data:font/ttf;base64,${narrowData})}`;
  document.head.append(style);
  const cssFace = [...document.fonts].find(face => face.family === 'ReviewCss');
  await cssFace.load();
  span.style.fontFamily = 'ReviewCss';
  const cssVsManual = [width()];
  const manual = new FontFace('ReviewCss', bytes(wideData));
  document.fonts.add(manual); cssVsManual.push(width());
  document.fonts.delete(manual); cssVsManual.push(width());

  const lateManual = new FontFace('ReviewLateCss', bytes(wideData));
  document.fonts.add(lateManual);
  span.style.fontFamily = 'ReviewLateCss';
  const lateCss = [width()];
  const lateStyle = document.createElement('style');
  lateStyle.textContent = `@font-face {font-family:ReviewLateCss;src:url(data:font/ttf;base64,${narrowData})}`;
  document.head.append(lateStyle);
  await [...document.fonts].find(face => face.family === 'ReviewLateCss').load();
  lateCss.push(width());

  const frame = document.createElement('iframe');
  document.body.append(frame);
  const child = frame.contentDocument;
  child.open();
  child.write('<!doctype html><span id="target" style="display:inline-block;font:40px ReviewOrder">AAAA</span>');
  child.close();
  child.fonts.add(wide); child.fonts.add(narrow);
  span.style.fontFamily = 'ReviewOrder';
  const widths = () => [width(), child.getElementById('target').getBoundingClientRect().width];
  const owners = [widths()];
  document.fonts.delete(narrow); document.fonts.add(narrow); owners.push(widths());
  child.fonts.delete(wide); child.fonts.add(wide); owners.push(widths());
  return {descriptorRanges, text, properties, calculations, capabilityBeforeRange, descriptorWrites, scalars, familyOrder,
    order, cssVsManual, lateCss, owners};
}"""


EXPECTED_FONT_MATCHING = {
    "descriptorRanges": [[1, True, 160, 80, 80] for _ in range(9)],
    "text": {"checks": [True, True], "loads": [[], []], "status": "unloaded", "included": False},
    "properties": [
        [True, ["ReviewWeight"], "unloaded", False],
        [True, ["ReviewStyle"], "unloaded", False],
        [True, ["ReviewWidth"], "unloaded", False],
    ],
    "calculations": [
        [False, "NetworkError"],
        [False, "NetworkError"],
        [True, ["ReviewMathWidth"]],
        [False, "NetworkError"],
    ],
    "capabilityBeforeRange": [True, [], "unloaded"],
    "descriptorWrites": [True, False, True, False],
    "scalars": [True, False],
    "familyOrder": ["ReviewSecondFamily", "ReviewFirstFamily"],
    "order": [80, 160, 80, 160],
    "cssVsManual": [80, 160, 80],
    "lateCss": [160, 160],
    "owners": [[160, 80], [80, 80], [80, 160]],
}


def font_matching_payload() -> dict[str, str]:
    fixtures = REPO_ROOT / "moli-layout/tests/fixtures"
    return {
        label + "Data": base64.b64encode((fixtures / filename).read_bytes()).decode("ascii")
        for label, filename in [("narrow", "moli-ahem-narrow.ttf"), ("wide", "moli-ahem-wide.ttf")]
    }


async def run_font_matching_contract(browser: Any, fixture: str, results: list[dict[str, Any]]) -> None:
    context = await browser.new_context()
    try:
        page = await context.new_page()
        await page.goto(f"{fixture}/plain", wait_until="load", timeout=10_000)
        observed = await page.evaluate(FONT_MATCHING_PROBE, font_matching_payload())
        expected = EXPECTED_FONT_MATCHING
        # Chromium 145.0.7632.6 retains stale selection metadata after a
        # descriptor write, in both check() and layout. A fresh equivalent family
        # must still select correctly. Moli and Chromium 147 use the updated
        # descriptor immediately; keep those assertions strict.
        if not is_moli_browser() and browser.version == "145.0.7632.6":
            expected = {**expected, "descriptorWrites": [True, True, True, False],
                        "descriptorRanges": [[1, True, 160, 160, 80] for _ in range(9)]}
        assert_equal(observed, expected, "FontFaceSet matching and per-document selection order")
        record_contract(
            results,
            "font_face_matching_and_owner_order",
            contract="FontFaceSet matches width/style/weight before text ranges and selects faces in each owner's CSS/manual insertion order.",
            source="CSS Font Loading matching rules; Chromium 147 and pinned Chromium 145.0.7632.6",
            commands=["Runtime.evaluate"],
            observed=observed,
        )
    finally:
        await context.close()
