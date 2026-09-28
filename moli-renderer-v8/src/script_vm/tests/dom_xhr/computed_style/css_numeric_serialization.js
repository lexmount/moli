(() => {
  const equal = (actual, expected, label) => {
    if (!Object.is(actual, expected)) throw new Error(`${label}: ${actual} !== ${expected}`);
  };
  const throws = callback => {
    try { callback(); } catch (error) { if (error instanceof TypeError) return; throw error; }
    throw new Error('expected TypeError');
  };
  const cases = [
    [-0, '0'], [3.14, '3.14'], [.031400000000000004, '0.0314'],
    [.12345678912345678, '0.123457'], [1.23456789, '1.23457'],
    [123456.789, '123457'], [1234567.89, '1.23457e+06'],
    [999999.5, '1e+06'], [999999.4, '999999'], [1e-4, '0.0001'],
    [1e-5, '1e-05'], [1e-7, '1e-07'], [1e20, '1e+20'], [1e21, '1e+21'],
    [1e100, '1e+100'], [Number.MAX_VALUE, '1.79769e+308'],
    [Number.MIN_VALUE, '4.94066e-324'], [Number.MIN_VALUE * 2, '9.88131e-324'],
    [Number.MAX_SAFE_INTEGER, '9.0072e+15'], [1.234565, '1.23456'], [-1.234565, '-1.23456'],
  ];
  for (const [number, expected] of cases) {
    for (const [unit, suffix] of [['number', ''], ['percent', '%'], ['px', 'px']]) {
      const value = new CSSUnitValue(number, unit);
      equal(String(value), expected + suffix, `format ${number}${unit}`);
      equal(value.value, number, 'serialization does not round the stored double');
      value.value = number * -1;
      equal(value.value, number * -1, 'setter retains the double');
    }
  }
  const parsed = CSSStyleValue.parse('opacity', '3.14%');
  equal(parsed.value, 3.14 * 0.01, 'parsed percentage double');
  equal(String(parsed), '0.0314', 'parsed percentage formatting');
  for (const source of ['1e100px', '1e1000px']) {
    const value = CSSStyleValue.parse('width', source);
    equal(value.value, 3.4028234663852886e38, 'CSS parser numeric range');
    equal(String(value), '3.40282e+38px', 'range-clamped value formatting');
  }
  for (const [source, expected] of [
    ['0.123456789123456789px', '0.12345678912345678px'],
    ['0.123456789123456789', '0.12345678912345678'],
    ['0.123456789123456789%', '0.123457%'],
    ['+01.2300e+2px', '123px'], ['+01.2300e+2', '123.0'],
    ['1.0', '1.0'], ['-0', '0'], ['-0.0', '0.0'], ['-0px', '0px'],
    ['+0.0%', '0%'], ['1e1000px', '3.4028234663852886e+38px'],
    ['1e1000%', '3.40282e+38%'], ['1e-999px', '0px'], ['1e-7', '1e-7'],
    ['1e+21px', '1e+21px'], ['1em', '1em'], ['1e2em', '100em'],
    ['1p\\78', '1px'], ['2147483648', '2147483648'],
    ['9223372036854775808', '9223372036854775807'],
  ]) {
    equal(String(new CSSUnparsedValue([source])), expected, 'constructed token ' + source);
    const value = CSSStyleValue.parse('--raw', source);
    equal(value[0], expected, 'reified token ' + source);
    equal(String(value), expected, 'reified token serialization ' + source);
  }
  const reference = new CSSVariableReferenceValue('--x', new CSSUnparsedValue(['0.123456789123456789px']));
  equal(String(new CSSUnparsedValue([reference])), 'var(--x,0.12345678912345678px)', 'fallback precision');
  reference.fallback[0] = '0.234567890123456789px';
  equal(String(new CSSUnparsedValue([reference])), 'var(--x,0.2345678901234568px)', 'live fallback mutation');
  equal(String(new CSSUnparsedValue(['1.0', 'px'])), '1.0/**/px', 'number/identifier boundary');

  const other = document.getElementById('child').contentWindow;
  const foreign = new other.CSSUnitValue(.031400000000000004, 'number');
  equal(CSSStyleValue.prototype.toString.call(foreign), '0.0314', 'cross-realm native value');
  const value = new CSSUnitValue(1, 'px');
  for (const invalid of [NaN, Infinity, -Infinity]) {
    throws(() => new CSSUnitValue(invalid, 'px'));
    throws(() => { value.value = invalid; });
    equal(value.value, 1, 'invalid setter does not mutate');
  }
  const precision = Number.prototype.toPrecision, numberString = Number.prototype.toString;
  try {
    Number.prototype.toPrecision = Number.prototype.toString = () => { throw new Error('author numeric formatting'); };
    equal(String(foreign), '0.0314', 'native formatter ignores author methods');
    equal(String(new CSSUnparsedValue(['0.123456789123456789px'])), '0.12345678912345678px', 'native token formatting');
  } finally {
    Number.prototype.toPrecision = precision;
    Number.prototype.toString = numberString;
  }
  return true;
})()
