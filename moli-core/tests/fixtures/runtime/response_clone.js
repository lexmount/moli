async function runResponseCloneProbe(scenario, url) {
  const errors = [];
  const check = (value, label) => { if (!value) errors.push(label); };
  const throwsTypeError = (callback, label) => {
    try { callback(); errors.push(label + ': accepted'); }
    catch (error) { check(error instanceof TypeError, label + ': ' + error); }
  };
  const bytesOf = value => new Uint8Array(value instanceof ArrayBuffer ? value : value.buffer,
    value.byteOffset || 0, value.byteLength);
  const sameBytes = (left, right) => left.length === right.length && left.every((value, i) => value === right[i]);
  const makeStream = text => {
    const bytes = new TextEncoder().encode(text);
    let offset = 0;
    return new ReadableStream({async pull(controller) {
      await Promise.resolve();
      if (offset === bytes.length) controller.close();
      else { controller.enqueue(bytes.slice(offset, offset + 1)); ++offset; }
    }});
  };

  if (scenario === 'chunks') {
    const makeViews = [
      b => b, b => new DataView(b, 2, 12),
      ...[Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array,
        Uint32Array, BigInt64Array, BigUint64Array, Float16Array, Float32Array, Float64Array]
        .map(Type => b => new Type(b, Type.BYTES_PER_ELEMENT, 2)),
    ];
    for (const [index, makeView] of makeViews.entries()) {
      const buffer = new ArrayBuffer(32);
      new Uint8Array(buffer).forEach((_, i, bytes) => { bytes[i] = i + 1; });
      const input = makeView(buffer);
      const expected = [...bytesOf(input)];
      const stream = new ReadableStream({start(c) { c.enqueue(input); c.close(); }});
      stream.tee = stream.getReader = () => { throw new Error('public stream operation called'); };
      const response = new Response(stream);
      const clone = response.clone();
      check(response.body !== stream && stream.locked, index + ': native tee');
      const originalChunk = (await response.body.getReader().read()).value;
      check(originalChunk === input, index + ': original chunk identity');
      bytesOf(originalChunk).fill(0);
      const copied = (await clone.body.getReader().read()).value;
      check(copied !== input && Object.getPrototypeOf(copied) === Object.getPrototypeOf(input), index + ': copied type');
      check(sameBytes(bytesOf(copied), expected), index + ': isolated bytes');
      if (!(input instanceof ArrayBuffer)) check(copied.byteOffset === input.byteOffset, index + ': view offset');
    }
    const object = {nested: {value: 1}, map: new Map([['value', 2]])};
    const response = new Response(new ReadableStream({start(c) { c.enqueue(object); c.close(); }}));
    const clone = response.clone();
    const first = (await response.body.getReader().read()).value;
    first.nested.value = 9;
    first.map.set('value', 9);
    const second = (await clone.body.getReader().read()).value;
    check(second !== first && second.nested.value === 1 && second.map.get('value') === 2, 'structured object clone');
    const altered = makeStream('native brand');
    Object.setPrototypeOf(altered, null);
    const branded = new Response(altered);
    check(await branded.clone().text() === 'native brand' && await branded.text() === 'native brand', 'native stream brand');
    const byteStream = new ReadableStream({type: 'bytes', start(c) { c.enqueue(new Uint8Array([65, 66])); c.close(); }});
    const byteResponse = new Response(byteStream);
    const byteClone = byteResponse.clone();
    const byob = byteClone.body.getReader({mode: 'byob'});
    check(sameBytes((await byob.read(new Uint8Array(2))).value, [65, 66]), 'byte clone retains BYOB');
    check(await byteResponse.text() === 'AB', 'byte original remains independent');
  } else if (scenario === 'lifecycle') {
    const original = new Response(makeStream('{"value":"AB"}'));
    const clone = original.clone();
    const second = clone.clone();
    check(await original.text() === '{"value":"AB"}' && !clone.bodyUsed && !second.bodyUsed, 'reading original preserves clones');
    check((await clone.json()).value === 'AB' && await second.text() === '{"value":"AB"}', 'late bytes in repeated clones');
    throwsTypeError(() => original.clone(), 'consumed clone');
    const kept = new Response(makeStream('kept'));
    const keptClone = kept.clone();
    const cancelled = kept.body.cancel('one');
    check(await keptClone.text() === 'kept', 'one branch cancellation preserves other');
    await cancelled;
    let reasons;
    const cancelError = {cancel: 'rejected'};
    const pending = new Response(new ReadableStream({cancel(value) { reasons = value; return Promise.reject(cancelError); }}));
    const pendingClone = pending.clone();
    const cancellations = await Promise.allSettled([pending.body.cancel('first'), pendingClone.body.cancel('second')]);
    check(sameBytes(reasons, ['first', 'second']), 'ordered cancellation reasons');
    check(cancellations.every(value => value.status === 'rejected' && value.reason === cancelError), 'cancellation rejection reaches both branches');
    const error = {source: 'failed'};
    const failed = new Response(new ReadableStream({start(c) { c.error(error); }}));
    const failedClone = failed.clone();
    const failures = await Promise.allSettled([failed.text(), failedClone.text()]);
    check(failures.every(value => value.status === 'rejected' && value.reason === error), 'source error identity');
    let cloneError;
    const invalid = new Response(new ReadableStream({start(c) { c.enqueue(() => {}); }, cancel(error) { cloneError = error; }}));
    const invalidClone = invalid.clone();
    const invalidResults = await Promise.allSettled([invalid.body.getReader().read(), invalidClone.body.getReader().read()]);
    check(invalidResults.every(value => value.status === 'rejected' && value.reason.name === 'DataCloneError' && value.reason === cloneError), 'uncloneable chunk errors both and cancels source');
    const callable = () => {};
    const onlyOriginal = new Response(new ReadableStream({start(c) { c.enqueue(callable); c.close(); }}));
    const unused = onlyOriginal.clone();
    const cancellation = unused.body.cancel();
    const reader = onlyOriginal.body.getReader();
    check((await reader.read()).value === callable && (await reader.read()).done, 'cancelled second branch needs no chunk clone');
    await cancellation;
  } else if (scenario === 'surface') {
    const NativeResponse = Response;
    const NativeHeaders = Headers;
    const descriptors = ['Response', 'Headers'].map(name => [name, Object.getOwnPropertyDescriptor(globalThis, name)]);
    const factories = [
      ['buffered', () => new NativeResponse('buffered', {status: 201, statusText: 'Created', headers: {'X-Value': 'original'}}), false],
      ['stream', () => new NativeResponse(makeStream('stream'), {headers: {'X-Value': 'original'}}), false],
      ['null', () => new NativeResponse(null), false],
      ['error', () => NativeResponse.error(), true],
      ['redirect', () => NativeResponse.redirect(url), true],
      ['network', () => fetch(url), true],
      ['head', () => fetch(url, {method: 'HEAD'}), true],
      ['opaque', () => { const target = new URL(url); target.hostname = 'localhost'; return fetch(target, {mode: 'no-cors'}); }, true],
    ];
    for (const [name, make, immutable] of factories) {
      const response = await make();
      const headers = response.headers;
      const body = response.body;
      const keys = ['status', 'statusText', 'type', 'url', 'ok', 'redirected'];
      const expected = keys.map(key => response[key]);
      let clone;
      for (const [name] of descriptors) Object.defineProperty(globalThis, name, {configurable: true, get() { throw new Error('public ' + name + ' accessed'); }});
      try { clone = response.clone(); }
      finally { for (const [name, descriptor] of descriptors) Object.defineProperty(globalThis, name, descriptor); }
      check(Object.getPrototypeOf(clone) === NativeResponse.prototype, name + ': intrinsic Response prototype');
      check(Object.getPrototypeOf(clone.headers) === NativeHeaders.prototype, name + ': intrinsic Headers prototype');
      check(keys.every((key, i) => clone[key] === expected[i]), name + ': metadata');
      check(clone.headers !== headers && JSON.stringify([...clone.headers]) === JSON.stringify([...headers]), name + ': independent headers');
      if (immutable) {
        throwsTypeError(() => headers.set('X-Value', 'changed'), name + ': immutable original headers');
        throwsTypeError(() => clone.headers.set('X-Value', 'changed'), name + ': immutable cloned headers');
      }
      else {
        clone.headers.set('X-Value', 'changed');
        check(headers.get('X-Value') !== 'changed', name + ': header mutation isolation');
        clone.headers.set('Set-Cookie', 'hidden=1');
        check(!clone.headers.has('Set-Cookie'), name + ': response header guard');
      }
      check((body === null) === (clone.body === null), name + ': body presence');
      check(await response.text() === await clone.text(), name + ': body data');
      check(response.bodyUsed === (body !== null) && clone.bodyUsed === (body !== null), name + ': consumption state');
    }
    class DerivedResponse extends NativeResponse {}
    check(Object.getPrototypeOf(new DerivedResponse('subclass').clone()) === NativeResponse.prototype, 'clone is a base Response');
    if (typeof document === 'object') {
      const iframe = document.createElement('iframe');
      const loaded = new Promise(resolve => { iframe.onload = resolve; });
      iframe.srcdoc = '<!doctype html><script>self.response = new Response(new ReadableStream({start(c) { c.enqueue(new Uint8Array([65,66])); c.close(); }}));<\/script>';
      document.body.appendChild(iframe);
      await loaded;
      const foreign = iframe.contentWindow;
      const clone = NativeResponse.prototype.clone.call(foreign.response);
      check(Object.getPrototypeOf(clone) === foreign.Response.prototype, 'borrowed clone uses response realm');
      check(clone.headers instanceof foreign.Headers, 'clone headers use response realm');
      check(clone.body instanceof foreign.ReadableStream, 'clone body uses response realm');
      check(await clone.text() === 'AB' && await foreign.response.text() === 'AB', 'cross-realm clone bytes');
      iframe.remove();
    }
  }
  return {errors};
}
