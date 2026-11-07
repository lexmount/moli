globalThis.probeWebSocketStreamNativeAbort = async (realm, signalRealm, url, mode) => {
  const controller = new signalRealm.AbortController();
  const signal = controller.signal;
  let gets = 0, calls = 0;
  const marker = {};
  if (mode === 'getter') {
    for (const key of ['addEventListener', 'removeEventListener', 'aborted', 'reason']) {
      Object.defineProperty(signal, key, {get() {gets++; throw marker;}});
    }
  } else if (mode === 'noop') {
    Object.defineProperty(signal, 'addEventListener', {get() {gets++; return () => calls++;}});
  } else if (mode === 'stopping') {
    signalRealm.EventTarget.prototype.addEventListener.call(signal, 'abort', event => event.stopImmediatePropagation());
  } else if (mode === 'prototype') {
    Object.setPrototypeOf(signal, null);
  }
  const stream = new realm.WebSocketStream(url, {signal});
  const opened = stream.opened.then(() => {stream.close(); return null;}, error => error);
  const closed = stream.closed.then(() => null, error => error);
  controller.abort();
  const a = await opened, b = await closed;
  const nativeError = error => error instanceof realm.DOMException || error instanceof signalRealm.DOMException;
  if (gets || calls || !nativeError(a) || !nativeError(b) || a !== b ||
      a.name !== 'AbortError' || b.name !== 'AbortError') throw Error('native cancellation: ' + mode);
  return {gets, calls, opened: a.name, closed: b.name};
};

globalThis.probeWebSocketStreamSyntheticAbort = async (realm, signalRealm, url) => {
  const controller = new signalRealm.AbortController();
  const signal = controller.signal;
  let cleanupReads = 0;
  Object.defineProperty(signal, 'removeEventListener', {get() {cleanupReads++; throw Error('public cleanup');}});
  const stream = new realm.WebSocketStream(url, {signal});
  const closed = stream.closed;
  closed.catch(() => {});
  signalRealm.EventTarget.prototype.dispatchEvent.call(signal, new signalRealm.Event('abort'));
  if (signal.aborted) throw Error('synthetic abort changed native state');
  const {readable, writable} = await stream.opened;
  controller.abort();
  const reader = readable.getReader(), writer = writable.getWriter();
  await writer.write('alive-after-synthetic-abort');
  const value = await reader.read();
  if (value.done || value.value !== 'alive-after-synthetic-abort') throw Error('synthetic event closed transport');
  await writer.close();
  await closed;
  if (cleanupReads) throw Error('public abort cleanup');
  return true;
};

globalThis.probeWebSocketStreamAbortOptions = (realm, signalRealm) => {
  const controller = new signalRealm.AbortController();
  controller.abort();
  const signal = controller.signal;
  const revoked = Proxy.revocable(signal, {});revoked.revoke();
  const url = 'ws://127.0.0.1:1/not-started';
  const reject = init => {
    let error;
    try {
      const s = new realm.WebSocketStream(url, init);
      s.opened.catch(() => {});s.closed.catch(() => {});s.close();
    } catch (e) {error = e;}
    if (!(error instanceof realm.TypeError)) throw Error('signal/sequence TypeError');
  };
  for (const bad of [null, {}, Object.create(signalRealm.AbortSignal.prototype), Object.create(signal), new Proxy(signal, {}), revoked.proxy, 7]) reject({signal: bad});
  reject({protocols: null, signal});
  for (const marker of [{}, null, undefined, 7]) {
    const reads = [];
    let caught = Symbol();
    try {
      new realm.WebSocketStream('ftp://invalid', {
        get protocols() {reads.push('protocols');return {[Symbol.iterator]() {reads.push('iterator');throw marker;}};},
        get signal() {reads.push('signal');return {};}
      });
    } catch (e) {caught = e;}
    if (caught !== marker || reads.join() !== 'protocols,iterator') throw Error('sequence conversion before signal and URL validation');
  }
  let traps = 0;
  const guarded = new Proxy(signal, {get() {traps++;throw 7;}, getPrototypeOf() {traps++;throw 7;}});
  reject({signal: guarded});
  if (traps) throw Error('native signal brand must ignore author traps');
  Object.setPrototypeOf(signal, null);
  const valid = new realm.WebSocketStream(url, {signal});
  valid.opened.catch(() => {});valid.closed.catch(() => {});
  return true;
};
