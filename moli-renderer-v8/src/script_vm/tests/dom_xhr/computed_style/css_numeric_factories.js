(() => {
  const equal = (actual, expected, label) => {
    if (!Object.is(actual, expected)) throw new Error(`${label}: ${actual} !== ${expected}`);
  };
  const raises = (callback, Type = TypeError) => {
    try { callback(); } catch (error) { if (error instanceof Type) return; throw error; }
    throw new Error('expected ' + Type.name);
  };
  const units = ["number", "percent", "cap", "ch", "em", "ex", "ic", "lh", "rcap", "rch", "rem", "rex", "ric", "rlh", "vw", "vh", "vi", "vb", "vmin", "vmax", "svw", "svh", "svi", "svb", "svmin", "svmax", "lvw", "lvh", "lvi", "lvb", "lvmin", "lvmax", "dvw", "dvh", "dvi", "dvb", "dvmin", "dvmax", "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax", "cm", "mm", "Q", "in", "pt", "pc", "px", "deg", "grad", "rad", "turn", "s", "ms", "Hz", "kHz", "dpi", "dpcm", "dppx", "fr"];
  for (const unit of units) {
    const descriptor = Object.getOwnPropertyDescriptor(CSS, unit);
    equal(descriptor.enumerable, true, unit + ' enumerable');
    equal(descriptor.configurable, true, unit + ' configurable');
    equal(descriptor.writable, true, unit + ' writable');
    const factory = descriptor.value;
    equal(factory.name, unit, unit + ' name');
    equal(factory.length, 1, unit + ' arity');
    equal(Object.hasOwn(factory, 'prototype'), false, unit + ' non-constructor');
    const value = factory(12.3);
    equal(Object.getPrototypeOf(value), CSSUnitValue.prototype, unit + ' native prototype');
    equal(value.value, 12.3, unit + ' double');
    equal(value.unit, unit.toLowerCase(), unit + ' canonical unit');
    equal(value instanceof CSSNumericValue, true, unit + ' numeric brand');
    equal(value instanceof CSSStyleValue, true, unit + ' style value brand');
    equal(factory(12.3) === value, false, unit + ' fresh result');
    equal(new CSSUnitValue(12.3, unit.toUpperCase()).unit, unit.toLowerCase(), unit + ' constructor');
    raises(() => new factory(1));
    raises(() => factory());
    for (const invalid of [undefined, NaN, Infinity, -Infinity, 1n, Symbol('number')]) raises(() => factory(invalid));
    for (const [input, expected] of [[null, 0], [true, 1], ['2.5', 2.5], [-0, -0]]) {
      equal(factory(input).value, expected, unit + ' conversion');
    }
  }
  equal(String(CSS.px(.031400000000000004)), '0.0314px', 'CSS number serialization');
  equal(String(CSS.percent(3.14)), '3.14%', 'percentage serialization');
  const marker = {};
  let calls = 0;
  equal(CSS.px({valueOf() { calls++; return 5; }}, {valueOf() { throw marker; }}).value, 5, 'convert first argument');
  equal(calls, 1, 'one conversion');
  try { CSS.px({valueOf() { throw marker; }}); throw new Error('missing exception'); }
  catch (error) { equal(error, marker, 'conversion exception identity'); }
  const {proxy, revoke} = Proxy.revocable({}, {});
  revoke();
  equal(CSS.px.call(proxy, 4).value, 4, 'namespace operation has no receiver check');
  const other = document.getElementById('child').contentWindow;
  const foreign = other.CSS.px(2);
  equal(Object.getPrototypeOf(foreign), other.CSSUnitValue.prototype, 'callee realm result');
  equal(CSSStyleValue.prototype.toString.call(foreign), '2px', 'cross-realm native brand');
  raises(() => other.CSS.px(NaN), other.TypeError);
  raises(() => CSSStyleValue.prototype.toString.call(new Proxy(foreign, {})));
  raises(() => CSSStyleValue.prototype.toString.call(Object.create(foreign)));
  const NativeUnitValue = CSSUnitValue, NativeNumericValue = CSSNumericValue, NativeStyleValue = CSSStyleValue;
  try {
    globalThis.CSSUnitValue = globalThis.CSSNumericValue = globalThis.CSSStyleValue = function() { throw marker; };
    const value = CSS.px(5);
    equal(Object.getPrototypeOf(value), NativeUnitValue.prototype, 'ignore author constructors');
    equal(NativeStyleValue.prototype.toString.call(value), '5px', 'native object after author replacement');
  } finally {
    globalThis.CSSUnitValue = NativeUnitValue;
    globalThis.CSSNumericValue = NativeNumericValue;
    globalThis.CSSStyleValue = NativeStyleValue;
  }
  return true;
})()
