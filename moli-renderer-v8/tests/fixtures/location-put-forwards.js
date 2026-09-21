async function probeLocationPutForwards({sameURL, crossURL}) {
  let checks = 0, conversions = 0, traps = 0;
  const failures = [], frames = [];
  const apply = Reflect.apply;
  const equal = (actual, expected, label) => {
    checks++;
    if (JSON.stringify(actual) !== JSON.stringify(expected)) failures.push({label, actual, expected});
  };
  const descriptor = (object, name) => {
    while (object) {
      const result = Object.getOwnPropertyDescriptor(object, name);
      if (result) return result;
      object = Object.getPrototypeOf(object);
    }
  };
  const makeFrame = async (url = sameURL) => {
    const frame = document.createElement('iframe');
    frames.push(frame);
    const loaded = new Promise(resolve => frame.onload = resolve);
    frame.src = url;
    document.body.append(frame);
    await loaded;
    return frame.contentWindow;
  };
  const realms = [['parent', window], ['child-a', await makeFrame()], ['child-b', await makeFrame()]]
    .map(([name, global]) => {
      const detached = global.document.implementation.createHTMLDocument('');
      const xml = new global.DOMParser().parseFromString('<root/>', 'text/xml');
      return {
        name, global, document:global.document, location:global.location, detached, xml,
        TypeError:global.TypeError,
        windowSet:descriptor(global, 'location').set,
        documentSet:descriptor(global.document, 'location').set,
        documentGet:descriptor(global.document, 'location').get,
        detachedSet:descriptor(detached, 'location').set,
        hrefSet:descriptor(global.location, 'href').set
      };
    });
  const marker = {};
  const input = {[Symbol.toPrimitive](hint) { conversions++; equal(hint, 'string', 'string conversion hint'); throw marker; }};
  const handler = new Proxy({}, {get() { traps++; throw marker; }});
  const observe = callback => {
    try { callback(); return 'returned'; }
    catch (error) {
      if (error === marker) return 'marker';
      return [error.name, realms.findIndex(realm => Object.getPrototypeOf(error) === realm.TypeError.prototype)];
    }
  };
  const observeRead = callback => {
    try { return callback(); }
    catch (error) { return {error:error.name, realm:realms.findIndex(realm => Object.getPrototypeOf(error) === realm.TypeError.prototype)}; }
  };
  const families = ['windowSet', 'documentSet', 'detachedSet', 'hrefSet'];
  const receiverFor = (family, realm) => family === 'windowSet' ? realm.global : family === 'hrefSet' ? realm.location : realm.document;
  try {
    for (const [calleeIndex, callee] of realms.entries()) {
      for (const [targetIndex, target] of realms.entries()) {
        for (const family of families) {
          const receiver = receiverFor(family, target);
          const errorIndex = family === 'hrefSet' ? calleeIndex : targetIndex;
          equal(observe(() => apply(callee[family], receiver, [Symbol('url')])),
            ['TypeError', errorIndex], callee.name + ' ' + family + ' Symbol to ' + target.name);
          equal(observe(() => apply(callee[family], receiver, [input])),
            'marker', callee.name + ' ' + family + ' preserves thrown conversion value for ' + target.name);
        }
      }
    }
    equal(conversions, 36, 'each accepted call converts once');
    const acceptedConversions = conversions;
    for (const [calleeIndex, callee] of realms.entries()) {
      for (const family of families) {
        const real = receiverFor(family, callee);
        const revoked = Proxy.revocable(real, {});
        revoked.revoke();
        const invalid = [
          ['plain', {}], ['inherited', Object.create(real)],
          ['proxy', new Proxy(real, handler)], ['revoked', revoked.proxy],
          ['element', document.createElement('div')], ['primitive', 1]
        ];
        if (family !== 'windowSet') invalid.push(['null', null], ['undefined', undefined]);
        for (const [kind, receiver] of invalid) {
          equal(observe(() => apply(callee[family], receiver, [input])),
            ['TypeError', calleeIndex], callee.name + ' ' + family + ' rejects ' + kind + ' before conversion');
        }
      }
      const revokedDocument = Proxy.revocable(callee.document, {});
      revokedDocument.revoke();
      for (const [kind, receiver] of [
        ['plain', {}], ['inherited', Object.create(callee.document)],
        ['proxy', new Proxy(callee.document, handler)], ['revoked', revokedDocument.proxy],
        ['element', document.createElement('div')], ['primitive', 1], ['null', null], ['undefined', undefined]
      ]) {
        equal(observe(() => apply(callee.documentGet, receiver, [])),
          ['TypeError', calleeIndex], callee.name + ' documentGet rejects ' + kind);
      }
      for (const target of realms) {
        equal(observeRead(() => apply(callee.documentGet, target.document, []) === target.location), true,
          callee.name + ' documentGet returns ' + target.name + ' Location');
        for (const kind of ['detached', 'xml']) {
          equal(observeRead(() => apply(callee.documentGet, target[kind], [])), null,
            callee.name + ' documentGet returns null for ' + target.name + ' ' + kind);
        }
        for (const family of ['documentSet', 'detachedSet']) {
          for (const kind of ['detached', 'xml']) {
            equal(observe(() => apply(callee[family], target[kind], [input])),
              ['TypeError', calleeIndex], callee.name + ' ' + family + ' cannot forward null location of ' + target.name + ' ' + kind);
          }
        }
      }
    }
    equal(conversions, acceptedConversions, 'invalid receiver and null location never convert the assigned value');
    equal(traps, 0, 'brand checks do not invoke author Proxy traps');
    for (const [targetIndex, target] of realms.entries()) {
      const original = target.global.TypeError;
      try {
        target.global.TypeError = function ForbiddenTypeError() { throw marker; };
        for (const callee of realms) {
          for (const family of ['windowSet', 'documentSet', 'detachedSet']) {
            equal(observe(() => apply(callee[family], receiverFor(family, target), [Symbol('url')])),
              ['TypeError', targetIndex], callee.name + ' ' + family + ' uses intrinsic forwarded error in ' + target.name);
          }
        }
      } finally {
        target.global.TypeError = original;
      }
    }
    const remote = await makeFrame(crossURL);
    for (const [calleeIndex, callee] of realms.entries()) {
      for (const caller of realms) {
        const invoke = caller.global.Function('setter', 'target', 'setter.call(target, Symbol("url"))');
        equal(observe(() => invoke(callee.windowSet, remote)), ['TypeError', calleeIndex],
          caller.name + ' calls ' + callee.name + ' windowSet on a cross-origin Window');
      }
    }
  } finally {
    for (const frame of frames) frame.remove();
  }
  return {checks, failures};
}
