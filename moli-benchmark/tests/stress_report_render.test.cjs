const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');
const root = path.join(__dirname, '..', 'moli_benchmark');
const source = fs.readFileSync(path.join(root, 'stress_report.py'), 'utf8');
const d3 = require(path.join(root, 'stress_assets', 'd3.min.js'));

// Execute the report's actual embedded formatting/domain code with native
// JavaScript and the vendored D3. This does not simulate a browser rendering.
const formatting = source.slice(source.indexOf('const fmt='), source.indexOf('const set='));
const cards = source.split('\n').find(line => line.startsWith("set('card-duration'"));

test('missing latency cards display unavailable, while observed zero stays zero', () => {
  for (const [value, expected] of [[null, '— / —s'], [0, '0.00 / 0.00s'], [1500, '1.50 / 1.50s']]) {
    const values = {};
    vm.runInNewContext(formatting + '\n' + cards, {
      d3,
      S: {duration_seconds: 60, throughput_navigations_per_second: 1,
        latency: {load_ms: {p50: value, p95: value, p99: value}}},
      set: (id, text) => { values[id] = text; },
    });
    assert.equal(values['card-load'], expected);
    assert.equal(values['card-load-sub'], `P99 ${expected.split(' / ')[0]}s`);
  }
});

test('axes remain finite for an empty observed series without adding data', () => {
  const domainCode = source.split('\n').find(line => line.startsWith(' const domain=axis=>'));
  for (const zeroY of [false, true]) {
    const context = vm.createContext({d3, options: {zeroY}, data: [{load: null}], series: [{axis: 'left', key: 'load'}]});
    vm.runInContext(formatting + '\n' + domainCode, context);
    const domain = vm.runInContext("domain('left')", context);
    assert.deepEqual(Array.from(domain), [0, 1]);
  }
  const context = vm.createContext({d3, options: {}, data: [{load: 100}, {load: 200}], series: [{axis: 'left', key: 'load'}]});
  vm.runInContext(formatting + '\n' + domainCode, context);
  assert.deepEqual(Array.from(vm.runInContext("domain('left')", context)), [94, 208]);
});
