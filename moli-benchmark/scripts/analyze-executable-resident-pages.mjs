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

function clusterSymbol(name, engine) {
  if (/(^| )v8::|^(v8_|Builtins_|Builtins::|icu_|u[a-z]+_\d+|std::__Cr::)/.test(name)) {
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
    ['-S', '--size-sort', '--demangle', '--defined-only', binary],
    { encoding: 'utf8', maxBuffer: 1024 * 1024 * 256 },
  );
  for (const line of nm.split('\n')) {
    const match = line.match(/^([0-9a-f]+)\s+([0-9a-f]+)\s+([tTwW])\s+(.*)$/);
    if (!match) continue;
    const virtualAddress = Number.parseInt(match[1], 16);
    const size = Number.parseInt(match[2], 16);
    if (size === 0 || virtualAddress < segment.virtualAddress) continue;
    const fileStart = segment.offset + virtualAddress - segment.virtualAddress;
    const fileEnd = fileStart + size;
    if (fileStart >= segment.offset + segment.fileSize) continue;
    symbols.push({
      name: match[4],
      fileStart,
      fileEnd: Math.min(fileEnd, segment.offset + segment.fileSize),
    });
  }

  const byCluster = new Map();
  const otherByPrefix = new Map();
  const bySymbol = [];
  let symbolResidentBytes = 0;
  for (const symbol of symbols) {
    let residentBytes = 0;
    const firstPage = Math.floor(symbol.fileStart / pageSize);
    const lastPage = Math.floor((symbol.fileEnd - 1) / pageSize);
    for (let page = firstPage; page <= lastPage; page += 1) {
      if (!presentPages.has(page)) continue;
      residentBytes += Math.max(
        0,
        Math.min(symbol.fileEnd, (page + 1) * pageSize) - Math.max(symbol.fileStart, page * pageSize),
      );
    }
    if (residentBytes === 0) continue;
    symbolResidentBytes += residentBytes;
    const cluster = clusterSymbol(symbol.name, run.engine);
    add(byCluster, cluster, residentBytes);
    if (cluster === 'Other') add(otherByPrefix, symbolPrefix(symbol.name), residentBytes);
    bySymbol.push({ name: symbol.name, cluster, residentBytes });
  }

  const residentBytes = presentPages.size * pageSize;
  return {
    label: run.label,
    engine: run.engine,
    binary: path.resolve(binary),
    residentBytes,
    smapsExecutablePssBytes: run.memoryBeforeDiagnostics.buckets.mainExecutable.pss,
    symbolResidentBytes,
    unattributedBytes: Math.max(0, residentBytes - symbolResidentBytes),
    byCluster: [...byCluster.entries()]
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
  const binary = run.engine === 'moli' ? artifact.binaries.moli.path : artifact.binaries.lightpanda.path;
  process.stdout.write(`${JSON.stringify(analyzeRun(run, binary))}\n`);
}
