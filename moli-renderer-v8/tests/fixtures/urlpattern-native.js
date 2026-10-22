function checkURLPattern(realm) {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const Pattern = realm.URLPattern;
  const expectTypeError = callback => {
    try { callback(); } catch (error) {
      assert(error instanceof realm.TypeError, 'TypeError belongs to the binding realm');
      return;
    }
    throw new Error('expected TypeError');
  };
  assert(typeof Pattern === 'function' && Pattern.length === 0, 'native constructor metadata');
  assert(realm.Function.prototype.toString.call(Pattern).includes('[native code]'), 'native function');
  expectTypeError(() => Pattern());
  expectTypeError(() => new Pattern('/relative'));
  const pattern = new Pattern('https://*.example.test/users/:id');
  assert(pattern.protocol === 'https' && pattern.hostname === '*.example.test', 'canonical component strings');
  assert(pattern.pathname === '/users/:id' && !pattern.hasRegExpGroups, 'named group metadata');
  assert(pattern.test('https://api.example.test/users/42'), 'string match');
  assert(!pattern.test('https://api.example.test/other/42'), 'nonmatching path');
  const match = pattern.exec('/users/42', 'https://api.example.test/');
  assert(match.pathname.groups.id === '42' && match.hostname.groups[0] === 'api', 'component captures');
  assert(match.inputs.length === 2 && match.inputs[0] === '/users/42', 'converted string inputs retained');
  assert(Object.getPrototypeOf(match) === realm.Object.prototype, 'result realm');
  assert(Object.getPrototypeOf(match.pathname.groups) === realm.Object.prototype, 'group record realm');
  assert(Object.keys(match).join(',') === 'hash,hostname,inputs,password,pathname,port,protocol,search,username', 'result dictionary member order');
  assert(Object.keys(match.pathname).join(',') === 'groups,input', 'component dictionary member order');
  const optional = new Pattern({ pathname: '/:id?' });
  assert(Object.hasOwn(optional.exec({ pathname: '' }).pathname.groups, 'id'), 'unmatched group exists');
  assert(optional.exec({ pathname: '' }).pathname.groups.id === undefined, 'unmatched group is undefined');
  assert(optional.exec({ pathname: '/hello' }).pathname.groups.id === 'hello', 'matched optional group');
  const regex = new Pattern({ pathname: '/:id(\\d+)' });
  assert(regex.hasRegExpGroups && regex.test({ pathname: '/123' }), 'custom RegExp group');
  assert(!regex.test({ pathname: '/abc' }), 'custom RegExp rejects');
  const lookahead = new Pattern({ protocol: '(http(?=s)s)', pathname: '/:id' });
  assert(lookahead.test('https://example.test/yes'), 'ECMAScript lookahead also selects special-scheme parsing');
  assert(new Pattern('(http(?=s)s)://example.test/:id').test('https://example.test/yes'), 'constructor string uses the same native protocol matcher');
  assert(new Pattern('foo-bar://example.test/:id').test('foo-bar://example.test/yes'), 'ECMAScript escaping of literal protocol text');
  assert(new Pattern({ protocol: '((?:h?){1000000000})', pathname: '/:id' }).hasRegExpGroups, 'protocol inference uses the bounded native matcher');
  const exhausted = new Pattern({ pathname: '((?:h?){1000000000})' });
  assert(exhausted.test({pathname:'x'}) === false && exhausted.exec({pathname:'x'}) === null, 'native match resource limits remain boolean/null results');
  const sets = new Pattern({pathname:'/([[a-z]--a])'});
  assert(sets.test({pathname:'/z'}) && !sets.test({pathname:'/a'}), 'Unicode set subtraction uses v mode');
  const folded = new Pattern({ pathname: '/:id(abc)' }, { ignoreCase: true });
  assert(folded.test({ pathname: '/ABC' }), 'case-insensitive RegExp');
  assert(new Pattern({ pathname: '/\uD800' }).test({ pathname: '/\uFFFD' }), 'USVString conversion');
  const protoGroup = new Pattern({ pathname: '/:__proto__' }).exec({ pathname: '/value' }).pathname.groups;
  assert(Object.hasOwn(protoGroup, '__proto__') && protoGroup.__proto__ === 'value', 'record defines own data properties');
  assert(new Pattern().test(), 'default empty dictionaries');
  const empty = new Pattern().exec(null);
  assert(Object.keys(empty.inputs[0]).length === 0, 'null converts to empty input dictionary');
  assert(new Pattern().exec('not a URL') === null, 'invalid URL does not match');
  expectTypeError(() => new Pattern().test({}, 'https://example.test/'));
  expectTypeError(() => new Pattern({ pathname: '/(a' }));
  expectTypeError(() => new Pattern({ pathname: '/:id([a)' }));
  const order = [];
  const expectedOrder = ['baseURL', 'hash', 'hostname', 'password', 'pathname', 'port', 'protocol', 'search', 'username'];
  const init = {};
  for (const name of expectedOrder) Object.defineProperty(init, name, {
    get() { order.push(name); return name === 'pathname' ? '/:id' : undefined; }
  });
  new Pattern(init, { get ignoreCase() { order.push('ignoreCase'); return false; } });
  assert(order.join(',') === expectedOrder.concat('ignoreCase').join(','), 'derived dictionary conversion order');
  const sentinel = {};
  let laterRead = false;
  try {
    new Pattern({ get pathname() { throw sentinel; } }, { get ignoreCase() { laterRead = true; } });
    throw new Error('missing conversion error');
  } catch (error) { assert(error === sentinel && !laterRead, 'original getter exception stops later conversion'); }
  const source = { pathname: { toString() { return '/42'; } } };
  const snapshot = new Pattern().exec(source).inputs[0];
  source.pathname = '/changed';
  assert(snapshot.pathname === '/42', 'input is a converted snapshot');
  const orderedInput = new Pattern().exec({protocol:'https', pathname:'/p', hostname:'example.test'}).inputs[0];
  assert(Object.keys(orderedInput).join(',') === 'hostname,pathname,protocol', 'converted input dictionary member order');
  let conversions = 0;
  const value = { toString() { ++conversions; return '/ignored'; } };
  const getter = Object.getOwnPropertyDescriptor(Pattern.prototype, 'pathname').get;
  for (const fake of [{}, Object.create(pattern), new Proxy(pattern, {})]) {
    expectTypeError(() => Pattern.prototype.test.call(fake, { pathname: value }));
    expectTypeError(() => getter.call(fake));
  }
  const revocable = Proxy.revocable(pattern, {});
  revocable.revoke();
  expectTypeError(() => Pattern.prototype.exec.call(revocable.proxy, { pathname: value }));
  assert(conversions === 0, 'receiver validation precedes input conversion');
  class Derived extends Pattern {}
  assert(new Derived() instanceof Derived, 'ordinary subclass construction');
  const originalRegExp = realm.RegExp;
  const originalExec = originalRegExp.prototype.exec;
  const globalDescriptor = Object.getOwnPropertyDescriptor(realm, 'RegExp');
  try {
    originalExec.call(new originalRegExp('(sentinel)'), 'sentinel');
    originalRegExp.prototype.exec = () => { throw new Error('author exec'); };
    Object.defineProperty(realm, 'RegExp', { configurable: true, get() { throw new Error('author RegExp'); } });
    const internal = new Pattern({ pathname: '/:id(\\d+)' });
    assert(internal.exec({ pathname: '/123' }).pathname.groups.id === '123', 'host matching ignores author RegExp');
    assert(originalRegExp.$1 === 'sentinel', 'host matching preserves legacy RegExp statics');
  } finally {
    Object.defineProperty(realm, 'RegExp', globalDescriptor);
    originalRegExp.prototype.exec = originalExec;
  }
  return 'ok';
}
