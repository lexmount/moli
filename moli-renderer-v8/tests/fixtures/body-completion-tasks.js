(async () => {
  const failures = [];
  let total = 0;
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const run = async (name, action) => {
    total++;
    try { await action(); } catch (error) { failures.push(name + ': ' + error); }
  };
  const checkpoint = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
  };
  const methods = ['text', 'json', 'arrayBuffer', 'bytes', 'blob', 'formData'];
  const make = (owner, body, type = 'text/plain') => owner === 'Request'
    ? new Request('https://body-completion.test/', {method: 'POST', body, duplex: 'half', headers: {'Content-Type': type}})
    : new Response(body, {headers: {'Content-Type': type}});

  for (const owner of ['Request', 'Response']) {
    for (const method of methods) for (const mode of ['null', 'text', 'bytes', 'blob', 'closed', 'pending']) {
      await run(owner + ' ' + method + ' ' + mode, async () => {
        const text = method === 'json' ? '{"answer":42}' : method === 'formData' ? 'answer=42' : 'hello';
        const chunk = new TextEncoder().encode(text);
        let controller;
        const body = mode === 'null' ? null : mode === 'text' ? text : mode === 'bytes' ? chunk :
          mode === 'blob' ? new Blob([chunk]) : new ReadableStream({start(c) {
            controller = c;
            if (mode === 'closed') { c.enqueue(chunk); c.close(); }
          }});
        const type = method === 'formData' ? 'application/x-www-form-urlencoded' : 'text/plain';
        const object = make(owner, body, type);
        const promise = object[method]();
        let settled = false;
        const result = promise.then(value => { settled = true; return {value}; }, error => { settled = true; return {error}; });
        check(promise instanceof Promise, 'Promise belongs to the method realm');
        check(object.bodyUsed === (mode !== 'null'), 'bodyUsed changes synchronously');
        check(object.body === null || object.body.locked, 'reader is acquired synchronously');
        if (mode === 'pending') { controller.enqueue(chunk); controller.close(); }
        chunk.fill(90);
        if (method === 'blob') object.headers.set('Content-Type', 'application/synchronous');
        let secondRejected = false;
        if (mode !== 'null') object.text().catch(error => { secondRejected = error instanceof TypeError; });
        await checkpoint();
        check(settled === (mode === 'null'), 'only a null Body completes during the microtask checkpoint');
        if (mode !== 'null') check(secondRejected, 'unusable Body rejection does not wait for a fetch task');
        if (method === 'blob') object.headers.set('Content-Type', 'application/after-checkpoint');
        const output = await result;
        if (method === 'json' && mode === 'null') {
          check(output.error instanceof SyntaxError, 'null JSON rejects with SyntaxError');
          return;
        }
        check(!('error' in output), 'conversion succeeds: ' + output.error);
        const value = output.value;
        const expected = mode === 'null' ? '' : text;
        if (method === 'text') check(value === expected, 'text preserves copied bytes');
        else if (method === 'json') check(value.answer === 42, 'JSON conversion uses the consumed bytes');
        else if (method === 'arrayBuffer' || method === 'bytes') {
          check(value instanceof (method === 'bytes' ? Uint8Array : ArrayBuffer), 'binary result prototype');
          check(new TextDecoder().decode(value) === expected, 'binary result preserves copied bytes');
        } else if (method === 'blob') {
          check(value instanceof Blob, 'Blob result prototype');
          check(value.type === (mode === 'null' ? 'text/plain' : 'application/after-checkpoint'), 'MIME is read in the conversion step');
          check(await value.text() === expected, 'Blob preserves copied bytes');
        } else {
          check(value instanceof FormData, 'FormData result prototype');
          check(value.get('answer') === (mode === 'null' ? null : '42'), 'FormData content');
        }
      });
    }

    for (const method of methods) for (const mode of ['errored', 'bad-chunk']) {
      await run(owner + ' ' + method + ' ' + mode, async () => {
        const reason = {reason: 'stream error'};
        const stream = new ReadableStream({start(controller) {
          if (mode === 'errored') controller.error(reason);
          else { controller.enqueue(new Uint16Array([1])); controller.close(); }
        }});
        const object = make(owner, stream);
        let settled = false;
        const result = object[method]().then(
          () => { settled = true; return {success: true}; },
          error => { settled = true; return {error}; },
        );
        await checkpoint();
        check(!settled, 'read errors use a fetch task');
        check(object.bodyUsed && stream.locked, 'failed reads still disturb and lock the stream');
        const output = await result;
        check(!output.success, 'read must fail');
        check(mode === 'errored' ? output.error === reason : output.error instanceof TypeError, 'read error identity and type');
      });
    }

    for (const mode of ['bytes', 'stream', 'error']) {
      await run(owner + ' completions have separate checkpoints ' + mode, async () => {
        const events = [];
        const source = value => mode === 'bytes' ? value : new ReadableStream({start(c) {
          if (mode === 'error') c.error(value);
          else { c.enqueue(new TextEncoder().encode(value)); c.close(); }
        }});
        const complete = value => { events.push(value); queueMicrotask(() => events.push(value + '-microtask')); };
        const first = make(owner, source('first')).text().then(complete, complete);
        const second = make(owner, source('second')).text().then(complete, complete);
        await Promise.all([first, second]);
        await Promise.resolve();
        check(events.join('|') === 'first|first-microtask|second|second-microtask', 'each completion gets its own task-end checkpoint: ' + events);
      });
    }

    await run(owner + ' completion cannot be canceled as a timer', async () => {
      const before = setTimeout(() => {}, 60000);
      const result = make(owner, 'hello').text();
      const after = setTimeout(() => {}, 60000);
      for (let id = before; id <= after; id++) { clearTimeout(id); clearInterval(id); }
      check(await result === 'hello', 'browser completion survives author timer cancellation');
    });

    for (const initialValid of [false, true]) {
      await run(owner + ' formData reads final MIME ' + initialValid, async () => {
        const supported = 'application/x-www-form-urlencoded';
        const object = make(owner, 'answer=42', initialValid ? supported : 'text/plain');
        const result = object.formData().then(value => ({value}), error => ({error}));
        await checkpoint();
        object.headers.set('Content-Type', initialValid ? 'text/plain' : supported);
        const output = await result;
        check(initialValid ? output.error instanceof TypeError : output.value?.get('answer') === '42', 'conversion uses MIME at task execution');
      });
    }
  }
  return JSON.stringify({total, failures});
})();
