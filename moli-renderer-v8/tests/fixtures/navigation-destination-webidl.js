async function navigationDestinationWebIdl(base, targetKind) {
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const tick = () => new Promise(resolve => setTimeout(resolve, 0));
  const frame = document.createElement('iframe');
  frame.src = base + '/common/blank.html';
  await new Promise(resolve => { frame.onload = resolve; document.body.appendChild(frame); });
  await tick();
  const win = targetKind === 'child' ? frame.contentWindow : window;
  const other = targetKind === 'child' ? window : frame.contentWindow;
  const nav = win.navigation;
  const C = win.NavigationDestination;
  const proto = C && C.prototype;
  const TypeErrorCtor = win.TypeError;
  const attributes = ['url', 'key', 'id', 'index', 'sameDocument'];
  const getters = {};
  const rejects = (fn, label, ErrorCtor = TypeErrorCtor) => {
    let error;
    try { fn(); } catch (value) { error = value; }
    check(error instanceof ErrorCtor, label);
  };
  const capture = async (owner, url, options, before) => {
    let destination;
    owner.navigation.addEventListener('navigate', event => {
      destination = event.destination;
      event.intercept(before ? {precommitHandler: controller => before(controller, destination)} : {});
    }, {once: true});
    await owner.navigation.navigate(url, options).finished;
    return destination;
  };
  check(typeof C === 'function', 'Window exposes NavigationDestination');
  if (C) {
    check(C.name === 'NavigationDestination' && C.length === 0, 'constructor name and length');
    rejects(() => new C(), 'illegal constructor');
    rejects(() => C(), 'illegal constructor call');
    check(Object.getPrototypeOf(proto) === win.Object.prototype, 'prototype parent realm');
    const tag = Object.getOwnPropertyDescriptor(proto, Symbol.toStringTag);
    check(tag?.value === 'NavigationDestination' && !tag.writable && !tag.enumerable && tag.configurable, 'prototype tag descriptor');
    for (const name of attributes) {
      const d = Object.getOwnPropertyDescriptor(proto, name);
      check(typeof d?.get === 'function' && d.set === undefined && d.enumerable && d.configurable, name + ' prototype accessor');
      if (d?.get) {
        getters[name] = d.get;
        check(d.get.name === 'get ' + name && d.get.length === 0, name + ' getter metadata');
      }
    }
    const method = Object.getOwnPropertyDescriptor(proto, 'getState');
    check(typeof method?.value === 'function' && method.writable && method.enumerable && method.configurable, 'getState prototype descriptor');
    check(proto.getState.name === 'getState' && proto.getState.length === 0, 'getState metadata');
  }
  const first = await capture(win, '#one', {state: {name: 'one', nested: {value: 1}}});
  const second = await capture(win, '#two', {state: {name: 'two'}});
  for (const [destination, name] of [[first, 'one'], [second, 'two']]) {
    check(Object.getPrototypeOf(destination) === proto, name + ' native prototype');
    check(Object.prototype.toString.call(destination) === '[object NavigationDestination]', name + ' native tag');
    check(Object.getOwnPropertyNames(destination).length === 0, name + ' no own IDL members');
    check(destination.key === '' && destination.id === '' && destination.index === -1, name + ' non-traverse entry fields');
    check(destination.sameDocument && new URL(destination.url).hash === '#' + name, name + ' URL and sameDocument');
  }
  const getState = proto?.getState || first.getState;
  check(getState.call(second).name === 'two', 'getState uses actual receiver');
  const clone = getState.call(first);
  clone.nested.value = 9;
  check(getState.call(first).nested.value === 1, 'getState returns fresh clone');
  const otherDestination = await capture(other, '#other', {state: {name: 'other'}});
  check(getState.call(otherDestination).name === 'other', 'cross-realm genuine receiver');
  check(Object.getPrototypeOf(getState.call(otherDestination)) === win.Object.prototype, 'state deserialized in method realm');
  const borrowed = otherDestination.getState;
  check(borrowed.call(first).name === 'one', 'foreign getState uses receiver');
  check(Object.getPrototypeOf(borrowed.call(first)) === other.Object.prototype, 'foreign state method realm');
  let traps = 0;
  const proxy = new Proxy(first, {get() { traps++; throw 1; }, getPrototypeOf() { traps++; throw 2; }});
  const revoked = Proxy.revocable(first, {}); revoked.revoke();
  const invalid = [{}, Object.create(first), Object.create(proto || Object.prototype), proxy, revoked.proxy, null, undefined, 1];
  for (const [index, value] of invalid.entries()) {
    rejects(() => getState.call(value), 'getState receiver ' + index);
    for (const [name, getter] of Object.entries(getters))
      rejects(() => getter.call(value), name + ' receiver ' + index);
  }
  check(traps === 0, 'receiver checks avoid author Proxy traps');
  const undefinedDestination = await capture(win, '#undefined', {state: undefined});
  check(getState.call(undefinedDestination) === undefined, 'undefined state preserved');
  const nullDestination = await capture(win, '#null', {state: null});
  check(getState.call(nullDestination) === null, 'null state preserved');
  Object.setPrototypeOf(first, null);
  check(getState.call(first).name === 'one', 'native brand survives prototype mutation');
  if (getters.url) check(new URL(getters.url.call(first)).hash === '#one', 'borrowed getter uses native slots');
  const originalGlobal = Object.getOwnPropertyDescriptor(win, 'NavigationDestination');
  let constructorReads = 0;
  Object.defineProperty(win, 'NavigationDestination', {configurable: true, get() { constructorReads++; throw new Error('public constructor read'); }});
  try {
    const native = await capture(win, '#intrinsic', {state: 'intrinsic'});
    check(Object.getPrototypeOf(native) === proto, 'factory keeps intrinsic prototype');
    check(constructorReads === 0, 'factory ignores public constructor');
  } finally {
    if (originalGlobal) Object.defineProperty(win, 'NavigationDestination', originalGlobal);
    else delete win.NavigationDestination;
  }
  const redirected = await capture(win, '#before-redirect', {state: 'before'}, (controller, destination) => {
    Object.freeze(destination);
    controller.redirect('#redirected', {state: {name: 'redirected'}});
    check(new URL(destination.url).hash === '#redirected', 'frozen destination URL reflects redirect');
    check(destination.getState().name === 'redirected', 'frozen destination state reflects redirect');
  });
  check(win.location.hash === '#redirected', 'redirect commits internal URL');
  check(Object.getOwnPropertyNames(redirected).length === 0, 'redirect does not create own IDL fields');
  const target = nav.currentEntry;
  const expected = {key: target.key, id: target.id, index: target.index, url: target.url};
  await capture(win, '#later', {state: 'later'});
  let back;
  nav.addEventListener('navigate', event => {
    back = event.destination;
    check(event.hashChange && event.canIntercept, 'traverse uses internal destination flags');
  }, {once: true});
  const nativeDescriptors = {};
  let destinationReads = 0;
  if (proto) {
    for (const name of ['url', 'sameDocument']) {
      nativeDescriptors[name] = Object.getOwnPropertyDescriptor(proto, name);
      Object.defineProperty(proto, name, {configurable: true, get() {
        destinationReads++;
        throw new Error('public destination getter');
      }});
    }
  }
  try {
    await nav.back().finished;
  } finally {
    for (const [name, descriptor] of Object.entries(nativeDescriptors))
      Object.defineProperty(proto, name, descriptor);
  }
  check(destinationReads === 0, 'traverse ignores public destination getters');
  check(back.key === expected.key && back.id === expected.id && back.index === expected.index, 'traverse entry identity');
  check(getState.call(back).name === 'redirected', 'traverse captures state');
  Object.freeze(back);
  let indexReads = 0;
  Object.defineProperty(target, 'index', {configurable: true, get() { indexReads++; return 9000; }});
  check(back.index === expected.index && indexReads === 0, 'destination index reads native entry state');
  await capture(win, '#replace-target', {history: 'replace', state: 'replacement'});
  check(back.index === -1, 'frozen destination index updates after entry removal');
  check(back.key === expected.key && back.id === expected.id, 'removed entry retains key and id');
  check(getState.call(back).name === 'redirected', 'removed entry keeps captured state');
  if (targetKind === 'child') {
    frame.remove();
    check(back.key === '' && back.id === '' && back.index === -1, 'detached entry fields');
    check(back.url === expected.url, 'detached URL remains available');
    check(getState.call(back).name === 'redirected', 'detached destination keeps state');
  } else {
    frame.remove();
  }
  return {checks, failures};
}
