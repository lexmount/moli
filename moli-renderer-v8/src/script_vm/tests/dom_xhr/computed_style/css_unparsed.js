(() => {
  'use strict';
  const equal = (actual, expected, label) => {
    if (!Object.is(actual, expected)) throw new Error(`${label}: ${actual} !== ${expected}`);
  };
  const throws = (callback, constructor, label) => {
    try { callback(); } catch (error) {
      if (error instanceof constructor) return;
      throw new Error(`${label}: wrong exception ${error}`);
    }
    throw new Error(`${label}: did not throw`);
  };
  const U = CSSUnparsedValue, R = CSSVariableReferenceValue;
  equal(U.length, 1, 'sequence constructor arity');
  equal(R.length, 1, 'reference constructor arity');
  equal(Object.getPrototypeOf(U.prototype), CSSStyleValue.prototype, 'inheritance');
  throws(() => U([]), TypeError, 'requires new');
  throws(() => new U(), TypeError, 'required sequence');
  throws(() => new U('text'), TypeError, 'sequence must be object');
  throws(() => new R('ordinary'), TypeError, 'custom property prefix');
  throws(() => new R('--a', {}), TypeError, 'fallback interface');
  equal(new R('--').variable, '--', 'prefix-only name is allowed');
  const fallback = new U(['red']);
  const reference = new R('--color', fallback);
  const value = new U(['initial', reference]);
  equal(reference.fallback, fallback, 'fallback identity');
  equal(new R('--x', undefined).fallback, null, 'default fallback');
  equal(value[1], reference, 'union retains native reference');
  equal(value.length, 2, 'initial length');
  value[0] = 'blue';
  value[2] = 'end';
  equal(value.length, 3, 'append');
  equal(value[0], 'blue', 'replace');
  throws(() => { value[4] = 'gap'; }, RangeError, 'gap');
  equal(Reflect.deleteProperty(value, 0), false, 'supported index cannot be deleted');
  equal(Reflect.deleteProperty(value, 9), true, 'missing index deletion');
  equal(Reflect.preventExtensions(value), false, 'legacy object remains extensible');
  equal(Object.isExtensible(value), true, 'extensible after failed prevention');
  const descriptor = Object.getOwnPropertyDescriptor(value, 0);
  equal(descriptor.value, 'blue', 'descriptor value');
  equal(descriptor.writable && descriptor.enumerable && descriptor.configurable, true, 'descriptor flags');
  equal(Object.keys(value).join(','), '0,1,2', 'indexed enumeration');
  value['01'] = 'own';
  value[-1] = 'negative';
  equal(value.length, 3, 'ordinary property does not append');
  equal(value['01'], 'own', 'noncanonical index');
  const derived = Object.create(value);
  derived[0] = 'shadow';
  derived[3] = 'own-gap';
  equal(derived[0], 'shadow', 'inherited index set uses receiver');
  equal(Object.hasOwn(derived, 0), true, 'derived owns index');
  equal(value[0], 'blue', 'inherited assignment leaves native value');
  equal(value.length, 3, 'inherited assignment leaves length');
  const proxy = new Proxy(value, {});
  proxy[0] = 'through-proxy';
  equal(value[0], 'through-proxy', 'proxy forwards indexed write');
  equal(proxy[1], reference, 'proxy forwards indexed read');
  throws(() => proxy.length, TypeError, 'length requires brand');
  throws(() => new R('--x', proxy), TypeError, 'proxy is not native fallback');

  const lengthGetter = Object.getOwnPropertyDescriptor(U.prototype, 'length').get;
  const variableSetter = Object.getOwnPropertyDescriptor(R.prototype, 'variable').set;
  let conversions = 0, traps = 0;
  const converted = { toString() { ++conversions; return '--updated'; } };
  const revoked = Proxy.revocable(reference, {}); revoked.revoke();
  for (const fake of [{}, Object.create(reference), new Proxy(reference, {
    get() { ++traps; throw new Error('author trap'); },
    getPrototypeOf() { ++traps; throw new Error('author trap'); }
  }), revoked.proxy]) {
    throws(() => variableSetter.call(fake, converted), TypeError, 'strict receiver');
  }
  equal(conversions, 0, 'brand check before conversion');
  equal(traps, 0, 'brand check bypasses author traps');
  variableSetter.call(reference, converted);
  equal(conversions, 1, 'one conversion');
  equal(reference.variable, '--updated', 'variable setter');
  throws(() => { reference.variable = 'bad'; }, TypeError, 'invalid mutation');
  equal(reference.variable, '--updated', 'invalid mutation is atomic');
  const other = document.getElementById('child').contentWindow;
  const foreign = new other.CSSUnparsedValue(['foreign']);
  equal(lengthGetter.call(foreign), 1, 'cross-realm brand');
  equal(new R('--foreign', foreign).fallback, foreign, 'cross-realm fallback');
  const foreignSetter = Object.getOwnPropertyDescriptor(other.CSSVariableReferenceValue.prototype, 'variable').set;
  throws(() => foreignSetter.call({}, converted), other.TypeError, 'callee realm TypeError');
  equal(conversions, 1, 'foreign receiver before conversion');

  class Subclass extends U {}
  const subclass = new Subclass(['subclass']);
  equal(subclass instanceof Subclass && subclass instanceof CSSStyleValue, true, 'subclass');
  subclass[1] = reference;
  equal(subclass.length, 2, 'subclass indexed append');
  function Target() {}
  const custom = Reflect.construct(U, [['custom']], Target);
  equal(Object.getPrototypeOf(custom), Target.prototype, 'custom newTarget prototype');
  equal(lengthGetter.call(custom), 1, 'custom newTarget brand');
  custom[1] = 'target';
  equal(custom[1], 'target', 'custom newTarget interceptor');
  equal(CSSStyleValue.prototype.toString.call(custom), 'custom/**/target', 'custom newTarget serialization');

  const live = new U(['first']);
  const iterator = live.values();
  equal(iterator.next().value, 'first', 'iterator first');
  live[1] = 'second';
  equal(iterator.next().value, 'second', 'iterator sees append');
  equal(iterator.next().done, true, 'iterator completes');
  let calls = 0;
  live.forEach((item, index, self) => {
    equal(self, live, 'forEach receiver argument');
    ++calls;
    if (index === 0) live[2] = 'third';
  });
  equal(calls, 2, 'forEach snapshots length');
  for (const method of ['entries', 'keys', 'values', 'forEach']) {
    equal(U.prototype[method], Array.prototype[method], `intrinsic ${method}`);
  }
  equal(U.prototype[Symbol.iterator], Array.prototype.values, 'intrinsic iterator');
  equal(Array.from(U.prototype.values.call({0:'generic', length:1})).join(), 'generic', 'generic iterable methods');
  const reentrant = new U([]);
  reentrant[1] = { toString() { reentrant[0] = 'first'; return 'second'; } };
  equal(reentrant.length, 2, 'conversion precedes range check');
  equal(reentrant[1], 'second', 'reentrant append');
  let closed = false;
  const sentinel = new Error('conversion failure');
  const sequence = { *[Symbol.iterator]() {
    try { yield { toString() { throw sentinel; } }; } finally { closed = true; }
  }};
  try { new U(sequence); throw new Error('expected conversion failure'); }
  catch (error) { equal(error, sentinel, 'conversion exception identity'); }
  equal(closed, false, 'Web IDL sequence conversion propagates without IteratorClose');

  const text = parts => String(new U(parts));
  for (const [parts, expected] of [
    [['a', 'b'], 'a/**/b'], [['a', ' b'], 'a b'],
    [['a', '('], 'a/**/('], [['var(', '--x', ')'], 'var(--x)'],
    [['1', 'px'], '1/**/px'], [['#', 'red'], '#/**/red'],
    [['foo/*comment*/', 'bar'], 'foo/**/bar'], [['', 'x', ''], 'x']
  ]) equal(text(parts), expected, 'token boundaries');
  const branch = new R('--a', fallback);
  equal(text([branch, branch]), 'var(--a,red)var(--a,red)', 'shared fallback is not a cycle');
  fallback[0] = 'green';
  equal(text([branch]), 'var(--a,green)', 'live fallback');
  Object.defineProperty(branch, 'variable', {get() { throw new Error('author getter'); }});
  branch.toString = () => { throw new Error('author stringifier'); };
  equal(text([branch]), 'var(--a,green)', 'native serialization ignores author properties');
  const cycle = new U([]);
  cycle[0] = new R('--cycle', cycle);
  equal(String(cycle), '', 'self cycle');
  const parent = new U([new R('--nested', cycle)]);
  equal(String(parent), '', 'nested cycle');
  cycle[0] = 'repaired';
  equal(String(parent), 'var(--nested,repaired)', 'cycle can be repaired');
  equal(text([new R('--empty', new U([]))]), 'var(--empty,)', 'empty fallback');
  let deep = new U(['leaf']);
  for (let i = 0; i < 300; ++i) deep = new U([new R('--x', deep)]);
  equal(String(deep), 'var(--x,'.repeat(300) + 'leaf' + ')'.repeat(300), 'deep fallback graph');

  const arrayIndex = Object.getOwnPropertyDescriptor(Array.prototype, '0');
  try {
    Object.defineProperty(Array.prototype, '0', {set() { throw new Error('poisoned array'); }, configurable:true});
    const protectedValue = new U({ *[Symbol.iterator]() { yield 'safe'; } });
    protectedValue[0] = 'still-safe';
    equal(protectedValue[0], 'still-safe', 'private array uses own data properties');
  } finally {
    if (arrayIndex) Object.defineProperty(Array.prototype, '0', arrayIndex);
    else delete Array.prototype[0];
  }
  return true;
})()
