(() => {
  const assert = (value, label) => { if (!value) throw Error(label); };
  const base = ['bubbles', 'cancelable', 'composed'];
  const cases = [
    [ClipboardEvent, ['clipboardData']], [CloseEvent, ['code', 'reason', 'wasClean']],
    [CommandEvent, ['command', 'source']], [ToggleEvent, ['newState', 'oldState', 'source']],
    [InterestEvent, ['source']], [PopStateEvent, ['hasUAVisualTransition', 'state']],
    [HashChangeEvent, ['newURL', 'oldURL']], [PageTransitionEvent, ['persisted']]
  ];
  for (const [Constructor, own] of cases) {
    const members = [...base, ...own];
    for (const value of [undefined, null, {}]) {
      const event = new Constructor('audit', value);
      assert(!event.bubbles && !event.cancelable && !event.composed, Constructor.name + ' flags default');
    }
    for (const value of [1, true, 'init', 1n, Symbol('init')]) {
      let caught;
      try { new Constructor('audit', value); } catch (error) { caught = error; }
      assert(caught instanceof TypeError, Constructor.name + ' primitive dictionary');
    }
    const reads = [];
    const options = new Proxy({}, {
      get(_, key) { reads.push(key); return base.includes(key) ? true : undefined; },
      has() { throw Error('dictionary must use Get'); }
    });
    const event = new Constructor('audit', options);
    assert(reads.join() === members.join(), Constructor.name + ' order: ' + reads);
    assert(event.bubbles && event.cancelable && event.composed, Constructor.name + ' flags');
    for (const member of members) for (const sentinel of [{}, null, undefined, 9]) {
      reads.length = 0;
      const bad = new Proxy({}, { get(_, key) {
        reads.push(key);
        if (key === member) throw sentinel;
        return undefined;
      }});
      let caught = Symbol();
      try { new Constructor('audit', bad); } catch (error) { caught = error; }
      assert(caught === sentinel, Constructor.name + '/' + member + ' original exception');
      assert(reads.join() === members.slice(0, members.indexOf(member) + 1).join(),
        Constructor.name + '/' + member + ' stop on abrupt completion: ' + reads);
    }
  }
  const object = {};
  for (const Constructor of [CommandEvent, ToggleEvent, InterestEvent]) {
    for (const source of [{}, new Proxy(document.body, {})]) {
      let caught;
      try { new Constructor('audit', {source}); } catch (error) { caught = error; }
      assert(caught instanceof TypeError, Constructor.name + ' source brand');
    }
    const source = document.createElement('span');
    Object.setPrototypeOf(source, null);
    assert(new Constructor('audit', {source}).source === source, Constructor.name + ' native identity');
  }
  const transfer = new DataTransfer();
  Object.setPrototypeOf(transfer, null);
  assert(new ClipboardEvent('audit', {clipboardData: transfer}).clipboardData === transfer, 'clipboard native identity');
  for (const clipboardData of [{}, new Proxy(transfer, {})]) {
    let caught;
    try { new ClipboardEvent('audit', {clipboardData}); } catch (error) { caught = error; }
    assert(caught instanceof TypeError, 'clipboard brand');
  }
  for (const [input, expected] of [[-1, 65535], [65536, 0], [1.9, 1], [Infinity, 0], [NaN, 0]])
    assert(new CloseEvent('audit', {code: input}).code === expected, 'close unsigned short');
  const utf16 = '\ud800\0\udc00';
  for (const [Constructor, member] of [[CloseEvent, 'reason'], [CommandEvent, 'command'],
    [ToggleEvent, 'newState'], [ToggleEvent, 'oldState']])
    assert(new Constructor('audit', {[member]: utf16})[member] === utf16, Constructor.name + ' UTF-16');
  assert(new HashChangeEvent('audit', {newURL: '\ud800', oldURL: '\udc00'}).newURL === '\ud800', 'HashChange DOMString');
  assert(new PopStateEvent('audit', {state: object, hasUAVisualTransition: 1}).state === object, 'state any identity');
  assert(new PopStateEvent('audit', {hasUAVisualTransition: 1}).hasUAVisualTransition, 'visual transition bool');
  assert(new PageTransitionEvent('audit', {persisted: []}).persisted, 'persisted bool');
  return true;
})()
