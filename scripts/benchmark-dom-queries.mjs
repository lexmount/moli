#!/usr/bin/env node
// Reproduce #1617 without changing the browser's layout or query policy.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { parseArgs } from 'node:util';

const usage = `Usage:
  MOLI_LAYOUT=true moli serve --port 9222
  node scripts/benchmark-dom-queries.mjs --cdp http://127.0.0.1:9222 --label moli
  node scripts/benchmark-dom-queries.mjs --chromium --label chromium

Options:
  --rows N          Fixture rows (default: 2000; four elements per row).
  --repeats N       Independent samples per case (default: 3).
  --timeout N       Playwright timeout in milliseconds (default: 120000).
  --output PATH     Save metadata, raw samples, and medians as JSON.
  --executable PATH Chromium executable, with --chromium only.
  --label NAME      Revision or browser label for the results.
  --help            Show this help.

Install playwright-core or set PLAYWRIGHT_CORE_MODULE to its module path.
Each case/sample navigates a new page to a locally served, script-free fixture.
First and repeated reads share that page; separate cases do not share warm state.
Navigation timings end at DOMContentLoaded, not first paint. Results are timings,
not performance assertions or claims of equivalent rendering across engines.`;

const { values } = parseArgs({ options: {
  cdp: { type: 'string' }, chromium: { type: 'boolean' },
  executable: { type: 'string' }, label: { type: 'string' },
  rows: { type: 'string', default: '2000' },
  repeats: { type: 'string', default: '3' },
  timeout: { type: 'string', default: '120000' },
  output: { type: 'string' }, help: { type: 'boolean' },
} });

if (values.help) {
  console.log(usage);
  process.exit(0);
}
if (Boolean(values.cdp) === Boolean(values.chromium))
  throw new Error('Specify exactly one of --cdp or --chromium.\n' + usage);
if (values.executable && !values.chromium)
  throw new Error('--executable requires --chromium');
const rows = Number(values.rows);
const repeats = Number(values.repeats);
const timeout = Number(values.timeout);
for (const [name, value] of Object.entries({ rows, repeats, timeout })) {
  if (!Number.isSafeInteger(value) || value < 1)
    throw new Error(`--${name} must be a positive safe integer`);
}

const require = createRequire(import.meta.url);
const moduleName = process.env.PLAYWRIGHT_CORE_MODULE || 'playwright-core';
const { chromium } = require(moduleName);
const fixture = '<!doctype html><html><head><meta charset="utf-8">' +
  '<title>DOM query benchmark</title><style>' +
  '.row{display:flex;gap:8px;height:28px}.c{width:180px}' +
  'button,input{width:100px}</style></head><body>' +
  Array.from({ length: rows }, (_, i) =>
    `<div class="row"><div class="c">Row ${i}</div>` +
    `<button>Action ${i}</button><input value="Value ${i}"></div>`).join('') +
  '</body></html>';

// This function is serialized into the browser; it must have no outer bindings.
function measureReads({ name, rows }) {
  const elements = Array.from(document.querySelectorAll('.c, button'));
  if (elements.length !== rows * 2) throw new Error('Unexpected fixture size');
  // Separate initial layout from the first full scan of distinct elements.
  if (name === 'rects-warm-layout') elements[0].getBoundingClientRect();
  function read() {
    if (name === 'inner-text') return document.body.innerText.length;
    if (name === 'element-inner-text')
      return elements.slice(0, 160).reduce((sum, el) => sum + el.innerText.length, 0);
    let checksum = name === 'observe' ? document.body.innerText.length : 0;
    const targets = name === 'single-rect-after-screenshot' ? elements.slice(0, 1) : elements;
    for (const element of targets) {
      const rect = element.getBoundingClientRect();
      if (![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) ||
          rect.width <= 0 || rect.height <= 0)
        throw new Error('Expected finite, nonempty fixture geometry; enable real layout');
      checksum += rect.x + rect.y + rect.width + rect.height;
      if (name === 'observe') checksum += element.innerText.length;
    }
    return checksum;
  }
  const started = performance.now();
  const firstChecksum = read();
  const firstEnd = performance.now();
  const repeatChecksum = read();
  const repeatEnd = performance.now();
  if (firstChecksum !== repeatChecksum || firstChecksum <= 0)
    throw new Error('Unchanged reads returned inconsistent or empty results');
  return { firstMs: firstEnd - started, repeatMs: repeatEnd - firstEnd,
    checksum: firstChecksum, elements: elements.length };
}

function summarizeElements(elements) {
  const started = performance.now();
  let checksum = 0;
  for (const element of elements) checksum += element.textContent.length;
  return { workMs: performance.now() - started, checksum, elements: elements.length };
}

const cases = ['rects', 'rects-warm-layout', 'rects-after-screenshot',
  'single-rect-after-screenshot', 'inner-text', 'element-inner-text', 'observe',
  'evaluate', 'evaluate-all'];
const samples = [];
const server = createServer((request, response) => {
  if (request.url !== '/heavy.html') {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8',
    'Cache-Control': 'no-store' });
  response.end(fixture);
});
let browser;
try {
  await new Promise((resolveListen, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolveListen);
  });
  const url = `http://127.0.0.1:${server.address().port}/heavy.html`;
  browser = values.chromium
    ? await chromium.launch({ executablePath: values.executable, headless: true, timeout })
    : await chromium.connectOverCDP(values.cdp, { timeout });
  const context = values.chromium
    ? await browser.newContext({ viewport: { width: 1280, height: 720 } })
    : browser.contexts()[0];
  assert(context, 'The CDP endpoint must expose a browser context');
  const metadata = { label: values.label || (values.chromium ? 'chromium' : 'cdp'),
    browserVersion: browser.version(), playwrightVersion: require(`${moduleName}/package.json`).version,
    platform: process.platform, arch: process.arch, nodeVersion: process.version,
    rows, elementsPerRow: 4, selectedElements: rows * 2, repeats,
    viewport: { width: 1280, height: 720 },
    fixtureSha256: createHash('sha256').update(fixture).digest('hex'),
    timestamp: new Date().toISOString(),
    note: 'First/repeat reads are per fresh page. Navigation ends at DOMContentLoaded. ' +
      'Read cases use in-page performance.now(); evaluate/evaluate-all use client wall time ' +
      'and separately report callback work time. CDP uses the existing layout policy.' };
  for (let sample = 0; sample < repeats; sample++) {
    // Alternate order to reduce systematic bias from browser/process warmup.
    const order = sample % 2 ? [...cases].reverse() : cases;
    for (const name of order) {
      const page = await context.newPage();
      page.setDefaultTimeout(timeout);
      page.setDefaultNavigationTimeout(timeout);
      try {
        await page.setViewportSize({ width: 1280, height: 720 });
        const navigationStart = performance.now();
        await page.goto(url, { waitUntil: 'domcontentloaded' });
        const navigationMs = performance.now() - navigationStart;
        // Refresh layout through an observable paint, outside the query timer.
        if (name.endsWith('-after-screenshot')) await page.screenshot();
        let result;
        if (name === 'evaluate' || name === 'evaluate-all') {
          result = {};
          let checksum;
          for (const phase of ['first', 'repeat']) {
            const start = performance.now();
            const value = name === 'evaluate-all'
              ? await page.locator('.c, button').evaluateAll(summarizeElements)
              : await page.evaluate(`(${summarizeElements})(Array.from(document.querySelectorAll('.c, button')))`);
            result[`${phase}Ms`] = performance.now() - start;
            result[`${phase}WorkMs`] = value.workMs;
            assert.equal(value.elements, rows * 2);
            assert(value.checksum > 0);
            if (checksum !== undefined) assert.equal(value.checksum, checksum);
            checksum = value.checksum;
          }
          result.checksum = checksum;
          result.elements = rows * 2;
        } else {
          result = await page.evaluate(measureReads, { name, rows });
        }
        const record = { case: name, sample, navigationMs, ...result };
        samples.push(record);
        console.log(JSON.stringify(record));
      } finally {
        await page.close();
      }
    }
  }
  function median(values) {
    const sorted = [...values].sort((a, b) => a - b);
    const middle = Math.floor(sorted.length / 2);
    return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
  }
  const medians = Object.fromEntries(cases.map(name => {
    const records = samples.filter(record => record.case === name);
    return [name, Object.fromEntries(Object.keys(records[0])
      .filter(key => key.endsWith('Ms'))
      .map(key => [key, median(records.map(record => record[key]))]))];
  }));
  const report = { metadata, samples, medians };
  if (values.output) {
    const path = resolve(values.output);
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, JSON.stringify(report, null, 2) + '\n');
  }
  console.log(JSON.stringify({ metadata, medians }, null, 2));
} finally {
  // For CDP attachments, close disconnects this client; only our pages were closed.
  try {
    if (browser) await browser.close();
  } finally {
    if (server.listening) await new Promise(resolveClose => server.close(resolveClose));
  }
}
