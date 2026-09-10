async function runPreabortedUploadProbe(scenario, url) {
  const errors = [];
  const check = (condition, label) => { if (!condition) errors.push(label); };
  const rejection = async (promise, reason, label) => {
    try { await promise; errors.push(label + ': fulfilled'); }
    catch (error) { check(error === reason, label + ': rejection identity'); }
  };
  const controllerWith = reason => {
    const controller = new AbortController();
    controller.abort(reason);
    return controller;
  };
  if (scenario === 'cancel') {
    const unhandled = [];
    const onUnhandled = event => { unhandled.push(event.reason); event.preventDefault(); };
    addEventListener('unhandledrejection', onUnhandled);
    for (const input of ['init', 'request']) {
      for (const behavior of ['plain', 'throw', 'reject', 'pending']) {
        for (const reason of [undefined, null, 0, Symbol('abort'), {get then() {throw new Error('abort reason then')}}]) {
          const controller = controllerWith(reason);
          const expected = controller.signal.reason;
          const label = input + '/' + behavior + '/' + typeof reason;
          const calls = [];
          const cancelError = new Error('cancel failure');
          const stream = new ReadableStream({
            start(controller) { controller.enqueue(new Uint8Array([42])); },
            cancel(value) {
              calls.push(value);
              if (behavior === 'throw') throw cancelError;
              if (behavior === 'reject') return Promise.reject(cancelError);
              if (behavior === 'pending') return new Promise(() => {});
            }
          });
          const init = {method: 'POST', body: stream, duplex: 'half', signal: controller.signal};
          const request = input === 'request' ? new Request(url, init) : null;
          const promise = request ? fetch(request) : fetch(url, init);
          check(calls.length === 1 && calls[0] === expected, label + ': synchronous cancel');
          check(stream.locked === !!request, label + ': lock state');
          if (request) check(request.bodyUsed, label + ': input bodyUsed');
          await rejection(promise, expected, label);
          check(calls.length === 1, label + ': exactly one cancel');
        }
      }
    }
    for (const state of ['closed', 'errored']) {
      let canceled = false;
      const stream = new ReadableStream({
        start(controller) { if (state === 'closed') controller.close(); else controller.error(undefined); },
        cancel() { canceled = true; }
      });
      const controller = controllerWith('abort');
      await rejection(fetch(url, {method: 'POST', body: stream, duplex: 'half', signal: controller.signal}), 'abort', state);
      check(!canceled, state + ': no cancel');
      check(!new Response(stream).bodyUsed, state + ': not disturbed');
    }
    await new Promise(resolve => setTimeout(resolve, 10));
    removeEventListener('unhandledrejection', onUnhandled);
    check(unhandled.length === 0, 'cancel failures must be handled');
  } else if (scenario === 'selection') {
    for (const override of ['absent', 'null', 'undefined', 'stream', 'bytes']) {
      const controller = controllerWith('selected abort');
      const calls = [];
      const original = new ReadableStream({cancel(reason) {calls.push(['original', reason])}});
      const replacement = new ReadableStream({cancel(reason) {calls.push(['replacement', reason])}});
      const request = new Request(url, {method: 'POST', body: original, duplex: 'half'});
      const init = {signal: controller.signal};
      if (override === 'null') init.body = null;
      if (override === 'undefined') init.body = undefined;
      if (override === 'stream') {init.body = replacement; init.duplex = 'half';}
      if (override === 'bytes') init.body = 'replacement';
      const promise = fetch(request, init);
      const expected = override === 'stream' ? 'replacement' : override === 'bytes' ? null : 'original';
      check(calls.length === (expected ? 1 : 0) && (!expected || calls[0][0] === expected && calls[0][1] === controller.signal.reason), override + ': selected body');
      check(request.bodyUsed === (expected === 'original'), override + ': input consumed');
      await rejection(promise, controller.signal.reason, override);
    }
    const controller = controllerWith('getter abort');
    let reads = 0, cancels = 0;
    const stream = new ReadableStream({cancel() {cancels++}});
    const promise = fetch(url, {method: 'POST', duplex: 'half', signal: controller.signal, get body() {reads++; return stream;}});
    check(reads === 1 && cancels === 1, 'body getter read once');
    await rejection(promise, controller.signal.reason, 'body getter');
    let innerPromise;
    const inner = new ReadableStream({cancel() {cancels++}});
    const outer = new ReadableStream({cancel() {
      innerPromise = fetch(url, {method: 'POST', body: inner, duplex: 'half', signal: controller.signal});
    }});
    const outerPromise = fetch(url, {method: 'POST', body: outer, duplex: 'half', signal: controller.signal});
    check(!!innerPromise && cancels === 2, 'cancel callback can reenter fetch');
    await rejection(outerPromise, controller.signal.reason, 'reentrant outer');
    if (innerPromise) await rejection(innerPromise, controller.signal.reason, 'reentrant inner');
  } else if (scenario === 'validation') {
    for (const invalid of ['duplex', 'method', 'empty-get', 'mode', 'keepalive', 'window', 'locked']) {
      const controller = controllerWith('abort');
      let canceled = false;
      const stream = new ReadableStream({cancel() {canceled = true;}});
      const init = {method: 'POST', body: stream, duplex: 'half', signal: controller.signal};
      let reader;
      if (invalid === 'duplex') delete init.duplex;
      if (invalid === 'method') init.method = 'GET';
      if (invalid === 'empty-get') {init.method = 'GET'; init.body = '';}
      if (invalid === 'mode') init.mode = 'no-cors';
      if (invalid === 'keepalive') init.keepalive = true;
      if (invalid === 'window') init.window = {};
      if (invalid === 'locked') reader = stream.getReader();
      const promise = fetch(url, init);
      check(!canceled, invalid + ': construction error must not cancel');
      try { await promise; errors.push(invalid + ': fulfilled'); }
      catch (error) { check(error instanceof TypeError, invalid + ': construction error wins'); }
      if (reader) reader.releaseLock();
    }
    // Buffered no-cors bodies are valid despite having a public body stream.
    const controller = controllerWith('buffered abort');
    const request = new Request(url, {method: 'POST', body: 'bytes', mode: 'no-cors'});
    await rejection(fetch(request, {signal: controller.signal}), controller.signal.reason, 'buffered no-cors');
    check(request.bodyUsed, 'buffered input consumed');
  } else if (scenario === 'intrinsics') {
    const controller = controllerWith('intrinsic abort');
    const calls = [];
    const source = {cancel(reason) {calls.push(reason)}};
    const stream = new ReadableStream(source);
    source.cancel = () => {throw new Error('replaced source.cancel')};
    Object.defineProperty(stream, 'cancel', {get() {throw new Error('stream.cancel')}});
    Object.defineProperty(stream, 'getReader', {get() {throw new Error('stream.getReader')}});
    const then = Object.getOwnPropertyDescriptor(Promise.prototype, 'then');
    const catchDescriptor = Object.getOwnPropertyDescriptor(Promise.prototype, 'catch');
    let promise;
    try {
      Object.defineProperty(Promise.prototype, 'then', {configurable: true, get() {throw new Error('public Promise.then')}});
      Object.defineProperty(Promise.prototype, 'catch', {configurable: true, get() {throw new Error('public Promise.catch')}});
      promise = fetch(url, {method: 'POST', body: stream, duplex: 'half', signal: controller.signal});
      check(calls.length === 1 && calls[0] === controller.signal.reason, 'native synchronous cancellation');
    } catch (error) { errors.push('fetch threw synchronously: ' + String(error)); }
    finally {
      Object.defineProperty(Promise.prototype, 'then', then);
      Object.defineProperty(Promise.prototype, 'catch', catchDescriptor);
    }
    if (promise) await rejection(promise, controller.signal.reason, 'intrinsics');
  } else throw new Error('unknown scenario');
  return {errors};
}
