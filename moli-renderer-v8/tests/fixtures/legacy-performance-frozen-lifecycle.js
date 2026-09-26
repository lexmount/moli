(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const retainedTiming = performance.timing;
  const snapshots = [];
  let shadowReads = 0;
  const toJSON = retainedTiming.toJSON;
  const snapshot = phase => snapshots.push({phase, value: toJSON.call(retainedTiming)});
  snapshot('initial');
  Object.defineProperty(retainedTiming, 'loadEventEnd', {get() { shadowReads++; return -1; }});
  Object.freeze(retainedTiming);
  document.addEventListener('DOMContentLoaded', () => snapshot('dcl'));
  addEventListener('load', () => snapshot('load'));
  return () => {
    assert(snapshots.map(snapshot => snapshot.phase).join() === 'initial,dcl,load', 'lifecycle phases observed');
    assert(snapshots[0].value.domContentLoadedEventStart === 0 && snapshots[0].value.loadEventEnd === 0, 'initial pending timestamps');
    assert(snapshots[1].value.domContentLoadedEventStart > 0 && snapshots[1].value.domContentLoadedEventEnd === 0, 'DOMContentLoaded in progress');
    assert(snapshots[2].value.domContentLoadedEventEnd > 0 && snapshots[2].value.loadEventStart > 0 && snapshots[2].value.loadEventEnd === 0, 'load in progress');
    const value = performance.timing;
    assert(value === retainedTiming && Object.isFrozen(value), 'retained frozen object');
    const final = PerformanceTiming.prototype.toJSON.call(value);
    assert(final.loadEventEnd >= final.loadEventStart && final.loadEventEnd > 0, 'native end timestamp updated');
    const getter = Object.getOwnPropertyDescriptor(PerformanceTiming.prototype, 'loadEventEnd').get;
    assert(getter.call(value) === final.loadEventEnd, 'getter reads updated state');
    assert(shadowReads === 0, 'lifecycle and JSON bypass author getter');
    assert(value.loadEventEnd === -1 && shadowReads === 1, 'author property remains intact');
    return true;
  };
})();
