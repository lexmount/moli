#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const artifactPath = process.argv[2];
if (!artifactPath) {
  throw new Error('usage: analyze-executable-resident-pages.mjs ARTIFACT.json');
}

function executableSegment(binary) {
  const output = execFileSync('readelf', ['-lW', binary], { encoding: 'utf8' });
  for (const line of output.split('\n')) {
    const fields = line.trim().split(/\s+/);
    if (fields[0] !== 'LOAD' || !fields.includes('E')) continue;
    return {
      offset: Number.parseInt(fields[1], 16),
      virtualAddress: Number.parseInt(fields[2], 16),
      fileSize: Number.parseInt(fields[4], 16),
    };
  }
  throw new Error(`${binary}: executable LOAD segment not found`);
}

function clusterSymbol(name, engine, isRust) {
  // A Rust adapter implementing a v8 trait is not native C++ V8 code. Nor is
  // a Rust function native V8 just because one of its type arguments is V8.
  if (isRust && /(^v8::| as v8::)/.test(name)) return 'rusty_v8 bindings / callback adapters';
  if (!isRust && /(^| )v8::|^(v8_|Builtins_|Builtins::|icu_|u[a-z]+_\d+|std::__Cr::)/.test(name)) {
    return 'V8 / ICU / bundled libc++';
  }
  if (/^(aws_lc_|SSL_|CRYPTO_|OPENSSL_|bssl::|ring::)/.test(name)) return 'TLS / crypto';
  if (/^(Curl_|curl_|nghttp2_)/.test(name)) return 'curl / HTTP2';
  if (engine === 'moli') {
    if (name.includes('moli_renderer_v8::')) return 'moli-renderer-v8';
    if (name.includes('moli_protocol::')) return 'moli-protocol';
    if (name.includes('moli_core::')) return 'moli-core';
    if (/(moli_fetch|moli_network|moli_tls|moli_cookie)/.test(name)) return 'Moli fetch / network';
    if (/(^|<)(style::|selectors::|cssparser::|servo_arc::)|stylo_/.test(name)) return 'Servo style';
    if (name.includes('taffy::')) return 'Taffy layout';
    if (/(^|<)(tokio::|mio::|futures_|futures::|hyper::|h2::)/.test(name)) return 'Rust async / HTTP';
    if (/(^|<)(std::|core::|alloc::|hashbrown::|regex)/.test(name)) return 'Rust std / collections';
    if (/(^|<)moli_/.test(name)) return 'Other Moli crates';
  } else {
    if (name.startsWith('browser.webapi.')) return 'Lightpanda Web APIs';
    if (name.startsWith('browser.js.')) return 'Lightpanda V8 bridge';
    if (name.startsWith('browser.')) return 'Lightpanda browser';
    if (name.startsWith('cdp.')) return 'Lightpanda CDP';
    if (/^(Io\.|json\.|array_|hash_map\.|multi_array_|mem\.|fmt\.|fs\.|debug\.|sort\.|Thread\.)/.test(name)) {
      return 'Zig std / generated';
    }
  }
  return 'Other';
}

function add(map, key, bytes) {
  map.set(key, (map.get(key) ?? 0) + bytes);
}

function symbolPrefix(name) {
  const rust = name.indexOf('::');
  const zig = name.indexOf('.');
  const end = [rust, zig].filter((value) => value > 0).sort((a, b) => a - b)[0];
  if (end !== undefined) return name.slice(0, end);
  const c = name.indexOf('_');
  return c > 0 ? name.slice(0, c) : name.split(/[<( ]/, 1)[0];
}

function v8Subsystem(name) {
  if (name.includes('v8::internal::compiler::')) return 'v8::internal::compiler::*';
  if (name.includes('v8::internal::maglev::')) return 'v8::internal::maglev::*';
  return 'Remaining V8 / ICU / bundled libc++';
}

function analyzeRun(run, binary) {
  const pageMap = run.executableResidentPageMap;
  if (!pageMap) throw new Error(`${run.label}: missing executableResidentPageMap`);
  const pageSize = pageMap.mappings[0]?.pageSize ?? 4096;
  const presentPages = new Set();
  for (const mapping of pageMap.mappings) {
    for (const [start, end] of mapping.presentFilePageRanges) {
      for (let page = start; page < end; page += 1) presentPages.add(page);
    }
  }

  const segment = executableSegment(binary);
  const symbols = [];
  const nm = execFileSync(
    'nm',
    ['-S', '--no-sort', '--demangle', '--defined-only', binary],
    { encoding: 'utf8', maxBuffer: 1024 * 1024 * 256 },
  );
  const rawNm = execFileSync('nm', ['-S', '--no-sort', '--defined-only', binary],
    { encoding: 'utf8', maxBuffer: 1024 * 1024 * 256 }).split('\n');
  const demangledRows = nm.split('\n');
  if (rawNm.length !== demangledRows.length) throw new Error('nm symbol table row count changed');
  for (const [index, line] of demangledRows.entries()) {
    const match = line.match(/^([0-9a-f]+)\s+([0-9a-f]+)\s+([tTwW])\s+(.*)$/);
    if (!match) continue;
    const virtualAddress = Number.parseInt(match[1], 16);
    const size = Number.parseInt(match[2], 16);
    if (size === 0 || virtualAddress < segment.virtualAddress) continue;
    const fileStart = segment.offset + virtualAddress - segment.virtualAddress;
    const fileEnd = fileStart + size;
    if (fileStart >= segment.offset + segment.fileSize) continue;
    // --no-sort preserves ELF symbol-table order in both traversals. Check
    // address, size and type before pairing raw mangling with display names.
    const raw = rawNm[index].match(/^([0-9a-f]+)\s+([0-9a-f]+)\s+([tTwW])\s+(.*)$/);
    if (!raw || raw[1] !== match[1] || raw[2] !== match[2] || raw[3] !== match[3]) {
      throw new Error('raw and demangled nm rows do not match');
    }
    symbols.push({
      name: match[4],
      isRust: /^_R/.test(raw[4]) || /^_ZN.*17h[0-9a-f]{16}E(?:\.|$)/.test(raw[4]),
      fileStart,
      fileEnd: Math.min(fileEnd, segment.offset + segment.fileSize),
    });
  }

  const byCluster = new Map();
  const otherByPrefix = new Map();
  const v8BySubsystem = new Map();
  const bySymbol = [];
  const events = [];
  let symbolReferenceResidentBytes = 0;
  for (const symbol of symbols) {
    let residentBytes = 0;
    const cluster = clusterSymbol(symbol.name, run.engine, symbol.isRust);
    const prefix = symbolPrefix(symbol.name);
    const v8Part = v8Subsystem(symbol.name);
    const firstPage = Math.floor(symbol.fileStart / pageSize);
    const lastPage = Math.floor((symbol.fileEnd - 1) / pageSize);
    for (let page = firstPage; page <= lastPage; page += 1) {
      if (!presentPages.has(page)) continue;
      const start = Math.max(symbol.fileStart, page * pageSize);
      const end = Math.min(symbol.fileEnd, (page + 1) * pageSize);
      residentBytes += end - start;
      events.push({ offset: start, cluster, prefix, v8Part, delta: 1 });
      events.push({ offset: end, cluster, prefix, v8Part, delta: -1 });
    }
    if (residentBytes === 0) continue;
    symbolReferenceResidentBytes += residentBytes;
    bySymbol.push({ name: symbol.name, cluster, residentBytes });
  }

  // Linker aliases (including identical-code folding) can cover the same bytes.
  // Count their union once; mixed-category aliases remain explicitly ambiguous.
  events.sort((left, right) => left.offset - right.offset);
  const activeClusters = new Map();
  const activeOtherPrefixes = new Map();
  const activeV8Subsystems = new Map();
  const adjust = (map, key, delta) => {
    const count = (map.get(key) ?? 0) + delta;
    if (count === 0) map.delete(key);
    else map.set(key, count);
  };
  let symbolResidentBytes = 0;
  let previousOffset = 0;
  for (const event of events) {
    const bytes = event.offset - previousOffset;
    if (bytes > 0 && activeClusters.size > 0) {
      const cluster = activeClusters.size === 1
        ? activeClusters.keys().next().value
        : 'Shared / overlapping symbols';
      add(byCluster, cluster, bytes);
      symbolResidentBytes += bytes;
      if (cluster === 'Other') {
        const prefix = activeOtherPrefixes.size === 1
          ? activeOtherPrefixes.keys().next().value
          : '<shared-symbol-aliases>';
        add(otherByPrefix, prefix, bytes);
      }
      if (cluster === 'V8 / ICU / bundled libc++') {
        const subsystem = activeV8Subsystems.size === 1
          ? activeV8Subsystems.keys().next().value
          : 'Shared / overlapping V8 subsystems';
        add(v8BySubsystem, subsystem, bytes);
      }
    }
    adjust(activeClusters, event.cluster, event.delta);
    if (event.cluster === 'Other') adjust(activeOtherPrefixes, event.prefix, event.delta);
    if (event.cluster === 'V8 / ICU / bundled libc++') {
      adjust(activeV8Subsystems, event.v8Part, event.delta);
    }
    previousOffset = event.offset;
  }
  const residentBytes = presentPages.size * pageSize;
  if (symbolResidentBytes > residentBytes || activeClusters.size !== 0) {
    throw new Error(`${run.label}: invalid resident symbol coverage`);
  }
  if ([...v8BySubsystem.values()].reduce((sum, bytes) => sum + bytes, 0)
    !== (byCluster.get('V8 / ICU / bundled libc++') ?? 0)) {
    throw new Error(`${run.label}: V8 subcategories do not partition their parent`);
  }
  return {
    label: run.label,
    round: run.round,
    url: run.url,
    engine: run.engine,
    binary: path.resolve(binary),
    residentBytes,
    smapsExecutablePssBytes: run.memoryBeforeDiagnostics.buckets.mainExecutable.pss,
    symbolResidentBytes,
    symbolReferenceResidentBytes,
    symbolAliasOvercountBytes: symbolReferenceResidentBytes - symbolResidentBytes,
    unattributedBytes: residentBytes - symbolResidentBytes,
    attributionMethod: 'disjoint resident byte intervals; mixed-category aliases reported separately',
    namespaceClassification: 'raw Rust mangling separates rusty_v8 bindings from native C++ V8; demangled signatures alone do not establish ownership',
    topSymbolsAreAdditive: false,
    byCluster: [...byCluster.entries()]
      .map(([name, bytes]) => ({ name, bytes }))
      .sort((left, right) => right.bytes - left.bytes),
    v8BySubsystem: [...v8BySubsystem.entries()]
      .map(([name, bytes]) => ({ name, bytes }))
      .sort((left, right) => right.bytes - left.bytes),
    otherByPrefix: [...otherByPrefix.entries()]
      .map(([name, bytes]) => ({ name, bytes }))
      .sort((left, right) => right.bytes - left.bytes)
      .slice(0, 50),
    topSymbols: bySymbol
      .sort((left, right) => right.residentBytes - left.residentBytes)
      .slice(0, 50),
  };
}

const artifact = JSON.parse(fs.readFileSync(artifactPath, 'utf8'));
for (const run of artifact.runs) {
  const binary = run.binaryPath
    ?? (run.engine === 'moli' ? artifact.binaries.moli.path : artifact.binaries.lightpanda.path);
  process.stdout.write(`${JSON.stringify(analyzeRun(run, binary))}\n`);
}
