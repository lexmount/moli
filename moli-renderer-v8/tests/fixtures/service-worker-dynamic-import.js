async function serviceWorkerDynamicImportProbe() {
  const result = {checks: 0, failures: []};
  const check = (name, actual, expected) => {
    result.checks++;
    if (actual !== expected) result.failures.push({name, actual, expected});
  };
  const NativeTypeError = TypeError;
  // The restriction belongs to the native global, not its exposed constructor.
  self.ServiceWorkerGlobalScope = undefined;
  check('static script executed once', self.staticScriptRuns, 1);
  const actions = [
    ['cached script', () => import('./cached.js')],
    ['uncached script', () => import('./unfetched.js')],
    ['JSON', () => import('./unfetched.json', {with: {type: 'json'}})],
    ['CSS', () => import('./unfetched.css', {with: {type: 'css'}})],
    ['data script', () => import('data:text/javascript,self.dynamicScriptRan=true;export default 1')],
    ['data JSON', () => import('data:application/json,%7B%22answer%22%3A42%7D', {with: {type: 'json'}})],
    ['source import', () => import.source('./unfetched.wasm')],
    ['data source import', () => import.source('data:application/wasm;base64,AGFzbQEAAAA=')],
    ['Function', () => Function('return import("./function.js")')()],
    ['eval', () => eval('import("./eval.js")')],
  ];
  for (const [label, action] of actions) {
    let synchronous = true;
    try {
      const promise = action();
      check(label + ': Promise', promise instanceof Promise, true);
      const settled = promise.then(
        () => { result.failures.push({name: label + ': import fulfilled'}); },
        error => {
          check(label + ': asynchronous rejection', synchronous, false);
          check(label + ': intrinsic TypeError', Object.getPrototypeOf(error), NativeTypeError.prototype);
        }
      );
      synchronous = false;
      await settled;
    } catch (error) {
      result.failures.push({name: label + ': synchronous exception', error: String(error)});
    }
  }
  check('dynamic script never executed', self.dynamicScriptRan, undefined);
  check('cached script not reexecuted', self.staticScriptRuns, 1);

  const sentinel = {};
  let specifierConversions = 0;
  await import({toString() { specifierConversions++; throw sentinel; }}).then(
    () => result.failures.push({name: 'specifier conversion fulfilled'}),
    error => check('specifier exception preserved', error, sentinel)
  );
  check('specifier converted once', specifierConversions, 1);
  let optionReads = 0;
  await import('./unfetched.js', {get with() { optionReads++; throw sentinel; }}).then(
    () => result.failures.push({name: 'options conversion fulfilled'}),
    error => check('options exception preserved', error, sentinel)
  );
  check('options read once', optionReads, 1);
  return result;
}
