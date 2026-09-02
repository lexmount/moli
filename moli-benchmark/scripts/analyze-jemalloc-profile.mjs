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
    throw new Error('usage: analyze-jemalloc-profile.mjs --binary BIN PROFILE...');
  }
  return args;
}

function parseMapping(line) {
  const match = line.match(
    /^([0-9a-f]+)-([0-9a-f]+)\s+(\S+)\s+([0-9a-f]+)\s+\S+\s+\d+\s*(.*)$/,
  );
  if (!match) {
    return null;
  }
  return {
    start: BigInt(`0x${match[1]}`),
    end: BigInt(`0x${match[2]}`),
    permissions: match[3],
    offset: BigInt(`0x${match[4]}`),
    mappedPath: match[5],
  };
}

function adjustedSample(rawObjects, rawBytes, interval) {
  const meanObjectSize = rawBytes / rawObjects;
  const scale = 1 / -Math.expm1(-meanObjectSize / interval);
  return {
    adjustedBytes: rawBytes * scale,
    adjustedObjects: rawObjects * scale,
    scale,
  };
}

function parseProfile(profilePath) {
  const lines = fs.readFileSync(profilePath, 'utf8').split('\n');
  const intervalMatch = lines[0].match(/^heap_v2\/(\d+)$/);
  if (!intervalMatch) {
    throw new Error(`${profilePath}: unsupported jemalloc profile header ${lines[0]}`);
  }
  const interval = Number.parseInt(intervalMatch[1], 10);
  const mappingsStart = lines.indexOf('MAPPED_LIBRARIES:');
  if (mappingsStart < 0) {
    throw new Error(`${profilePath}: missing MAPPED_LIBRARIES section`);
  }

  const headerMatch = lines
    .slice(1, mappingsStart)
    .find((line) => /^\s+t\*:/.test(line))
    ?.match(/^\s+t\*:\s+(\d+):\s+(\d+)/);
  const samples = [];
  for (let index = 1; index < mappingsStart; index += 1) {
    if (!lines[index].startsWith('@ ')) {
      continue;
    }
    const frames = lines[index].slice(2).trim().split(/\s+/).filter(Boolean);
    const totalsMatch = lines[index + 1]?.match(/^\s+t\*:\s+(\d+):\s+(\d+)/);
    if (!totalsMatch) {
      throw new Error(`${profilePath}:${index + 2}: stack is missing its t* sample`);
    }
    const rawObjects = Number.parseInt(totalsMatch[1], 10);
    const rawBytes = Number.parseInt(totalsMatch[2], 10);
    if (rawObjects === 0 || rawBytes === 0) {
      continue;
    }
    samples.push({
      frames,
      rawObjects,
      rawBytes,
      ...adjustedSample(rawObjects, rawBytes, interval),
    });
  }

  const mappings = lines
    .slice(mappingsStart + 1)
    .map(parseMapping)
    .filter((mapping) => mapping !== null);
  return {
    profilePath,
    interval,
    header: headerMatch
      ? {
        sampledObjects: Number.parseInt(headerMatch[1], 10),
        sampledBytes: Number.parseInt(headerMatch[2], 10),
      }
      : null,
    samples,
    mappings,
  };
}

function mappingForAddress(mappings, address) {
  const numericAddress = BigInt(address);
  return mappings.find(
    (mapping) => numericAddress >= mapping.start && numericAddress < mapping.end,
  );
}

function findMainMapping(profile) {
  const executablePaths = new Set(
    profile.mappings
      .filter((mapping) => mapping.permissions.includes('x') && mapping.mappedPath.startsWith('/'))
      .map((mapping) => mapping.mappedPath),
  );
  const pathScores = new Map([...executablePaths].map((mappedPath) => [mappedPath, 0]));
  for (const sample of profile.samples) {
    for (const frame of sample.frames) {
      const mapping = mappingForAddress(profile.mappings, frame);
      if (mapping && pathScores.has(mapping.mappedPath)) {
        pathScores.set(
          mapping.mappedPath,
          pathScores.get(mapping.mappedPath) + sample.adjustedBytes,
        );
      }
    }
  }
  const [mainPath] = [...pathScores.entries()]
    .sort((left, right) => right[1] - left[1])[0] ?? [];
  if (!mainPath) {
    throw new Error(`${profile.profilePath}: could not identify the main executable mapping`);
  }
  const baseMapping = profile.mappings.find(
    (mapping) => mapping.mappedPath === mainPath && mapping.offset === 0n,
  );
  if (!baseMapping) {
    throw new Error(`${profile.profilePath}: main executable has no offset-zero mapping`);
  }
  return { mappedPath: mainPath, loadBias: baseMapping.start };
}

function normalizeMainAddress(address, mainMapping) {
  const numericAddress = BigInt(address);
  const relativeAddress = numericAddress - mainMapping.loadBias;
  const callerAddress = relativeAddress > 0n ? relativeAddress - 1n : relativeAddress;
  return `0x${callerAddress.toString(16)}`;
}

function symbolize(binary, profiles) {
  const normalizedAddresses = new Set();
  for (const profile of profiles) {
    for (const sample of profile.samples) {
      for (const address of sample.frames) {
        const mapping = mappingForAddress(profile.mappings, address);
        if (mapping?.mappedPath === profile.mainMapping.mappedPath) {
          normalizedAddresses.add(normalizeMainAddress(address, profile.mainMapping));
        }
      }
    }
  }

  const result = new Map();
  const unique = [...normalizedAddresses];
  for (let start = 0; start < unique.length; start += 500) {
    const batch = unique.slice(start, start + 500);
    const child = spawnSync('addr2line', ['-Cfpe', binary, ...batch], {
      encoding: 'utf8',
      maxBuffer: 32 * 1024 * 1024,
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

function describeFrame(profile, symbols, address) {
  const mapping = mappingForAddress(profile.mappings, address);
  if (!mapping) {
    return `${address} at (unmapped)`;
  }
  if (mapping.mappedPath === profile.mainMapping.mappedPath) {
    const normalized = normalizeMainAddress(address, profile.mainMapping);
    return symbols.get(normalized) ?? `${normalized} at ??`;
  }
  const location = BigInt(address) - mapping.start + mapping.offset;
  const label = mapping.mappedPath
    ? path.basename(mapping.mappedPath)
    : `(anonymous ${mapping.permissions})`;
  return `${label}+0x${location.toString(16)}`;
}

function isAllocatorPlumbing(symbol) {
  return symbol.includes('prof_backtrace')
    || symbol.includes('je_prof_')
    || symbol.includes('prof_alloc_')
    || symbol.includes('prof_sample_')
    || symbol.includes('prof_tctx_')
    || symbol.includes('jemalloc')
    || symbol.includes('_rjem_')
    || symbol.includes('tikv_jemalloc')
    || symbol.startsWith('operator new(')
    || symbol.includes('OPENSSL_malloc')
    || symbol.includes('OPENSSL_realloc')
    || symbol.includes('mallocx')
    || symbol.includes('imalloc')
    || symbol.includes('ialloc')
    || symbol.includes('arena_malloc')
    || symbol.includes('arena_ralloc')
    || symbol.includes('arena_dalloc')
    || symbol.includes('tcache_alloc')
    || symbol.includes('__rust_alloc')
    || symbol.includes('__rdl_alloc')
    || symbol.includes('alloc::alloc::')
    || symbol.includes('alloc::raw_vec::RawVecInner')
    || symbol.includes('alloc::raw_vec::RawVec<')
    || symbol.includes('hashbrown::raw::RawTableInner::fallible_with_capacity')
    || symbol.includes('hashbrown::raw::RawTable<T,A>::reserve_rehash')
    || symbol.includes('smallvec::SmallVec<A>::try_grow')
    || symbol.includes('thin_vec::ThinVec<T>::reserve')
    || symbol.includes('alloc::raw_vec::RawVecInner::try_allocate_in')
    || symbol.includes('Allocator::allocate')
    || symbol.includes('Allocator::grow');
}

function aggregate(samples, keyForSample) {
  const totals = new Map();
  for (const sample of samples) {
    const key = keyForSample(sample);
    const current = totals.get(key) ?? {
      adjustedBytes: 0,
      adjustedObjects: 0,
      sampledBytes: 0,
      sampledObjects: 0,
      stacks: 0,
    };
    current.adjustedBytes += sample.adjustedBytes;
    current.adjustedObjects += sample.adjustedObjects;
    current.sampledBytes += sample.rawBytes;
    current.sampledObjects += sample.rawObjects;
    current.stacks += 1;
    totals.set(key, current);
  }
  return [...totals.entries()]
    .map(([key, totalsForKey]) => ({ key, ...totalsForKey }))
    .sort((left, right) => right.adjustedBytes - left.adjustedBytes);
}

function categoryForSample(sample) {
  const stack = sample.consumerStack.join('\n');
  const matches = (pattern) => pattern.test(stack);

  if (matches(/SubresourceResponseBody|SubresourceNetworkRecord|ScriptNetworkOutput|TargetNetworkOutput|TargetSubresource|CapturedBodyWriter|renderer_network_observation|response::ResponseBody/)) {
    return 'network observations / retained response bodies';
  }
  if (matches(/style::|selectors::|cssparser::|Stylesheet|stylesheet|CSSRule|css_rule|css_stylesheet|style_engine/)) {
    return 'CSS / Stylo / CSSOM';
  }
  if (matches(/ModuleScript|module_script|module_runtime|script_planning|ScriptPreload|ImportMap|import_map|decode_classic_script|external_script_source/)) {
    return 'scripts / modules / import maps';
  }
  if (matches(/moli_dom::|NativeDom|native_bridge|document_runtime|html5ever|moli_parser/)) {
    return 'DOM / bindings / parser';
  }
  if (matches(/v8::|Builtins_|icu_|_uhash_|cppgc/)) {
    return 'V8 / ICU native';
  }
  if (matches(/aws_lc_|OPENSSL_|SSL_|bssl::|CRYPTO_/)) {
    return 'TLS / crypto';
  }
  if (matches(/Curl_|curl::|nghttp2_|http2\.c/)) {
    return 'curl / HTTP2';
  }
  if (matches(/tokio::|mio::|futures::|moli_local_executor|runtime::owner/)) {
    return 'async runtime / scheduling';
  }
  if (matches(/moli_protocol::|moli_protocol_server::/)) {
    return 'protocol / CDP';
  }
  if (matches(/moli_/)) {
    return 'other Moli';
  }
  return 'other';
}

function analyze(profile, symbols) {
  const samples = profile.samples.map((sample) => {
    const stack = sample.frames.map((address) => describeFrame(profile, symbols, address));
    const consumerStack = stack.filter((symbol) => !isAllocatorPlumbing(symbol));
    return {
      ...sample,
      stack,
      consumer: consumerStack[0] ?? stack[0] ?? '(unknown)',
      consumerStack,
    };
  });
  const adjustedBytes = samples.reduce((total, sample) => total + sample.adjustedBytes, 0);
  const adjustedObjects = samples.reduce((total, sample) => total + sample.adjustedObjects, 0);
  const sampledBytes = samples.reduce((total, sample) => total + sample.rawBytes, 0);
  const sampledObjects = samples.reduce((total, sample) => total + sample.rawObjects, 0);
  const topCategories = aggregate(samples, categoryForSample).map((category) => ({
    ...category,
    adjustedFraction: adjustedBytes === 0 ? 0 : category.adjustedBytes / adjustedBytes,
  }));

  return {
    profile: profile.profilePath,
    interval: profile.interval,
    mainMapping: {
      path: profile.mainMapping.mappedPath,
      loadBias: `0x${profile.mainMapping.loadBias.toString(16)}`,
    },
    header: profile.header,
    sampling: {
      liveStackRecords: samples.length,
      sampledBytes,
      sampledObjects,
      adjustedBytes,
      adjustedObjects,
      headerBytesMatch: profile.header?.sampledBytes === sampledBytes,
      headerObjectsMatch: profile.header?.sampledObjects === sampledObjects,
    },
    topCategories,
    topCategoryConsumers: aggregate(
      samples,
      (sample) => `${categoryForSample(sample)}\t${sample.consumer}`,
    ).slice(0, 100),
    topConsumers: aggregate(samples, (sample) => sample.consumer).slice(0, 50),
    topConsumerStacks: aggregate(
      samples,
      (sample) => sample.consumerStack.slice(0, 10).join(' <- ') || sample.consumer,
    ).slice(0, 50),
    largestSampledStacks: samples
      .sort((left, right) => right.adjustedBytes - left.adjustedBytes)
      .slice(0, 100)
      .map((sample) => ({
        sampledObjects: sample.rawObjects,
        sampledBytes: sample.rawBytes,
        adjustedObjects: sample.adjustedObjects,
        adjustedBytes: sample.adjustedBytes,
        scale: sample.scale,
        consumer: sample.consumer,
        stack: sample.consumerStack.slice(0, 16),
      })),
  };
}

const args = parseArgs(process.argv.slice(2));
const profiles = args.profiles.map(parseProfile);
for (const profile of profiles) {
  profile.mainMapping = findMainMapping(profile);
}
const symbols = symbolize(args.binary, profiles);
console.log(JSON.stringify(profiles.map((profile) => analyze(profile, symbols)), null, 2));
