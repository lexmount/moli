(() => {
  const equal = (actual, expected) => {
    if (actual !== expected) throw new Error(`${JSON.stringify(actual)} !== ${JSON.stringify(expected)}`);
  };
  // Current Typed OM IDL uses USVString. Chromium 145 still uses DOMString
  // for these slots, so keep these normative checks separate from its control.
  const value = new CSSUnparsedValue(['\uD800', '\uDC00', '\uD83D\uDE00']);
  equal(value[0], '\uFFFD');
  equal(value[1], '\uFFFD');
  equal(value[2], '\uD83D\uDE00');
  value[0] = '\uDFFF';
  equal(value[0], '\uFFFD');
  const reference = new CSSVariableReferenceValue('--\uD800');
  equal(reference.variable, '--\uFFFD');
  reference.variable = '--\uDC00';
  equal(reference.variable, '--\uFFFD');
  const name = new CSSVariableReferenceValue('--a b');
  equal(String(new CSSUnparsedValue([name])), 'var(--a\\ b)');
  name.variable = '--a)';
  equal(String(new CSSUnparsedValue([name])), 'var(--a\\))');
  name.variable = '--\0';
  equal(String(new CSSUnparsedValue([name])), 'var(--\uFFFD)');
  // Web IDL legacy platform objects reject accessor descriptors for indices.
  equal(Reflect.defineProperty(value, 0, {get() { return 'wrong'; }}), false);
  equal(Reflect.defineProperty(value, 0, {}), false);
  equal(Reflect.defineProperty(value, 0, {enumerable: false}), false);
  equal(value[0], '\uFFFD');
  return true;
})()
