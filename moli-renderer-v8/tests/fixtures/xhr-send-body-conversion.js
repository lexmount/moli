function xhrSendBodyConversionProbe() {
  const checks = [];
  const check = (label, actual, wanted) => checks.push({label, actual, wanted,
    pass: JSON.stringify(actual) === JSON.stringify(wanted)});
  const views = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array,
    Int32Array, Uint32Array, BigInt64Array, BigUint64Array, Float16Array,
    Float32Array, Float64Array, DataView];
  const Shared = new WebAssembly.Memory({shared: true, initial: 1, maximum: 1}).buffer.constructor;
  const factories = [];
  for (const [kind, make] of [
    ['shared', () => new Shared(32)],
    ['growable', () => new Shared(32, {maxByteLength: 64})],
    ['resizable', () => new ArrayBuffer(32, {maxByteLength: 64})],
  ]) {
    factories.push([kind + '/buffer', make]);
    for (const View of views)
      factories.push([kind + '/' + View.name, () => new View(make(), 8,
        View === DataView ? 8 : 8 / View.BYTES_PER_ELEMENT)]);
  }
  factories.push(['empty-shared', () => new Shared(0)],
    ['empty-resizable', () => new ArrayBuffer(0, {maxByteLength: 16})],
    ['out-of-bounds', () => { const b = new ArrayBuffer(16, {maxByteLength: 32});
      const view = new Uint8Array(b, 8, 8); b.resize(0); return view; }],
    ['symbol', () => Symbol('body')], ['throwing-string', () => ({})]);
  const dataUrl = 'data:text/plain,complete';
  for (const phase of ['unsent', 'GET', 'HEAD', 'POST', 'pending', 'done']) {
    for (const [kind, create] of factories) {
      const label = phase + '/' + kind;
      const xhr = new XMLHttpRequest();
      if (phase === 'pending') { xhr.open('POST', dataUrl); xhr.send(); }
      else if (phase === 'done') { xhr.open('GET', dataUrl, false); xhr.send(); }
      else if (phase !== 'unsent') xhr.open(phase, dataUrl);
      const before = [xhr.readyState, xhr.status, xhr.responseText];
      let events = 0;
      for (const type of ['readystatechange', 'loadstart', 'progress', 'load', 'error', 'loadend'])
        xhr.addEventListener(type, () => events++);
      let coerced = 0;
      const marker = new RangeError('body coercion');
      const body = create();
      if (typeof body === 'object') {
        Object.defineProperty(body, Symbol.toPrimitive, {value() {
          coerced++;
          if (kind === 'throwing-string') throw marker;
          return 'unexpected fallback';
        }});
        if (ArrayBuffer.isView(body)) {
          for (const property of ['buffer', 'byteOffset', 'byteLength'])
            Object.defineProperty(body, property, {get() { throw marker; }});
        }
      }
      let caught;
      try { xhr.send(body); } catch (error) { caught = error; }
      check(label + '/error', caught && caught.name || null,
        kind === 'throwing-string' ? 'RangeError' : 'TypeError');
      check(label + '/identity', kind === 'throwing-string' ? caught === marker : caught instanceof TypeError, true);
      check(label + '/coercion', coerced, kind === 'throwing-string' ? 1 : 0);
      check(label + '/state', [xhr.readyState, xhr.status, xhr.responseText], before);
      check(label + '/events', events, 0);
      xhr.abort();
    }
  }
  for (const phase of ['unsent', 'GET', 'HEAD', 'POST', 'pending', 'done']) {
    const xhr = new XMLHttpRequest();
    if (phase === 'pending') { xhr.open('POST', dataUrl); xhr.send(); }
    else if (phase === 'done') { xhr.open('GET', dataUrl, false); xhr.send(); }
    else if (phase !== 'unsent') xhr.open(phase, dataUrl);
    let calls = 0, error = null;
    try { xhr.send({toString() { calls++; return 'converted'; }}); }
    catch (caught) { error = caught.name; }
    check(phase + '/ordinary/coercion', calls, 1);
    check(phase + '/ordinary/error', error,
      ['unsent', 'pending', 'done'].includes(phase) ? 'InvalidStateError' : null);
    xhr.abort();
  }
  const revoked = Proxy.revocable(new XMLHttpRequest(), {}); revoked.revoke();
  for (const [index, receiver] of [null, {}, XMLHttpRequest.prototype,
    Object.create(XMLHttpRequest.prototype), Object.create(new XMLHttpRequest()),
    new Proxy(new XMLHttpRequest(), {}), revoked.proxy].entries()) {
    let calls = 0, caught;
    try { XMLHttpRequest.prototype.send.call(receiver, {toString() { calls++; return 'converted'; }}); }
    catch (error) { caught = error; }
    check('receiver/' + index + '/error', caught instanceof TypeError, true);
    check('receiver/' + index + '/coercion', calls, 0);
  }
  for (const kind of ['fixed', 'detached']) {
    const buffer = new ArrayBuffer(8);
    const view = new Uint8Array(buffer, 2, 4);
    const dataView = new DataView(buffer, 2, 4);
    if (kind === 'detached') buffer.transfer();
    for (const [index, body] of [buffer, view, dataView].entries()) {
      const xhr = new XMLHttpRequest(); xhr.open('POST', dataUrl, false);
      let error = null; try { xhr.send(body); } catch (caught) { error = caught.name; }
      check(kind + '/' + index + '/error', error, null);
      check(kind + '/' + index + '/status', xhr.status, 200);
    }
  }
  const retry = new XMLHttpRequest(); retry.open('POST', dataUrl, false);
  let rejected = false;
  try { retry.send(new Uint8Array(new Shared(8))); } catch (error) { rejected = error instanceof TypeError; }
  check('retry/rejected', rejected, true);
  let retryError = null;
  try { retry.send(new Uint8Array([65])); } catch (error) { retryError = error.name; }
  check('retry/error', retryError, null);
  check('retry/status', retry.status, 200);
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
