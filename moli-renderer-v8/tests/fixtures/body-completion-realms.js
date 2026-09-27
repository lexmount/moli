(async () => {
  const failures = [];
  let total = 0;
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const run = async (name, action) => {
    total++;
    try { await action(); } catch (error) { failures.push(name + ': ' + error); }
  };
  const frame = document.createElement('iframe');
  document.body.append(frame);
  const child = frame.contentWindow;
  for (const owner of ['Request', 'Response']) for (const method of ['arrayBuffer', 'bytes']) {
    for (const mode of ['bytes', 'stream', 'null']) for (const reverse of [false, true]) {
      await run([owner, method, mode, reverse].join(' '), async () => {
        const receiverRealm = reverse ? globalThis : child;
        const methodRealm = reverse ? child : globalThis;
        const body = mode === 'null' ? null : mode === 'bytes' ? 'hello' : new methodRealm.ReadableStream({start(c) {
          c.enqueue(new TextEncoder().encode('hello')); c.close();
        }});
        const object = owner === 'Request'
          ? new receiverRealm.Request(location.href, {method: 'POST', body, duplex: 'half'})
          : new receiverRealm.Response(body);
        const promise = methodRealm[owner].prototype[method].call(object);
        let settled = false;
        promise.then(() => settled = true, () => settled = true);
        for (let i = 0; i < 8; i++) await Promise.resolve();
        check(settled === (mode === 'null'), 'completion crosses the task boundary');
        const result = await promise;
        const type = method === 'bytes' ? 'Uint8Array' : 'ArrayBuffer';
        check(result instanceof receiverRealm[type], 'binary result belongs to the receiver realm');
        check(!(result instanceof methodRealm[type]), 'borrowed method does not select the binary realm');
        check(new TextDecoder().decode(result) === (mode === 'null' ? '' : 'hello'), 'binary result contents');
      });
    }
  }
  frame.remove();

  for (const stream of [false, true]) {
    await run('retiring method realm preserves receiver task ' + stream, async () => {
      const frame = document.createElement('iframe'); document.body.append(frame);
      const method = frame.contentWindow.Response.prototype.text;
      const body = stream ? new ReadableStream({start(c) { c.enqueue(new TextEncoder().encode('hello')); c.close(); }}) : 'hello';
      const promise = method.call(new Response(body));
      frame.remove();
      check(await promise === 'hello', 'receiver task remains live after method realm retirement');
    });
    await run('retiring receiver realm drops pending task ' + stream, async () => {
      const frame = document.createElement('iframe'); document.body.append(frame);
      const other = frame.contentWindow;
      const body = stream ? new ReadableStream({start(c) { c.enqueue(new TextEncoder().encode('old')); c.close(); }}) : 'old';
      const response = new other.Response(body);
      let settled = false;
      Response.prototype.text.call(response).then(() => settled = true, () => settled = true);
      frame.remove(); document.body.append(frame);
      const replacement = await new frame.contentWindow.Response('replacement').text();
      await new Promise(resolve => setTimeout(resolve, 0));
      check(replacement === 'replacement', 'replacement realm runs its own task');
      check(!settled, 'old task cannot settle in the replacement browsing context');
      frame.remove();
    });
  }
  return JSON.stringify({total, failures});
})();
