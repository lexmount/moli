(() => {
  const facts = {complete: false, checks: [], traces: [], versionShapes: []};
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const same = (actual, expected) => assert(JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify({actual, expected}));
  const minimal = () => ({challenge: 'dGVzdA', rp: {name: 'rp'}, user: {id: 'AA', name: 'user', displayName: 'User'}, pubKeyCredParams: [{alg: -7, type: 'public-key'}]});
  const bytes = value => [...new Uint8Array(value)];
  const worlds = [['main', globalThis], ['iframe', document.getElementById('child').contentWindow]];
  for (const [realm, w] of worlds) {
    const create = w.PublicKeyCredential.parseCreationOptionsFromJSON;
    const request = w.PublicKeyCredential.parseRequestOptionsFromJSON;
    const run = (name, fn) => {
      try {
        assert(typeof create === 'function' && typeof request === 'function', 'Missing JSON conversion methods');
        fn(); facts.checks.push({realm, name, passed: true});
      } catch (error) {
        facts.checks.push({realm, name, passed: false, error: error.name, message: error.message});
      }
    };
    const throws = (fn, Constructor, name) => {
      let error; try { fn(); } catch (caught) { error = caught; }
      assert(error instanceof Constructor, 'Wrong exception constructor or realm');
      if (name) { assert(error.name === name && error.code === 0, 'Wrong DOMException name or code'); }
      return error;
    };
    run('native descriptors and static receivers', () => {
      let traps = 0;
      const trap = () => { traps++; throw Error('Author receiver or extra argument'); };
      const revoked = Proxy.revocable({}, {}); revoked.revoke();
      for (const [name, fn] of [['parseCreationOptionsFromJSON', create], ['parseRequestOptionsFromJSON', request]]) {
        const descriptor = Object.getOwnPropertyDescriptor(w.PublicKeyCredential, name);
        assert(descriptor.enumerable && descriptor.configurable && descriptor.writable && fn.length === 1 && fn.name === name, 'Method descriptor');
        assert(Function.prototype.toString.call(fn).includes('[native code]'), 'Native function');
        for (const receiver of [null, {}, revoked.proxy, new Proxy({}, {get: trap, getPrototypeOf: trap})]) {
          fn.call(receiver, fn === create ? minimal() : {challenge: 'AA'}, {toString: trap});
        }
      }
      assert(traps === 0, 'Static receivers and excess arguments are ignored');
    });
    run('request defaults and dictionary data properties', () => {
      const result = request({challenge: 'AA'});
      same(Object.keys(result), ['allowCredentials', 'challenge', 'hints', 'userVerification']);
      same(bytes(result.challenge), [0]);
      assert(result.challenge instanceof w.ArrayBuffer && Object.getPrototypeOf(result) === w.Object.prototype, 'Callee objects');
      assert(result.allowCredentials instanceof w.Array && result.hints instanceof w.Array, 'Callee arrays');
      assert(result.userVerification === 'preferred', 'Verification default');
      for (const key of Object.keys(result)) {
        const d = Object.getOwnPropertyDescriptor(result, key);
        assert(d.enumerable && d.configurable && d.writable && !d.get && !d.set, 'Own data property');
      }
    });
    run('creation buffers, required entities and defaults', () => {
      const result = create(minimal());
      same(bytes(result.challenge), [116, 101, 115, 116]); same(bytes(result.user.id), [0]);
      same(Object.keys(result.rp), ['name']); same(Object.keys(result.user), ['name', 'displayName', 'id']);
      same(result.excludeCredentials, []); same(result.hints, []);
      assert(result.attestation === 'none' && result.pubKeyCredParams[0].alg === -7, 'Defaults and parameters');
      assert(result.user.id instanceof w.ArrayBuffer && Object.getPrototypeOf(result.user) === w.Object.prototype, 'Nested callee values');
      facts.versionShapes.push({realm, creationKeys: Object.keys(result), selection: create({...minimal(), authenticatorSelection: {}}).authenticatorSelection, emptyExtensions: request({challenge: 'AA', extensions: {}}).extensions});
    });
    run('current-idl/attestationFormats default and conversion', () => {
      same(create(minimal()).attestationFormats, []);
      same(create({...minimal(), attestationFormats: new Set(['packed', 'future', 'packed'])}).attestationFormats, ['packed', 'future']);
    });
    run('current-idl/selection verification default', () => {
      const result = create({...minimal(), authenticatorSelection: null}).authenticatorSelection;
      assert(result.requireResidentKey === false && result.userVerification === 'preferred', 'Selection defaults');
    });
    run('current-idl/optional credProps is absent', () => {
      const result = request({challenge: 'AA', extensions: null}).extensions;
      assert(!Object.hasOwn(result, 'credProps') && result.enforceCredentialProtectionPolicy === false, 'Optional and default extension fields');
    });
    run('current-idl/remoteClientDataJSON is DOMString', () => {
      const result = request({challenge: 'AA', extensions: {remoteClientDataJSON: 'not base64\ud800'}});
      assert(result.extensions.remoteClientDataJSON === 'not base64\ud800', 'Remote client data string');
    });
    const valid = [['', []], ['Zg', [102]], ['Zh', [102]], ['-w', [251]], ['_w', [255]], ['AA', [0]], ['null', [158, 233, 101]], ['AAEC_w', [0, 1, 2, 255]]];
    for (const [input, expected] of valid) run('base64url accepts/' + JSON.stringify(input), () => {
      same(bytes(request({challenge: input}).challenge), expected);
      same(bytes(create({...minimal(), challenge: input}).challenge), expected);
    });
    for (const input of ['Zg==', 'Zh==', 'Zg=', 'Zg===', 'Zg====', 'Z', 'Zg ', ' Zg', 'Zg\n', '+w', '/w', 'AA==', 'AA=', 'A===', '====', 'a\ud800']) run('base64url rejects/' + JSON.stringify(input), () => {
      throws(() => request({challenge: input}), w.DOMException, 'EncodingError');
      throws(() => create({...minimal(), challenge: input}), w.DOMException, 'EncodingError');
    });
    const invalidPaths = [
      ['user.id', () => create({...minimal(), user: {id: '=', name: 'N', displayName: 'D'}})],
      ['excludeCredentials.id', () => create({...minimal(), excludeCredentials: [{id: '=', type: 'public-key'}]})],
      ['allowCredentials.id', () => request({challenge: 'AA', allowCredentials: [{id: '=', type: 'public-key'}]})],
      ['credBlob', () => request({challenge: 'AA', extensions: {credBlob: '='}})],
      ['largeBlob.write', () => request({challenge: 'AA', extensions: {largeBlob: {write: '='}}})],
      ['prf.eval.first', () => request({challenge: 'AA', extensions: {prf: {eval: {first: '='}}}})],
      ['prf.eval.second', () => request({challenge: 'AA', extensions: {prf: {eval: {first: 'AA', second: '='}}}})],
      ['prf.record.first', () => request({challenge: 'AA', extensions: {prf: {evalByCredential: {key: {first: '='}}}}})],
      ['prf.record.second', () => request({challenge: 'AA', extensions: {prf: {evalByCredential: {key: {first: 'AA', second: '='}}}}})]
    ];
    for (const [path, fn] of invalidPaths) run('nested EncodingError/' + path, () => throws(fn, w.DOMException, 'EncodingError'));
    run('all recognized binary extension fields', () => {
      const extensions = {appid: 'app', appidExclude: 'exclude', credProps: true, credBlob: '-w', getCredBlob: true, hmacCreateSecret: true, minPinLength: true, credentialProtectionPolicy: 'future', enforceCredentialProtectionPolicy: true, largeBlob: {read: true, support: 'future', write: 'AA'}, prf: {eval: {first: 'Zg', second: ''}, evalByCredential: {key: {first: '_w', second: 'Zh'}}}};
      for (const result of [create({...minimal(), extensions}), request({challenge: 'AA', extensions})]) {
        const e = result.extensions;
        same(bytes(e.credBlob), [251]); same(bytes(e.largeBlob.write), [0]);
        same(bytes(e.prf.eval.first), [102]); same(bytes(e.prf.eval.second), []);
        same(bytes(e.prf.evalByCredential.key.first), [255]); same(bytes(e.prf.evalByCredential.key.second), [102]);
        assert(e.appid === 'app' && e.appidExclude === 'exclude' && e.credProps && e.getCredBlob && e.hmacCreateSecret && e.minPinLength && e.enforceCredentialProtectionPolicy, 'Recognized scalars');
        assert(e.credentialProtectionPolicy === 'future' && e.largeBlob.support === 'future', 'Future string values');
      }
    });
    run('DOMString code units and unknown future string values', () => {
      const s = '\ud800X\udfff';
      const result = create({...minimal(), attestation: s, hints: [s, s], rp: {name: s, id: s}, user: {id: 'AA', name: s, displayName: s}, pubKeyCredParams: [{alg: -7, type: s}], authenticatorSelection: {authenticatorAttachment: s, residentKey: s, userVerification: s}, excludeCredentials: [{id: '', type: s, transports: [s]}], extensions: {appid: s, credentialProtectionPolicy: s}});
      assert(result.attestation === s && result.rp.name === s && result.rp.id === s && result.user.name === s && result.user.displayName === s && result.pubKeyCredParams[0].type === s, 'Lossless DOMString');
      same(result.hints, [s, s]); same(result.excludeCredentials[0].transports, [s]);
      assert(result.authenticatorSelection.residentKey === s && result.extensions.appid === s, 'Nested lossless DOMString');
      assert(result.extensions.credentialProtectionPolicy === '\ufffdX\ufffd', 'USVString FIDO field');
    });
    run('integer modulo, null strings and optional absence', () => {
      const r = request({challenge: null, rpId: null, timeout: -1, userVerification: 'future'});
      same(bytes(r.challenge), [158, 233, 101]);
      assert(r.rpId === 'null' && r.timeout === 4294967295 && r.userVerification === 'future', 'WebIDL scalar conversions');
      const c = create({...minimal(), timeout: 4294967297, pubKeyCredParams: [{alg: 4294967289, type: 'future'}], excludeCredentials: [{id: '', type: 'future'}]});
      assert(c.timeout === 1 && c.pubKeyCredParams[0].alg === -7 && !Object.hasOwn(c.excludeCredentials[0], 'transports'), 'Integer modulo and optional fields');
    });
    run('arity, primitive dictionaries and required nested fields', () => {
      for (const fn of [create, request]) {
        throws(() => fn(), w.TypeError);
        for (const value of [undefined, null, {}, 1, 'string', true, Symbol('dict')]) throws(() => fn(value), w.TypeError);
      }
      for (const options of [{...minimal(), rp: null}, {...minimal(), user: {}}, {...minimal(), pubKeyCredParams: [{}]}, {...minimal(), excludeCredentials: [{type: 'public-key'}]}]) throws(() => create(options), w.TypeError);
      throws(() => request({challenge: Symbol('string')}), w.TypeError);
      throws(() => request({challenge: 'AA', timeout: 1n}), w.TypeError);
      throws(() => request({challenge: 'AA', extensions: {prf: {eval: {}}}}), w.TypeError);
    });
    run('iterable sequences and conversion once', () => {
      let iterators = 0, strings = 0;
      const hints = {[Symbol.iterator]() { iterators++; return [ {toString() { strings++; return 'hint'; }} ][Symbol.iterator](); }};
      const result = create({...minimal(), hints, pubKeyCredParams: new Set([{alg: -7, type: 'public-key'}]), excludeCredentials: new Set([{id: 'AA', type: 'public-key', transports: new Set(['usb', 'nfc'])}])});
      same(result.hints, ['hint']); same(result.excludeCredentials[0].transports, ['usb', 'nfc']);
      assert(iterators === 1 && strings === 1, 'One iterable and DOMString conversion');
      for (const key of ['hints', 'excludeCredentials', 'pubKeyCredParams']) throws(() => create({...minimal(), [key]: null}), w.TypeError);
      throws(() => request({challenge: 'AA', hints: 'string'}), w.TypeError);
    });
    run('sequence conversion preserves abrupt completion without iterator close', () => {
      const marker = {marker: true}; let closes = 0;
      const iterable = {[Symbol.iterator]() {return {next() {return {done: false, value: {toString() {throw marker;}}};}, return() {closes++; return {};}};}};
      let caught; try { request({challenge: 'AA', hints: iterable}); } catch (error) {caught = error;}
      assert(caught === marker && closes === 0, 'Original exception and WebIDL iteration behavior');
    });
    run('getters finish before any base64url decoding', () => {
      const marker = {marker: true}; let calls = 0;
      let caught; try { create({...minimal(), challenge: '=', user: {displayName: 'D', id: '=', get name() {calls++; throw marker;}}}); } catch (error) {caught = error;}
      assert(caught === marker && calls === 1, 'Last getter error precedes invalid decoding');
      caught = undefined;
      try {request({challenge: '=', get userVerification() {throw marker;}});} catch (error) {caught = error;}
      assert(caught === marker, 'Request conversion finishes before decoding');
    });
    run('inherited dictionary getters, no unknown reads and no input mutations', () => {
      let calls = 0;
      const proto = {get challenge() {calls++; return 'AA';}};
      const input = Object.create(proto);
      Object.defineProperty(input, 'unknown', {get() {throw Error('Unknown member accessed');}});
      input.extensions = {prf: {eval: {first: 'AA'}}};
      Object.defineProperty(input.extensions, 'unknown', {get() {throw Error('Unknown extension accessed');}});
      const result = request(input);
      assert(calls === 1 && !Object.hasOwn(result, 'unknown') && !Object.hasOwn(result.extensions, 'unknown'), 'Known dictionary members only');
      assert(!Object.hasOwn(input, 'challenge') && input.extensions.prf.eval.first === 'AA', 'Input dictionaries preserved');
    });
    run('PRF records use own enumerable keys and preserve UTF-16', () => {
      const record = Object.create({inherited: {first: '='}});
      Object.defineProperty(record, 'nonEnumerable', {get() {throw Error('Non-enumerable getter');}});
      Object.defineProperty(record, '__proto__', {value: {first: 'AA'}, enumerable: true});
      record['\ud800'] = {first: 'Zh'};
      const result = request({challenge: 'AA', extensions: {prf: {evalByCredential: record}}}).extensions.prf.evalByCredential;
      same(Object.keys(result), ['__proto__', '\ud800']);
      assert(Object.getPrototypeOf(result) === w.Object.prototype && Object.hasOwn(result, '__proto__'), 'Record data properties');
      same(bytes(result.__proto__.first), [0]); same(bytes(result['\ud800'].first), [102]);
      record[Symbol('enumerable')] = {first: 'AA'};
      throws(() => request({challenge: 'AA', extensions: {prf: {evalByCredential: record}}}), w.TypeError);
    });
    run('fresh independent dictionaries and ArrayBuffers', () => {
      const options = {...minimal(), extensions: {prf: {eval: {first: 'AA'}}}};
      const first = create(options), second = create(options);
      new Uint8Array(first.challenge)[0] = 0; new Uint8Array(first.extensions.prf.eval.first)[0] = 255;
      first.hints.push('mutated'); first.user.name = 'changed';
      same(bytes(second.challenge), [116, 101, 115, 116]); same(bytes(second.extensions.prf.eval.first), [0]);
      same(second.hints, []); assert(second.user.name === 'user' && first.user.id !== second.user.id && first !== second, 'No mutable sharing');
    });
    run('output bypasses inherited object and array setters', () => {
      const input = {...minimal(), hints: ['hint'], extensions: {appid: 'app'}};
      let traps = 0; let result;
      const setter = () => {traps++; throw Error('Inherited setter');};
      const entries = [[w.Object.prototype, 'challenge'], [w.Object.prototype, 'appid'], [w.Array.prototype, '0']];
      const saved = entries.map(([object, key]) => Object.getOwnPropertyDescriptor(object, key));
      try {
        for (const [object, key] of entries) Object.defineProperty(object, key, {set: setter, configurable: true});
        result = create(input);
      } finally {
        entries.forEach(([object, key], i) => {if (saved[i]) Object.defineProperty(object, key, saved[i]); else delete object[key];});
      }
      assert(traps === 0 && result.extensions.appid === 'app' && result.hints[0] === 'hint', 'CreateDataProperty output');
    });
    run('dictionary getter order', () => {
      const trace = [];
      const track = (prefix, target) => new Proxy(target, {get(object, key, receiver) {trace.push(prefix + String(key)); return Reflect.get(object, key, receiver);}});
      const input = track('top.', {...minimal(), rp: track('rp.', {name: 'R'}), user: track('user.', {id: 'AA', name: 'N', displayName: 'D'}), pubKeyCredParams: [track('param.', {alg: -7, type: 'public-key'})], excludeCredentials: [track('exclude.', {id: 'AA', type: 'public-key'})], authenticatorSelection: track('selection.', {}), extensions: track('extensions.', {largeBlob: track('largeBlob.', {}), prf: track('prf.', {eval: track('eval.', {first: 'AA'})})})});
      create(input);
      facts.traces.push({realm, trace});
      const common = trace.filter(key => key !== 'top.attestationFormats' && key !== 'extensions.remoteClientDataJSON');
      same(common, ['top.attestation','top.authenticatorSelection','selection.authenticatorAttachment','selection.requireResidentKey','selection.residentKey','selection.userVerification','top.challenge','top.excludeCredentials','exclude.id','exclude.transports','exclude.type','top.extensions','extensions.appid','extensions.appidExclude','extensions.credBlob','extensions.credProps','extensions.credentialProtectionPolicy','extensions.enforceCredentialProtectionPolicy','extensions.getCredBlob','extensions.hmacCreateSecret','extensions.largeBlob','largeBlob.read','largeBlob.support','largeBlob.write','extensions.minPinLength','extensions.prf','prf.eval','eval.first','eval.second','prf.evalByCredential','top.hints','top.pubKeyCredParams','param.alg','param.type','top.rp','rp.name','rp.id','top.timeout','top.user','user.displayName','user.id','user.name']);
    });
    run('current-idl/additional JSON dictionary member order', () => {
      const trace = facts.traces.find(row => row.realm === realm).trace;
      assert(trace[1] === 'top.attestationFormats' && trace[trace.indexOf('extensions.prf') + 5] === 'extensions.remoteClientDataJSON', 'Current dictionary members included in order');
    });
  }
  const foreign = worlds[1][1];
  try {
    const request = foreign.PublicKeyCredential.parseRequestOptionsFromJSON;
    assert(typeof request === 'function', 'Missing foreign parser');
    const ObjectCtor = foreign.Object, ArrayCtor = foreign.Array, BufferCtor = foreign.ArrayBuffer;
    const keys = ['Object', 'Array', 'ArrayBuffer', 'PublicKeyCredential', 'DOMException', 'TypeError'];
    const originals = keys.map(key => foreign[key]);
    let result, encodingError, typeError;
    try {
      for (const key of keys) foreign[key] = function AuthorConstructor() {throw Error('Author constructor');};
      result = request({challenge: 'AA', hints: ['hint'], extensions: {prf: {eval: {first: 'Zh'}}}});
      try {request({challenge: '='});} catch (error) {encodingError = error;}
      try {request({});} catch (error) {typeError = error;}
    } finally { keys.forEach((key, i) => foreign[key] = originals[i]); }
    assert(Object.getPrototypeOf(result) === ObjectCtor.prototype && result.hints instanceof ArrayCtor && result.challenge instanceof BufferCtor, 'Intrinsic callee output constructors');
    assert(encodingError instanceof originals[4] && encodingError.name === 'EncodingError' && typeError instanceof originals[5], 'Intrinsic callee exception constructors');
    facts.checks.push({realm: 'cross-realm', name: 'overwritten public constructors', passed: true});
  } catch (error) {facts.checks.push({realm: 'cross-realm', name: 'overwritten public constructors', passed: false, error: error.name, message: error.message});}
  facts.complete = true;
  globalThis.__uiEventResults = facts;
  return facts.checks.every(row => row.passed);
})()
