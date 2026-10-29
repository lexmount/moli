(() => {
  const assert = (value, label) => { if (!value) throw Error(label); };
  let destination;
  const capture = e => { destination = e.destination; e.preventDefault(); };
  navigation.addEventListener('navigate', capture, {once: true});
  const result = navigation.navigate(location.href + '#dictionary');
  result.committed.catch(() => {}); result.finished.catch(() => {});
  assert(destination, 'genuine destination captured');
  const signal = new AbortController().signal, from = navigation.currentEntry;
  const formData = new FormData(), sourceElement = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  const base = ['bubbles', 'cancelable', 'composed'];
  const cases = [
    [NavigationCurrentEntryChangeEvent, ['from', 'navigationType'], {from}],
    [NavigateEvent, ['canIntercept', 'destination', 'downloadRequest', 'formData',
      'hasUAVisualTransition', 'hashChange', 'info', 'navigationType', 'signal', 'sourceElement', 'userInitiated'], {destination, signal}]
  ];
  for (const [C, own, values] of cases) {
    assert(C.length === 2, C.name + ' arity');
    let converted = false, caught;
    try { new C({toString() {converted = true; throw 7;}}); } catch (e) { caught = e; }
    assert(caught instanceof TypeError && !converted, C.name + ' required dictionary before type conversion');
    const members = [...base, ...own], reads = [];
    const options = new Proxy({}, {get(_, key) {
      reads.push(key); return key in values ? values[key] : undefined;
    }, has() {throw Error('Get required');}});
    new C('audit', options);
    assert(reads.join() === members.join(), C.name + ' order: ' + reads);
    for (const member of members) for (const sentinel of [{}, null, undefined, 7]) {
      reads.length = 0; let caught = Symbol();
      try { new C('audit', new Proxy({}, {get(_, key) {
        reads.push(key); if(key === member) throw sentinel;
        return key in values ? values[key] : undefined;
      }})); } catch(e) { caught = e; }
      assert(caught === sentinel, C.name + '/' + member + ' exception identity');
      assert(reads.join() === members.slice(0, members.indexOf(member) + 1).join(), C.name + '/' + member + ' abrupt completion');
    }
    for (const value of [1, true, 'init', 1n, Symbol()]) {
      let caught; try { new C('audit', value); } catch(e) { caught = e; }
      assert(caught instanceof TypeError, C.name + ' primitive dictionary');
    }
    for (const navigationType of ['bad', null]) {
      if (navigationType === null && C === NavigationCurrentEntryChangeEvent) continue;
      let caught; try { new C('audit', {...values, navigationType}); } catch(e) { caught = e; }
      assert(caught instanceof TypeError, C.name + ' enum conversion');
    }
  }
  for (const [C, member, valid, values] of [
    [NavigationCurrentEntryChangeEvent, 'from', from, {}],
    [NavigateEvent, 'destination', destination, {signal}], [NavigateEvent, 'signal', signal, {destination}],
    [NavigateEvent, 'formData', formData, {destination,signal}], [NavigateEvent, 'sourceElement', sourceElement, {destination,signal}]
  ]) {
    for (const value of [{}, new Proxy(valid, {})]) {
      let caught; try { new C('audit', {...values, [member]: value}); } catch(e) { caught = e; }
      assert(caught instanceof TypeError, C.name + '/' + member + ' native brand');
    }
    Object.setPrototypeOf(valid, null);
    assert(new C('audit', {...values, [member]: valid})[member] === valid, C.name + '/' + member + ' native identity');
  }
  const info = {}, utf16 = '\ud800\0\udc00';
  const event = new NavigateEvent('audit', {destination, signal, info, downloadRequest: utf16, formData, sourceElement});
  assert(event.info === info && event.downloadRequest === utf16 && event.formData === formData && event.sourceElement === sourceElement, 'native and any payload');
  assert(new NavigationCurrentEntryChangeEvent('audit', {from}).navigationType === null, 'nullable navigation type default');
  let caught; try {event.intercept();} catch(e) {caught=e;}
  assert(caught && caught.name === 'SecurityError', 'synthetic event remains unable to intercept navigation');
  return true;
})()
