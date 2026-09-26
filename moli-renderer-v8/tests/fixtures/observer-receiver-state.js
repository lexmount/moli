(() => {
  const rows = [];
  const check = (name, test) => {
    try { test(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const frame = document.body.appendChild(document.createElement('iframe'));
  const realms = [['main', window], ['child', frame.contentWindow]];
  const target = document.createElement('div');
  let reads = 0;
  const poison = () => { reads++; throw new Error('author code ran before receiver validation'); };
  const interfaces = [
    ['MutationObserver', ['observe', 'disconnect', 'takeRecords']],
    ['IntersectionObserver', ['observe', 'unobserve', 'disconnect', 'takeRecords']],
    ['ResizeObserver', ['observe', 'unobserve', 'disconnect']],
    ['PerformanceObserver', ['observe', 'disconnect', 'takeRecords']],
  ];
  const observers = [];
  const argsFor = (name, method, observable) => {
    if (method === 'observe') {
      if (name === 'MutationObserver') return [target, observable ? {get childList() { return poison(); }} : {childList: true}];
      if (name === 'ResizeObserver') return [target, observable ? {get box() { return poison(); }} : {box: 'content-box'}];
      if (name === 'PerformanceObserver') return [observable ? {get entryTypes() { return poison(); }} : {entryTypes: []}];
      return [target];
    }
    return method === 'unobserve' ? [target] : [];
  };
  const invalidReceivers = (prototype, real) => {
    const revoked = Proxy.revocable(real, {});
    revoked.revoke();
    return [
      ['null', null], ['undefined', undefined], ['plain', {}], ['prototype', prototype],
      ['inherited native object', Object.create(real)],
      ['copied own properties', Object.create(prototype, Object.getOwnPropertyDescriptors(real))],
      ['author Proxy', new Proxy(real, {get: poison, getPrototypeOf: poison, has: poison})],
      ['revoked Proxy', revoked.proxy],
    ];
  };
  const rejects = (realm, call) => {
    reads = 0;
    let error;
    try { call(); } catch (caught) { error = caught; }
    assert(error instanceof realm.TypeError, 'must throw the binding realm TypeError');
    assert(reads === 0, 'receiver validation must precede author getters and Proxy traps');
  };
  try {
    for (const [name, methods] of interfaces) {
      const instances = realms.map(([label, realm]) => {
        const instance = new realm[name](() => {});
        observers.push(instance);
        return [label, instance];
      });
      for (const [calleeName, realm] of realms) {
        const prototype = realm[name].prototype;
        for (const method of methods) {
          const fn = prototype[method];
          for (const [receiverName, receiver] of invalidReceivers(prototype, instances[0][1])) {
            check(`${calleeName} ${name}.${method} rejects ${receiverName}`, () => {
              rejects(realm, () => fn.apply(receiver, argsFor(name, method, true)));
            });
          }
          for (const [receiverRealm, receiver] of instances) {
            check(`${calleeName} ${name}.${method} accepts ${receiverRealm} native receiver`, () => {
              fn.apply(receiver, argsFor(name, method, false));
            });
          }
        }
        for (const [, instance] of instances) instance.disconnect();
      }
    }

    const attributes = ['root', 'rootMargin', 'scrollMargin', 'thresholds', 'delay', 'trackVisibility'];
    const instances = realms.map(([label, realm]) => [label, new realm.IntersectionObserver(() => {}, {
      root: target, rootMargin: '1px 2%', scrollMargin: '3px', threshold: [1, 0.5, 0.5, 0], delay: 123,
    })]);
    observers.push(...instances.map(([, observer]) => observer));
    for (const [calleeName, realm] of realms) {
      for (const attribute of attributes) {
        const getter = Object.getOwnPropertyDescriptor(realm.IntersectionObserver.prototype, attribute).get;
        for (const [receiverName, receiver] of invalidReceivers(realm.IntersectionObserver.prototype, instances[0][1])) {
          check(`${calleeName} IntersectionObserver.${attribute} rejects ${receiverName}`, () => {
            rejects(realm, () => getter.call(receiver));
          });
        }
        for (const [receiverRealm, observer] of instances) {
          check(`${calleeName} IntersectionObserver.${attribute} accepts ${receiverRealm} native receiver`, () => {
            const actual = getter.call(observer);
            if (attribute === 'root') assert(actual === target, 'root identity');
            else if (attribute === 'thresholds') assert(JSON.stringify(actual) === '[0,0.5,0.5,1]', 'sorted thresholds with duplicates');
            else assert(actual === ({rootMargin: '1px 2% 1px 2%', scrollMargin: '3px 3px 3px 3px', delay: 123, trackVisibility: false})[attribute], attribute);
          });
        }
      }
    }
    for (const [label, observer] of instances) {
      check(`${label} thresholds are a frozen Array`, () => {
        const array = observer.thresholds;
        assert(Array.isArray(array) && Object.isFrozen(array), 'FrozenArray binding');
        assert(!Reflect.set(array, '0', 1), 'element mutation must fail');
        assert(!Reflect.set(array, 'length', 0), 'length mutation must fail');
        assert(!Reflect.defineProperty(array, 'extra', {value: 1}), 'extension must fail');
        assert(JSON.stringify(observer.thresholds) === '[0,0.5,0.5,1]', 'state must remain unchanged');
      });
      check(`${label} IntersectionObserver has no exposed internal state`, () => {
        assert(Reflect.ownKeys(observer).length === 0, 'observer own keys must not expose native slots');
      });
    }

    for (const [name] of interfaces.slice(0, 2)) {
      check(`${name} ignores author properties shadowing its native ID`, () => {
        const observer = new window[name](() => {});
        const slot = `__lm${name}Id`;
        const saved = Object.getOwnPropertyDescriptor(observer, slot);
        try {
          Object.defineProperty(observer, slot, {get: poison, configurable: true});
          reads = 0;
          observer.observe(target, {childList: true});
          observer.takeRecords();
          if (name === 'IntersectionObserver') observer.unobserve(target);
          observer.disconnect();
          assert(reads === 0, 'native identity must not read author properties');
        } finally {
          if (saved) Object.defineProperty(observer, slot, saved);
          else Reflect.deleteProperty(observer, slot);
          observer.disconnect();
        }
      });
    }
    check('MutationObserver native queues cannot be redirected between genuine instances', () => {
      const first = new MutationObserver(() => {}), second = new MutationObserver(() => {});
      const left = document.createElement('div'), right = document.createElement('div');
      const slot = '__lmMutationObserverId';
      const saved = Object.getOwnPropertyDescriptor(second, slot);
      try {
        first.observe(left, {childList: true}); second.observe(right, {childList: true});
        second[slot] = first[slot];
        left.appendChild(document.createElement('span'));
        right.appendChild(document.createElement('span'));
        const a = first.takeRecords(), b = second.takeRecords();
        assert(a.length === 1 && a[0].target === left, 'first observer queue');
        assert(b.length === 1 && b[0].target === right, 'second observer queue');
      } finally {
        if (saved) Object.defineProperty(second, slot, saved); else Reflect.deleteProperty(second, slot);
        first.disconnect(); second.disconnect();
      }
    });
    for (const suffix of ['Root', 'RootMargin', 'ScrollMargin', 'Thresholds', 'Delay', 'TrackVisibility']) {
      check(`IntersectionObserver metadata ignores author __lmIntersectionObserver${suffix}`, () => {
        const observer = new IntersectionObserver(() => {}, {root: target});
        const slot = `__lmIntersectionObserver${suffix}`;
        const saved = Object.getOwnPropertyDescriptor(observer, slot);
        try {
          Object.defineProperty(observer, slot, {get: poison, configurable: true});
          reads = 0;
          assert(observer.root === target, 'root');
          assert(observer.rootMargin === '0px 0px 0px 0px', 'rootMargin');
          assert(observer.scrollMargin === '0px 0px 0px 0px', 'scrollMargin');
          assert(JSON.stringify(observer.thresholds) === '[0]', 'thresholds');
          assert(observer.delay === 0 && observer.trackVisibility === false, 'visibility options');
          assert(reads === 0, 'metadata must not read author properties');
        } finally {
          if (saved) Object.defineProperty(observer, slot, saved); else Reflect.deleteProperty(observer, slot);
          observer.disconnect();
        }
      });
    }
  } finally {
    observers.forEach(observer => observer.disconnect());
    frame.remove();
  }
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
