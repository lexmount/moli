async function probeWindowCrossOriginGetters({role, sameURL, crossURL}) {
  let checks = 0;
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
  const names = ['window', 'self', 'frames', 'top', 'parent', 'opener', 'closed', 'length', 'location'];
  const restricted = ['document', 'history', 'navigation', 'navigator', 'performance', 'frameElement', 'onclick', 'onmouseenter'];
  const read = (realm, target, expected, label) => {
    for (const name of names) {
      const getter = Object.getOwnPropertyDescriptor(realm, name).get;
      equal(observe(() => getter.call(target) === expected[name], realm), true, label + ' ' + name);
    }
  };
  const cross = (realm, target, label) => {
    for (const name of restricted) {
      const getter = Object.getOwnPropertyDescriptor(realm, name).get;
      equal(observe(() => { getter.call(target); return 'returned'; }, realm),
        ['SecurityError', false, true], label + ' denies ' + name);
    }
  };
  // HTML's window/self/frames getters return the realm's GlobalThisValue even
  // after its browsing context is discarded.
  const expected = (target, parentWindow, topWindow, location, closed = false) => ({
    window:target, self:target, frames:target, top:closed ? null : topWindow,
    parent:closed ? null : parentWindow, opener:null, closed, length:closed ? 0 : 2, location
  });
  if (role === 'child') {
    const parentWindow = parent;
    let shadowReads = 0;
    addEventListener('message', event => {
      if (event.data?.kind !== 'window-getter-request') return;
      checks = 0;
      failures.length = 0;
      if (event.data.action === 'parent') {
        read(window, parentWindow, expected(parentWindow, parentWindow, parentWindow, parentWindow.location), 'child to parent');
        cross(window, parentWindow, 'child to parent');
      } else if (event.data.action === 'shadow') {
        for (const name of ['self', 'frames', 'parent', 'opener', 'closed', 'length']) {
          Object.defineProperty(window, name, {
            get() { shadowReads++; return 'author value'; }, configurable:true
          });
        }
      } else if (event.data.action === 'shadow-count') {
        equal(shadowReads, 0, 'borrowed native getters do not invoke author replacements');
      }
      event.source.postMessage({kind:'window-getter-result', checks, failures}, '*');
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
  const ask = (action) => new Promise(resolve => {
    const listener = event => {
      if (event.source !== remote || event.data?.kind !== 'window-getter-result') return;
      removeEventListener('message', listener);
      checks += event.data.checks;
      failures.push(...event.data.failures);
      resolve();
    };
    addEventListener('message', listener);
    remote.postMessage({kind:'window-getter-request', action}, '*');
  });
  let traps = 0;
  const handler = new Proxy({}, {get() { traps++; throw new Error('unexpected trap'); }});
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
  for (const [label, realm] of [['parent', window], ['child', local]]) {
    read(realm, window, expected(window, window, window, location), label + ' to main');
    read(realm, local, expected(local, window, window, local.location), label + ' to same-origin child');
    read(realm, remote, expected(remote, window, window, remote.location), label + ' to cross-origin child');
    cross(realm, remote, label + ' to cross-origin child');
    for (const name of names) {
      const getter = Object.getOwnPropertyDescriptor(realm, name).get;
      for (const [kind, receiver] of invalid) {
        equal(observe(() => { getter.call(receiver); return 'returned'; }, realm),
          ['TypeError', true, false], label + ' ' + name + ' rejects ' + kind);
      }
      for (const receiver of [null, undefined]) {
        equal(observe(() => getter.call(receiver) === realm[name], realm), true, label + ' ' + name + ' global default receiver');
      }
    }
    const setter = Object.getOwnPropertyDescriptor(realm, 'location').set;
    equal(observe(() => setter.call(remote, Symbol('invalid URL')), realm),
      ['TypeError', true, false], label + ' location conversion error realm');
  }
  equal(traps, 0, 'brand checks invoke no author getters or Proxy traps');
  await ask('parent');
  await ask('shadow');
  read(window, remote, expected(remote, window, window, remote.location), 'ignores author replacements');
  await ask('shadow-count');
  let conversions = 0;
  await new Promise(resolve => {
    remoteFrame.onload = resolve;
    Object.getOwnPropertyDescriptor(local, 'location').set.call(remote, {
      toString() { conversions++; return sameURL; }
    });
  });
  equal(conversions, 1, 'cross-origin location setter converts once');
  equal(remoteFrame.contentWindow === remote, true, 'setter navigation preserves WindowProxy');
  read(local, remote, expected(remote, window, window, remote.location), 'same-origin after setter navigation');
  await navigate(remoteFrame, crossURL);
  equal(remoteFrame.contentWindow === remote, true, 'cross-origin navigation preserves WindowProxy');
  const oldLocation = remote.location;
  read(local, remote, expected(remote, window, window, oldLocation), 'cross-origin after navigation');
  cross(local, remote, 'cross-origin after navigation');
  remoteFrame.remove();
  read(local, remote, expected(remote, null, null, oldLocation, true), 'removed Window');
  await navigate(remoteFrame, crossURL);
  equal(remoteFrame.contentWindow !== remote, true, 'reinserted iframe creates a new WindowProxy');
  read(local, remote, expected(remote, null, null, oldLocation, true), 'old Window stays discarded');
  const replacement = remoteFrame.contentWindow;
  read(local, replacement, expected(replacement, window, window, replacement.location), 'replacement Window');
  remoteFrame.remove();
  localFrame.remove();
  return {checks, failures};
}
