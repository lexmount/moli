(async (parent, child) => {
  const result = { received: { parent: [], child: [] }, failures: [] };
  const check = (name, condition) => {
    if (!condition) result.failures.push(name);
  };
  let acknowledge;
  let delivered = 0;
  const delivery = new Promise(resolve => acknowledge = resolve);
  const observers = [parent, child].map((owner, index) => {
    const label = index ? 'child' : 'parent';
    const accept = (list, localList) => {
      check(label + '/delivery-list-realm', localList);
      result.received[label].push(...list.getEntries().map(entry => entry.name).sort());
      if (++delivered === 2) acknowledge();
    };
    const callback = owner.Function('accept', `return function(list) {
      accept(list, list instanceof PerformanceObserverEntryList);
    };`)(accept);
    const observer = new owner.PerformanceObserver(callback);
    observer.observe({ entryTypes: ['mark', 'measure'] });
    return observer;
  });
  for (const [callee, owner, label] of [[parent, child, 'child'], [child, parent, 'parent']]) {
    const markMethod = callee.Performance.prototype.mark;
    const measureMethod = callee.Performance.prototype.measure;
    const detail = { nested: [] };
    const mark = markMethod.call(owner.performance, label + '-mark', { startTime: 10, detail });
    const measure = measureMethod.call(owner.performance, label + '-measure', {
      start: label + '-mark', end: 20, detail
    });
    for (const [kind, entry] of [['mark', mark], ['measure', measure]]) {
      const type = kind === 'mark' ? 'PerformanceMark' : 'PerformanceMeasure';
      check(label + '/' + kind + '/prototype', Object.getPrototypeOf(entry) === owner[type].prototype);
      check(label + '/' + kind + '/detail-realm', entry.detail instanceof owner.Object);
      check(label + '/' + kind + '/nested-realm', entry.detail.nested instanceof owner.Array);
      check(label + '/' + kind + '/cloned-detail', entry.detail !== detail && entry.detail.nested !== detail.nested);
      check(label + '/' + kind + '/buffer-identity', owner.performance.getEntriesByName(entry.name)[0] === entry);
    }
    check(label + '/measure-boundaries', measure.startTime === 10 && measure.duration === 10);
    for (const [kind, name, invoke] of [
      ['negative-mark', 'TypeError', () => markMethod.call(owner.performance, 'negative', { startTime: -1 })],
      ['nonfinite-mark', 'TypeError', () => markMethod.call(owner.performance, 'nonfinite', { startTime: NaN })],
      ['invalid-measure', 'TypeError', () => measureMethod.call(owner.performance, 'invalid', { duration: 1 })],
      ['clone-mark', 'DataCloneError', () => markMethod.call(owner.performance, 'clone', { detail: () => {} })],
      ['clone-measure', 'DataCloneError', () => measureMethod.call(owner.performance, 'clone', { start: 0, end: 1, detail: () => {} })],
    ]) {
      let error;
      try { invoke(); } catch (caught) { error = caught; }
      check(label + '/' + kind + '/error-realm', error && error.name === name &&
            error instanceof callee[name === 'TypeError' ? 'TypeError' : 'DOMException']);
    }
    const sentinel = new callee.Error('detail getter');
    let caught;
    try {
      markMethod.call(owner.performance, 'getter', { detail: { get value() { throw sentinel; } } });
    } catch (error) { caught = error; }
    check(label + '/detail-getter-exception-identity', caught === sentinel);
  }
  await delivery;
  for (const observer of observers) observer.disconnect();
  return result;
})
