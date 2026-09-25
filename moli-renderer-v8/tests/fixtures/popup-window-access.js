function inspectPopupWindow(popup, childIndex) {
  const read = callback => { try { return callback(); } catch (error) { return error.name; } };
  const child = popup[childIndex];
  return Object.entries({popup, parent: child.parent, top: child.top}).map(([path, win]) => ({
    path, identity: win === popup,
    document: read(() => typeof win.document),
    name: read(() => typeof win.name),
    location: read(() => typeof win.location.href),
    expando: read(() => typeof win.__privateMarker),
    descriptor: read(() => typeof Object.getOwnPropertyDescriptor(win, 'document')),
    set: read(() => Reflect.set(win, '__probeValue', 1)),
    define: read(() => Reflect.defineProperty(win, '__probeDefined', {value: 1, configurable: true})),
    has: read(() => 'document' in win),
    del: read(() => Reflect.deleteProperty(win, '__probeValue')),
    closed: read(() => win.closed),
    self: read(() => win.window === win && win.self === win && win.frames === win),
    roots: read(() => win.top === win && win.parent === win),
    length: read(() => win.length),
    opener: read(() => win.opener === window),
    postMessage: read(() => typeof win.postMessage),
    postMessageStable: read(() => win.postMessage === win.postMessage),
    postMessageRealm: read(() => win.postMessage instanceof Function),
    close: read(() => typeof win.close),
    prototypeIsNull: read(() => Object.getPrototypeOf(win) === null),
    then: read(() => typeof win.then),
    iterator: read(() => typeof win[Symbol.iterator]),
  }));
}
