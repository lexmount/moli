async function probeWindowSyncMethodReceivers({sameURL, crossURL}) {
  let checks = 0, conversions = 0, traps = 0, reported = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
  };
  const observe = (callback, realm) => {
    try { callback(); return 'returned'; }
    catch (error) { return [error.name, error instanceof realm.TypeError, error instanceof realm.DOMException]; }
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
  const methods = ['getComputedStyle', 'getSelection', 'matchMedia', 'btoa', 'atob',
    'structuredClone', 'stop', 'find', 'captureEvents',
    'releaseEvents', 'print', 'alert', 'confirm', 'prompt', 'open', 'reportError',
    'addEventListener', 'removeEventListener', 'dispatchEvent'];
  const eventMethods = new Set(['addEventListener', 'removeEventListener', 'dispatchEvent']);
  const dialogs = new Set(['alert', 'confirm', 'prompt', 'open']);
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
  const sentinel = new Error('argument conversion reached');
  // Throw during conversion so an unfixed baseline never opens a dialog or navigates.
  const throwingInput = {toString() { conversions++; throw sentinel; }};
  const string = {toString() { conversions++; return 'YQ=='; }};
  const cloneInput = {get value() { conversions++; return 'copied'; }};
  const cloneOptions = {get transfer() { conversions++; return []; }};
  const listenerOptions = {get capture() { conversions++; return false; }};
  const callback = () => {};
  const argsFor = name => {
    if (dialogs.has(name)) return [[throwingInput], [Symbol('message')]];
    if (name === 'getComputedStyle') return [[], [document.documentElement, string], [null]];
    if (name === 'matchMedia' || name === 'btoa' || name === 'atob') return [[], [string], [Symbol('value')]];
    if (name === 'structuredClone') return [[], [cloneInput, cloneOptions]];
    if (name === 'reportError') return [[], [sentinel]];
    if (name === 'addEventListener' || name === 'removeEventListener') return [[], [string, callback, listenerOptions]];
    if (name === 'dispatchEvent') return [[], [new Event('receiver-probe')]];
    return [[], [string]];
  };
  const previousErrors = [window.onerror, local.onerror];
  window.onerror = local.onerror = () => { reported++; return true; };
  for (const {label, realm, functions} of realms) {
    for (const name of methods) {
      const receivers = [
        ['cross-origin', remote, ['SecurityError', false, true]],
        ...invalid.filter(([kind]) => kind !== 'document' || !eventMethods.has(name))
          .map(([kind, receiver]) => [kind, receiver, ['TypeError', true, false]])
      ];
      for (const [kind, receiver, expected] of receivers) {
        for (const [index, args] of argsFor(name).entries()) {
          equal(observe(() => functions[name].apply(receiver, args), realm), expected,
            label + ' ' + name + ' rejects ' + kind + ' argument case ' + index);
        }
      }
    }
  }
  equal(conversions, 0, 'denied methods check receivers before argument conversion');
  equal(traps, 0, 'receiver checks invoke no author getters or Proxy traps');
  equal(reported, 0, 'denied reportError calls do not dispatch an error event');
  for (const {label, realm, functions} of realms) {
    for (const receiver of [window, local, null, undefined]) {
      const target = receiver ?? realm;
      equal(functions.btoa.call(receiver, 'a'), 'YQ==', label + ' borrowed btoa');
      equal(functions.atob.call(receiver, 'YQ=='), 'a', label + ' borrowed atob');
      equal(typeof functions.getComputedStyle.call(receiver, target.document.documentElement).getPropertyValue,
        'function', label + ' borrowed getComputedStyle');
      equal(functions.getSelection.call(receiver) === target.getSelection(), true, label + ' borrowed getSelection');
      equal(typeof functions.matchMedia.call(receiver, '(min-width:0px)').matches, 'boolean', label + ' borrowed matchMedia');
      const clone = functions.structuredClone.call(receiver, {value: 'copied'});
      equal([clone.value, clone instanceof target.Object], ['copied', true], label + ' clone uses receiver realm');
      for (const name of ['stop', 'captureEvents', 'releaseEvents']) {
        equal(observe(() => functions[name].call(receiver), realm), 'returned', label + ' borrowed ' + name);
      }
      equal(typeof functions.find.call(receiver, ''), 'boolean', label + ' borrowed find');
      for (const name of dialogs) {
        let caught;
        try { functions[name].call(receiver, throwingInput); } catch (error) { caught = error; }
        equal(caught === sentinel, true, label + ' ' + name + ' propagates conversion exceptions after a valid receiver');
      }
      equal(functions.reportError.call(receiver, sentinel), undefined, label + ' borrowed reportError');
      functions.addEventListener.call(receiver, 'receiver-probe', callback);
      functions.removeEventListener.call(receiver, 'receiver-probe', callback);
      equal(functions.dispatchEvent.call(receiver, new target.Event('receiver-probe')), true,
        label + ' borrowed EventTarget methods');
    }
  }
  equal(reported, 8, 'genuine reportError calls dispatch to their target Window');
  equal(conversions, 32, 'only the genuine dialog calls reach the throwing conversion');
  // Window-specific checks must leave the inherited EventTarget methods usable on Nodes.
  for (const {label, realm, functions} of realms) {
    const node = realm.document.createElement('div');
    let events = 0;
    const listener = () => { events++; };
    functions.addEventListener.call(node, 'receiver-probe', listener);
    functions.dispatchEvent.call(node, new realm.Event('receiver-probe'));
    functions.removeEventListener.call(node, 'receiver-probe', listener);
    functions.dispatchEvent.call(node, new realm.Event('receiver-probe'));
    equal(events, 1, label + ' EventTarget accepts genuine Nodes');
  }
  window.onerror = previousErrors[0];
  local.onerror = previousErrors[1];
  remoteFrame.remove();
  for (const {label, realm, functions} of realms) {
    for (const name of methods) {
      equal(observe(() => functions[name].call(remote, throwingInput), realm), ['SecurityError', false, true],
        label + ' ' + name + ' denies a discarded cross-origin Window');
    }
  }
  localFrame.remove();
  return {checks, failures};
}
