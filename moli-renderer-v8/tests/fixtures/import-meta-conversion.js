async function() {
  const result = {checks: 0, failures: []};
  const check = (name, pass) => {
    result.checks++;
    if (!pass) result.failures.push(name);
  };
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const moduleURL = 'data:text/javascript,export const resolve = import.meta.resolve;';
  const local = (await import(moduleURL)).resolve;
  const frame = document.createElement('iframe');
  const loaded = new Promise(resolve => { frame.onload = resolve; });
  frame.srcdoc = '<!doctype html><body>child';
  body.appendChild(frame);
  await loaded;
  const child = frame.contentWindow;
  const foreign = (await child.eval('import(' + JSON.stringify(moduleURL) + ')')).resolve;
  try {
    for (const [realm, resolve, NativeTypeError] of [
      ['parent', local, TypeError],
      ['child', foreign, child.TypeError],
    ]) {
      const markers = [undefined, null, Symbol('thrown'), {}, new RangeError('sentinel')];
      for (const [index, marker] of markers.entries()) {
        for (const mode of ['primitive-getter', 'primitive-method', 'toString', 'valueOf', 'proxy']) {
          const name = realm + '/' + mode + '/' + index;
          let reads = 0;
          const fail = () => { reads++; throw marker; };
          let input;
          if (mode === 'primitive-getter') input = {get [Symbol.toPrimitive]() { return fail(); }};
          if (mode === 'primitive-method') input = {[Symbol.toPrimitive](hint) { check(name + '/hint', hint === 'string'); return fail(); }};
          if (mode === 'toString') input = {toString: fail, valueOf() { reads += 10; return 'unexpected'; }};
          if (mode === 'valueOf') input = {toString() { reads++; return {}; }, valueOf: fail};
          if (mode === 'proxy') input = new Proxy({}, {get: fail});
          let threw = false;
          try { resolve(input); }
          catch (error) {
            threw = true;
            check(name + '/identity', Object.is(error, marker));
          }
          check(name + '/threw', threw);
          check(name + '/reads', reads === (mode === 'valueOf' ? 2 : 1));
        }
      }
      for (const input of [Symbol('input'), {toString() { return Symbol('result'); }}]) {
        let threw = false;
        try { resolve(input); }
        catch (error) {
          threw = true;
          check(realm + '/native TypeError realm', Object.getPrototypeOf(error) === NativeTypeError.prototype);
        }
        check(realm + '/Symbol throws', threw);
      }
      check(realm + '/recovery', resolve('https://example.test/ok.js') === 'https://example.test/ok.js');
      let conversions = 0;
      check(realm + '/successful coercion', resolve({[Symbol.toPrimitive](hint) {
        conversions++;
        check(realm + '/successful hint', hint === 'string');
        return 'https://example.test/converted.js';
      }}) === 'https://example.test/converted.js');
      check(realm + '/successful conversion once', conversions === 1);
    }
    result.completed = true;
    return result;
  } finally {
    frame.remove();
  }
}
