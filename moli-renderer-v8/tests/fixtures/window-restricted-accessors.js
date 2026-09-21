async function probeWindowRestrictedAccessors({sameURL, crossURL}) {
  let checks = 0, traps = 0, conversions = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
  };
  const observe = (callback, realm) => {
    try { return callback(); }
    catch (error) {
      return [error.name, error instanceof realm.TypeError, error instanceof realm.DOMException];
    }
  };
  const frame = async url => {
    const element = document.createElement('iframe');
    const loaded = new Promise(resolve => element.onload = resolve);
    element.src = url;
    document.body.append(element);
    await loaded;
    return element;
  };
  const localFrame = await frame(sameURL), remoteFrame = await frame(crossURL);
  const local = localFrame.contentWindow, remote = remoteFrame.contentWindow;
  const getters = ['event', 'innerWidth', 'outerHeight', 'scrollX', 'screenLeft', 'screenTop',
    'screenX', 'screenY', 'origin', 'name', 'status', 'localStorage', 'sessionStorage', 'caches', 'trustedTypes'];
  const replaceable = ['self', 'parent', 'frames', 'event', 'innerWidth', 'outerHeight', 'navigation',
    'screen', 'performance', 'visualViewport', 'scrollX', 'length', 'origin',
    'screenLeft', 'screenTop', 'screenX', 'screenY'];
  const setters = [...replaceable, 'name', 'status'];
  const realms = [['parent', window], ['child', local]].map(([label, realm]) => ({
    label, realm, descriptors:Object.fromEntries([...new Set([...getters, ...setters])]
      .map(name => [name, Object.getOwnPropertyDescriptor(realm, name)]))
  }));
  const handler = new Proxy({}, {get() { traps++; throw new Error('unexpected Proxy trap'); }});
  const revoked = Proxy.revocable(remote, {});
  revoked.revoke();
  const invalid = [
    ['plain', {}], ['inherited', Object.create(window)], ['cross-inherited', Object.create(remote)],
    ['prototype', Object.create(Window.prototype)], ['proxy', new Proxy(window, handler)],
    ['cross-proxy', new Proxy(remote, handler)], ['revoked', revoked.proxy],
    ['forged bridge', {__moliNativeBridge:window.__moliNativeBridge}],
    ['bridge getter', {get __moliNativeBridge() { traps++; throw new Error('unexpected bridge getter'); }}],
    ['document', document], ['primitive', 1]
  ];
  const input = {toString() { conversions++; return 'converted'; }};
  // First access borrows a getter from the other realm, before the factory exists.
  for (const name of ['caches', 'trustedTypes']) {
    for (const {label, realm, descriptors} of realms) {
      const target = realm === window ? local : window;
      const value = descriptors[name].get.call(target);
      equal(value instanceof target.Object, true, label + ' ' + name + ' receiver realm');
      equal(value instanceof realm.Object, false, label + ' ' + name + ' is not created in getter realm');
      equal(value === target[name], true, label + ' ' + name + ' SameObject');
    }
  }
  for (const {label, realm, descriptors} of realms) {
    for (const name of getters) {
      const getter = descriptors[name].get;
      equal(observe(() => { getter.call(remote); return 'returned'; }, realm),
        ['SecurityError', false, true], label + ' ' + name + ' rejects cross-origin');
      for (const [kind, receiver] of invalid) {
        equal(observe(() => { getter.call(receiver); return 'returned'; }, realm),
          ['TypeError', true, false], label + ' ' + name + ' rejects ' + kind);
      }
      for (const target of [window, local]) {
        equal(getter.call(target) === target[name], true, label + ' ' + name + ' genuine receiver');
      }
      for (const receiver of [null, undefined]) {
        equal(getter.call(receiver) === realm[name], true, label + ' ' + name + ' global default receiver');
      }
    }
    for (const name of setters) {
      const setter = descriptors[name].set;
      for (const value of [input, Symbol('not coercible')]) {
        equal(observe(() => { setter.call(remote, value); return 'returned'; }, realm),
          ['SecurityError', false, true], label + ' ' + name + ' rejects cross-origin before conversion');
      }
      for (const [kind, receiver] of invalid) {
        equal(observe(() => { setter.call(receiver, input); return 'returned'; }, realm),
          ['TypeError', true, false], label + ' ' + name + ' setter rejects ' + kind);
      }
    }
  }
  equal(conversions, 0, 'denied setters never convert their arguments');
  equal(traps, 0, 'receiver checks invoke no author getters or Proxy traps');
  for (const {label, realm, descriptors} of realms) {
    for (const name of replaceable) {
      for (const receiver of [window, local, null, undefined]) {
        const target = receiver ?? realm;
        const original = Object.getOwnPropertyDescriptor(target, name);
        const before = descriptors[name].get.call(target);
        descriptors[name].set.call(receiver, input);
        const actual = Object.getOwnPropertyDescriptor(target, name);
        equal([actual.value === input, actual.writable, actual.enumerable, actual.configurable],
          [true, true, true, true], label + ' ' + name + ' creates an own data property');
        equal(descriptors[name].get.call(target) === before, true, label + ' ' + name + ' saved getter keeps native value');
        Object.defineProperty(target, name, original);
      }
    }
    for (const name of ['name', 'status']) {
      for (const target of [window, local]) {
        const original = target[name];
        const setter = descriptors[name].set;
        setter.call(target, input);
        equal(target[name], 'converted', label + ' ' + name + ' converts on a genuine receiver');
        const sentinel = new realm.Error('conversion failed');
        let caught;
        try { setter.call(target, {toString() { throw sentinel; }}); } catch (error) { caught = error; }
        equal(caught === sentinel, true, label + ' ' + name + ' propagates conversion exception');
        equal(target[name], 'converted', label + ' ' + name + ' does not mutate after a thrown conversion');
        equal(observe(() => setter.call(target, Symbol('not coercible')), realm),
          ['TypeError', true, false], label + ' ' + name + ' Symbol error belongs to callee realm');
        equal(target[name], 'converted', label + ' ' + name + ' does not mutate after Symbol conversion');
        setter.call(target, original);
      }
    }
  }
  equal(conversions, 8, 'only genuine name and status setters convert arguments');
  const parentEvent = new Event('outer-restricted-probe');
  const childEvent = new local.Event('inner-restricted-probe');
  const parentGetter = realms[0].descriptors.event.get;
  const childGetter = realms[1].descriptors.event.get;
  const previousParentEvent = parentGetter.call(window);
  const previousChildEvent = childGetter.call(local);
  window.__restrictedEventProbe = () => {
    equal(parentGetter.call(local) === childEvent, true, 'parent getter reads child current event');
    equal(childGetter.call(window) === parentEvent, true, 'child getter reads parent current event');
  };
  local.eval("addEventListener('inner-restricted-probe', () => parent.__restrictedEventProbe(), {once:true})");
  window.addEventListener('outer-restricted-probe', () => {
    local.dispatchEvent(childEvent);
    equal(childGetter.call(local) === previousChildEvent, true, 'child current event restored after dispatch');
    equal(parentGetter.call(window) === parentEvent, true, 'parent current event survives child dispatch');
  }, {once:true});
  window.dispatchEvent(parentEvent);
  equal(parentGetter.call(window) === previousParentEvent, true, 'parent current event restored after dispatch');
  delete window.__restrictedEventProbe;
  remoteFrame.remove();
  for (const {label, realm, descriptors} of realms) {
    for (const name of getters) {
      equal(observe(() => { descriptors[name].get.call(remote); return 'returned'; }, realm),
        ['SecurityError', false, true], label + ' ' + name + ' still denies discarded cross-origin Window');
    }
  }
  localFrame.remove();
  return {checks, failures};
}
