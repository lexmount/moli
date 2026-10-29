(async () => {
  const assert = (value, label) => { if (!value) throw Error(label); };
  const base = ['bubbles', 'cancelable', 'composed'];
  const formData = new FormData(), submitter = document.createElement('button');
  const track = document.createElement('video').addTextTrack('subtitles');
  const promise = Promise.resolve(4);
  const cases = [
    [PromiseRejectionEvent, ['promise', 'reason'], {promise}, 2],
    [FormDataEvent, ['formData'], {formData}, 2],
    [SubmitEvent, ['submitter'], {submitter}, 1],
    [TrackEvent, ['track'], {track}, 1]
  ];
  for (const [C, own, values, length] of cases) {
    const members = [...base, ...own];
    assert(C.length === length, C.name + ' length');
    if (length === 2) {
      let converted = false, caught;
      try { new C({toString() {converted = true; throw 7;}}); } catch (e) { caught = e; }
      assert(caught instanceof TypeError && !converted, C.name + ' arity before type');
    }
    const reads = [];
    const options = new Proxy({}, {get(_, key) {
      if (members.includes(key)) reads.push(key);
      return key in values ? values[key] : undefined;
    }, has() {throw Error('Get required');}});
    const event = new C('audit', options);
    assert(reads.join() === members.join(), C.name + ' order: ' + reads);
    for (const member of members) for (const sentinel of [{}, null, undefined, 7]) {
      reads.length = 0;
      let caught = Symbol();
      try { new C('audit', new Proxy({}, {get(_, key) {
        if (members.includes(key)) reads.push(key);
        if (key === member) throw sentinel;
        return key in values ? values[key] : undefined;
      }})); } catch (e) { caught = e; }
      assert(caught === sentinel, C.name + '/' + member + ' exception identity');
      assert(reads.join() === members.slice(0, members.indexOf(member) + 1).join(), C.name + '/' + member + ' stop');
    }
    if (C !== PromiseRejectionEvent) for (const member of own) {
      for (const bad of [{}, new Proxy(values[member], {})]) {
        let caught;
        try { new C('audit', {...values, [member]: bad}); } catch (e) { caught = e; }
        assert(caught instanceof TypeError, C.name + '/' + member + ' native brand');
      }
      Object.setPrototypeOf(values[member], null);
      assert(new C('audit', values)[member] === values[member], C.name + '/' + member + ' identity');
    }
  }
  const reason = {};
  assert(new PromiseRejectionEvent('audit', {promise, reason}).reason === reason, 'reason any identity');
  assert(new PromiseRejectionEvent('audit', {promise}).reason === undefined, 'reason default');
  assert(new PromiseRejectionEvent('audit', {promise}).promise === promise, 'Promise identity');
  Object.defineProperty(promise, 'constructor', {get() {throw Error('must not get constructor');}});
  assert(new PromiseRejectionEvent('audit', {promise}).promise === promise, 'native Promise ignores public constructor');
  let thenRead = 0;
  const resolved = new PromiseRejectionEvent('audit', {promise: {get then() {thenRead++; throw reason;}}}).promise;
  assert(thenRead === 1 && await resolved.then(() => false, e => e === reason), 'thenable rejection');
  assert(new TrackEvent('audit').track === null && new SubmitEvent('audit').submitter === null, 'nullable payload defaults');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  let caught;
  try { new SubmitEvent('audit', {submitter: svg}); } catch (e) { caught = e; }
  assert(caught instanceof TypeError, 'submitter requires HTMLElement');
  return true;
})()
