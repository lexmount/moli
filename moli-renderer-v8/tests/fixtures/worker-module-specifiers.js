async function workerModuleSpecifierProbe(resolve) {
  const result = {checks: 0, failures: []};
  const check = (name, actual, expected) => {
    result.checks++;
    if (actual !== expected) result.failures.push({name, actual, expected});
  };
  const throws = (name, action, expected) => {
    try {
      action();
      result.failures.push({name: name + ': did not throw'});
    } catch (error) {
      check(name, expected === TypeError ? Object.getPrototypeOf(error) : error,
            expected === TypeError ? TypeError.prototype : expected);
    }
  };
  const first = await import('./cached.js');
  check('relative import exports', first.answer, 42);
  check('normalized relative import shares namespace', await import('././cached.js'), first);
  const invalid = [
    'cached.js', 'pkg/file.js', '.tomato', '..zucchini', '.\\yam.es', '',
    '#fragment', '?query', '.', '..', '\\root.js', ' ./cached.js',
    '%2e/cached.js', '@scope/pkg',
  ];
  const reject = async (name, action) => {
    let synchronous = true;
    const promise = action();
    check(name + ': Promise', promise instanceof Promise, true);
    const settled = promise.then(
      () => result.failures.push({name: name + ': fulfilled'}),
      error => {
        check(name + ': asynchronous', synchronous, false);
        check(name + ': TypeError', Object.getPrototypeOf(error), TypeError.prototype);
      }
    );
    synchronous = false;
    await settled;
  };
  for (const specifier of invalid) {
    if (resolve) throws('resolve ' + specifier, () => resolve(specifier), TypeError);
    await reject('import ' + specifier, () => import(specifier));
  }
  for (const specifier of ['cached.wasm', '#source']) {
    await reject('source import ' + specifier, () => import.source(specifier));
  }
  check('cached module only executed once', self.moduleRuns, 1);
  const marker = {};
  let conversions = 0;
  const throwingSpecifier = {toString() { conversions++; throw marker; }};
  await import(throwingSpecifier).then(
    () => result.failures.push({name: 'import conversion fulfilled'}),
    error => check('import preserves conversion exception', error, marker)
  );
  check('import converts once', conversions, 1);
  if (resolve) {
    for (const specifier of ['./cached.js', '../parent.js', '/root.js', '//example.test/mod.js']) {
      check('resolve relative ' + specifier, resolve(specifier), new URL(specifier, location.href).href);
    }
    for (const specifier of ['https://example.test/mod.js', 'http:short.test/mod.js', 'data:text/javascript,export default 42']) {
      check('resolve absolute ' + specifier, resolve(specifier), new URL(specifier).href);
    }
    throws('resolve missing argument', () => resolve(), TypeError);
    throws('resolve symbol', () => resolve(Symbol()), TypeError);
    throws('resolve preserves conversion exception', () => resolve(throwingSpecifier), marker);
    check('resolve converts once', conversions, 2);
  }
  result.completed = true;
  return result;
}
