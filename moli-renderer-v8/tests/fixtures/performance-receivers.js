globalThis.__runPerformanceReceiverChecks = () => {
  const rows = [];
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const check = (name, test) => {
    try { test(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const frame = typeof document === 'undefined' ? null :
    document.body.appendChild(document.createElement('iframe'));
  const realms = [['main', globalThis]];
  if (frame) realms.push(['child', frame.contentWindow]);
  let reads = 0;
  const poison = () => { reads++; throw new Error('author code ran'); };
  const text = {toString: poison};
  const number = {valueOf: poison};
  const methods = [
    ['now', [], []], ['toJSON', [], []],
    ['mark', [text, {get startTime() { return poison(); }}], ['receiver-mark', {startTime: 2}]],
    ['clearMarks', [text], ['receiver-mark']],
    ['measure', [text, {get start() { return poison(); }}], ['receiver-measure', {start: 1, end: 3}]],
    ['clearMeasures', [text], ['receiver-measure']],
    ['getEntries', [], []], ['getEntriesByType', [text], ['mark']],
    ['getEntriesByName', [text, text], ['receiver-mark', 'mark']],
    ['clearResourceTimings', [], []], ['setResourceTimingBufferSize', [number], [250]],
  ];
  const attributes = ['timeOrigin', 'onresourcetimingbufferfull'];
  if (frame) attributes.push('timing', 'navigation', 'eventCounts', 'memory');
  const invalidReceivers = realm => {
    const real = realm.performance, prototype = realm.Performance.prototype;
    const revoked = Proxy.revocable(real, {}); revoked.revoke();
    const getters = Object.fromEntries(['timeOrigin', 'timing', 'navigation'].map(name =>
      [name, {get: poison, configurable: true}]));
    return [
      ['null', null], ['undefined', undefined], ['plain', {}], ['prototype', prototype],
      ['inherited', Object.create(real)],
      ['copied', Object.create(prototype, Object.getOwnPropertyDescriptors(real))],
      ['author Proxy', new Proxy(real, {get: poison, getPrototypeOf: poison, has: poison})],
      ['revoked Proxy', revoked.proxy], ['other interface', new realm.EventTarget()],
      ['forged getters', Object.create(prototype, getters)],
    ];
  };
  const rejects = (realm, call) => {
    reads = 0;
    let error;
    try { call(); } catch (caught) { error = caught; }
    assert(error instanceof realm.TypeError, 'must throw the binding realm TypeError');
    assert(reads === 0, 'must validate before author getters, conversion, or Proxy traps');
  };
  try {
    for (const [calleeName, realm] of realms) {
      const prototype = realm.Performance.prototype;
      const invalid = invalidReceivers(realm);
      for (const [name, poisoned, valid] of methods) {
        const fn = prototype[name];
        for (const [kind, receiver] of invalid) {
          check(`${calleeName} ${name} rejects ${kind}`, () => rejects(realm, () => fn.apply(receiver, poisoned)));
        }
        for (const [owner, window] of realms) {
          check(`${calleeName} ${name} accepts ${owner} Performance`, () => {
            const result = fn.apply(window.performance, valid);
            if (name === 'now') assert(Number.isFinite(result) && result >= 0, 'finite relative time');
            else if (name === 'toJSON') assert(result.timeOrigin === window.performance.timeOrigin, 'receiver time origin');
            else if (name.startsWith('getEntries')) assert(Array.isArray(result), 'sequence result is an Array');
            else if (name === 'mark' || name === 'measure') assert(result.name === valid[0], 'created entry name');
            else assert(result === undefined, 'void result');
          });
        }
      }
      for (const name of attributes) {
        const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
        for (const [kind, receiver] of invalid) {
          check(`${calleeName} ${name} getter rejects ${kind}`, () => rejects(realm, () => descriptor.get.call(receiver)));
          if (descriptor.set) check(`${calleeName} ${name} setter rejects ${kind}`, () =>
            rejects(realm, () => descriptor.set.call(receiver, poison)));
        }
        for (const [owner, window] of realms) {
          check(`${calleeName} ${name} getter accepts ${owner} Performance`, () => {
            const result = descriptor.get.call(window.performance);
            if (name === 'memory') assert(typeof result.usedJSHeapSize === 'number', 'memory snapshot');
            else assert(result === window.performance[name], 'receiver attribute identity');
          });
          if (descriptor.set) check(`${calleeName} ${name} setter updates ${owner} Performance`, () => {
            const receiver = window.performance, saved = receiver[name];
            const handler = () => {};
            try {
              descriptor.set.call(receiver, handler);
              assert(receiver[name] === handler, 'receiver handler identity');
            } finally { descriptor.set.call(receiver, saved); }
          });
        }
      }
      for (const [owner, window] of realms) {
        check(`${calleeName} borrowed timing operations keep ${owner} entries isolated`, () => {
          const receiver = window.performance;
          const name = `receiver-isolation-${calleeName}-${owner}`;
          try {
            const mark = prototype.mark.call(receiver, name, {startTime: 10});
            const measure = prototype.measure.call(receiver, name, {start: name, end: 20});
            const entries = prototype.getEntriesByName.call(receiver, name);
            assert(entries.length === 2 && entries[0] === mark && entries[1] === measure, 'receiver owns entries');
            assert(measure.startTime === 10 && measure.duration === 10, 'measure resolves receiver marks');
            for (const [, other] of realms) if (other !== window)
              assert(other.performance.getEntriesByName(name).length === 0, 'other timeline remains separate');
            prototype.clearMarks.call(receiver, name);
            assert(receiver.getEntriesByName(name).length === 1, 'clearMarks affects receiver');
            prototype.clearMeasures.call(receiver, name);
            assert(receiver.getEntriesByName(name).length === 0, 'clearMeasures affects receiver');
          } finally { receiver.clearMarks(name); receiver.clearMeasures(name); }
        });
      }
    }
  } finally {
    for (const [, realm] of realms) {
      realm.performance.clearMarks('receiver-mark');
      realm.performance.clearMeasures('receiver-measure');
    }
    if (frame) frame.remove();
  }
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
};
__runPerformanceReceiverChecks();
