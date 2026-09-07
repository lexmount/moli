'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  COMMENT_MARKER,
  renderReport,
} = require('./render-ci-regression-comment.cjs');

const MAX_RESULTS_FOR_TEST = 2_001;

function writeJson(root, relativePath, value) {
  const output = path.join(root, relativePath);
  fs.mkdirSync(path.dirname(output), { recursive: true });
  fs.writeFileSync(output, `${JSON.stringify(value)}\n`, 'utf8');
}

function stats(p50, p95 = p50) {
  return { count: 1, min: p50, max: p95, median: p50, p50, p90: p95, p95 };
}

function startupSummary(offset) {
  return {
    total_failures: 0,
    gate_failures: 0,
    cases: {
      'binary-size': {
        binary_bytes: 100 + offset,
        stripped_binary_bytes: 80 + offset,
        tar_gz_bytes: 50 + offset,
      },
      'image-size': {
        image_uncompressed_bytes: 200 + offset,
        image_compressed_bytes: 90 + offset,
      },
      'serve-ready': { elapsed_ms: stats(20 + offset), pss_bytes: stats(1_000 + offset) },
      'cdp-first-page': { elapsed_ms: stats(30 + offset), pss_bytes: stats(2_000 + offset) },
      'cdp-warm-pages': {
        cdp_page_elapsed_p50_ms: 10 + offset,
        elapsed_ms: stats(100 + offset),
        pss_bytes: stats(3_000 + offset),
      },
      'cli-fetch-aboutblank': { elapsed_ms: stats(15 + offset), pss_bytes: stats(1_500 + offset) },
      'cli-fetch-local-js': { elapsed_ms: stats(18 + offset), pss_bytes: stats(1_800 + offset) },
    },
  };
}

function matrixSummary(offset) {
  return {
    total_failures: 0,
    gate_failures: 0,
    stability_failures: 0,
    concurrency_levels: [1],
    cases: {
      'static-html': {
        '1': { elapsed_p50_ms: stats(20 + offset), failures: 0, stable: true },
      },
    },
  };
}

function matrixRows(offset) {
  return [{
    concurrency: 1,
    case: 'static-html',
    elapsed_p50_ms: 20 + offset,
    peak_pss_p50_bytes: 4_000 + offset,
    failures: 0,
  }];
}

function agentTarget({ chrome = false, failure = false }) {
  return {
    episodes: 2,
    passed: failure ? 1 : 2,
    failures: failure ? 1 : 0,
    assertions_total: 4,
    assertions_passed: failure ? 3 : 4,
    ready_ms: stats(chrome ? 200 : 20),
    elapsed_ms: stats(chrome ? 80 : 30),
    operations: {
      navigate: stats(chrome ? 40 : 10),
      observe: stats(chrome ? 4 : 2),
      fill: stats(chrome ? 6 : 3),
      click: stats(chrome ? 8 : 4),
    },
    resources: {
      peak_rss_bytes: chrome ? 500 * 1024 : 100 * 1024,
      peak_pss_bytes: chrome ? 400 * 1024 : 80 * 1024,
      average_cpu_percent: chrome ? 4 : 2,
      peak_cpu_percent: chrome ? 20 : 10,
      peak_process_count: chrome ? 10 : 1,
      sampler_health: { healthy: true, pss_complete: !chrome },
    },
    cases: {
      'episode-safe': { failures: 0 },
      ...(failure ? { 'episode|unsafe<': { failures: 1 } } : {}),
    },
  };
}

function createArtifacts(root) {
  const releaseRoot = path.join(root, 'release');
  writeJson(releaseRoot, 'base-startup/startup/summary.json', startupSummary(0));
  writeJson(releaseRoot, 'head-startup/startup/summary.json', startupSummary(10));
  writeJson(releaseRoot, 'base-matrix/synthetic-matrix/summary.json', matrixSummary(0));
  writeJson(releaseRoot, 'head-matrix/synthetic-matrix/summary.json', matrixSummary(2));
  writeJson(releaseRoot, 'base-matrix/synthetic-matrix/matrix.json', matrixRows(0));
  writeJson(releaseRoot, 'head-matrix/synthetic-matrix/matrix.json', matrixRows(2));

  const frontendRoot = path.join(root, 'frontend');
  writeJson(frontendRoot, 'summary.json', {
    ok: false,
    referenceGate: { recoveredCases: 1 },
    counts: { match: 1, dom_mismatch: 1 },
    durationMs: 1_250,
    timeline: { chromiumFrames: 5, moliFrames: 5, mismatchedFrames: 1 },
    results: [
      {
        id: 'safe',
        status: 'match',
        mismatchedFrames: [],
        chromium: {
          previous_failures: [
            { error: 'Target.createBrowserContext failed: unknown' },
          ],
        },
      },
      {
        id: 'bad|case<',
        status: 'dom_mismatch',
        firstDifference: '$.body|<',
        mismatchedFrames: ['ready|<'],
      },
    ],
  });

  const agentRoot = path.join(root, 'agent');
  const fastAgent = {
    total_failures: 1,
    gate_failures: 1,
    targets: {
      'moli-cdp': agentTarget({ failure: true }),
      'chrome-cdp': agentTarget({ chrome: true }),
    },
  };
  const resourceAgent = {
    total_failures: 0,
    gate_failures: 0,
    targets: {
      'moli-cdp': agentTarget({}),
      'chrome-cdp': agentTarget({ chrome: true }),
    },
  };
  writeJson(agentRoot, 'agent-fast/agent-episode/summary.json', fastAgent);
  writeJson(agentRoot, 'agent-resource/agent-episode/summary.json', resourceAgent);

  const runtimeRoot = path.join(root, 'runtime');
  writeJson(runtimeRoot, 'runtime-synthetic/synthetic/summary.json', {
    total_failures: 0,
    cases: {
      static: { failures: 0, elapsed_ms: stats(20, 25), peak_pss_bytes: stats(2_048) },
      dynamic: { failures: 0, elapsed_ms: stats(30, 35), peak_pss_bytes: stats(4_096) },
    },
  });
  writeJson(runtimeRoot, 'cdp-session/cdp-session/summary.json', {
    total_failures: 0,
    gate_failures: 0,
    targets: {
      'moli-cdp': {
        cases: {
          session: { failures: 0, elapsed_ms: stats(15, 18) },
        },
      },
    },
  });

  const cdpRoot = path.join(root, 'cdp');
  writeJson(cdpRoot, 'summary.json', {
    ok: false,
    jobs: 4,
    groups: [
      { group: 'core', status: 'passed', scenarioCount: 10, durationSeconds: 2 },
      { group: 'bad|group<', status: 'failed', scenarioCount: 3, durationSeconds: 4, error: 'boom|<tag>' },
    ],
  });

  return { releaseRoot, frontendRoot, agentRoot, runtimeRoot, cdpRoot };
}

test('renders all five trusted artifact sections into one bounded report', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'moli-ci-report-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const roots = createArtifacts(root);
  const report = renderReport({
    ...roots,
    runUrl: 'https://github.com/lexmount/moli/actions/runs/123',
    conclusion: 'failure',
  });

  assert.ok(report.startsWith(COMMENT_MARKER));
  assert.match(report, /artifacts: `5\/5`/);
  assert.match(report, /Raw binary \| 100 B \| 110 B \| \+10 B \| \+10\.000000%/);
  assert.match(report, /Frontend differential/);
  assert.match(report, /1 Chromium reference recoveries/);
  assert.match(report, /Recovered Chromium reference infrastructure failures/);
  assert.match(report, /Target\.createBrowserContext failed: unknown/);
  assert.match(report, /`bad\\\|case&lt;`/);
  assert.match(report, /first difference `\$\.body\\\|&lt;`/);
  assert.match(report, /frames `ready\\\|&lt;`/);
  assert.match(report, /First failing episodes/);
  assert.match(report, /`episode\\\|unsafe&lt;`/);
  assert.match(report, /PSS partial/);
  assert.match(report, /Runtime and CDP session contracts/);
  assert.match(report, /`bad\\\|group&lt;`/);
  assert.ok(Buffer.byteLength(report, 'utf8') < 32 * 1024);
});

test('renders missing artifacts as unavailable without throwing', () => {
  const report = renderReport({
    runUrl: 'not-a-trusted-url',
    conclusion: 'timed_out',
  });

  assert.match(report, /artifacts: `0\/5`/);
  assert.equal((report.match(/Artifact unavailable or invalid\./g) || []).length, 5);
  assert.doesNotMatch(report, /\]\(not-a-trusted-url\)/);
});

test('rejects an unbounded frontend result list as unavailable', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'moli-ci-report-limit-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const frontendRoot = path.join(root, 'frontend');
  writeJson(frontendRoot, 'summary.json', {
    ok: false,
    counts: { infrastructure_error: MAX_RESULTS_FOR_TEST },
    results: Array.from({ length: MAX_RESULTS_FOR_TEST }, (_, index) => ({
      id: `case-${index}`,
      status: 'infrastructure_error',
    })),
  });

  const report = renderReport({ frontendRoot, conclusion: 'failure' });
  assert.match(report, /artifacts: `0\/5`/);
});
