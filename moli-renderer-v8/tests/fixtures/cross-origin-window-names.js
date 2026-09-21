async function probeCrossOriginWindowNames({hostURL, crossURL}) {
  const failures = [];
  let checks = 0;
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
  };
  const observe = callback => {
    try { return callback(); } catch (error) { return error.name; }
  };
  const host = document.createElement('iframe');
  const loaded = new Promise(resolve => host.onload = resolve);
  host.src = hostURL;
  document.body.append(host);
  await loaded;
  const win = host.contentWindow;
  const ask = action => new Promise((resolve, reject) => {
    const listener = event => {
      if (event.source !== win || event.data?.kind !== 'named-window-result' || event.data.action !== action) return;
      removeEventListener('message', listener);
      if (event.data.error) reject(new Error(event.data.error)); else resolve(event.data);
    };
    addEventListener('message', listener);
    win.postMessage({kind:'named-window-action', action, crossURL}, '*');
  });
  const checkName = (name, value, phase, denied = false) => {
    const prefix = phase + ' ' + name;
    equal(observe(() => win[name] === value), denied ? 'SecurityError' : true, prefix + ' get');
    equal(observe(() => {
      const d = Object.getOwnPropertyDescriptor(win, name);
      return [d.value === value, d.writable, d.enumerable, d.configurable];
    }), denied ? 'SecurityError' : [true, false, false, true], prefix + ' descriptor');
    equal(observe(() => name in win), denied ? 'SecurityError' : true, prefix + ' has');
    equal(observe(() => Object.hasOwn(win, name)), denied ? 'SecurityError' : true, prefix + ' hasOwn');
  };
  await ask('init');
  const children = Array.from({length:15}, (_, i) => win[i]);
  const names = ['document', 'open', 'globalThis', 'then', 'constructor', '__proto__', '01', '4294967295'];
  names.forEach((name, i) => checkName(name, children[i], 'initial'));
  checkName('duplicate', children[13], 'initial');
  for (const name of ['close', 'postMessage']) equal(typeof win[name], 'function', name + ' keeps the IDL method');
  equal(win.frames === win, true, 'frames keeps the IDL getter');
  equal(win.length, 15, 'length keeps the IDL getter');
  equal(win['0'] === children[0] && win['0'] !== children[12], true, 'canonical array index takes precedence');
  for (const name of ['document', 'then']) {
    equal(observe(() => Reflect.set(win, name, null)), 'SecurityError', name + ' set');
    equal(observe(() => Reflect.defineProperty(win, name, {value:null})), 'SecurityError', name + ' define');
    equal(observe(() => Reflect.deleteProperty(win, name)), 'SecurityError', name + ' delete');
  }
  equal(Reflect.ownKeys(win).filter(key => names.includes(key) && key !== 'then'), [], 'named children stay out of own keys');
  equal(observe(() => children[0].document), 'SecurityError', 'named access does not expose the child Document');
  await ask('rename');
  checkName('document', undefined, 'renamed', true);
  checkName('renamed', children[0], 'renamed');
  await ask('restore');
  checkName('document', children[0], 'restored');
  checkName('renamed', undefined, 'restored', true);
  const crossDuplicate = await ask('cross-duplicate');
  checkName('duplicate', undefined, 'cross-origin first duplicate', true);
  equal(crossDuplicate.duplicateType, 'undefined', 'same-origin caller also filters the first duplicate');
  equal(win[13] === children[13], true, 'cross-origin first duplicate remains indexed');
  equal(win[14] === children[14], true, 'second duplicate remains indexed');
  const removed = await ask('remove-duplicate');
  checkName('duplicate', children[14], 'first duplicate removed');
  equal(removed.duplicateIsSecond, true, 'same-origin caller sees the next duplicate after removal');
  await ask('cross-document');
  checkName('document', undefined, 'cross-origin document child', true);
  equal(win[0] === children[0], true, 'navigation preserves the indexed WindowProxy');
  equal(children[0].document.body.textContent.startsWith('named destination'), true, 'indexed child is checked against the caller origin');
  await ask('cross-then');
  checkName('then', undefined, 'cross-origin then child uses the fallback');
  host.remove();
  checkName('duplicate', undefined, 'removed host', true);
  checkName('then', undefined, 'removed host fallback');
  equal(win.closed, true, 'removed host closed');
  equal(win.length, 0, 'removed host length');
  return {checks, failures};
}
