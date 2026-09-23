async function fontFaceSetBindingsProbe(realm, fonts) {
  const failures = [];
  let checks = 0;
  const check = (condition, label) => {
    ++checks;
    if (!condition) failures.push(label);
  };
  const throws = (callback, expected, label) => {
    try { callback(); check(false, label + ': accepted'); }
    catch (error) { check(error instanceof expected, label + ': exception realm'); }
  };
  const rejects = async (callback, expected, label) => {
    let promise;
    try { promise = callback(); }
    catch (_) { check(false, label + ': synchronous exception'); return; }
    check(promise instanceof realm.Promise, label + ': promise realm');
    try { await promise; check(false, label + ': fulfilled'); }
    catch (error) { check(error instanceof expected, label + ': exception realm'); }
  };
  // Use the intrinsic prototype even in browsers that do not expose the
  // FontFaceSet interface object as a global.
  const prototype = realm.Object.getPrototypeOf(fonts);
  check(realm.Object.getPrototypeOf(prototype) === realm.EventTarget.prototype, 'prototype inheritance');
  check(fonts instanceof realm.EventTarget, 'EventTarget identity');
  if (realm.FontFaceSet) {
    check(realm.Object.getPrototypeOf(realm.FontFaceSet) === realm.EventTarget, 'constructor inheritance');
    throws(() => new realm.FontFaceSet(), realm.TypeError, 'illegal zero-argument constructor');
  }
  for (const name of ['addEventListener', 'removeEventListener', 'dispatchEvent']) {
    check(!Object.hasOwn(prototype, name), 'inherited ' + name);
    check(fonts[name] === realm.EventTarget.prototype[name], 'shared ' + name);
  }

  const face = new realm.FontFace('BindingsProbe', 'url(unused-font.ttf)');
  let conversions = 0;
  let traps = 0;
  const query = {toString() { ++conversions; return '12px BindingsProbe'; }};
  const revoked = Proxy.revocable(fonts, {});
  revoked.revoke();
  const invalid = [null, undefined, 1, {}, Object.create(prototype), Object.create(fonts),
    new Proxy(fonts, {get() { ++traps; }, getPrototypeOf() { ++traps; return prototype; }}), revoked.proxy];
  const operations = [
    ['add', value => prototype.add.call(value, face)],
    ['has', value => prototype.has.call(value, face)],
    ['delete', value => prototype.delete.call(value, face)],
    ['clear', value => prototype.clear.call(value)],
    ['check', value => prototype.check.call(value, query)],
    ['keys', value => prototype.keys.call(value)],
    ['values', value => prototype.values.call(value)],
    ['entries', value => prototype.entries.call(value)],
    ['forEach', value => prototype.forEach.call(value, () => { ++conversions; })],
    ['addEventListener', value => fonts.addEventListener.call(value, query, () => {})],
    ['removeEventListener', value => fonts.removeEventListener.call(value, query, () => {})],
    ['dispatchEvent', value => fonts.dispatchEvent.call(value, new realm.Event('probe'))],
  ];
  for (const name of ['status', 'size', 'onloading', 'onloadingdone', 'onloadingerror']) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    operations.push(['get ' + name, value => descriptor.get.call(value)]);
    if (descriptor.set) operations.push(['set ' + name, value => descriptor.set.call(value, () => {})]);
  }
  const ready = Object.getOwnPropertyDescriptor(prototype, 'ready').get;
  for (const value of invalid) {
    for (const [name, operation] of operations) {
      // EventTarget has Global descendants: null/undefined this selects the
      // callback realm's global, which is a valid EventTarget.
      if (value == null && ['addEventListener', 'removeEventListener', 'dispatchEvent'].includes(name)) continue;
      throws(() => operation(value), realm.TypeError, name);
    }
    await rejects(() => prototype.load.call(value, query), realm.TypeError, 'load receiver');
    await rejects(() => ready.call(value), realm.TypeError, 'ready receiver');
  }
  check(conversions === 0, 'receiver validation before conversion');
  check(traps === 0, 'receiver validation without proxy traps');
  check(fonts.ready === ready.call(fonts), 'ready promise identity');
  await rejects(() => prototype.load.call(fonts), realm.TypeError, 'load required argument');
  await rejects(() => prototype.load.call(fonts, Symbol()), realm.TypeError, 'load string conversion');
  const marker = {};
  let conversionPromise;
  try {
    conversionPromise = prototype.load.call(fonts, {toString() { throw marker; }});
  } catch (_) { check(false, 'load conversion threw synchronously'); }
  if (conversionPromise) {
    check(conversionPromise instanceof realm.Promise, 'conversion promise realm');
    check(await conversionPromise.catch(error => error) === marker, 'conversion exception identity');
  }

  const methods = realm.EventTarget.prototype;
  const calls = [];
  const listener = event => {
    calls.push('listener');
    check(event.target === fonts && event.currentTarget === fonts, 'dispatch target');
    check(event.eventPhase === realm.Event.AT_TARGET, 'dispatch phase');
    throws(() => methods.dispatchEvent.call(fonts, event), realm.DOMException, 'recursive dispatch');
  };
  methods.addEventListener.call(fonts, 'loading', listener);
  fonts.onloading = () => calls.push('old handler');
  methods.addEventListener.call(fonts, 'loading', () => calls.push('once'), {once: true});
  fonts.onloading = () => { calls.push('handler'); return false; };
  methods.addEventListener.call(fonts, 'loading', () => calls.push('capture'), true);
  const controller = new realm.AbortController();
  methods.addEventListener.call(fonts, 'loading', () => check(false, 'aborted listener'), {signal: controller.signal});
  controller.abort();
  const event = new realm.Event('loading', {cancelable: true});
  check(methods.dispatchEvent.call(fonts, event) === false, 'event cancellation');
  check(calls.join() === 'capture,listener,handler,once', 'event handler order: ' + calls);
  check(event.currentTarget === null && event.eventPhase === 0, 'dispatch cleanup');
  methods.removeEventListener.call(fonts, 'loading', listener);
  fonts.onloading = null;
  calls.length = 0;
  check(fonts.dispatchEvent(new realm.Event('loading')), 'uncanceled dispatch');
  check(calls.join() === 'capture', 'shared removal and once');
  const fakeEvent = {get type() { ++traps; return 'loading'; }};
  throws(() => methods.dispatchEvent.call(fonts, fakeEvent), realm.TypeError, 'unbranded event');
  throws(() => methods.dispatchEvent.call(fonts, new Proxy(event, {})), realm.TypeError, 'proxy event');
  check(traps === 0, 'unbranded event properties not read');

  // The event interface is shared by script-created and native loading events.
  if (realm.FontFaceSetLoadEvent) {
    const getFaces = Object.getOwnPropertyDescriptor(realm.FontFaceSetLoadEvent.prototype, 'fontfaces').get;
    const loadEvent = new realm.FontFaceSetLoadEvent('loadingdone', {fontfaces: [face]});
    check(getFaces.call(loadEvent) === loadEvent.fontfaces && Object.isFrozen(loadEvent.fontfaces), 'frozen SameObject faces');
    for (const value of [{}, Object.create(loadEvent), new realm.Event('loadingdone'), new Proxy(loadEvent, {})]) {
      throws(() => getFaces.call(value), realm.TypeError, 'fontfaces receiver');
    }
  }
  return {checks, failures};
}
