(() => {
  const assert = (condition, message) => {
    if (!condition) throw Error(message);
  };
  const target = typeof document === 'undefined' ? globalThis : document;
  const violations = [];
  target.addEventListener('securitypolicyviolation', event => {
    if (event.blockedURI === 'trusted-types-sink') {
      violations.push({disposition: event.disposition, sample: event.sample});
    }
  });
  const policy = trustedTypes.createPolicy('function-report-only', {
    createScript: value => value
  });
  const parameter = policy.createScript('value');
  const body = policy.createScript('return value + 1;');
  const constructors = [
    values => Function(...values),
    values => new Function(...values),
    values => Reflect.apply(Function, undefined, values),
    values => Reflect.construct(Function, values)
  ];
  for (const construct of constructors) {
    assert(construct([parameter, body])(41) === 42, 'trusted source remains usable');
  }
  assert(Function('value', 'return value + 10;')(1) === 11,
    'report-only policy permits and reports plain source');

  const defaults = [];
  let transform = false;
  trustedTypes.createPolicy('default', {
    createScript(value, type, sink) {
      defaults.push({type, sink});
      return transform ? value.replace('value + 10', 'value + 100') : value;
    }
  });
  let conversions = 0;
  for (const construct of constructors) {
    const before = defaults.length;
    assert(construct([parameter, body])(41) === 42, 'native brand retains trusted source');
    assert(defaults.length === before, 'trusted source skips the default policy');

    const changed = policy.createScript('return value + 1;');
    changed.toString = () => {
      conversions++;
      return 'return value + 10;';
    };
    assert(construct([parameter, changed])(1) === 11,
      'changed conversion goes through the transparent report-only default policy');
    assert(defaults.length === before + 1, 'default conversion occurs once');
  }
  transform = true;
  let rejected = 0;
  for (const construct of constructors) {
    const before = defaults.length;
    assert(construct([parameter, body])(41) === 42, 'native brand skips a transforming policy');
    assert(defaults.length === before, 'transforming policy is not called for trusted source');
    const changed = policy.createScript('return value + 1;');
    changed.toString = () => {
      conversions++;
      return 'return value + 10;';
    };
    try {
      construct([parameter, changed]);
    } catch (error) {
      assert(error instanceof EvalError, 'Function rejects transformed source with EvalError');
      rejected++;
    }
    assert(defaults.length === before + 1, 'transforming default policy is called once');
  }
  assert(rejected === constructors.length, 'all Function entry points reject transformed source');
  assert(conversions === constructors.length * 2, 'each author conversion runs once');
  assert(defaults.every(value => value.type === 'TrustedScript' && value.sink === 'Function'),
    'default policy receives the Function sink metadata');
  return {violations, defaults: defaults.length, conversions, rejected};
})()
