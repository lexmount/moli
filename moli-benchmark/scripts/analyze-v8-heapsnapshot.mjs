#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';

const inputs = process.argv.slice(2);
if (inputs.length === 0) {
  throw new Error('usage: analyze-v8-heapsnapshot.mjs SNAPSHOT...');
}

function topEntries(map, limit) {
  return [...map.entries()]
    .map(([name, value]) => ({ name, ...value }))
    .sort((left, right) => right.selfSize - left.selfSize || right.count - left.count)
    .slice(0, limit);
}

function add(map, key, selfSize) {
  const value = map.get(key) ?? { count: 0, selfSize: 0 };
  value.count += 1;
  value.selfSize += selfSize;
  map.set(key, value);
}

function analyze(input) {
  const snapshot = JSON.parse(fs.readFileSync(input, 'utf8'));
  const fields = snapshot.snapshot.meta.node_fields;
  const fieldCount = fields.length;
  const typeIndex = fields.indexOf('type');
  const nameIndex = fields.indexOf('name');
  const selfSizeIndex = fields.indexOf('self_size');
  if ([typeIndex, nameIndex, selfSizeIndex].includes(-1)) {
    throw new Error(`${input}: unsupported node_fields`);
  }
  if (snapshot.nodes.length % fieldCount !== 0) {
    throw new Error(`${input}: truncated nodes array`);
  }

  const nodeTypes = snapshot.snapshot.meta.node_types[typeIndex];
  const byType = new Map();
  const byName = new Map();
  const byTypeAndName = new Map();
  let selfSize = 0;

  for (let offset = 0; offset < snapshot.nodes.length; offset += fieldCount) {
    const type = nodeTypes[snapshot.nodes[offset + typeIndex]];
    const name = snapshot.strings[snapshot.nodes[offset + nameIndex]];
    const size = snapshot.nodes[offset + selfSizeIndex];
    selfSize += size;
    add(byType, type, size);
    add(byName, name, size);
    add(byTypeAndName, `${type}\t${name}`, size);
  }

  return {
    path: path.resolve(input),
    bytes: fs.statSync(input).size,
    nodeCount: snapshot.nodes.length / fieldCount,
    selfSize,
    byType: topEntries(byType, byType.size),
    topNames: topEntries(byName, 60),
    topTypeNames: topEntries(byTypeAndName, 100).map((entry) => {
      const [type, name] = entry.name.split('\t', 2);
      return { type, name, count: entry.count, selfSize: entry.selfSize };
    }),
  };
}

for (const input of inputs) {
  process.stdout.write(`${JSON.stringify(analyze(input))}\n`);
}
