(async () => {
  const rows = [];
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const check = (name, run) => {
    try { run(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const timingNames = ['navigationStart', 'unloadEventStart', 'unloadEventEnd', 'redirectStart', 'redirectEnd',
    'fetchStart', 'domainLookupStart', 'domainLookupEnd', 'connectStart', 'connectEnd', 'secureConnectionStart',
    'requestStart', 'responseStart', 'responseEnd', 'domLoading', 'domInteractive', 'domContentLoadedEventStart',
    'domContentLoadedEventEnd', 'domComplete', 'loadEventStart', 'loadEventEnd'];
  const definitions = [
    ['PerformanceTiming', 'timing', timingNames],
    ['PerformanceNavigation', 'navigation', ['type', 'redirectCount']],
  ];
  const frame = document.body.appendChild(document.createElement('iframe'));
  const realms = {main: window, child: frame.contentWindow};
  try {
    for (const [ownerName, owner] of Object.entries(realms)) {
      const performance = owner.performance;
      for (const [name, member, attributes] of definitions) {
        const value = performance[member], prototype = owner[name].prototype;
        const initial = value.toJSON();
        check(`${ownerName} ${name} interface and instance shape`, () => {
          assert(Object.getPrototypeOf(value) === prototype, 'receiver realm prototype');
          assert(Object.getOwnPropertyNames(value).length === 0, 'no own IDL attributes or operations');
          assert(Object.prototype.toString.call(value) === `[object ${name}]`, 'native tag');
          assert(Object.isExtensible(value), 'platform object remains extensible');
          assert(performance[member] === value, 'SameObject');
          assert(owner[name].name === name && owner[name].length === 0, 'interface signature');
          for (const construct of [() => owner[name](), () => new owner[name]()]) {
            let error; try { construct(); } catch (caught) { error = caught; }
            assert(error instanceof owner.TypeError, 'illegal constructor');
          }
        });
        for (const attribute of attributes) {
          check(`${ownerName} ${name}.${attribute} readonly integer`, () => {
            const saved = value[attribute];
            assert(Number.isInteger(saved) && saved >= 0, 'unsigned integer value');
            const written = Reflect.set(value, attribute, 42);
            if (written) Reflect.set(value, attribute, saved);
            assert(!written && value[attribute] === saved, 'assignment cannot alter native data');
          });
        }
        for (const [calleeName, callee] of Object.entries(realms)) {
          const calleePrototype = callee[name].prototype;
          const methods = attributes.map(attribute => [attribute, () => Object.getOwnPropertyDescriptor(calleePrototype, attribute)?.get]);
          methods.push(['toJSON', () => calleePrototype.toJSON]);
          for (const [attribute, method] of methods) {
            const label = `${ownerName} ${name}.${attribute} via ${calleeName}`;
            check(`${label} descriptor and genuine receiver`, () => {
              const descriptor = Object.getOwnPropertyDescriptor(calleePrototype, attribute);
              assert(descriptor?.enumerable && descriptor.configurable, 'enumerable configurable prototype member');
              const fn = method();
              assert(typeof fn === 'function' && fn.length === 0, 'arity');
              assert(Object.getPrototypeOf(fn) === callee.Function.prototype, 'function realm');
              if (attribute === 'toJSON') {
                assert(descriptor.writable && fn.name === 'toJSON', 'operation descriptor');
                const result = fn.call(value);
                assert(Object.getPrototypeOf(result) === callee.Object.prototype, 'JSON allocated in binding realm');
                assert(Object.keys(result).sort().join() === attributes.slice().sort().join(), 'complete JSON attributes');
                for (const key of attributes) {
                  const d = Object.getOwnPropertyDescriptor(result, key);
                  assert(d.enumerable && d.configurable && d.writable && d.value === initial[key], 'JSON own data values');
                }
                result[attributes[0]] = -1;
                assert(value[attributes[0]] === initial[attributes[0]], 'JSON is an independent snapshot');
              } else {
                assert(descriptor.set === undefined && fn.name === `get ${attribute}`, 'readonly accessor descriptor');
                assert(fn.call(value) === initial[attribute], 'genuine foreign receiver');
              }
            });
            let calls = 0;
            const poison = () => { calls++; throw new Error('author trap'); };
            const revoked = Proxy.revocable(value, {}); revoked.revoke();
            const invalid = [null, undefined, {}, calleePrototype, Object.create(value),
              Object.create(prototype, Object.getOwnPropertyDescriptors(value)),
              new Proxy(value, {get: poison, getPrototypeOf: poison, has: poison}), revoked.proxy,
              performance[member === 'timing' ? 'navigation' : 'timing']];
            invalid.forEach((receiver, index) => check(`${label} rejects receiver ${index}`, () => {
              const fn = method();
              assert(typeof fn === 'function', 'prototype binding exists');
              calls = 0;
              let error; try { fn.call(receiver); } catch (caught) { error = caught; }
              assert(error instanceof callee.TypeError, 'TypeError belongs to binding realm');
              assert(calls === 0, 'receiver validation executes no author code');
            }));
            check(`${label} reads native data after shadowing and prototype mutation`, () => {
              const fn = method();
              assert(typeof fn === 'function', 'prototype binding exists');
              const saved = Object.getOwnPropertyDescriptors(value);
              try {
                for (const key of attributes) Object.defineProperty(value, key, {get: poison, configurable: true});
                Object.setPrototypeOf(value, null);
                calls = 0;
                const result = fn.call(value);
                if (attribute === 'toJSON') {
                  for (const key of attributes) assert(result[key] === initial[key], 'native JSON value');
                } else assert(result === initial[attribute], 'native attribute value');
                assert(calls === 0, 'public properties must not participate');
              } finally {
                Object.setPrototypeOf(value, prototype);
                for (const key of attributes) Reflect.deleteProperty(value, key);
                Object.defineProperties(value, saved);
              }
            });
          }
        }
      }
      for (const [calleeName, callee] of Object.entries(realms)) {
        check(`${ownerName} measure via ${calleeName} uses native legacy timestamps`, () => {
          const timing = performance.timing, saved = Object.getOwnPropertyDescriptors(timing);
          let reads = 0;
          try {
            for (const key of ['navigationStart', 'unloadEventStart']) Object.defineProperty(timing, key, {
              configurable: true, get() { reads++; return key === 'navigationStart' ? 123 : 456; }
            });
            const measure = callee.Performance.prototype.measure.call(performance, 'native-legacy-time', 'navigationStart', 'navigationStart');
            assert(measure.startTime === 0 && measure.duration === 0, 'navigationStart relative to itself');
            let error;
            try { callee.Performance.prototype.measure.call(performance, 'unavailable-time', 'unloadEventStart'); }
            catch (caught) { error = caught; }
            assert(error?.name === 'InvalidAccessError', 'unavailable native timestamp stays unavailable');
            assert(reads === 0, 'measure ignores author accessors');
          } finally {
            for (const key of ['navigationStart', 'unloadEventStart']) Reflect.deleteProperty(timing, key);
            Object.defineProperties(timing, saved);
            performance.clearMeasures();
          }
        });
        check(`${ownerName} Performance.toJSON via ${calleeName} ignores shadowing`, () => {
          const timing = performance.timing, navigation = performance.navigation;
          const expected = {timing: timing.toJSON(), navigation: navigation.toJSON(), timeOrigin: performance.timeOrigin};
          const objects = [[performance, ['timeOrigin', 'timing', 'navigation']], [timing, timingNames], [navigation, ['type', 'redirectCount']]];
          const saved = objects.map(([object]) => Object.getOwnPropertyDescriptors(object));
          let reads = 0;
          try {
            for (const [object, names] of objects) for (const name of names) Object.defineProperty(object, name, {
              configurable: true, get() { reads++; throw new Error('public timing getter'); }
            });
            const result = callee.Performance.prototype.toJSON.call(performance);
            assert(Object.getPrototypeOf(result) === callee.Object.prototype, 'outer JSON realm');
            assert(result.timeOrigin === expected.timeOrigin, 'native timeOrigin');
            assert(result.timing === timing && result.navigation === navigation, 'IDL object-valued attributes retain identity');
            const serialized = JSON.parse(JSON.stringify(result));
            for (const key of ['timing', 'navigation']) {
              for (const name of Object.keys(expected[key])) assert(serialized[key][name] === expected[key][name], 'native nested JSON value');
            }
            assert(reads === 0, 'serialization executes no author getters');
          } finally {
            objects.forEach(([object, names], index) => {
              for (const name of names) Reflect.deleteProperty(object, name);
              Object.defineProperties(object, saved[index]);
            });
          }
        });
      }
    }
  } finally { frame.remove(); }

  const lifecycleFrame = document.createElement('iframe');
  try {
    const loaded = new Promise(resolve => { lifecycleFrame.onload = resolve; });
    lifecycleFrame.srcdoc = `<script>
      globalThis.retainedTiming = performance.timing;
      globalThis.snapshots = [];
      globalThis.shadowReads = 0;
      const toJSON = retainedTiming.toJSON;
      const snapshot = phase => snapshots.push({phase, value: toJSON.call(retainedTiming)});
      snapshot('initial');
      Object.defineProperty(retainedTiming, 'loadEventEnd', {get() { shadowReads++; return -1; }});
      Object.freeze(retainedTiming);
      document.addEventListener('DOMContentLoaded', () => snapshot('dcl'));
      addEventListener('load', () => snapshot('load'));
    <\/script>`;
    document.body.appendChild(lifecycleFrame);
    await loaded;
    await new Promise(resolve => setTimeout(resolve, 0));
    check('frozen materialized timing receives lifecycle updates without overwriting author properties', () => {
      const realm = lifecycleFrame.contentWindow;
      const snapshots = realm.snapshots;
      assert(snapshots.length === 3, 'initial, DOMContentLoaded and load observed');
      assert(snapshots[0].value.domContentLoadedEventStart === 0 && snapshots[0].value.loadEventEnd === 0, 'initial pending timestamps');
      assert(snapshots[1].value.domContentLoadedEventStart > 0 && snapshots[1].value.domContentLoadedEventEnd === 0, 'DOMContentLoaded in progress');
      assert(snapshots[2].value.domContentLoadedEventEnd > 0 && snapshots[2].value.loadEventStart > 0 && snapshots[2].value.loadEventEnd === 0, 'load in progress');
      const value = realm.performance.timing;
      assert(value === realm.retainedTiming && Object.isFrozen(value), 'retained frozen object');
      const final = realm.PerformanceTiming.prototype.toJSON.call(value);
      assert(final.loadEventEnd >= final.loadEventStart && final.loadEventEnd > 0, 'native end timestamp updated');
      const getter = Object.getOwnPropertyDescriptor(realm.PerformanceTiming.prototype, 'loadEventEnd').get;
      assert(getter.call(value) === final.loadEventEnd, 'getter reads updated state');
      assert(realm.shadowReads === 0, 'lifecycle and JSON bypass author getter');
      assert(value.loadEventEnd === -1 && realm.shadowReads === 1, 'author property remains intact');
    });
  } finally { lifecycleFrame.remove(); }
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
