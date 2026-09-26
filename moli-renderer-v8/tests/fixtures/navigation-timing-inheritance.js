(async () => {
  const rows = [];
  const mainNavigation = performance.getEntriesByType('navigation')[0];
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const check = (name, run) => {
    try { run(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const baseNames = ['name', 'entryType', 'startTime', 'duration'];
  const resourceNames = ['initiatorType', 'nextHopProtocol', 'workerStart', 'redirectStart', 'redirectEnd',
    'fetchStart', 'domainLookupStart', 'domainLookupEnd', 'connectStart', 'connectEnd', 'secureConnectionStart',
    'requestStart', 'responseStart', 'responseEnd', 'transferSize', 'encodedBodySize', 'decodedBodySize',
    'renderBlockingStatus', 'responseStatus'];
  // contentType and the new entry identity fields are covered by the Rust
  // regression: the Chromium reference does not expose them yet.
  const navigationNames = ['unloadEventStart', 'unloadEventEnd', 'domInteractive', 'domContentLoadedEventStart',
    'domContentLoadedEventEnd', 'domComplete', 'loadEventStart', 'loadEventEnd', 'type', 'redirectCount'];
  const allNames = [...baseNames, ...resourceNames, ...navigationNames];
  let observer;
  const resourceReady = new Promise(resolve => {
    observer = new PerformanceObserver(list => {
      const resource = list.getEntriesByName(__navigationResourceUrl, 'resource')[0];
      if (resource) { observer.disconnect(); resolve(resource); }
    });
    observer.observe({type: 'resource'});
  });
  let resource;
  try {
    await new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('GET', __navigationResourceUrl);
      xhr.onload = () => {
        if (xhr.status === 200 && xhr.responseText === 'resource') resolve();
        else reject(new Error('resource response body'));
      };
      xhr.onerror = () => reject(new Error('resource request failed'));
      xhr.send();
    });
    resource = await resourceReady;
  } finally { observer.disconnect(); }
  const frame = document.body.appendChild(document.createElement('iframe'));
  const realms = {main: window, child: frame.contentWindow};
  const retained = [];
  try {
    for (const [ownerName, owner] of Object.entries(realms)) {
      const navigation = owner.performance.getEntriesByType('navigation')[0];
      assert(navigation, 'navigation entry exists');
      const initial = Object.fromEntries(allNames.map(name => [name, navigation[name]]));
      retained.push({navigation, owner, initial});
      check(`${ownerName} native interface inheritance`, () => {
        assert(Object.getPrototypeOf(owner.PerformanceNavigationTiming) === owner.PerformanceResourceTiming, 'constructor parent');
        assert(Object.getPrototypeOf(owner.PerformanceNavigationTiming.prototype) === owner.PerformanceResourceTiming.prototype, 'prototype parent');
        assert(navigation instanceof owner.PerformanceNavigationTiming && navigation instanceof owner.PerformanceResourceTiming && navigation instanceof owner.PerformanceEntry, 'instance chain');
        assert(Object.getOwnPropertyNames(navigation).length === 0, 'no own IDL fields or methods');
        for (const name of resourceNames) assert(!Object.hasOwn(owner.PerformanceNavigationTiming.prototype, name), 'resource members are inherited');
        assert(Object.prototype.toString.call(navigation) === '[object PerformanceNavigationTiming]', 'most-derived interface tag');
      });
      for (const name of allNames) check(`${ownerName} ${name} readonly native value`, () => {
        const value = navigation[name];
        assert(value !== undefined, 'complete initialized attribute');
        const written = Reflect.set(navigation, name, null);
        if (written) Reflect.set(navigation, name, value);
        assert(!written && navigation[name] === value, 'readonly attribute');
      });
      for (const [calleeName, callee] of Object.entries(realms)) {
        for (const [iface, names] of [
          ['PerformanceEntry', baseNames],
          ['PerformanceResourceTiming', resourceNames],
          ['PerformanceNavigationTiming', navigationNames],
        ]) {
          const prototype = callee[iface].prototype;
          const methods = names.map(name => [name, () => Object.getOwnPropertyDescriptor(prototype, name)?.get]);
          methods.push(['toJSON', () => prototype.toJSON]);
          for (const [name, method] of methods) {
            const label = `${ownerName} ${iface}.${name} via ${calleeName}`;
            check(`${label} genuine navigation receiver`, () => {
              const descriptor = Object.getOwnPropertyDescriptor(prototype, name), fn = method();
              assert(descriptor?.enumerable && descriptor.configurable, 'enumerable configurable prototype binding');
              assert(typeof fn === 'function' && fn.length === 0, 'zero-argument native binding');
              if (name === 'toJSON') {
                assert(fn.name === 'toJSON' && descriptor.writable, 'operation descriptor');
                const json = fn.call(navigation);
                assert(Object.getPrototypeOf(json) === callee.Object.prototype, 'result belongs to binding realm');
                const expected = iface === 'PerformanceEntry' ? baseNames : iface === 'PerformanceResourceTiming' ? [...baseNames, ...resourceNames] : allNames;
                for (const key of expected) {
                  const data = Object.getOwnPropertyDescriptor(json, key);
                  assert(data?.writable && data.enumerable && data.configurable && data.value === initial[key], `native JSON ${key}`);
                }
                // The interface-specific JSON key set is checked separately
                // against Web IDL; Chromium includes derived attributes here.
                json.name = 'changed';
                assert(navigation.name === initial.name, 'snapshot is independent');
              } else {
                assert(descriptor.set === undefined && fn.name === `get ${name}`, 'readonly getter signature');
                assert(fn.call(navigation) === initial[name], 'borrowed getter sees canonical native data');
              }
            });
            let calls = 0;
            const poison = () => { calls++; throw new Error('author trap ran'); };
            const revoked = Proxy.revocable(navigation, {}); revoked.revoke();
            const invalid = [null, undefined, {}, prototype, Object.create(navigation),
              new Proxy(navigation, {get: poison, getPrototypeOf: poison, has: poison}), revoked.proxy];
            if (iface === 'PerformanceNavigationTiming') invalid.push(resource, new owner.PerformanceMark('wrong-subtype'));
            else if (iface === 'PerformanceResourceTiming') invalid.push(new owner.PerformanceMark('wrong-subtype'));
            invalid.forEach((receiver, index) => check(`${label} rejects receiver ${index}`, () => {
              const fn = method();
              assert(typeof fn === 'function', 'prototype binding exists');
              calls = 0;
              let error; try { fn.call(receiver); } catch (caught) { error = caught; }
              assert(error instanceof callee.TypeError && calls === 0, 'binding-realm TypeError without author traps');
            }));
            check(`${label} survives expando and prototype tampering`, () => {
              const fn = method();
              assert(typeof fn === 'function', 'prototype binding exists');
              const saved = Object.getOwnPropertyDescriptors(navigation), originalPrototype = Object.getPrototypeOf(navigation);
              try {
                for (const key of allNames) Object.defineProperty(navigation, key, {get: poison, configurable: true});
                Object.setPrototypeOf(navigation, null); calls = 0;
                const result = fn.call(navigation);
                if (name === 'toJSON') {
                  const expected = iface === 'PerformanceEntry' ? baseNames : iface === 'PerformanceResourceTiming' ? [...baseNames, ...resourceNames] : allNames;
                  for (const key of expected) assert(result[key] === initial[key], `serialized native ${key}`);
                } else assert(result === initial[name], 'native getter state');
                assert(calls === 0, 'public accessors are ignored');
              } finally {
                Object.setPrototypeOf(navigation, originalPrototype);
                for (const key of allNames) Reflect.deleteProperty(navigation, key);
                Object.defineProperties(navigation, saved);
              }
            });
          }
        }
        check(`${ownerName} resource bindings via ${calleeName} still accept resource entries`, () => {
          for (const name of resourceNames) {
            const getter = Object.getOwnPropertyDescriptor(callee.PerformanceResourceTiming.prototype, name).get;
            assert(getter.call(resource) === resource[name], name);
          }
          const json = callee.PerformanceResourceTiming.prototype.toJSON.call(resource);
          assert(json.name === __navigationResourceUrl && json.initiatorType === 'xmlhttprequest', 'real loaded resource');
          assert(!Object.hasOwn(json, 'loadEventEnd'), 'resource remains separate from navigation');
        });
      }
      check(`${ownerName} navigation entry is shared by timeline and buffered observer`, () => {
        const observer = new owner.PerformanceObserver(() => {});
        try {
          observer.observe({type: 'navigation', buffered: true});
          const records = observer.takeRecords();
          assert(records.length === 1 && records[0] === navigation, 'buffered entry identity');
          assert(owner.performance.getEntriesByName(navigation.name, 'navigation')[0] === navigation, 'timeline identity');
          if (owner === window) assert(navigation === mainNavigation, 'navigation identity survives resource loading');
        } finally { observer.disconnect(); }
      });
    }
  } finally { frame.remove(); }
  for (const {navigation, owner, initial} of retained) check('retained entry keeps native inheritance state after frame removal', () => {
    const json = owner.PerformanceNavigationTiming.prototype.toJSON.call(navigation);
    assert(json.name === initial.name && json.type === initial.type && json.loadEventEnd === initial.loadEventEnd, 'retained navigation state');
    assert(owner.PerformanceResourceTiming.prototype.toJSON.call(navigation).initiatorType === 'navigation', 'retained resource state');
  });
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
