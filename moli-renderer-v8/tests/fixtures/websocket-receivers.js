(() => {
  const failures = [];
  let checks = 0;
  function check(ok, label) { checks++; if (!ok) failures.push(label); }
  const iframe = document.createElement('iframe'); document.body.appendChild(iframe);
  const realms = [globalThis, iframe.contentWindow];
  const sockets = realms.map(realm => new realm.WebSocket('wss://example.test/receiver'));
  for (const [index, realm] of realms.entries()) {
    const prototype = realm.WebSocket.prototype, socket = sockets[1 - index];
    let traps = 0, conversions = 0;
    const author = new Proxy(socket, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}});
    const revoked = Proxy.revocable(socket, {}); revoked.revoke();
    const bad = [null, undefined, true, 7, Symbol('receiver'), {}, Object.create(prototype), Object.create(socket), author, revoked.proxy];
    function rejects(callback, label) {
      let error; try {callback();} catch (caught) {error = caught;}
      check(error !== undefined && Object.getPrototypeOf(error) === realm.TypeError.prototype, index + '/' + label + '/TypeError realm');
      check(traps === 0 && conversions === 0, index + '/' + label + '/no author code');
    }
    for (const name of ['url','readyState','bufferedAmount','extensions','protocol','binaryType','onopen','onmessage','onerror','onclose']) {
      const d = Object.getOwnPropertyDescriptor(prototype, name);
      check(d.enumerable && d.configurable && d.get.length === 0, index + '/' + name + '/descriptor');
      check(d.get.call(socket) === socket[name], index + '/' + name + '/genuine cross realm');
      for (const [n, receiver] of bad.entries()) {
        rejects(() => d.get.call(receiver), name + '/' + n + '/get');
        if (d.set) rejects(() => d.set.call(receiver, {[Symbol.toPrimitive]() {conversions++; throw 43;}}), name + '/' + n + '/set');
      }
    }
    for (const [name, argument] of [['send',{toString() {conversions++; throw 44;}}],['close',{valueOf() {conversions++; throw 45;}}]]) {
      for (const [n, receiver] of bad.entries()) rejects(() => prototype[name].call(receiver, argument), name + '/' + n);
    }
    rejects(() => prototype.send.call(socket), 'send/missing argument');
    const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
    check(tag.value === 'WebSocket' && !tag.get && !tag.set && !tag.writable && !tag.enumerable && tag.configurable, index + '/tag/data descriptor');
    check(Object.prototype.toString.call(prototype) === '[object WebSocket]', index + '/tag/prototype');
  }
  sockets.forEach(socket => socket.close()); iframe.remove();
  return {checks, failures};
})()
