#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

function parseArgs(argv) {
  const args = { binary: null, profiles: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === '--binary') {
      args.binary = path.resolve(argv[++index]);
    } else {
      args.profiles.push(path.resolve(value));
    }
  }
  if (!args.binary || args.profiles.length === 0) {
    throw new Error('usage: analyze-lightpanda-pa-profile.mjs --binary BIN PROFILE...');
  }
  return args;
}

function parseFields(line) {
  return Object.fromEntries(
    [...line.matchAll(/(\w+)=([^\s]+)/g)].map((match) => {
      const raw = match[2];
      const value = raw.startsWith('0x') ? raw : Number.parseInt(raw, 10);
      return [match[1], value];
    }),
  );
}

function parseProfile(profilePath) {
  const lines = fs.readFileSync(profilePath, 'utf8').trim().split('\n');
  const header = parseFields(lines.find((line) => line.startsWith('H ')) ?? '');
  const histogram = lines
    .filter((line) => line.startsWith('B '))
    .map(parseFields);
  const samples = lines
    .filter((line) => line.startsWith('S '))
    .map((line) => {
      const fields = parseFields(line);
      return {
        size: fields.size,
        weight: fields.weight,
        key: fields.key,
        depth: fields.depth,
        frames: [...line.matchAll(/\s(0x[0-9a-f]+)(?=\s|$)/g)]
          .map((match) => match[1]),
      };
    });
  return { profilePath, header, histogram, samples };
}

function symbolize(binary, addresses) {
  const result = new Map();
  const unique = [...new Set(addresses)];
  for (let start = 0; start < unique.length; start += 500) {
    const batch = unique.slice(start, start + 500);
    const child = spawnSync('addr2line', ['-Cfpe', binary, ...batch], {
      encoding: 'utf8',
      maxBuffer: 16 * 1024 * 1024,
    });
    if (child.status !== 0) {
      throw new Error(`addr2line failed: ${child.stderr}`);
    }
    const lines = child.stdout.trimEnd().split('\n');
    if (lines.length !== batch.length) {
      throw new Error(`addr2line returned ${lines.length} lines for ${batch.length} addresses`);
    }
    for (let index = 0; index < batch.length; index += 1) {
      result.set(batch[index], lines[index]);
    }
  }
  return result;
}

function isProfilerFrame(symbol) {
  return symbol.includes('pa_profile.cpp')
    || symbol.includes('RecordAllocation')
    || symbol.includes('DispatchAlloc');
}

function isAllocatorPlumbing(symbol) {
  return isProfilerFrame(symbol)
    || /^(malloc|calloc|realloc|memalign|aligned_alloc|operator new)/.test(symbol)
    || symbol.includes('allocator_shim::')
    || symbol.includes('Arena.rawAlloc')
    || symbol.includes('ArenaAllocator.alloc')
    || symbol.includes('mem.Allocator.rawAlloc')
    || symbol.includes('mem.Allocator.allocBytesWithAlignment')
    || symbol.includes('mem.Allocator.allocWithOptions');
}

function aggregate(samples, keyForSample) {
  const totals = new Map();
  for (const sample of samples) {
    const key = keyForSample(sample);
    const current = totals.get(key) ?? { estimatedBytes: 0, sampledBytes: 0, samples: 0 };
    current.estimatedBytes += sample.size * sample.weight;
    current.sampledBytes += sample.size;
    current.samples += 1;
    totals.set(key, current);
  }
  return [...totals.entries()]
    .map(([key, totalsForKey]) => ({ key, ...totalsForKey }))
    .sort((left, right) => right.estimatedBytes - left.estimatedBytes);
}

function categoryForSample(sample) {
  const stack = sample.consumerStack.join('\n');
  const matches = (pattern) => pattern.test(stack);

  if (matches(/Notification\.dispatch|HttpClient\.Transfer\.deliver|HttpClient\.drainInbox/)) {
    return 'network response buffers / notifications';
  }
  if (matches(/Timers\.|setTimeout|setInterval/)) return 'timers';
  if (matches(/FinalizerCallback|mapZigInstanceToJs|browser\.js\.Local/)) {
    return 'JS wrapper finalizers';
  }
  if (matches(/StyleManager|loadExternalStylesheet|stylesheet|CSSRule|cssparser|style::|selectors::/)) {
    return 'CSS / Stylo / CSSOM';
  }
  if (matches(/ScriptManager|ImportMap|resolveModule|browser\.js\.Module/)) {
    return 'scripts / modules / import maps';
  }
  if (matches(/node_factory|browser\.parser|slab\.Slab|html5ever/)) {
    return 'DOM / bindings / parser';
  }
  if (matches(/ArenaPool|ArenaAllocator/)) return 'arena pools';
  if (matches(/v8::|Builtins_|icu_|_uhash_|cppgc/)) return 'V8 / ICU native';
  if (matches(/OPENSSL_|SSL_|CRYPTO_|bssl::/)) return 'TLS / crypto';
  if (matches(/Curl_|nghttp2_|libcurl/)) return 'curl / HTTP2';
  if (matches(/browser\.|cdp\./)) return 'other Lightpanda browser / CDP';
  return 'other';
}

function analyze(profile, symbols) {
  const samples = profile.samples.map((sample) => {
    const stack = sample.frames.map((address) => symbols.get(address) ?? `${address} at ??`);
    const externalStack = stack.filter((symbol) => !isProfilerFrame(symbol));
    const consumerStack = stack.filter((symbol) => !isAllocatorPlumbing(symbol));
    return {
      ...sample,
      stack,
      leaf: externalStack[0] ?? '(profiler only)',
      consumer: consumerStack[0] ?? externalStack[0] ?? '(unknown)',
      consumerStack,
    };
  });
  const estimatedRequestedBytes = samples.reduce(
    (total, sample) => total + sample.size * sample.weight,
    0,
  );
  const sampledRequestedBytes = samples.reduce((total, sample) => total + sample.size, 0);
  const exactLargeSamples = samples.filter((sample) => sample.weight === 1);
  const exactLargeRequestedBytes = exactLargeSamples.reduce(
    (total, sample) => total + sample.size,
    0,
  );
  const histogramLargeRequestedBytes = profile.histogram
    .filter((bin) => bin.bin >= 17)
    .reduce((total, bin) => total + bin.requested_bytes, 0);
  const topCategories = aggregate(samples, categoryForSample).map((category) => ({
    ...category,
    estimatedFraction: estimatedRequestedBytes === 0
      ? 0
      : category.estimatedBytes / estimatedRequestedBytes,
  }));

  return {
    profile: profile.profilePath,
    header: profile.header,
    histogram: profile.histogram,
    sampling: {
      liveSamples: samples.length,
      sampledRequestedBytes,
      estimatedRequestedBytes,
      estimateToExactRatio: estimatedRequestedBytes / profile.header.live_requested_bytes,
      exactLargeSamples: exactLargeSamples.length,
      exactLargeRequestedBytes,
      histogramLargeRequestedBytes,
      exactLargeMatchesHistogram: exactLargeRequestedBytes === histogramLargeRequestedBytes,
    },
    topCategories,
    topCategoryConsumers: aggregate(
      samples,
      (sample) => `${categoryForSample(sample)}\t${sample.consumer}`,
    ).slice(0, 100),
    topConsumers: aggregate(samples, (sample) => sample.consumer).slice(0, 40),
    topConsumerStacks: aggregate(
      samples,
      (sample) => sample.consumerStack.slice(0, 8).join(' <- ') || sample.consumer,
    ).slice(0, 40),
    largestLiveAllocations: exactLargeSamples
      .sort((left, right) => right.size - left.size)
      .slice(0, 100)
      .map((sample) => ({
        size: sample.size,
        key: sample.key,
        consumer: sample.consumer,
        stack: sample.consumerStack.slice(0, 12),
      })),
  };
}

const args = parseArgs(process.argv.slice(2));
const profiles = args.profiles.map(parseProfile);
const symbols = symbolize(
  args.binary,
  profiles.flatMap((profile) => profile.samples.flatMap((sample) => sample.frames)),
);
console.log(JSON.stringify(profiles.map((profile) => analyze(profile, symbols)), null, 2));
