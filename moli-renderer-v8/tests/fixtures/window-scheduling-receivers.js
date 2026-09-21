async function probeWindowSchedulingReceivers({sameURL, crossURL}) {
  let checks = 0, conversions = 0, traps = 0, deniedCallbacks = 0, acceptedMicrotasks = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
  };
  const observe = (callback, realm) => {
    try { callback(); return 'returned'; }
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
  const methods = ['setTimeout', 'setInterval', 'clearTimeout', 'clearInterval',
    'requestAnimationFrame', 'cancelAnimationFrame', 'requestIdleCallback', 'cancelIdleCallback',
    'queueMicrotask'];
  const cancellations = {setTimeout:'clearTimeout', setInterval:'clearInterval',
    requestAnimationFrame:'cancelAnimationFrame', requestIdleCallback:'cancelIdleCallback'};
  const realms = [['parent', window], ['child', local]].map(([label, realm]) => ({
    label, realm, functions:Object.fromEntries(methods.map(name => [name, realm[name]]))
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
  const deniedCallback = () => { deniedCallbacks++; };
  const number = {valueOf() { conversions++; return 2147483647; }};
  const source = {toString() { conversions++; return ''; }};
  const options = {get timeout() { conversions++; return 2147483647; }};
  const argumentsFor = name => {
    if (name === 'setTimeout' || name === 'setInterval') {
      return [[source, number], [deniedCallback, number], [Symbol('source')], []];
    }
    if (name === 'requestIdleCallback') return [[deniedCallback, options], [null], []];
    if (name === 'requestAnimationFrame' || name === 'queueMicrotask') return [[deniedCallback], [null], []];
    return [[number], [Symbol('handle')], []];
  };
  for (const {label, realm, functions} of realms) {
    for (const name of methods) {
      for (const [kind, receiver, expected] of [
        ['cross-origin', remote, ['SecurityError', false, true]],
        ...invalid.map(([kind, receiver]) => [kind, receiver, ['TypeError', true, false]])
      ]) {
        for (const args of argumentsFor(name)) {
          equal(observe(() => {
            const id = functions[name].apply(receiver, args);
            // Keep an unfixed baseline bounded if it incorrectly schedules work.
            if (cancellations[name]) functions[cancellations[name]].call(realm, id);
          }, realm), expected, label + ' ' + name + ' rejects ' + kind + ' with ' + args.length + ' arguments');
        }
      }
    }
  }
  equal(conversions, 0, 'denied receivers are checked before string, number, or dictionary conversion');
  equal(traps, 0, 'receiver checks invoke no author getters or Proxy traps');
  await Promise.resolve();
  equal(deniedCallbacks, 0, 'denied calls do not enqueue microtasks or callbacks');

  for (const {label, realm, functions} of realms) {
    for (const receiver of [window, local, null, undefined]) {
      const target = receiver ?? realm;
      for (const name of methods) {
        const callback = new Proxy(() => {
          if (name === 'queueMicrotask') acceptedMicrotasks++;
          else deniedCallbacks++;
        }, {});
        const args = name === 'setTimeout' || name === 'setInterval' ? [callback, 2147483647] :
          name === 'requestAnimationFrame' || name === 'requestIdleCallback' || name === 'queueMicrotask' ?
            [callback] : [2147483647];
        equal(observe(() => {
          const id = functions[name].apply(receiver, args);
          if (cancellations[name]) {
            equal(Number.isInteger(id) && id > 0, true, label + ' ' + name + ' returns a timer handle');
            target[cancellations[name]](id);
          } else {
            equal(id, undefined, label + ' ' + name + ' returns undefined');
          }
        }, realm), 'returned', label + ' ' + name + ' accepts a genuine or nullish receiver');
      }
    }
  }
  await Promise.resolve();
  equal(acceptedMicrotasks, 8, 'accepted callable Proxies run as microtasks');
  equal(deniedCallbacks, 0, 'cancelled work does not execute');
  remoteFrame.remove();
  for (const {label, realm, functions} of realms) {
    for (const name of methods) {
      equal(observe(() => functions[name].call(remote), realm), ['SecurityError', false, true],
        label + ' ' + name + ' still denies a discarded cross-origin Window');
    }
  }
  localFrame.remove();
  return {checks, failures};
}
