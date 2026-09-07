'use strict';

const fs = require('node:fs');
const path = require('node:path');

const COMMENT_MARKER = '<!-- moli-ci-regression-report -->';
const MAX_JSON_BYTES = 8 * 1024 * 1024;
const MAX_COMMENT_BYTES = 32 * 1024;
const MAX_RESULT_ROWS = 2_000;
const MAX_DETAIL_ROWS = 10;
const MAX_MATRIX_ROWS = 5_000;
const MAX_CDP_GROUPS = 100;
const MIB = 1024 * 1024;

function isObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function finiteNumber(value) {
  return typeof value === 'number' && Number.isFinite(value) && Math.abs(value) <= Number.MAX_SAFE_INTEGER
    ? value
    : null;
}

function nonNegativeNumber(value) {
  const number = finiteNumber(value);
  return number !== null && number >= 0 ? number : null;
}

function count(value) {
  const number = nonNegativeNumber(value);
  return number !== null && Number.isInteger(number) ? number : null;
}

function objectAt(value, key) {
  return isObject(value) && isObject(value[key]) ? value[key] : {};
}

function safeText(value, maximumLength = 120) {
  if (typeof value !== 'string') {
    return 'unknown';
  }
  return value
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .replace(/\\/g, '\\\\')
    .replace(/\|/g, '\\|')
    .replace(/`/g, "'")
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .slice(0, maximumLength) || 'unknown';
}

function code(value, maximumLength = 120) {
  return `\`${safeText(value, maximumLength)}\``;
}

function readJson(root, relativePath) {
  if (typeof root !== 'string' || root.length === 0) {
    throw new Error('artifact root is unavailable');
  }
  const filePath = path.join(root, relativePath);
  const metadata = fs.lstatSync(filePath);
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    throw new Error('artifact JSON is not a regular file');
  }
  if (metadata.size > MAX_JSON_BYTES) {
    throw new Error('artifact JSON exceeds the trusted renderer limit');
  }
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

function loadSection(loader) {
  try {
    return { available: true, data: loader() };
  } catch {
    return { available: false, data: null };
  }
}

function median(values) {
  const numbers = values.filter((value) => finiteNumber(value) !== null).sort((left, right) => left - right);
  if (numbers.length === 0) {
    return null;
  }
  const middle = Math.floor(numbers.length / 2);
  return numbers.length % 2 === 0
    ? (numbers[middle - 1] + numbers[middle]) / 2
    : numbers[middle];
}

function sum(values) {
  const numbers = values.map(finiteNumber).filter((value) => value !== null);
  return numbers.length === 0 ? null : numbers.reduce((total, value) => total + value, 0);
}

function formatInteger(value) {
  const number = count(value);
  return number === null ? '—' : number.toLocaleString('en-US');
}

function formatDecimal(value, digits = 2) {
  const number = finiteNumber(value);
  return number === null ? '—' : number.toFixed(digits);
}

function formatMilliseconds(value) {
  const number = finiteNumber(value);
  return number === null ? '—' : `${number.toFixed(2)} ms`;
}

function formatDurationMilliseconds(value) {
  const number = nonNegativeNumber(value);
  if (number === null) {
    return '—';
  }
  if (number < 1_000) {
    return formatMilliseconds(number);
  }
  const totalSeconds = number / 1_000;
  if (totalSeconds < 60) {
    return `${totalSeconds.toFixed(2)} s`;
  }
  const minutes = Math.floor(totalSeconds / 60);
  return `${minutes}m ${(totalSeconds - minutes * 60).toFixed(2)}s`;
}

function formatSeconds(value) {
  const number = finiteNumber(value);
  return number === null ? '—' : `${number.toFixed(2)} s`;
}

function formatPercentValue(value) {
  const number = finiteNumber(value);
  return number === null ? '—' : `${number.toFixed(2)}%`;
}

function formatBytes(value) {
  const number = finiteNumber(value);
  if (number === null) {
    return '—';
  }
  const absolute = Math.abs(number);
  if (absolute >= MIB) {
    return `${(absolute / MIB).toFixed(2)} MiB`;
  }
  if (absolute >= 1024) {
    return `${(absolute / 1024).toFixed(2)} KiB`;
  }
  return `${Math.round(absolute)} B`;
}

function signed(value, formatter) {
  const number = finiteNumber(value);
  if (number === null) {
    return '—';
  }
  if (number === 0) {
    return formatter(0);
  }
  return `${number > 0 ? '+' : '−'}${formatter(Math.abs(number))}`;
}

function percentDelta(base, head, digits = 2) {
  const baseline = finiteNumber(base);
  const current = finiteNumber(head);
  if (baseline === null || current === null || baseline === 0) {
    return '—';
  }
  const value = ((current - baseline) / Math.abs(baseline)) * 100;
  if (value === 0) {
    return '0.00%';
  }
  return `${value > 0 ? '+' : '−'}${Math.abs(value).toFixed(digits)}%`;
}

function comparisonCells(base, head, formatter, percentDigits = 2) {
  const baseline = finiteNumber(base);
  const current = finiteNumber(head);
  return [
    formatter(baseline),
    formatter(current),
    baseline === null || current === null ? '—' : signed(current - baseline, formatter),
    percentDelta(baseline, current, percentDigits),
  ];
}

function metric(summary, caseName, ...pathParts) {
  let value = objectAt(objectAt(summary, 'cases'), caseName);
  for (const part of pathParts) {
    value = isObject(value) ? value[part] : null;
  }
  return finiteNumber(value);
}

function failures(summary) {
  return count(summary?.gate_failures) ?? count(summary?.total_failures) ?? null;
}

function statusIcon(failureCount) {
  if (failureCount === null) {
    return '⚪';
  }
  return failureCount === 0 ? '✅' : '❌';
}

function loadRelease(root) {
  const baseStartup = readJson(root, 'base-startup/startup/summary.json');
  const headStartup = readJson(root, 'head-startup/startup/summary.json');
  const baseMatrix = readJson(root, 'base-matrix/synthetic-matrix/summary.json');
  const headMatrix = readJson(root, 'head-matrix/synthetic-matrix/summary.json');
  const baseMatrixRows = readJson(root, 'base-matrix/synthetic-matrix/matrix.json');
  const headMatrixRows = readJson(root, 'head-matrix/synthetic-matrix/matrix.json');
  if (
    !isObject(baseStartup) ||
    !isObject(headStartup) ||
    !isObject(baseMatrix) ||
    !isObject(headMatrix) ||
    !Array.isArray(baseMatrixRows) ||
    !Array.isArray(headMatrixRows) ||
    baseMatrixRows.length > MAX_MATRIX_ROWS ||
    headMatrixRows.length > MAX_MATRIX_ROWS
  ) {
    throw new Error('invalid release regression artifact');
  }
  return { baseStartup, headStartup, baseMatrix, headMatrix, baseMatrixRows, headMatrixRows };
}

function matrixAggregate(summary, rows, concurrency) {
  const selected = rows.filter(
    (row) => isObject(row) && finiteNumber(row.concurrency) === concurrency
  );
  const cases = objectAt(summary, 'cases');
  let unstable = 0;
  for (const caseValue of Object.values(cases)) {
    const cell = isObject(caseValue) ? caseValue[String(concurrency)] : null;
    if (isObject(cell) && cell.stable === false) {
      unstable += 1;
    }
  }
  return {
    latency: median(selected.map((row) => row.elapsed_p50_ms)),
    pss: median(selected.map((row) => row.peak_pss_p50_bytes)),
    failures: sum(selected.map((row) => row.failures)) ?? 0,
    unstable,
  };
}

function releaseOverview(section) {
  if (!section.available) {
    return { status: '⚪', signal: 'artifact unavailable or invalid' };
  }
  const { baseStartup, headStartup, baseMatrix, headMatrix } = section.data;
  const baseFailures = (failures(baseStartup) ?? 0) + (failures(baseMatrix) ?? 0);
  const headFailures = (failures(headStartup) ?? 0) + (failures(headMatrix) ?? 0);
  const rawBase = metric(section.data.baseStartup, 'binary-size', 'binary_bytes');
  const rawHead = metric(headStartup, 'binary-size', 'binary_bytes');
  return {
    status: headFailures > 0 ? '❌' : baseFailures > 0 ? '⚠️' : '✅',
    signal: `HEAD/base failures ${formatInteger(headFailures)}/${formatInteger(baseFailures)}; raw binary ${percentDelta(rawBase, rawHead, 6)}`,
  };
}

function renderRelease(section) {
  const overview = releaseOverview(section);
  const lines = [`<details open><summary><strong>Release regression</strong> — ${overview.status} ${overview.signal}</summary>`, ''];
  if (!section.available) {
    lines.push('Artifact unavailable or invalid. See the source CI run for infrastructure details.', '', '</details>');
    return lines;
  }

  const { baseStartup, headStartup, baseMatrix, headMatrix, baseMatrixRows, headMatrixRows } = section.data;
  const sizeRows = [
    ['Raw binary', metric(baseStartup, 'binary-size', 'binary_bytes'), metric(headStartup, 'binary-size', 'binary_bytes')],
    ['Stripped binary', metric(baseStartup, 'binary-size', 'stripped_binary_bytes'), metric(headStartup, 'binary-size', 'stripped_binary_bytes')],
    ['gzip binary', metric(baseStartup, 'binary-size', 'tar_gz_bytes'), metric(headStartup, 'binary-size', 'tar_gz_bytes')],
    ['Rootfs', metric(baseStartup, 'image-size', 'image_uncompressed_bytes'), metric(headStartup, 'image-size', 'image_uncompressed_bytes')],
    ['gzip rootfs', metric(baseStartup, 'image-size', 'image_compressed_bytes'), metric(headStartup, 'image-size', 'image_compressed_bytes')],
  ];
  lines.push(
    '#### Package and image size',
    '',
    '| Metric | Base | HEAD | Delta | Delta % |',
    '| --- | ---: | ---: | ---: | ---: |'
  );
  for (const [label, base, head] of sizeRows) {
    lines.push(`| ${label} | ${comparisonCells(base, head, formatBytes, 6).join(' | ')} |`);
  }

  const startupRows = [
    ['Serve ready', 'serve-ready', ['elapsed_ms', 'p50']],
    ['CDP first page', 'cdp-first-page', ['elapsed_ms', 'p50']],
    ['CDP warm page', 'cdp-warm-pages', ['cdp_page_elapsed_p50_ms']],
    ['CLI about:blank', 'cli-fetch-aboutblank', ['elapsed_ms', 'p50']],
    ['CLI local JS', 'cli-fetch-local-js', ['elapsed_ms', 'p50']],
  ];
  lines.push(
    '',
    '#### Startup latency and PSS',
    '',
    '| Case | Base p50 | HEAD p50 | Delta | Base PSS | HEAD PSS | Delta |',
    '| --- | ---: | ---: | ---: | ---: | ---: | ---: |'
  );
  for (const [label, caseName, latencyPath] of startupRows) {
    const baseLatency = metric(baseStartup, caseName, ...latencyPath);
    const headLatency = metric(headStartup, caseName, ...latencyPath);
    const basePss = metric(baseStartup, caseName, 'pss_bytes', 'p50');
    const headPss = metric(headStartup, caseName, 'pss_bytes', 'p50');
    const latencyCells = comparisonCells(baseLatency, headLatency, formatMilliseconds).slice(0, 3);
    const pssCells = comparisonCells(basePss, headPss, formatBytes).slice(0, 3);
    lines.push(`| ${label} | ${[...latencyCells, ...pssCells].join(' | ')} |`);
  }

  const levels = [...new Set([
    ...(Array.isArray(baseMatrix.concurrency_levels) ? baseMatrix.concurrency_levels : []),
    ...(Array.isArray(headMatrix.concurrency_levels) ? headMatrix.concurrency_levels : []),
  ])]
    .map(finiteNumber)
    .filter((value) => value !== null && Number.isInteger(value) && value > 0)
    .sort((left, right) => left - right)
    .slice(0, 10);
  lines.push(
    '',
    '#### Concurrency matrix',
    '',
    '| Concurrency | Base p50 | HEAD p50 | Delta | Base PSS | HEAD PSS | Delta | HEAD failures | Unstable B/H |',
    '| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |'
  );
  for (const level of levels) {
    const base = matrixAggregate(baseMatrix, baseMatrixRows, level);
    const head = matrixAggregate(headMatrix, headMatrixRows, level);
    const latency = comparisonCells(base.latency, head.latency, formatMilliseconds).slice(0, 3);
    const pss = comparisonCells(base.pss, head.pss, formatBytes).slice(0, 3);
    lines.push(
      `| ${level} | ${[...latency, ...pss, formatInteger(head.failures), `${base.unstable}/${head.unstable}`].join(' | ')} |`
    );
  }
  lines.push('', '</details>');
  return lines;
}

function loadFrontend(root) {
  const summary = readJson(root, 'summary.json');
  if (!isObject(summary) || !isObject(summary.counts) || !Array.isArray(summary.results) || summary.results.length > MAX_RESULT_ROWS) {
    throw new Error('invalid frontend differential artifact');
  }
  return summary;
}

function frontendOverview(section) {
  if (!section.available) {
    return { status: '⚪', signal: 'artifact unavailable or invalid' };
  }
  const summary = section.data;
  const total = summary.results.length;
  const matches = (count(summary.counts.match) ?? 0) + (count(summary.counts.reference_ok) ?? 0);
  const problems = Math.max(0, total - matches);
  const recoveries = count(objectAt(summary, 'referenceGate').recoveredCases) ?? 0;
  return {
    status: summary.ok === true && problems === 0 ? '✅' : '❌',
    signal: `${formatInteger(matches)}/${formatInteger(total)} cases matched; ${formatInteger(problems)} issues${recoveries ? `; ${formatInteger(recoveries)} Chromium reference recoveries` : ''}`,
  };
}

function renderFrontend(section) {
  const overview = frontendOverview(section);
  const lines = [`<details><summary><strong>Frontend differential</strong> — ${overview.status} ${overview.signal}</summary>`, ''];
  if (!section.available) {
    lines.push('Artifact unavailable or invalid. See the source CI run for infrastructure details.', '', '</details>');
    return lines;
  }
  const summary = section.data;
  const timeline = objectAt(summary, 'timeline');
  lines.push(
    '| Match | DOM mismatch | Diagnostic mismatch | Moli error | Reference error | Infrastructure error | Mismatched frames | Duration |',
    '| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |',
    `| ${formatInteger(count(summary.counts.match) ?? 0)} | ${formatInteger(count(summary.counts.dom_mismatch) ?? 0)} | ${formatInteger(count(summary.counts.diagnostic_mismatch) ?? 0)} | ${formatInteger(count(summary.counts.moli_error) ?? 0)} | ${formatInteger(count(summary.counts.reference_error) ?? 0)} | ${formatInteger(count(summary.counts.infrastructure_error) ?? 0)} | ${formatInteger(timeline.mismatchedFrames)} | ${formatDurationMilliseconds(summary.durationMs)} |`
  );
  const recoveredReferences = summary.results
    .filter((result) => {
      const chromium = objectAt(result, 'chromium');
      return Array.isArray(chromium.previous_failures) && chromium.previous_failures.length !== 0;
    })
    .slice(0, MAX_DETAIL_ROWS);
  if (recoveredReferences.length !== 0) {
    lines.push('', '**Recovered Chromium reference infrastructure failures**', '');
    for (const result of recoveredReferences) {
      const chromium = objectAt(result, 'chromium');
      const failures = chromium.previous_failures;
      const first = isObject(failures[0]) ? failures[0] : {};
      const error = typeof first.error === 'string' ? first.error : 'unknown';
      lines.push(
        `- ${code(result.id, 160)} — ${formatInteger(failures.length)} failed attempt(s) before recovery · ${code(error, 160)}`
      );
    }
  }
  const issues = summary.results
    .filter((result) => isObject(result) && !['match', 'reference_ok'].includes(result.status))
    .slice(0, MAX_DETAIL_ROWS);
  if (issues.length !== 0) {
    lines.push('', '**First failing cases**', '');
    for (const issue of issues) {
      const details = [];
      if (typeof issue.firstDifference === 'string' && issue.firstDifference.length !== 0) {
        details.push(`first difference ${code(issue.firstDifference, 160)}`);
      }
      if (Array.isArray(issue.mismatchedFrames) && issue.mismatchedFrames.length !== 0) {
        const frames = issue.mismatchedFrames.slice(0, 3).map((frame) => code(frame, 80));
        const remainder = issue.mismatchedFrames.length - frames.length;
        details.push(`frames ${frames.join(', ')}${remainder > 0 ? ` +${remainder} more` : ''}`);
      }
      const moli = objectAt(issue, 'moli');
      const chromium = objectAt(issue, 'chromium');
      const error = typeof moli.error === 'string'
        ? `Moli error ${code(moli.error, 160)}`
        : typeof chromium.error === 'string'
          ? `Chromium error ${code(chromium.error, 160)}`
          : null;
      if (error !== null) {
        details.push(error);
      }
      lines.push(`- ${code(issue.id)} — ${code(issue.status)}${details.length ? ` · ${details.join(' · ')}` : ''}`);
    }
  }
  lines.push('', '</details>');
  return lines;
}

function loadAgent(root) {
  const fast = readJson(root, 'agent-fast/agent-episode/summary.json');
  const resource = readJson(root, 'agent-resource/agent-episode/summary.json');
  if (!isObject(fast) || !isObject(resource) || !isObject(fast.targets) || !isObject(resource.targets)) {
    throw new Error('invalid agent episode artifact');
  }
  return { fast, resource };
}

function target(summary, name) {
  return objectAt(objectAt(summary, 'targets'), name);
}

function agentOverview(section) {
  if (!section.available) {
    return { status: '⚪', signal: 'artifact unavailable or invalid' };
  }
  const totalFailures = (failures(section.data.fast) ?? 0) + (failures(section.data.resource) ?? 0);
  const moli = target(section.data.fast, 'moli-cdp');
  return {
    status: statusIcon(totalFailures),
    signal: `${formatInteger(moli.passed)}/${formatInteger(moli.episodes)} Moli episodes passed; ${formatInteger(totalFailures)} failures`,
  };
}

function renderAgent(section) {
  const overview = agentOverview(section);
  const lines = [`<details><summary><strong>Agent episodes · Moli vs Chromium</strong> — ${overview.status} ${overview.signal}</summary>`, ''];
  if (!section.available) {
    lines.push('Artifact unavailable or invalid. See the source CI run for infrastructure details.', '', '</details>');
    return lines;
  }
  const { fast, resource } = section.data;
  lines.push(
    '| Fast contract | Episodes | Passed | Assertions | Ready p50 | Episode p50 |',
    '| --- | ---: | ---: | ---: | ---: | ---: |'
  );
  for (const name of ['moli-cdp', 'chrome-cdp']) {
    const item = target(fast, name);
    lines.push(
      `| ${name === 'moli-cdp' ? 'Moli' : 'Chromium'} | ${formatInteger(item.episodes)} | ${formatInteger(item.passed)} | ${formatInteger(item.assertions_passed)}/${formatInteger(item.assertions_total)} | ${formatMilliseconds(objectAt(item, 'ready_ms').p50)} | ${formatMilliseconds(objectAt(item, 'elapsed_ms').p50)} |`
    );
  }
  lines.push(
    '',
    '| Operation p50 | Chromium | Moli | Delta | Delta % |',
    '| --- | ---: | ---: | ---: | ---: |'
  );
  for (const operation of ['navigate', 'observe', 'fill', 'click']) {
    const chromeValue = finiteNumber(objectAt(objectAt(target(fast, 'chrome-cdp'), 'operations'), operation).p50);
    const moliValue = finiteNumber(objectAt(objectAt(target(fast, 'moli-cdp'), 'operations'), operation).p50);
    lines.push(`| ${operation} | ${comparisonCells(chromeValue, moliValue, formatMilliseconds).join(' | ')} |`);
  }
  lines.push(
    '',
    '| Idle-resource episode | Peak RSS | Peak PSS | Average CPU | Peak CPU | Peak processes | Sampler |',
    '| --- | ---: | ---: | ---: | ---: | ---: | --- |'
  );
  for (const name of ['moli-cdp', 'chrome-cdp']) {
    const resources = objectAt(target(resource, name), 'resources');
    const sampler = objectAt(resources, 'sampler_health');
    const samplerStatus = sampler.healthy !== true
      ? '❌'
      : sampler.pss_complete === false
        ? '⚠️ PSS partial'
        : '✅';
    lines.push(
      `| ${name === 'moli-cdp' ? 'Moli' : 'Chromium'} | ${formatBytes(resources.peak_rss_bytes)} | ${formatBytes(resources.peak_pss_bytes)} | ${formatPercentValue(resources.average_cpu_percent)} | ${formatPercentValue(resources.peak_cpu_percent)} | ${formatDecimal(resources.peak_process_count, 0)} | ${samplerStatus} |`
    );
  }
  const failingCases = [];
  for (const [profile, summary] of [['fast', fast], ['resource', resource]]) {
    for (const name of ['moli-cdp', 'chrome-cdp']) {
      for (const [caseName, caseValue] of Object.entries(objectAt(target(summary, name), 'cases'))) {
        if ((count(isObject(caseValue) ? caseValue.failures : null) ?? 0) > 0) {
          failingCases.push({ profile, target: name, caseName });
        }
      }
    }
  }
  if (failingCases.length !== 0) {
    lines.push('', '**First failing episodes**', '');
    for (const failure of failingCases.slice(0, MAX_DETAIL_ROWS)) {
      lines.push(`- ${code(failure.caseName)} — ${failure.profile} / ${failure.target}`);
    }
  }
  lines.push('', '</details>');
  return lines;
}

function loadRuntime(root) {
  const synthetic = readJson(root, 'runtime-synthetic/synthetic/summary.json');
  const cdpSession = readJson(root, 'cdp-session/cdp-session/summary.json');
  if (!isObject(synthetic) || !isObject(cdpSession)) {
    throw new Error('invalid runtime contract artifact');
  }
  return { synthetic, cdpSession };
}

function summarizeCases(cases, includePss) {
  const entries = Object.entries(isObject(cases) ? cases : {}).slice(0, MAX_RESULT_ROWS);
  return {
    cases: entries.length,
    failures: sum(entries.map(([, value]) => (isObject(value) ? value.failures : null))) ?? 0,
    latencyP50: median(entries.map(([, value]) => objectAt(value, 'elapsed_ms').p50)),
    latencyP95: median(entries.map(([, value]) => objectAt(value, 'elapsed_ms').p95)),
    pssP50: includePss
      ? median(entries.map(([, value]) => objectAt(value, 'peak_pss_bytes').p50))
      : null,
    failing: entries
      .filter(([, value]) => (count(isObject(value) ? value.failures : null) ?? 0) > 0)
      .map(([name]) => name)
      .slice(0, MAX_DETAIL_ROWS),
  };
}

function runtimeOverview(section) {
  if (!section.available) {
    return { status: '⚪', signal: 'artifact unavailable or invalid' };
  }
  const synthetic = summarizeCases(section.data.synthetic.cases, true);
  const cdp = summarizeCases(target(section.data.cdpSession, 'moli-cdp').cases, false);
  const caseFailures = synthetic.failures + cdp.failures;
  const reportedFailures = (failures(section.data.synthetic) ?? 0) + (failures(section.data.cdpSession) ?? 0);
  const totalFailures = Math.max(caseFailures, reportedFailures);
  return {
    status: statusIcon(totalFailures),
    signal: `${formatInteger(synthetic.cases + cdp.cases)} contract cases; ${formatInteger(totalFailures)} failures`,
  };
}

function renderRuntime(section) {
  const overview = runtimeOverview(section);
  const lines = [`<details><summary><strong>Runtime and CDP session contracts</strong> — ${overview.status} ${overview.signal}</summary>`, ''];
  if (!section.available) {
    lines.push('Artifact unavailable or invalid. See the source CI run for infrastructure details.', '', '</details>');
    return lines;
  }
  const synthetic = summarizeCases(section.data.synthetic.cases, true);
  const cdp = summarizeCases(target(section.data.cdpSession, 'moli-cdp').cases, false);
  lines.push(
    '| Suite | Cases | Failures | Median case p50 | Median case p95 | Median peak PSS p50 |',
    '| --- | ---: | ---: | ---: | ---: | ---: |',
    `| Synthetic fetch | ${formatInteger(synthetic.cases)} | ${formatInteger(synthetic.failures)} | ${formatMilliseconds(synthetic.latencyP50)} | ${formatMilliseconds(synthetic.latencyP95)} | ${formatBytes(synthetic.pssP50)} |`,
    `| Long-lived CDP session | ${formatInteger(cdp.cases)} | ${formatInteger(cdp.failures)} | ${formatMilliseconds(cdp.latencyP50)} | ${formatMilliseconds(cdp.latencyP95)} | — |`
  );
  const failing = [...synthetic.failing, ...cdp.failing].slice(0, MAX_DETAIL_ROWS);
  if (failing.length !== 0) {
    lines.push('', `**Failing cases:** ${failing.map((name) => code(name)).join(', ')}`);
  }
  lines.push('', '</details>');
  return lines;
}

function loadCdp(root) {
  const summary = readJson(root, 'summary.json');
  if (!isObject(summary) || !Array.isArray(summary.groups) || summary.groups.length > MAX_CDP_GROUPS) {
    throw new Error('invalid CDP smoke artifact');
  }
  return summary;
}

function cdpStats(summary) {
  const groups = summary.groups.filter(isObject);
  const passed = groups.filter((group) => group.status === 'passed').length;
  const scenarios = sum(groups.map((group) => group.scenarioCount)) ?? 0;
  const cumulativeSeconds = sum(groups.map((group) => group.durationSeconds));
  const failures = groups.filter((group) => group.status !== 'passed');
  const slowest = [...groups]
    .filter((group) => nonNegativeNumber(group.durationSeconds) !== null)
    .sort((left, right) => right.durationSeconds - left.durationSeconds)
    .slice(0, 5);
  return { groups, passed, scenarios, cumulativeSeconds, failures, slowest };
}

function cdpOverview(section) {
  if (!section.available) {
    return { status: '⚪', signal: 'artifact unavailable or invalid' };
  }
  const stats = cdpStats(section.data);
  return {
    status: section.data.ok === true && stats.failures.length === 0 ? '✅' : '❌',
    signal: `${formatInteger(stats.passed)}/${formatInteger(stats.groups.length)} groups passed; ${formatInteger(stats.scenarios)} scenarios`,
  };
}

function renderCdp(section) {
  const overview = cdpOverview(section);
  const lines = [`<details><summary><strong>CDP smoke</strong> — ${overview.status} ${overview.signal}</summary>`, ''];
  if (!section.available) {
    lines.push('Artifact unavailable or invalid. See the source CI run for infrastructure details.', '', '</details>');
    return lines;
  }
  const stats = cdpStats(section.data);
  lines.push(
    `Workers: **${formatInteger(section.data.jobs)}** · cumulative group time: **${formatSeconds(stats.cumulativeSeconds)}** · failed groups: **${formatInteger(stats.failures.length)}**`,
    '',
    '| Slowest group | Scenarios | Duration | Status |',
    '| --- | ---: | ---: | --- |'
  );
  for (const group of stats.slowest) {
    lines.push(`| ${code(group.group)} | ${formatInteger(group.scenarioCount)} | ${formatSeconds(group.durationSeconds)} | ${group.status === 'passed' ? '✅' : '❌'} |`);
  }
  if (stats.failures.length !== 0) {
    lines.push('', '**Failed groups**', '');
    for (const group of stats.failures.slice(0, MAX_DETAIL_ROWS)) {
      lines.push(`- ${code(group.group)} — ${code(group.status)}${typeof group.error === 'string' ? ` · ${code(group.error, 160)}` : ''}`);
    }
  }
  lines.push('', '</details>');
  return lines;
}

function trustedRunUrl(value) {
  try {
    const url = new URL(value);
    if (url.protocol === 'https:' && url.hostname === 'github.com') {
      return url.toString();
    }
  } catch {
    // Fall through to a non-link label.
  }
  return null;
}

function renderReport({ releaseRoot, frontendRoot, agentRoot, runtimeRoot, cdpRoot, runUrl, conclusion }) {
  const sections = {
    release: loadSection(() => loadRelease(releaseRoot)),
    frontend: loadSection(() => loadFrontend(frontendRoot)),
    agent: loadSection(() => loadAgent(agentRoot)),
    runtime: loadSection(() => loadRuntime(runtimeRoot)),
    cdp: loadSection(() => loadCdp(cdpRoot)),
  };
  const overviews = [
    ['Release regression', releaseOverview(sections.release)],
    ['Frontend differential', frontendOverview(sections.frontend)],
    ['Agent episodes', agentOverview(sections.agent)],
    ['Runtime/CDP contracts', runtimeOverview(sections.runtime)],
    ['CDP smoke', cdpOverview(sections.cdp)],
  ];
  const available = Object.values(sections).filter((section) => section.available).length;
  const sourceConclusion = ['success', 'failure', 'cancelled', 'timed_out', 'in_progress'].includes(conclusion)
    ? conclusion
    : 'unknown';
  const link = trustedRunUrl(runUrl);
  const lines = [
    COMMENT_MARKER,
    '## CI Regression Report',
    '',
    `${link ? `[Source CI run](${link})` : 'Source CI run'} · source state at render: \`${sourceConclusion}\` · artifacts: \`${available}/5\``,
    '',
    '| Check | Status | Signal |',
    '| --- | :---: | --- |',
  ];
  for (const [label, overview] of overviews) {
    lines.push(`| ${label} | ${overview.status} | ${overview.signal} |`);
  }
  lines.push(
    '',
    ...renderRelease(sections.release),
    '',
    ...renderFrontend(sections.frontend),
    '',
    ...renderAgent(sections.agent),
    '',
    ...renderRuntime(sections.runtime),
    '',
    ...renderCdp(sections.cdp),
    '',
    '_All artifact fields are parsed by the trusted default-branch renderer; missing or invalid inputs remain visible as unavailable._',
    ''
  );
  const rendered = lines.join('\n');
  if (Buffer.byteLength(rendered, 'utf8') > MAX_COMMENT_BYTES) {
    throw new Error('rendered CI regression report exceeds 32 KiB');
  }
  return rendered;
}

function parseArgs(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith('--') || value === undefined) {
      throw new Error('arguments must be --name value pairs');
    }
    values[key.slice(2)] = value;
  }
  if (!values.output) {
    throw new Error('--output is required');
  }
  return values;
}

function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  const rendered = renderReport({
    releaseRoot: args.release,
    frontendRoot: args.frontend,
    agentRoot: args.agent,
    runtimeRoot: args.runtime,
    cdpRoot: args.cdp,
    runUrl: args['run-url'],
    conclusion: args.conclusion,
  });
  fs.writeFileSync(args.output, rendered, 'utf8');
}

if (require.main === module) {
  main();
}

module.exports = {
  COMMENT_MARKER,
  loadCdp,
  loadFrontend,
  loadRelease,
  renderReport,
};
