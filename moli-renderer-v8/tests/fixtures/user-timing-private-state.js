(owner = globalThis) => {
  const native = owner.performance;
  const constructor = owner.PerformanceMark;
  const key = '__moliWindowPerformance';
  const previous = Object.getOwnPropertyDescriptor(owner, key);
  const checks = [];
  const check = (name, passed, details = '') => checks.push({name, passed: !!passed, details});
  let reads = 0;
  try {
    Object.defineProperty(owner, key, {configurable: true, writable: true, value: {}});
    check('native-performance-after-expando', owner.performance === native);
    try {
      const entry = new constructor('__private_clock_object');
      check('constructor-after-expando', entry instanceof owner.PerformanceMark
        && Number.isFinite(entry.startTime));
    } catch (error) {
      check('constructor-after-expando', false, String(error));
    }
    Object.defineProperty(owner, key, {configurable: true, get() {
      reads++;
      throw new Error('author expando getter');
    }});
    try {
      const entry = new constructor('__private_clock_getter');
      check('constructor-after-expando-getter', entry instanceof owner.PerformanceMark
        && Number.isFinite(entry.startTime));
    } catch (error) {
      check('constructor-after-expando-getter', false, String(error));
    }
    check('no-author-expando-getter', reads === 0, String(reads));
  } finally {
    if (previous) Object.defineProperty(owner, key, previous);
    else delete owner[key];
  }
  return checks;
}
