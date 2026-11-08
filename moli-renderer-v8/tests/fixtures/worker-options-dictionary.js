(() => {
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const base = ['credentials', 'name', 'type'];
  const invalidUrl = 'http://[';
  for (const Constructor of [Worker, SharedWorker]) {
    const members = Constructor === Worker ? base : [...base, 'sameSiteCookies'];
    const trace = [];
    const options = {};
    for (const key of members) Object.defineProperty(options, key, {
      configurable: true, get() { trace.push(key); }
    });
    let error;
    try { new Constructor(invalidUrl, options); } catch (caught) { error = caught; }
    assert(error instanceof DOMException && error.name === 'SyntaxError', Constructor.name + ' URL validation');
    assert(trace.join() === members.join(), Constructor.name + ' inheritance order: ' + trace);
    for (const stop of members) for (const sentinel of [undefined, null, false, 0, 'sentinel', {}, new Error('sentinel')]) {
      trace.length = 0;
      const bad = Object.create(options);
      Object.defineProperty(bad, stop, {get() { trace.push(stop); throw sentinel; }});
      let error, threw = false;
      try { new Constructor(invalidUrl, bad); } catch (caught) { error = caught; threw = true; }
      assert(threw && Object.is(error, sentinel), Constructor.name + '/' + stop + ' original exception');
      assert(trace.join() === members.slice(0, members.indexOf(stop) + 1).join(), Constructor.name + '/' + stop + ' short circuit');
    }
    for (const member of ['credentials', 'type', ...(Constructor === SharedWorker ? ['sameSiteCookies'] : [])]) {
      for (const value of [null, 'invalid', Symbol('enum')]) {
        let error;
        try { new Constructor(invalidUrl, {[member]: value}); } catch (caught) { error = caught; }
        assert(error instanceof TypeError, Constructor.name + '/' + member + ' enum conversion');
      }
    }
    const ignored = new Proxy({}, {has() { throw Error('unexpected has'); }, get(_target, key) {
      if (!['credentials', 'name', 'type', 'sameSiteCookies', 'extendedLifetime'].includes(key)) throw Error('unknown member ' + String(key));
    }});
    error = undefined;
    try { new Constructor(invalidUrl, ignored); } catch (caught) { error = caught; }
    assert(error && error.name === 'SyntaxError', Constructor.name + ' only reads dictionary members with Get');
  }
  for (const value of [undefined, null, {}, new String('boxed')]) for (const Constructor of [Worker, SharedWorker]) {
    let error;
    try { new Constructor(invalidUrl, value); } catch (caught) { error = caught; }
    assert(error && error.name === 'SyntaxError', Constructor.name + ' nullish / object dictionary');
  }
  for (const value of [1, true, 'name', 1n, Symbol('name')]) {
    let error;
    try { new Worker(invalidUrl, value); } catch (caught) { error = caught; }
    assert(error instanceof TypeError, 'Worker primitive dictionary rejected');
    error = undefined;
    try { new SharedWorker(invalidUrl, value); } catch (caught) { error = caught; }
    assert(error && (typeof value === 'symbol' ? error instanceof TypeError : error.name === 'SyntaxError'), 'SharedWorker string-or-options discrimination');
  }
  return true;
})()
