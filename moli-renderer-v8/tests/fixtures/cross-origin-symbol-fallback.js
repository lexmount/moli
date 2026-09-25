async function probeCrossOriginSymbolFallback({crossURL, sameURL}) {
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
    for (const key of symbols) {
      const prefix = label + ' ' + String(key);
      equal(object[key] === undefined, true, prefix + ' value');
      equal(observe(() => {
        const d = Object.getOwnPropertyDescriptor(object, key);
        return [d.value === undefined, d.writable, d.enumerable, 'get' in d, 'set' in d];
      }), [true, false, false, false, false], prefix + ' descriptor');
      equal([
        observe(() => Object.hasOwn(object, key)),
        observe(() => Reflect.has(object, key))
      ], [true, true], prefix + ' presence');

    }
    equal(Object.getOwnPropertySymbols(object).map(String), symbols.map(String), label + ' own symbols');

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
  const navigate = (frame, url) => new Promise(resolve => {
    frame.onload = resolve;
    frame.src = url;
    if (!frame.isConnected) document.body.append(frame);
  });
  samePair(window, 'main before');
  const remote = document.createElement('iframe');
  await navigate(remote, crossURL);
  const win = remote.contentWindow;
  const oldLocation = win.location;
  crossPair(win, 'main to child');
  const local = document.createElement('iframe');
  await navigate(local, sameURL);
  samePair(local.contentWindow, 'main to same-origin child');
  await navigate(remote, sameURL);
  equal(remote.contentWindow === win, true, 'same-origin navigation preserves WindowProxy');
  samePair(win, 'child navigated same-origin');
  cross(oldLocation, 'retained old cross-origin Location');
  await navigate(remote, crossURL);
  equal(remote.contentWindow === win, true, 'cross-origin navigation preserves WindowProxy');
  crossPair(win, 'child navigated cross-origin again');
  remote.remove();
  local.remove();
  samePair(window, 'main after');
  return {checks, failures};
}
