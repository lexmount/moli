(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const afterRendering = () => new Promise(resolve =>
    requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(resolve, 0))));
  const withEntryList = async (owner, check) => {
    const frame = document.body.appendChild(document.createElement('iframe'));
    const other = frame.contentWindow;
    const realm = owner === 'child' ? other : window;
    let observer;
    try {
      const list = await new Promise(resolve => {
        observer = new realm.PerformanceObserver(resolve);
        observer.observe({entryTypes: ['mark']});
        realm.performance.mark('observer-late', {startTime: 20});
        realm.performance.mark('observer-early', {startTime: 10});
      });
      observer.disconnect();
      check(list, other);
    } finally {
      if (observer) observer.disconnect();
      realm.performance.clearMarks('observer-late');
      realm.performance.clearMarks('observer-early');
      frame.remove();
    }
  };
  const cases = [
    {
      name: 'IntersectionObserver IDs stay attached to their native receiver after author assignment',
      async run() {
        const left = document.body.appendChild(document.createElement('div'));
        const right = document.body.appendChild(document.createElement('div'));
        left.style.cssText = right.style.cssText = 'width:20px;height:20px';
        const firstEntries = [], secondEntries = [];
        const first = new IntersectionObserver((entries, observer) => {
          firstEntries.push(...entries.map(entry => entry.target === left && observer === first));
        });
        const second = new IntersectionObserver((entries, observer) => {
          secondEntries.push(...entries.map(entry => entry.target === right && observer === second));
        });
        const slot = '__lmIntersectionObserverId';
        const saved = Object.getOwnPropertyDescriptor(second, slot);
        try {
          second[slot] = first[slot];
          first.observe(left);
          second.observe(right);
          await afterRendering();
          assert(firstEntries.join(',') === 'true', 'first callback and entry identity');
          assert(secondEntries.join(',') === 'true', 'second callback and entry identity');
        } finally {
          if (saved) Object.defineProperty(second, slot, saved); else Reflect.deleteProperty(second, slot);
          first.disconnect(); second.disconnect(); left.remove(); right.remove();
        }
      }
    },
    ...['main', 'child'].flatMap(owner => [{
      name: `${owner} PerformanceObserverEntryList validates native receivers before conversion`,
      run() {
        return withEntryList(owner, (list, other) => {
          let reads = 0;
          const poison = () => { reads++; throw new Error('author conversion or Proxy trap'); };
          const revoked = Proxy.revocable(list, {}); revoked.revoke();
          for (const callee of [window, other]) {
            const prototype = callee.PerformanceObserverEntryList.prototype;
            for (const method of ['getEntries', 'getEntriesByType', 'getEntriesByName']) {
              const fn = prototype[method];
              const args = method === 'getEntries' ? [] : [{toString: poison}];
              for (const invalid of [null, undefined, {}, prototype, Object.create(list),
                  new Proxy(list, {get: poison, getPrototypeOf: poison}), revoked.proxy]) {
                reads = 0;
                let error;
                try { fn.apply(invalid, args); } catch (caught) { error = caught; }
                assert(error instanceof callee.TypeError, `${method} must reject in its binding realm`);
                assert(reads === 0, `${method} receiver validation must precede conversion`);
              }
              const input = method === 'getEntries' ? [] : [method === 'getEntriesByType' ? 'mark' : 'observer-early'];
              const entries = fn.apply(list, input);
              const expected = method === 'getEntriesByName' ? 'observer-early' : 'observer-early,observer-late';
              assert(entries.map(entry => entry.name).join(',') === expected, `${method} accepts cross-realm native list`);
            }
          }
        });
      }
    }, {
      name: `${owner} PerformanceObserverEntryList returns independent sorted sequences`,
      run() {
        return withEntryList(owner, list => {
          const expected = list.getEntriesByType('mark');
          const original = list.getEntries();
          assert(!Object.isFrozen(original), 'sequence result remains mutable');
          original.reverse(); original.pop(); original.push({name: 'author entry'});
          const again = list.getEntries();
          assert(again !== original, 'each sequence conversion produces a new Array');
          assert(again.map(entry => entry.name).join(',') === 'observer-early,observer-late', 'entries are sorted by startTime');
          assert(again.length === 2 && again.every((entry, i) => entry === expected[i]), 'list retains original entry identities and order');
          assert(list.getEntriesByType('mark').length === 2, 'mutating a returned Array cannot alter filtering');
          assert(list.getEntriesByName('observer-early')[0] === expected[0], 'getEntriesByName retains native snapshot');
        });
      }
    }])
  ];
  globalThis.__observerNativeState = {
    result: null,
    start(index) {
      this.result = null;
      cases[index].run().then(() => { this.result = 'pass'; }, error => { this.result = String(error); });
      return cases[index].name;
    },
    snapshot() { return {actual: this.result, expected: 'pass'}; }
  };
  return cases.length;
})();
