(async () => {
  const checks = [];
  globalThis.__codecChecks = checks;
  const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
  const check = async (name, body) => {
    globalThis.__codecCurrentCheck = name;
    try { await body(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error?.stack || error)}); }
  };
  const throws = (invoke, name, realm = globalThis) => {
    let caught;
    try { invoke(); } catch (error) { caught = error; }
    assert(caught && caught.name === name, `expected ${name}, got ${caught}`);
    if (name === 'TypeError' || name === 'RangeError') assert(caught instanceof realm[name], 'error realm');
    else assert(caught instanceof realm.DOMException, 'DOMException realm');
    return caught;
  };
  const rejected = async promise => {
    assert(promise && typeof promise.then === 'function', 'a Promise is required');
    let result;
    await promise.then(() => { throw Error('unexpected fulfillment'); }, error => { result = error; });
    return result;
  };
  const finish = () => { globalThis.__uiEventResults = {complete: true, checks, total: checks.length, passed: checks.filter(row => row.passed).length}; return checks.every(row => row.passed); };
  if (!isSecureContext) {
    await check('secure-only WebTransport interfaces stay absent', () => {
      for (const name of ['WebTransport', 'WebTransportDatagramDuplexStream', 'WebTransportDatagramsWritable', 'WebTransportSendGroup', 'WebTransportBidirectionalStream', 'WebTransportError']) assert(!(name in globalThis), name);
    });
    return finish();
  }
  const C = WebTransport;
  const make = (options, realm = globalThis) => new realm.WebTransport('https://web-platform.localhost:0/', options);
  const getter = (C, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(C.prototype, name);
    assert(descriptor && typeof descriptor.get === 'function', name + ' descriptor');
    return descriptor.get;
  };
  const setter = (C, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(C.prototype, name);
    assert(descriptor && typeof descriptor.set === 'function', name + ' setter');
    return descriptor.set;
  };
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const other = frame.contentWindow;

  await check('constructor requires new and a converted URL', () => {
    assert(C.name === 'WebTransport' && C.length === 1);
    throws(() => C('https://example.test/'), 'TypeError');
    throws(() => new C(), 'TypeError');
    throws(() => new C(Symbol()), 'TypeError');
  });
  await check('absolute URLs reject wrong schemes malformed hosts and fragments', () => {
    for (const url of ['http://example.test/', 'ws://example.test/', 'https:///', 'https://example.test/#', 'https://example.test/#x', 'https://example.test:65536/']) throws(() => new C(url), 'SyntaxError');
  });
  await check('relative URLs use the live API base URL', () => {
    const base = document.createElement('base');
    document.head.prepend(base);
    try {
      base.href = 'https://example.test/directory/';
      for (const input of [null, '', 'relative', '/root']) new C(input).close();
      base.href = 'http://example.test/';
      throws(() => new C('relative'), 'SyntaxError');
      new C('https://example.test/').close();
    } finally { base.remove(); }
  });
  await check('protocols reject duplicates controls and non-ASCII', () => {
    for (const protocols of [[''], ['a', 'a'], ['\u8aa4'], ['a\n'], ['\x7f'], ['\xff'], ['a'.repeat(513)]]) throws(() => make({protocols}), 'SyntaxError');
  });
  await check('structured protocol strings retain printable punctuation', () => {
    make({protocols: ['a b', 'quote"slash\\', '~!#$%&*()']}).close();
  });
  await check('512 byte protocol boundary follows the current specification', () => {
    make({protocols: ['a'.repeat(512)]}).close();
  });
  await check('constructor dictionaries convert recognized members lexically', () => {
    const log = [];
    const members = ['allowPooling', 'anticipatedConcurrentIncomingBidirectionalStreams', 'anticipatedConcurrentIncomingUnidirectionalStreams', 'congestionControl', 'datagramsReadableType', 'headers', 'protocols', 'requireUnreliable', 'serverCertificateHashes'];
    const options = {};
    for (const member of members) Object.defineProperty(options, member, {get() {
      log.push(member);
      if (member === 'headers') return {get a() { log.push('header.value'); return 'b'; }};
      if (member === 'protocols') return [{toString() { log.push('protocol.value'); return 'a'; }}];
      if (member === 'serverCertificateHashes') return [{get algorithm() { log.push('hash.algorithm'); return 'sha-256'; }, get value() { log.push('hash.value'); return new Uint8Array(32); }}];
      return undefined;
    }});
    const wt = new C({toString() { log.push('url'); return 'https://example.test/'; }}, options);
    wt.close();
    const expected = ['url', ...members.slice(0, 6), 'header.value', 'protocols', 'protocol.value', 'requireUnreliable', 'serverCertificateHashes', 'hash.algorithm', 'hash.value'];
    assert(JSON.stringify(log) === JSON.stringify(expected), JSON.stringify(log));
  });
  await check('dictionary exceptions precede URL semantic validation', () => {
    const sentinel = {};
    let caught;
    try { new C('http://invalid.test/', {get serverCertificateHashes() { throw sentinel; }}); } catch (error) { caught = error; }
    assert(caught === sentinel);
  });
  await check('certificate dictionaries validate required BufferSource identities', () => {
    for (const hash of [{algorithm: 'sha-256'}, {value: new Uint8Array(32)}, {algorithm: 'sha-256', value: new Proxy(new Uint8Array(32), {})}]) throws(() => make({serverCertificateHashes: [hash], protocols: ['']}), 'TypeError');
    const buffer = new ArrayBuffer(32, {maxByteLength: 64});
    throws(() => make({serverCertificateHashes: [{algorithm: 'sha-256', value: buffer}]}), 'TypeError');
  });
  await check('pooling rejects certificate hashes before protocol validation', () => {
    throws(() => make({allowPooling: true, serverCertificateHashes: [{algorithm: 'sha-256', value: new Uint8Array(32)}], protocols: ['']}), 'NotSupportedError');
  });
  await check('HeadersInit conversion preserves iterator and ByteString failures', () => {
    throws(() => make({headers: null}), 'TypeError');
    throws(() => make({headers: {a: '\u1000'}}), 'TypeError');
    const sentinel = {};
    let caught;
    try { make({headers: {[Symbol.iterator]() { throw sentinel; }}}); } catch (error) { caught = error; }
    assert(caught === sentinel);
  });
  await check('HTTP header syntax validation follows complete dictionary conversion', () => {
    const log = [];
    throws(() => make({headers: [['bad name', 'x']], get serverCertificateHashes() { log.push('converted'); return []; }}), 'TypeError');
    assert(log.join() === 'converted');
    throws(() => make({headers: [['a']]}), 'TypeError');
    throws(() => make({headers: [['a', 'b\nc']]}), 'TypeError');
    throws(() => make({headers: {'WT-Available-Protocols': 'x'}}), 'TypeError');
    make({headers: new Headers({a: 'b'})}).close();
  });
  await check('supportsReliableOnly is an enumerable static accessor', () => {
    const d = Object.getOwnPropertyDescriptor(C, 'supportsReliableOnly');
    assert(d && d.enumerable && d.configurable && d.set === undefined);
    assert(d.get.length === 0 && d.get.name === 'get supportsReliableOnly');
    assert(typeof d.get.call({}) === 'boolean');
    assert(!Object.hasOwn(C.prototype, 'supportsReliableOnly'));
  });
  await check('initial session exposes stable promises and native incoming streams', () => {
    const wt = make();
    for (const name of ['ready', 'closed', 'draining']) assert(wt[name] instanceof Promise && wt[name] === wt[name], name);
    for (const name of ['incomingBidirectionalStreams', 'incomingUnidirectionalStreams']) assert(wt[name] instanceof ReadableStream && wt[name] === wt[name] && !wt[name].locked, name);
    assert(wt.reliability === 'pending' && wt.protocol === '' && wt.responseHeaders === null);
    wt.close();
  });
  await check('congestion preferences normalize to an implementation-supported mode', () => {
    for (const congestionControl of ['default', 'throughput', 'low-latency']) {
      const wt = make({congestionControl});
      assert(['default', congestionControl].includes(wt.congestionControl));
      wt.close();
    }
    throws(() => make({congestionControl: 'invalid'}), 'TypeError');
  });
  await check('anticipated stream limits enforce nullable unsigned short conversion', () => {
    const wt = make({anticipatedConcurrentIncomingBidirectionalStreams: 7});
    for (const name of ['anticipatedConcurrentIncomingBidirectionalStreams', 'anticipatedConcurrentIncomingUnidirectionalStreams']) {
      assert(wt[name] === (name.includes('Bidirectional') ? 7 : null));
      wt[name] = 65535.9; assert(wt[name] === 65535);
      for (const value of [-1, 65536, NaN, Infinity, 1n]) throws(() => { wt[name] = value; }, 'TypeError');
      assert(wt[name] === 65535);
      wt[name] = null; assert(wt[name] === null);
    }
    wt.close();
  });
  await check('brand checks reject forged and author Proxy receivers before conversion', async () => {
    const wt = make();
    const revoked = Proxy.revocable(wt, {}); revoked.revoke();
    let conversions = 0, traps = 0;
    const value = {toString() { conversions++; return ''; }};
    for (const receiver of [{}, Object.create(wt), Object.create(C.prototype), new Proxy(wt, {get() { traps++; throw Error('trap'); }}), revoked.proxy]) {
      throws(() => C.prototype.close.call(receiver, {reason: value}), 'TypeError');
      throws(() => getter(C, 'reliability').call(receiver), 'TypeError');
      const e = await rejected(C.prototype.exportKeyingMaterial.call(receiver, value, value, value));
      assert(e instanceof TypeError);
      assert(await rejected(getter(C, 'ready').call(receiver)) instanceof TypeError);
    }
    assert(conversions === 0 && traps === 0);
    wt.close();
  });
  await check('native brands survive JavaScript prototype replacement', () => {
    const wt = make();
    const ready = wt.ready;
    Object.setPrototypeOf(wt, {});
    assert(getter(C, 'ready').call(wt) === ready);
    C.prototype.close.call(wt);
  });
  await check('cross-realm receivers use callee errors and owner result identities', async () => {
    throws(() => other.WebTransport.prototype.close.call({}, {reason: Symbol()}), 'TypeError', other);
    const wt = make(undefined, other);
    assert(getter(C, 'ready').call(wt) instanceof other.Promise);
    const group = C.prototype.createSendGroup.call(wt);
    assert(group instanceof other.WebTransportSendGroup);
    const writable = WebTransportDatagramDuplexStream.prototype.createWritable.call(wt.datagrams, {sendGroup: group});
    assert(writable instanceof other.WebTransportDatagramsWritable);
    const pending = C.prototype.getStats.call(wt);
    assert(pending instanceof Promise && !(pending instanceof other.Promise));
    const e = await rejected(wt.ready);
    assert(e instanceof other.WebTransportError && !(e instanceof WebTransportError));
    const resumed = await rejected(pending);
    assert(resumed instanceof DOMException && !(resumed instanceof other.DOMException) && resumed.name === 'InvalidStateError');
    assert(await rejected(C.prototype.getStats.call(wt)) instanceof DOMException);
  });
  await check('datagram duplex initial values and stream identity are stable', () => {
    const wt = make(), d = wt.datagrams;
    assert(d instanceof WebTransportDatagramDuplexStream && d === wt.datagrams);
    assert(d.readable instanceof ReadableStream && d.readable === d.readable);
    assert(Number.isInteger(d.maxDatagramSize) && d.maxDatagramSize >= 0);
    assert(d.incomingMaxAge === null && d.outgoingMaxAge === null);
    assert(d.incomingMaxBufferedDatagrams >= 1 && d.outgoingMaxBufferedDatagrams >= 1);
    wt.close();
  });
  await check('datagramsReadableType selects native byte reading support', async () => {
    const wt = make({datagramsReadableType: 'bytes'});
    const reader = wt.datagrams.readable.getReader({mode: 'byob'});
    const read = reader.read(new Uint8Array(32));
    wt.close();
    assert(await rejected(read) === await rejected(wt.closed));
    const ordinary = make();
    throws(() => ordinary.datagrams.readable.getReader({mode: 'byob'}), 'TypeError');
    ordinary.close();
    throws(() => make({datagramsReadableType: 'other'}), 'TypeError');
  });
  await check('datagram maximum ages preserve infinity and normalize zero', () => {
    const wt = make(), d = wt.datagrams;
    for (const name of ['incomingMaxAge', 'outgoingMaxAge']) {
      for (const value of [0.5, Infinity]) { d[name] = value; assert(d[name] === value); }
      for (const value of [-1, -Infinity, NaN]) throws(() => { d[name] = value; }, 'RangeError');
      assert(d[name] === Infinity);
      for (const value of [0, -0, null, undefined]) { d[name] = value; assert(d[name] === null); }
    }
    wt.close();
  });
  await check('datagram buffering limits use unsigned long wrapping and a minimum of one', () => {
    const wt = make(), d = wt.datagrams;
    for (const name of ['incomingMaxBufferedDatagrams', 'outgoingMaxBufferedDatagrams']) {
      for (const [input, expected] of [[-1, 4294967295], [4294967296, 1], [0, 1], [2.7, 2], [NaN, 1], [Infinity, 1]]) { d[name] = input; assert(d[name] === expected); }
      throws(() => { d[name] = 1n; }, 'TypeError');
    }
    wt.close();
  });
  await check('datagram setters validate brands before numeric conversion', () => {
    const wt = make(), d = wt.datagrams;
    let conversions = 0, traps = 0;
    const value = {valueOf() { conversions++; return 1; }};
    for (const name of ['incomingMaxAge', 'outgoingMaxAge', 'incomingMaxBufferedDatagrams', 'outgoingMaxBufferedDatagrams']) {
      const set = setter(WebTransportDatagramDuplexStream, name);
      throws(() => set.call(new Proxy(d, {get() { traps++; }}), value), 'TypeError');
      throws(() => set.call(Object.create(d), value), 'TypeError');
    }
    assert(conversions === 0 && traps === 0);
    wt.close();
  });
  await check('datagram writables have concrete native WritableStream ancestry', () => {
    const wt = make(), a = wt.datagrams.createWritable(), b = wt.datagrams.createWritable();
    assert(a !== b && a instanceof WebTransportDatagramsWritable && a instanceof WritableStream);
    assert(Object.getPrototypeOf(a) === WebTransportDatagramsWritable.prototype);
    assert(a.sendGroup === null && a.sendOrder === 0 && !a.locked);
    wt.close();
  });
  await check('send groups retain native owner identity and reject foreign associations', () => {
    const a = make(), b = make(), ga = a.createSendGroup(), gb = b.createSendGroup();
    assert(ga instanceof WebTransportSendGroup);
    const writable = a.datagrams.createWritable({sendGroup: ga, sendOrder: 4});
    assert(writable.sendGroup === ga && writable.sendOrder === 4);
    throws(() => a.datagrams.createWritable({sendGroup: gb}), 'TypeError');
    throws(() => { writable.sendGroup = gb; }, 'InvalidStateError');
    assert(writable.sendGroup === ga);
    writable.sendGroup = null; assert(writable.sendGroup === null);
    throws(() => a.datagrams.createWritable({sendGroup: new Proxy(ga, {})}), 'TypeError');
    a.close(); b.close();
  });
  await check('sendOrder conversion remains usable after session failure', () => {
    const wt = make(), writable = wt.datagrams.createWritable(); wt.close();
    for (const [input, expected] of [[3.7, 3], [-3.7, -3], [NaN, 0], [Infinity, 0], [2 ** 63, -(2 ** 63)]]) { writable.sendOrder = input; assert(writable.sendOrder === expected); }
    throws(() => { writable.sendOrder = 1n; }, 'TypeError');
  });
  await check('empty send-group stats aggregate locally on an owner network task', async () => {
    const wt = make(), group = wt.createSendGroup();
    const promise = group.getStats();
    let settled = false; promise.then(() => { settled = true; });
    await Promise.resolve(); assert(!settled);
    const stats = await promise;
    assert(stats.bytesWritten === 0 && stats.bytesSent === 0 && stats.bytesAcknowledged === 0);
    assert(await rejected(wt.ready) instanceof WebTransportError);
    const again = await group.getStats();
    assert(again !== stats && again.bytesWritten === 0);
  });
  await check('bad datagram chunks reject through the native WritableStream sink', async () => {
    const wt = make(), writer = wt.datagrams.createWritable().getWriter();
    const error = await rejected(writer.write({}));
    assert(error instanceof TypeError);
    assert(await rejected(writer.closed) === error);
    wt.close();
  });
  await check('oversize datagrams are dropped before transport delivery', async () => {
    const wt = make(), writer = wt.datagrams.createWritable().getWriter();
    assert(await writer.write(new Uint8Array(wt.datagrams.maxDatagramSize + 1)) === undefined);
    wt.close();
    await rejected(writer.closed);
  });
  await check('close while connecting synchronously rejects a single session error', async () => {
    const wt = make();
    const reader = wt.incomingBidirectionalStreams.getReader();
    const read = reader.read();
    wt.close({closeCode: 33, reason: 'bye'});
    const e = await rejected(wt.closed);
    assert(e instanceof WebTransportError && e.source === 'session' && e.streamErrorCode === null);
    assert(await rejected(wt.ready) === e && await rejected(read) === e);
    throws(() => wt.createSendGroup(), 'InvalidStateError');
  });
  await check('closed sessions still convert closeInfo before the no-op', () => {
    const wt = make(); wt.close();
    const sentinel = {}, log = [];
    let caught;
    try { wt.close({get closeCode() { log.push('code'); return 1; }, get reason() { log.push('reason'); throw sentinel; }}); } catch (error) { caught = error; }
    assert(caught === sentinel && log.join() === 'code,reason');
  });
  await check('connection failure waits for the networking task source', async () => {
    const wt = make(); let settled = false;
    wt.ready.catch(() => { settled = true; });
    await Promise.resolve(); assert(!settled);
    assert(await rejected(wt.ready) instanceof WebTransportError);
    assert(settled);
  });
  await check('failure rejects ready closed and pending readable requests with one error', async () => {
    const wt = make();
    const reads = [wt.incomingBidirectionalStreams, wt.incomingUnidirectionalStreams, wt.datagrams.readable].map(stream => rejected(stream.getReader().read()));
    const error = await rejected(wt.ready);
    assert(error === await rejected(wt.closed));
    assert((await Promise.all(reads)).every(value => value === error));
    assert(error.source === 'session' && error.streamErrorCode === null);
  });
  await check('waiting methods reject on later network tasks after connection failure', async () => {
    const wt = make(), log = [];
    const promises = [wt.getStats(), wt.createBidirectionalStream(), wt.createUnidirectionalStream(), wt.exportKeyingMaterial(new Uint8Array(), new Uint8Array(), 32)];
    const errors = promises.map((promise, index) => rejected(promise).then(error => { log.push(index); return error; }));
    wt.ready.catch(() => log.push('ready'));
    for (const error of await Promise.all(errors)) assert(error instanceof DOMException && error.name === 'InvalidStateError');
    assert(JSON.stringify(log) === '["ready",0,1,2,3]', JSON.stringify(log));
  });
  await check('key export validates current BufferSource lengths before session state', async () => {
    const wt = make(); wt.close();
    for (const args of [[new Uint8Array(256), new Uint8Array(), 1], [new Uint8Array(), new Uint8Array(256), 1], [new Uint8Array(), new Uint8Array(), 0], [new Uint8Array(), new Uint8Array(), 4097]]) assert(await rejected(wt.exportKeyingMaterial(...args)) instanceof RangeError);
    assert(await rejected(wt.exportKeyingMaterial(new Uint8Array(), new Uint8Array(), 1)) instanceof DOMException);
    assert(await rejected(wt.exportKeyingMaterial(new Uint8Array())) instanceof TypeError);
    const label = new Uint8Array(256);
    const length = {valueOf() { structuredClone(label.buffer, {transfer: [label.buffer]}); return 1; }};
    const detached = await rejected(wt.exportKeyingMaterial(label, new Uint8Array(), length));
    assert(detached instanceof DOMException && detached.name === 'InvalidStateError', 'length checked after all argument conversion');
  });
  await check('failed session factories retain conversion and foreign-group precedence', async () => {
    const a = make(), b = make(), group = b.createSendGroup(); a.close();
    assert(await rejected(a.createBidirectionalStream({sendGroup: group})) instanceof TypeError);
    throws(() => a.datagrams.createWritable({sendGroup: group}), 'TypeError');
    throws(() => a.datagrams.createWritable(), 'InvalidStateError');
    assert((await rejected(a.createUnidirectionalStream())).name === 'InvalidStateError');
    b.close();
  });
  await check('buffered datagram writes and writer promises settle on failure', async () => {
    const wt = make(); wt.datagrams.outgoingMaxBufferedDatagrams = 1;
    const writer = wt.datagrams.createWritable().getWriter();
    const writes = [writer.write(new Uint8Array([1])), writer.write(new Uint8Array([2]))].map(rejected);
    const ready = rejected(writer.ready), closed = rejected(writer.closed);
    const error = await rejected(wt.ready);
    assert((await Promise.all([...writes, ready, closed])).every(value => value === error));
  });
  await check('intrinsic allocation survives public constructor replacement', async () => {
    const names = ['WebTransport', 'WebTransportError', 'WebTransportDatagramsWritable', 'WebTransportSendGroup', 'ReadableStream', 'WritableStream', 'DOMException'];
    const saved = Object.fromEntries(names.map(name => [name, globalThis[name]]));
    try {
      for (const name of names) globalThis[name] = function Poison() { throw Error('public constructor'); };
      const wt = new saved.WebTransport('https://example.test:0/');
      assert(wt.datagrams.readable instanceof saved.ReadableStream);
      assert(wt.datagrams.createWritable() instanceof saved.WebTransportDatagramsWritable);
      assert(wt.createSendGroup() instanceof saved.WebTransportSendGroup);
      wt.close(); assert(await rejected(wt.ready) instanceof saved.WebTransportError);
    } finally { for (const name of names) globalThis[name] = saved[name]; }
  });
  await check('unobserved ready and closed failures are internally handled', async () => {
    const errors = [];
    const listener = event => { if (event.reason instanceof WebTransportError) { errors.push(event.reason); event.preventDefault(); } };
    addEventListener('unhandledrejection', listener);
    try {
      make();
      await new Promise(resolve => setTimeout(resolve, 0));
      await new Promise(resolve => setTimeout(resolve, 0));
      assert(errors.length === 0);
    } finally { removeEventListener('unhandledrejection', listener); }
  });
  await check('author timer cancellation cannot cancel the connection task', async () => {
    const wt = make();
    for (let id = 1; id <= 128; id++) { clearTimeout(id); clearInterval(id); }
    assert(await rejected(wt.ready) instanceof WebTransportError);
  });
  await check('dedicated workers own failure tasks and stream promises', async () => {
    const source = `onmessage=async()=>{try{const wt=new WebTransport('https://web-platform.localhost:0/',{datagramsReadableType:'bytes'});const reader=wt.datagrams.readable.getReader({mode:'byob'});const read=reader.read(new Uint8Array(16)).catch(e=>e);const stats=wt.getStats().catch(e=>e);let before=true;wt.ready.catch(()=>{before=false});await Promise.resolve();const pending=before;const e=await wt.ready.catch(e=>e);const closed=await wt.closed.catch(e=>e);const result=await stats;postMessage({ok:pending&&e instanceof WebTransportError&&e===closed&&await read===e&&result.name==='InvalidStateError'&&wt.reliability==='pending'});}catch(e){postMessage({ok:false,error:String(e.stack||e)})}};`;
    const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
    const worker = new Worker(url);
    try {
      const response = await new Promise((resolve, reject) => { worker.onmessage = event => resolve(event.data); worker.onerror = event => reject(Error(event.message)); worker.postMessage('go'); });
      assert(response.ok, JSON.stringify(response));
    } finally { worker.terminate(); URL.revokeObjectURL(url); }
  });
  return finish();
})().catch(error => { globalThis.__uiEventResults = {complete: true, checks: [{name: 'fixture setup', passed: false, error: String(error?.stack || error)}], total: 1, passed: 0}; return false; });
