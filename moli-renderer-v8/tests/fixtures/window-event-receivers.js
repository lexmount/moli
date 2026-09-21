async function probeWindowEventReceivers({role, sameURL, crossURL}) {
  let checks = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
  };
  const observe = (callback, realm) => {
    try { return callback() === undefined ? 'undefined' : 'other'; }
    catch (error) {
      return [error.name, error instanceof realm.TypeError, error instanceof realm.DOMException];
    }
  };
  const names = ['onmouseenter', 'onmouseleave', 'onclick', 'onerror', 'onmessageerror',
    'onunhandledrejection', 'onrejectionhandled'];
  const cross = (realm, receiver, label) => {
    for (const name of names) {
      const d = Object.getOwnPropertyDescriptor(realm, name);
      equal(observe(() => d.get.call(receiver), realm), ['SecurityError', false, true], label + ' ' + name + ' get');
      equal(observe(() => d.set.call(receiver, () => {}), realm), ['SecurityError', false, true], label + ' ' + name + ' set');
    }
  };
  if (role === 'child') {
    addEventListener('message', event => {
      if (event.data?.kind !== 'event-receiver-request') return;
      cross(window, parent, 'child to parent');
      event.source.postMessage({kind:'event-receiver-result', checks, failures}, '*');
    });
    return;
  }
  const navigate = (frame, url) => new Promise(resolve => {
    frame.onload = resolve;
    frame.src = url;
    if (!frame.isConnected) document.body.append(frame);
  });
  const localFrame = document.createElement('iframe');
  const remoteFrame = document.createElement('iframe');
  await navigate(localFrame, sameURL);
  await navigate(remoteFrame, crossURL);
  const local = localFrame.contentWindow;
  const remote = remoteFrame.contentWindow;
  let traps = 0;
  const handler = new Proxy({}, {get() { traps++; throw new Error('unexpected Proxy trap'); }});
  const revoked = Proxy.revocable(window, {});
  revoked.revoke();
  const forged = {__moliNativeBridge: window.__moliNativeBridge};
  const bridgeGetter = {get __moliNativeBridge() { traps++; throw new Error('unexpected bridge getter'); }};
  const invalid = [
    ['plain', {}], ['inherited', Object.create(window)], ['prototype', Object.create(Window.prototype)],
    ['proxy', new Proxy(window, handler)], ['revoked', revoked.proxy], ['forged', forged],
    ['bridge getter', bridgeGetter], ['primitive', 1], ['document', document],
    ['element', document.body], ['cross-origin proxy', new Proxy(remote, handler)],
    ['cross-origin prototype', Object.create(remote)]
  ];
  for (const [label, realm] of [['parent', window], ['child', local]]) {
    for (const name of names) {
      const d = Object.getOwnPropertyDescriptor(realm, name);
      const sentinel = () => {};
      realm[name] = sentinel;
      const lenient = name === 'onmouseenter' || name === 'onmouseleave';
      const expected = lenient ? 'undefined' : ['TypeError', true, false];
      for (const [kind, receiver] of invalid) {
        const prefix = label + ' ' + name + ' ' + kind;
        equal(observe(() => d.get.call(receiver), realm), expected, prefix + ' get');
        equal(observe(() => d.set.call(receiver, () => {}), realm), expected, prefix + ' set');
        equal(observe(() => d.set.call(receiver), realm), expected, prefix + ' missing value');
      }
      equal(realm[name] === sentinel, true, label + ' ' + name + ' invalid writes leave handler unchanged');
      realm[name] = null;
      for (const [kind, receiver, target] of [
        ['main', window, window], ['child', local, local],
        ['null', null, realm], ['undefined', undefined, realm]
      ]) {
        const other = target === window ? local : window;
        const callback = () => {};
        const prefix = label + ' ' + name + ' valid ' + kind;
        equal(d.get.call(receiver), null, prefix + ' initially empty');
        equal(d.set.call(receiver, callback), undefined, prefix + ' set result');
        equal(d.get.call(receiver) === callback && target[name] === callback, true, prefix + ' receiver owns handler');
        equal(other[name], null, prefix + ' other Window unchanged');
        d.set.call(receiver, null);
        equal(d.get.call(receiver), null, prefix + ' cleared');
      }
    }
    cross(realm, remote, label + ' to cross-origin child');
    for (const name of ['frameElement', 'navigator']) {
      const d = Object.getOwnPropertyDescriptor(realm, name);
      equal(observe(() => d.get.call(forged), realm), ['TypeError', true, false], label + ' shared brand ' + name);
    }
  }
  equal(traps, 0, 'receiver validation invokes no author getters or Proxy traps');
  await new Promise(resolve => {
    const listener = event => {
      if (event.source !== remote || event.data?.kind !== 'event-receiver-result') return;
      removeEventListener('message', listener);
      checks += event.data.checks;
      failures.push(...event.data.failures);
      resolve();
    };
    addEventListener('message', listener);
    remote.postMessage({kind:'event-receiver-request'}, '*');
  });
  await navigate(remoteFrame, sameURL);
  equal(remoteFrame.contentWindow === remote, true, 'same-origin navigation preserves WindowProxy');
  for (const name of names) {
    const d = Object.getOwnPropertyDescriptor(window, name);
    const callback = () => {};
    d.set.call(remote, callback);
    equal(d.get.call(remote) === callback && remote[name] === callback, true, name + ' access allowed after navigation');
    d.set.call(remote, null);
  }
  await navigate(remoteFrame, crossURL);
  equal(remoteFrame.contentWindow === remote, true, 'cross-origin navigation preserves WindowProxy');
  cross(window, remote, 'cross-origin navigation revokes access');
  remoteFrame.remove();
  localFrame.remove();
  return {checks, failures};
}
