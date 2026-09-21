async function probeWindowPromiseMethodReceivers({sameURL, crossURL}) {
  let checks = 0, conversions = 0, traps = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
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
  const methods = ['fetch', 'createImageBitmap'];
  const realms = [['parent', window], ['child', local]].map(([label, realm]) => ({
    label, realm, Promise:realm.Promise, TypeError:realm.TypeError, DOMException:realm.DOMException,
    functions:Object.fromEntries(methods.map(name => [name, realm[name]]))
  }));
  const sentinel = new Error('argument conversion reached');
  const input = {toString() { conversions++; throw sentinel; }};
  const init = {get method() { conversions++; throw sentinel; }};
  const image = new ImageData(1, 1);
  const options = {get imageOrientation() { conversions++; throw sentinel; }};
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
  const argsFor = name => name === 'fetch'
    ? [[], [input], ['data:text/plain,blocked', init]]
    : [[], [Symbol('source')], [image, options], [image, 0, 0, 0, 1, options]];
  const describe = (error, context) => error === sentinel ? 'sentinel' : [
    error.name, Object.getPrototypeOf(error) === context.TypeError.prototype,
    error instanceof context.DOMException
  ];
  const observe = async (callback, context) => {
    let promise;
    try { promise = callback(); }
    catch (error) { return {kind:'thrown', error:describe(error, context)}; }
    const promiseRealm = Object.getPrototypeOf(promise) === context.Promise.prototype;
    try {
      const value = await promise;
      if (value && typeof value.close === 'function') value.close();
      return {kind:'fulfilled', promiseRealm};
    } catch (error) {
      return {kind:'rejected', promiseRealm, error:describe(error, context)};
    }
  };
  const rejection = error => ({kind:'rejected', promiseRealm:true, error});
  for (const context of realms) {
    for (const name of methods) {
      for (const [kind, receiver, expected] of [
        ['cross-origin', remote, ['SecurityError', false, true]],
        ...invalid.map(([kind, receiver]) => [kind, receiver, ['TypeError', true, false]])
      ]) {
        for (const [index, args] of argsFor(name).entries()) {
          equal(await observe(() => context.functions[name].apply(receiver, args), context),
            rejection(expected), context.label + ' ' + name + ' rejects ' + kind + ' argument case ' + index);
        }
      }
    }
  }
  for (const context of realms) {
    try {
      for (const name of ['Promise', 'TypeError', 'DOMException']) {
        context.realm[name] = function ForbiddenConstructor() { throw sentinel; };
      }
      for (const name of methods) {
        equal(await observe(() => context.functions[name].call(remote), context),
          rejection(['SecurityError', false, true]), context.label + ' ' + name + ' uses intrinsic cross-origin rejection');
        equal(await observe(() => context.functions[name].call({}), context),
          rejection(['TypeError', true, false]), context.label + ' ' + name + ' uses intrinsic brand rejection');
      }
    } finally {
      for (const name of ['Promise', 'TypeError', 'DOMException']) context.realm[name] = context[name];
    }
    for (const name of methods) {
      let error;
      try { context.realm.Reflect.construct(context.functions[name], argsFor(name)[1]); }
      catch (caught) { error = caught; }
      equal(error instanceof context.TypeError, true, context.label + ' ' + name + ' is not a constructor');
    }
  }
  equal(conversions, 0, 'denied calls do not convert arguments');
  equal(traps, 0, 'brand checks do not run author getters or Proxy traps');
  for (const context of realms) {
    for (const receiver of [window, local, null, undefined]) {
      for (const name of methods) {
        equal(await observe(() => context.functions[name].call(receiver), context),
          rejection(['TypeError', true, false]), context.label + ' borrowed ' + name + ' missing argument');
        const args = name === 'fetch' ? [input] : [image, options];
        equal(await observe(() => context.functions[name].apply(receiver, args), context),
          rejection('sentinel'), context.label + ' borrowed ' + name + ' preserves conversion exception identity');
      }
    }
  }
  remoteFrame.remove();
  for (const context of realms) {
    for (const name of methods) {
      for (const [index, args] of argsFor(name).entries()) {
        equal(await observe(() => context.functions[name].apply(remote, args), context),
          rejection(['SecurityError', false, true]), context.label + ' ' + name + ' rejects discarded cross-origin argument case ' + index);
      }
    }
  }
  equal(conversions, 16, 'only valid receivers reach the throwing conversions');
  equal(traps, 0, 'discarded receiver checks do not run Proxy traps');
  localFrame.remove();
  return {checks, failures};
}
