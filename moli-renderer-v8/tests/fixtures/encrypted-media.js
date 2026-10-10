(async () => {
  const checks = [];
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const check = async (name, body) => {
    globalThis.__codecCurrentCheck = name;
    try { await body(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const rejected = async (realm, body, name, identity) => {
    let promise;
    try { promise = body(); } catch (error) { throw Error('synchronous throw: ' + error); }
    assert(promise instanceof realm.Promise, 'callee Promise');
    const error = await promise.then(() => { throw Error('unexpected resolution'); }, error => error);
    if (identity) assert(error === identity, 'original thrown identity');
    else if (name === 'TypeError') assert(error instanceof realm.TypeError, 'callee TypeError');
    else assert(error instanceof realm.DOMException && error.name === name, 'callee ' + name);
    return error;
  };
  const thrown = (realm, body) => {
    try { body(); } catch (error) { assert(error instanceof realm.TypeError, 'callee TypeError'); return; }
    throw Error('missing TypeError');
  };
  const declarations = {
    MediaKeySystemAccess: [['keySystem', 'get'], ['getConfiguration', 'sync', 0], ['createMediaKeys', 'promise', 0]],
    MediaKeys: [['createSession', 'sync', 0], ['getStatusForPolicy', 'promise', 0], ['setServerCertificate', 'promise', 1]],
    MediaKeySession: [['sessionId', 'get'], ['expiration', 'get'], ['closed', 'promise-get'], ['keyStatuses', 'get'], ['onmessage', 'handler'], ['onkeystatuseschange', 'handler'], ['generateRequest', 'promise', 2], ['load', 'promise', 1], ['update', 'promise', 1], ['close', 'promise', 0], ['remove', 'promise', 0]],
  };
  for (const [label, realm] of [['main', globalThis], ['iframe', document.getElementById('child').contentWindow]]) {
    const local = (name, body) => check(label + ':' + name, body);
    if (!realm.isSecureContext) {
      await local('insecure-exposure', () => {
        for (const name of Object.keys(declarations)) assert(!(name in realm), name);
        assert(!('requestMediaKeySystemAccess' in realm.Navigator.prototype), 'request hidden');
        for (const name of ['mediaKeys', 'setMediaKeys']) assert(!(name in realm.HTMLMediaElement.prototype), name + ' hidden');
      });
      continue;
    }
    const method = () => {
      const fn = realm.Navigator.prototype.requestMediaKeySystemAccess;
      assert(typeof fn === 'function', 'request entry exists'); return fn;
    };
    for (const [interfaceName, members] of Object.entries(declarations)) {
      await local(interfaceName + ':constructor', () => {
        const ctor = realm[interfaceName]; assert(typeof ctor === 'function', 'native constructor');
        thrown(realm, () => new ctor());
        if (interfaceName === 'MediaKeySession') assert(Object.getPrototypeOf(ctor.prototype) === realm.EventTarget.prototype, 'EventTarget inheritance');
      });
      for (const [name, kind, length] of members) {
        const member = () => {
          const d = Object.getOwnPropertyDescriptor(realm[interfaceName].prototype, name);
          assert(d && d.configurable && d.enumerable, 'own member descriptor');
          const fn = kind === 'get' || kind === 'promise-get' || kind === 'handler' ? d.get : d.value;
          assert(typeof fn === 'function', 'native member'); return [d, fn];
        };
        await local(interfaceName + '.' + name + ':descriptor', () => {
          const [d, fn] = member();
          if (kind === 'handler') assert(typeof d.set === 'function', 'handler setter');
          else if (kind === 'get' || kind === 'promise-get') assert(d.set === undefined, 'readonly getter');
          else assert(d.writable && fn.length === length, 'method descriptor and length');
          thrown(realm, () => realm.Reflect.construct(fn, []));
        });
        await local(interfaceName + '.' + name + ':receiver', async () => {
          const [d, fn] = member(); let conversions = 0, traps = 0;
          const poison = {toString() { conversions++; throw Error('conversion'); }};
          const handler = {get() { traps++; throw Error('trap'); }, getPrototypeOf() { traps++; throw Error('trap'); }};
          const revoked = Proxy.revocable({}, handler); revoked.revoke();
          for (const receiver of [{}, Object.create(realm[interfaceName].prototype), new Proxy({}, handler), revoked.proxy]) {
            if (kind === 'promise' || kind === 'promise-get') await rejected(realm, () => fn.call(receiver, poison, poison), 'TypeError');
            else thrown(realm, () => fn.call(receiver, poison, poison));
            if (kind === 'handler') thrown(realm, () => d.set.call(receiver, poison));
          }
          assert(conversions === 0 && traps === 0, 'brand check precedes author code');
        });
      }
    }
    await local('request-descriptor', () => {
      const fn = method(); assert(fn.length === 2, 'required argument count');
      const d = Object.getOwnPropertyDescriptor(realm.Navigator.prototype, 'requestMediaKeySystemAccess');
      assert(d.enumerable && d.configurable && d.writable, 'descriptor');
      thrown(realm, () => realm.Reflect.construct(fn, []));
    });
    await local('request-illegal-receivers', async () => {
      const fn = method(); let calls = 0;
      const poison = {toString() { calls++; throw Error('conversion'); }};
      const trap = new Proxy(realm.navigator, {get() { calls++; throw Error('trap'); }, getPrototypeOf() { calls++; throw Error('trap'); }});
      const revoked = Proxy.revocable(realm.navigator, {}); revoked.revoke();
      for (const receiver of [{}, Object.create(realm.navigator), Object.create(realm.Navigator.prototype), trap, revoked.proxy]) {
        await rejected(realm, () => fn.call(receiver, poison, poison), 'TypeError');
      }
      assert(calls === 0, 'no conversion or Proxy trap');
    });
    for (const [name, args] of [
      ['missing', []], ['one-argument', ['unsupported']], ['empty-key', ['', [{}]]], ['empty-sequence', ['unsupported', []]],
      ['primitive-sequence', ['unsupported', 'bad']], ['noniterable-sequence', ['unsupported', {}]],
      ['primitive-dictionary', ['unsupported', [7]]], ['invalid-requirement', ['unsupported', [{distinctiveIdentifier: 'bad'}]]],
      ['invalid-persistence', ['unsupported', [{persistentState: 'bad'}]]], ['invalid-capability', ['unsupported', [{audioCapabilities: [4]}]]],
      ['invalid-nested-sequence', ['unsupported', [{initDataTypes: 'bad'}]]], ['symbol-key', [Symbol(), [{}]]],
    ]) await local('request:' + name, () => rejected(realm, () => method().apply(realm.navigator, args), 'TypeError'));
    for (const [name, config] of [['empty', {}], ['nullable-capability', {audioCapabilities: [{encryptionScheme: null}]}], ['nullable-configuration', null], ['unknown-session-string', {sessionTypes: ['anything']}], ['valid-enum', {distinctiveIdentifier: 'not-allowed', persistentState: 'optional'}]]) {
      await local('unsupported:' + name, () => rejected(realm, () => method().call(realm.navigator, 'com.example.unsupported', [config]), 'NotSupportedError'));
    }
    await local('all-dictionary-getters-before-backend', async () => {
      const log = [];
      const capability = {};
      for (const name of ['contentType', 'encryptionScheme', 'robustness']) Object.defineProperty(capability, name, {get() { log.push(name); return name === 'encryptionScheme' ? null : ''; }});
      const config = {};
      const values = {audioCapabilities: [capability], distinctiveIdentifier: 'optional', initDataTypes: [], label: '', persistentState: 'optional', sessionTypes: ['temporary'], videoCapabilities: [capability]};
      for (const name of Object.keys(values)) Object.defineProperty(config, name, {get() { log.push(name); return values[name]; }});
      await rejected(realm, () => method().call(realm.navigator, {toString() { log.push('key'); return 'com.example.unsupported'; }}, [config]), 'NotSupportedError');
      assert(log.join() === 'key,audioCapabilities,contentType,encryptionScheme,robustness,distinctiveIdentifier,initDataTypes,label,persistentState,sessionTypes,videoCapabilities,contentType,encryptionScheme,robustness', 'complete lexicographic dictionary order: ' + log);
    });
    await local('empty-key-still-converts-configurations', async () => {
      const marker = {};
      await rejected(realm, () => method().call(realm.navigator, '', [{get label() { throw marker; }}]), undefined, marker);
    });
    await local('getter-exception-identity', async () => {
      const marker = {};
      await rejected(realm, () => method().call(realm.navigator, 'com.example.unsupported', [{audioCapabilities: [{get robustness() { throw marker; }}]}]), undefined, marker);
    });
    await local('sequence-conversion-is-interleaved', async () => {
      const log = [];
      const configs = {[Symbol.iterator]() { log.push('iterator'); let i = 0; return {next() { log.push('next' + i); if (i++ === 0) return {done: false, value: {get label() { log.push('label'); return ''; }}}; return {done: true}; }}; }};
      await rejected(realm, () => method().call(realm.navigator, 'com.example.unsupported', configs), 'NotSupportedError');
      assert(log.join() === 'iterator,next0,label,next1', 'convert item before next');
    });
    await local('sequence-conversion-error-does-not-close', async () => {
      let closed = 0;
      const configs = {[Symbol.iterator]() { return {next() { return {done: false, value: 5}; }, return() { closed++; return {done: true}; }}; }};
      await rejected(realm, () => method().call(realm.navigator, 'com.example.unsupported', configs), 'TypeError');
      assert(closed === 0, 'WebIDL conversion does not IteratorClose');
    });
    await local('native-error-constructor', async () => {
      const original = Object.getOwnPropertyDescriptor(realm, 'DOMException');
      const Native = realm.DOMException; let calls = 0;
      try {
        Object.defineProperty(realm, 'DOMException', {configurable: true, value: function() { calls++; throw Error('author constructor'); }});
        const p = method().call(realm.navigator, 'com.example.unsupported', [{}]);
        const e = await p.then(() => { throw Error('resolved'); }, e => e);
        assert(e instanceof Native && e.name === 'NotSupportedError' && calls === 0, 'intrinsic native error');
      } finally {Object.defineProperty(realm, 'DOMException', original);}
    });
    for (const tag of ['audio', 'video']) await local('mediaKeys:' + tag, async () => {
      const media = realm.document.createElement(tag), p = realm.HTMLMediaElement.prototype;
      assert(typeof p.setMediaKeys === 'function', 'setMediaKeys entry');
      assert(media.mediaKeys === null, 'initial null');
      for (const value of [null, undefined]) {
        const promise = p.setMediaKeys.call(media, value); assert(promise instanceof realm.Promise, 'callee Promise');
        assert(await promise === undefined && media.mediaKeys === null, 'same null keys');
      }
      await rejected(realm, () => p.setMediaKeys.call(media), 'TypeError');
      for (const value of [{}, Object.create(realm.MediaKeys.prototype), new Proxy({}, {})]) await rejected(realm, () => p.setMediaKeys.call(media, value), 'TypeError');
      const get = Object.getOwnPropertyDescriptor(p, 'mediaKeys').get;
      for (const receiver of [{}, realm.document.createElement('div'), new Proxy(media, {})]) {
        thrown(realm, () => get.call(receiver));
        await rejected(realm, () => p.setMediaKeys.call(receiver, null), 'TypeError');
      }
      const detached = realm.document.implementation.createHTMLDocument('').createElement(tag);
      assert(get.call(detached) === null && await p.setMediaKeys.call(detached, null) === undefined, 'native detached receiver');
    });
    await local('borrowed-media-element', async () => {
      const other = label === 'main' ? document.getElementById('child').contentWindow : globalThis;
      const fn = realm.HTMLMediaElement.prototype.setMediaKeys;
      assert(typeof fn === 'function', 'setMediaKeys entry');
      const p = fn.call(other.document.createElement('video'), null);
      assert(p instanceof realm.Promise && await p === undefined, 'callee Promise and genuine foreign receiver');
    });
  }
  if (isSecureContext) {
    await check('policy:receiver-document-and-conversion-order', async () => {
      const fn = Navigator.prototype.requestMediaKeySystemAccess; assert(typeof fn === 'function', 'request entry');
      const frame = document.createElement('iframe'); frame.setAttribute('allow', "encrypted-media 'none'"); frame.srcdoc = '<!doctype html><title>blocked EME</title>';
      const loaded = new Promise(resolve => frame.onload = resolve); document.body.appendChild(frame); await loaded;
      try {
        const other = frame.contentWindow, marker = {};
        await rejected(globalThis, () => fn.call(other.navigator, '', [{get label() { throw marker; }}]), undefined, marker);
        await rejected(globalThis, () => fn.call(other.navigator, '', [{}]), 'SecurityError');
        await rejected(globalThis, () => fn.call(other.navigator, 'com.example.unsupported', [{}]), 'SecurityError');
        await rejected(other, () => other.Navigator.prototype.requestMediaKeySystemAccess.call(navigator, 'com.example.unsupported', [{}]), 'NotSupportedError');
      } finally {frame.remove();}
    });
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
