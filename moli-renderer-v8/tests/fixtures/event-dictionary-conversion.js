(() => {
  const assert = (value, label) => { if (!value) throw Error(label); };
  const sentinel = { original: true };
  const bases = ['bubbles', 'cancelable', 'composed'];
  const ui = ['detail', 'sourceCapabilities', 'view'];
  const cases = [
    [Event, bases],
    [UIEvent, [...bases, ...ui]],
    [FocusEvent, [...bases, ...ui, 'relatedTarget']],
    [CompositionEvent, [...bases, ...ui, 'data']],
    [CustomEvent, [...bases, 'detail']]
  ];
  for (const [Constructor, members] of cases) {
    for (const value of [undefined, null, {}]) {
      const event = new Constructor('audit', value);
      assert(!event.bubbles && !event.cancelable && !event.composed, Constructor.name + ' defaults');
    }
    for (const value of [1, true, 'init', 1n, Symbol('init')]) {
      let caught;
      try {new Constructor('audit', value);} catch (error) {caught = error;}
      assert(caught instanceof TypeError, Constructor.name + ' primitive dictionary');
    }
    const reads = [];
    const options = {};
    for (const key of members) Object.defineProperty(options, key, {
      configurable: true, get() {
        reads.push(key);
        return bases.includes(key) ? true : undefined;
      }
    });
    const event = new Constructor('audit', options);
    assert(reads.join() === members.join(), Constructor.name + ' inheritance order: ' + reads);
    assert(event.bubbles && event.cancelable && event.composed, Constructor.name + ' flags');
    for (const key of members) {
      reads.length = 0;
      const bad = Object.create(options);
      Object.defineProperty(bad, key, {get() {reads.push(key); throw sentinel;}});
      let caught;
      try {new Constructor('audit', bad);} catch (error) {caught = error;}
      assert(caught === sentinel, Constructor.name + '/' + key + ' exception identity');
      assert(reads.join() === members.slice(0, members.indexOf(key) + 1).join(),
        Constructor.name + '/' + key + ' reads after abrupt completion: ' + reads);
    }
  }
  for (const Constructor of [UIEvent, FocusEvent, CompositionEvent]) {
    for (const [value, expected] of [[2147483648, -2147483648], [-2147483649, 2147483647],
      [4294967296, 0], [NaN, 0], [Infinity, 0], [-1.5, -1]])
      assert(new Constructor('audit', {detail: value}).detail === expected, Constructor.name + ' long conversion');
    for (const member of ['view', 'sourceCapabilities']) {
      let caught;
      try {new Constructor('audit', {[member]: {}});} catch (error) {caught = error;}
      assert(caught instanceof TypeError, Constructor.name + '/' + member + ' interface brand');
    }
  }
  for (const value of [{}, new Proxy(document.body, {})]) {
    let caught;
    try {new FocusEvent('audit', {relatedTarget: value});} catch (error) {caught = error;}
    assert(caught instanceof TypeError, 'FocusEvent relatedTarget brand');
  }
  assert(new FocusEvent('audit', {relatedTarget: document.body}).relatedTarget === document.body,
    'FocusEvent relatedTarget identity');
  const payload = {arbitrary: true};
  assert(new CustomEvent('audit', {detail: payload}).detail === payload, 'CustomEvent any payload identity');
  assert(new CompositionEvent('audit', {data: '\ud800\0\udc00'}).data === '\ud800\0\udc00', 'CompositionEvent UTF-16');
  return true;
})()
