async function probeCrossOriginSymbolFallback({role, crossURL, sameURL}) {
  let checks = 0;
  const failures = [];
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
  };
  const observe = callback => {
    try { return callback(); } catch (error) { return error.name; }
  };
  const symbols = [Symbol.toStringTag, Symbol.hasInstance, Symbol.isConcatSpreadable];
  const cross = (object, label) => {
    equal(Object.prototype.toString.call(object), '[object Object]', label + ' class string');
    for (const key of ['then', ...symbols]) {
      const prefix = label + ' ' + String(key);
      equal(object[key] === undefined, true, prefix + ' value');
      equal(observe(() => {
        const d = Object.getOwnPropertyDescriptor(object, key);
        return [d.value === undefined, d.writable, d.enumerable, d.configurable, 'get' in d, 'set' in d];
      }), [true, false, false, true, false, false], prefix + ' descriptor');
      equal([
        observe(() => Object.hasOwn(object, key)),
        observe(() => Reflect.has(object, key))
      ], [true, true], prefix + ' presence');
      equal([
        observe(() => Reflect.set(object, key, 'changed')),
        observe(() => Reflect.defineProperty(object, key, {value:'changed'})),
        observe(() => Reflect.deleteProperty(object, key))
      ], ['SecurityError', 'SecurityError', 'SecurityError'], prefix + ' mutations denied');
    }
    equal(Object.getOwnPropertySymbols(object).map(String), symbols.map(String), label + ' own symbols');
    const unknown = Symbol('Symbol.toStringTag');
    equal([
      observe(() => object[unknown]),
      observe(() => Object.getOwnPropertyDescriptor(object, unknown))
    ], ['SecurityError', 'SecurityError'], label + ' unrelated symbol denied');
  };
  const same = (object, tag, label) => {
    equal(object[Symbol.toStringTag], tag, label + ' same-origin tag');
    equal(Object.prototype.toString.call(object), `[object ${tag}]`, label + ' same-origin class string');
  };
  const crossPair = (win, label) => {
    cross(win, label + ' Window');
    cross(win.location, label + ' Location');
  };
  const samePair = (win, label) => {
    same(win, 'Window', label + ' Window');
    same(win.location, 'Location', label + ' Location');
  };
  if (role === 'child') {
    addEventListener('message', event => {
      if (event.data?.kind !== 'symbol-fallback-request') return;
      try {
        samePair(window, 'child self');
        crossPair(event.data.action === 'parent' ? parent : parent.frames[0], event.data.action);
        event.source.postMessage({kind:'symbol-fallback-result', checks, failures}, '*');
      } catch (error) {
        event.source.postMessage({kind:'symbol-fallback-result', error:String(error)}, '*');
      }
    });
    return;
  }
  const navigate = (frame, url) => new Promise(resolve => {
    frame.onload = resolve;
    frame.src = url;
    if (!frame.isConnected) document.body.append(frame);
  });
  const ask = (win, action) => new Promise((resolve, reject) => {
    const listener = event => {
      if (event.source !== win || event.data?.kind !== 'symbol-fallback-result') return;
      removeEventListener('message', listener);
      if (event.data.error) reject(new Error(event.data.error));
      else {
        checks += event.data.checks;
        failures.push(...event.data.failures);
        resolve();
      }
    };
    addEventListener('message', listener);
    win.postMessage({kind:'symbol-fallback-request', action}, '*');
  });
  samePair(window, 'main before');
  const remote = document.createElement('iframe');
  await navigate(remote, crossURL);
  const win = remote.contentWindow;
  const oldLocation = win.location;
  crossPair(win, 'main to child');
  await ask(win, 'parent');
  const local = document.createElement('iframe');
  await navigate(local, sameURL);
  await ask(local.contentWindow, 'sibling');
  await navigate(remote, sameURL);
  equal(remote.contentWindow === win, true, 'same-origin navigation preserves WindowProxy');
  samePair(win, 'child navigated same-origin');
  cross(oldLocation, 'retained old cross-origin Location');
  await navigate(remote, crossURL);
  equal(remote.contentWindow === win, true, 'cross-origin navigation preserves WindowProxy');
  crossPair(win, 'child navigated cross-origin again');
  remote.remove();
  equal(win.closed, true, 'removed child is closed');
  crossPair(win, 'removed child');
  local.remove();
  samePair(window, 'main after');
  return {checks, failures};
}
