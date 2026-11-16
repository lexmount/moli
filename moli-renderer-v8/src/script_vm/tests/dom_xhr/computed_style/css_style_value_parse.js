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
  const {parse, parseAll} = CSSStyleValue;
  for (const name of ['parse', 'parseAll']) {
    const descriptor = Object.getOwnPropertyDescriptor(CSSStyleValue, name);
    equal(descriptor.writable && descriptor.enumerable && descriptor.configurable, true, name + ' descriptor');
    equal(descriptor.value.length, 2, name + ' arity');
    equal(descriptor.value.name, name, name + ' function name');
    throws(() => new descriptor.value('width', '1px'), TypeError, name + ' not a constructor');
    throws(() => descriptor.value('width'), TypeError, name + ' missing argument');
    throws(() => descriptor.value(Symbol(), '1px'), TypeError, name + ' property conversion');
    throws(() => descriptor.value('width', Symbol()), TypeError, name + ' value conversion');
  }
  for (const [property, text] of [
    ['', '1px'], ['bad-property', '1px'], [' width', '1px'], ['width ', '1px'],
    ['cssFloat', 'left'], ['backgroundColor', 'red'], ['--', 'red'],
    ['width', '1deg'], ['width', '1px !important'], ['width', '1px;'],
    ['margin', '1px; color: red'], ['display', 'block inline grid'],
    ['--custom', ''], ['--custom', ' '], ['--custom', '/**/'],
    ['--custom', 'a;b'], ['--custom', '!important'], ['--custom', ')'],
    ['width', 'var(ordinary)']
  ]) {
    throws(() => parse(property, text), TypeError, `${property}: ${text}`);
    throws(() => parseAll(property, text), TypeError, `parseAll ${property}: ${text}`);
  }
  const sentinel = new Error('sentinel');
  let order = '';
  try {
    parse({toString() { order += 'p'; return 'bad-property'; }},
          {toString() { order += 'v'; throw sentinel; }});
    throw new Error('expected conversion exception');
  } catch (error) { equal(error, sentinel, 'conversion exception identity'); }
  equal(order, 'pv', 'both arguments converted before grammar validation');
  order = '';
  try {
    parse({toString() { order += 'p'; throw sentinel; }},
          {toString() { order += 'v'; return '1px'; }});
  } catch (error) { equal(error, sentinel, 'property exception identity'); }
  equal(order, 'p', 'property conversion failure short-circuits');

  for (const [property, text, value, unit] of [
    ['WiDtH', '10PX', 10, 'px'], ['width', '0', 0, 'px'],
    ['width', '25%', 25, 'percent'], ['line-height', '2', 2, 'number'],
    ['opacity', '25%', 0.25, 'number'], ['transition-duration', '2s', 2, 's'],
    ['width', '3.14px', 3.14, 'px'], ['width', '3.14%', 3.14, 'percent'],
    ['width', '0.123456789123456789px', 0.123456789123456789, 'px'],
    ['line-height', '3.14', 3.14, 'number'], ['opacity', '3.14%', 3.14 * 0.01, 'number'],
    ['width', '/*comment*/1.2345e-2em/**/', 0.012345, 'em'],
    ['width', '.314e1p\\78', 3.14, 'px'], ['width', '1em', 1, 'em']
  ]) {
    const result = parse.call(null, property, text);
    equal(Object.getPrototypeOf(result), CSSUnitValue.prototype, property + ' numeric prototype');
    equal(result.value, value, property + ' numeric value');
    equal(result.unit, unit, property + ' numeric unit');
    equal(result instanceof CSSStyleValue, true, 'numeric inheritance');
  }
  const keyword = parse('width', 'AuTo');
  equal(Object.getPrototypeOf(keyword), CSSKeywordValue.prototype, 'keyword prototype');
  equal(keyword.value, 'auto', 'keyword normalization');
  for (const [property, text] of [['color', 'red'], ['margin', '1px'], ['font-family', 'Arial, serif']]) {
    const result = parse(property, text);
    equal(Object.getPrototypeOf(result), CSSStyleValue.prototype, 'opaque property type');
    equal(String(result), text, 'opaque serialization');
    equal(parseAll(property, text).length, 1, 'opaque property remains one value');
  }
  const durations = parseAll('transition-duration', '1s, 2s, 3s');
  equal(Array.isArray(durations), true, 'parseAll sequence is array');
  equal(durations.map(String).join('|'), '1s|2s|3s', 'property-specific list');
  equal(parseAll('transition-duration', '3.14s, 2.718s').map(v => v.value).join('|'), '3.14|2.718', 'list precision');
  equal(String(parse('transition-duration', '1s, 2s')), '1s', 'parse returns first list item');
  const raw = parse('--a b', '  a /*comment*/b  ');
  equal(Object.getPrototypeOf(raw), CSSUnparsedValue.prototype, 'custom property native type');
  equal(raw.length, 1, 'custom property token fragment');
  equal(raw[0], 'a b', 'comments removed and whitespace retained');
  raw[0] = 'changed'; raw[1] = ' suffix';
  equal(Array.from(raw).join(''), 'changed suffix', 'parsed value indexed handlers');
  const ref = parse('width', 'VAR(--A, var(--B))')[0];
  equal(Object.getPrototypeOf(ref), CSSVariableReferenceValue.prototype, 'native reference');
  equal(ref.variable, '--A', 'case-sensitive variable');
  equal(ref.fallback[0], ' ', 'fallback whitespace');
  equal(ref.fallback[1].variable, '--B', 'nested reference');
  equal(ref.fallback[1].fallback, null, 'absent fallback');
  equal(String(parse('width', 'var(--a')), 'var(--a)', 'CSS EOF recovery');

  const other = document.getElementById('child').contentWindow;
  const foreignParse = other.CSSStyleValue.parse;
  const foreign = foreignParse('width', 'var(--cross, 1px)');
  equal(Object.getPrototypeOf(foreign), other.CSSUnparsedValue.prototype, 'callee realm return value');
  equal(Object.getPrototypeOf(foreign[0]), other.CSSVariableReferenceValue.prototype, 'callee realm reference');
  equal(Object.getPrototypeOf(foreign[0].fallback), other.CSSUnparsedValue.prototype, 'callee realm fallback');
  equal(Object.getPrototypeOf(other.CSSStyleValue.parseAll('width', '1px')), other.Array.prototype, 'callee realm array');
  throws(() => foreignParse('width', 'invalid'), other.TypeError, 'callee realm parse error');
  throws(() => foreignParse('width', Symbol()), other.TypeError, 'callee realm conversion error');
  equal(CSSStyleValue.prototype.toString.call(foreign), 'var(--cross, 1px)', 'cross-realm brand');

  const saved = [CSSStyleValue, CSSUnitValue, CSSKeywordValue, CSSUnparsedValue, CSSVariableReferenceValue];
  const arrayIndex = Object.getOwnPropertyDescriptor(Array.prototype, '0');
  const savedIterator = Array.prototype[Symbol.iterator];
  try {
    const poison = function() { throw new Error('author constructor or iterator'); };
    globalThis.CSSStyleValue = globalThis.CSSUnitValue = globalThis.CSSKeywordValue = poison;
    globalThis.CSSUnparsedValue = globalThis.CSSVariableReferenceValue = poison;
    Object.defineProperty(Array.prototype, '0', {set: poison, configurable:true});
    Array.prototype[Symbol.iterator] = poison;
    equal(Object.getPrototypeOf(parse('width', '1px')), saved[1].prototype, 'native numeric factory');
    equal(Object.getPrototypeOf(parse('width', 'auto')), saved[2].prototype, 'native keyword factory');
    const safe = parse('--safe', 'var(--x,1px)');
    equal(Object.getPrototypeOf(safe), saved[3].prototype, 'native unparsed factory');
    equal(Object.getPrototypeOf(safe[0]), saved[4].prototype, 'native reference factory');
    equal(safe[0].fallback[0], '1px', 'private fallback elements');
    equal(parseAll('width', '1px')[0].value, 1, 'native result sequence');
  } finally {
    Array.prototype[Symbol.iterator] = savedIterator;
    if (arrayIndex) Object.defineProperty(Array.prototype, '0', arrayIndex);
    else delete Array.prototype[0];
    [globalThis.CSSStyleValue, globalThis.CSSUnitValue, globalThis.CSSKeywordValue,
     globalThis.CSSUnparsedValue, globalThis.CSSVariableReferenceValue] = saved;
  }
  return true;
})()
