(async () => {
  const facts = {complete: false, checks: [], traces: []};
  globalThis.__uiEventResults = facts;
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const same = (actual, expected) => assert(JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify({actual, expected}));
  const foreign = document.getElementById('child').contentWindow;
  for (const [realm, w] of [['main', globalThis], ['iframe', foreign]]) {
    const run = async (name, fn) => {
      try { await fn(); facts.checks.push({realm, name, passed: true}); }
      catch (error) { facts.checks.push({realm, name, passed: false, error: error.name, message: error.message}); }
    };
    if (!w.navigator.credentials) {
      facts.checks.push({realm, name: 'navigator.credentials producer', passed: false, message: 'Missing producer'});
      continue;
    }
    const container = w.navigator.credentials;
    const prototype = w.CredentialsContainer.prototype;
    const PromiseCtor = w.Promise, TypeErrorCtor = w.TypeError, DOMExceptionCtor = w.DOMException;
    const methods = Object.fromEntries(['get', 'create', 'store', 'preventSilentAccess'].map(name => [name, prototype[name]]));
    const invoke = (name, receiver, args = []) => {
      let result;
      try { result = methods[name].apply(receiver, args); }
      catch (error) { throw Error('Synchronous exception: ' + error); }
      assert(result instanceof PromiseCtor, 'Promise must belong to function realm');
      return result;
    };
    const rejection = async (name, receiver, args, Constructor, exceptionName) => {
      let error, rejected = false;
      try { await invoke(name, receiver, args); } catch (caught) { rejected = true; error = caught; }
      assert(rejected, 'Expected a rejected Promise');
      if (Constructor) assert(error instanceof Constructor, 'Wrong exception constructor/realm: ' + String(error));
      if (exceptionName) {
        assert(error.name === exceptionName, 'Wrong exception name: ' + String(error));
        if (exceptionName === 'NotSupportedError') assert(error.code === 9, 'NotSupportedError code');
        if (exceptionName === 'InvalidStateError') assert(error.code === 11, 'InvalidStateError code');
      }
      return error;
    };
    const controller = reason => { const value = new w.AbortController(); value.abort(reason); return value; };
    const validCreation = () => ({rp: {name: 'Site'}, user: {name: 'user', displayName: 'User', id: new Uint8Array([1])}, challenge: new Uint8Array([2]), pubKeyCredParams: [{type: 'public-key', alg: -7}]});
    const validRequest = () => ({challenge: new Uint8Array([3])});
    const getters = (values, trace, prefix = '') => Object.defineProperties({}, Object.fromEntries(Object.entries(values).map(([key, value]) => [key, {enumerable: true, get() { trace.push(prefix + key); return value; }}])));
    await run('secure SameObject producer and descriptor', async () => {
      assert(w.isSecureContext, 'Fixture must run in a secure context');
      const descriptor = Object.getOwnPropertyDescriptor(w.Navigator.prototype, 'credentials');
      assert(descriptor.enumerable && descriptor.configurable && descriptor.set === undefined, 'Readonly enumerable accessor');
      assert(container === w.navigator.credentials && descriptor.get.call(w.navigator) === container, 'SameObject');
      assert(Object.getPrototypeOf(container) === prototype && container instanceof w.CredentialsContainer, 'Relevant-realm prototype');
      assert(Object.prototype.toString.call(container) === '[object CredentialsContainer]', 'Interface tag');
      let error; try { new w.CredentialsContainer(); } catch (caught) { error = caught; }
      assert(error instanceof TypeErrorCtor, 'Illegal constructor');
      const other = w === globalThis ? foreign : globalThis;
      assert(descriptor.get.call(other.navigator) === other.navigator.credentials, 'Borrowed Navigator getter uses receiver realm/cache');
      assert(Object.getPrototypeOf(descriptor.get.call(other.navigator)) === other.CredentialsContainer.prototype, 'Borrowed getter materializes in receiver realm');
    });
    await run('Navigator getter uses native brand without author traps', async () => {
      const getter = Object.getOwnPropertyDescriptor(w.Navigator.prototype, 'credentials').get;
      let reads = 0;
      const trap = () => { reads++; throw Error('Getter trap'); };
      const revoked = Proxy.revocable(w.navigator, {}); revoked.revoke();
      for (const fake of [{}, Object.create(w.navigator), Object.create(w.Navigator.prototype), new Proxy(w.navigator, {get: trap, getPrototypeOf: trap}), revoked.proxy, new EventTarget()]) {
        let error; try { getter.call(fake); } catch (caught) { error = caught; }
        assert(error instanceof TypeErrorCtor, 'Navigator getter accepted a forged receiver');
      }
      assert(reads === 0, 'Receiver check ran author traps');
    });
    const trap = () => { throw Error('Unexpected author trap'); };
    const revoked = Proxy.revocable(container, {}); revoked.revoke();
    const badReceivers = [null, undefined, {}, Object.create(container), Object.create(prototype), new Proxy(container, {get: trap, getPrototypeOf: trap}), revoked.proxy, new EventTarget()];
    for (const [name, length] of [['get', 0], ['create', 0], ['store', 1], ['preventSilentAccess', 0]]) {
      await run(name + '/native descriptor and arity', async () => {
        const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
        assert(descriptor.enumerable && descriptor.configurable && descriptor.writable, 'Writable enumerable method');
        assert(methods[name].name === name && methods[name].length === length, 'Name and arity');
        assert(Function.prototype.toString.call(methods[name]).includes('[native code]'), 'Native function');
      });
      for (const [index, receiver] of badReceivers.entries()) {
        await run(name + '/brand before argument conversion/' + index, async () => {
          let reads = 0;
          const argument = new Proxy({}, {get() { reads++; throw Error('Argument conversion ran'); }});
          await rejection(name, receiver, [argument], TypeErrorCtor);
          assert(reads === 0, 'Invalid receiver must fail before any argument conversion');
        });
      }
    }
    for (const name of ['get', 'create']) {
      for (const [label, args] of [['omitted', []], ['undefined', [undefined]], ['null', [null]], ['empty', [{}]], ['unknown', [{get unknown() { throw Error('Unknown member read'); }}]]]) {
        await run(name + '/empty dictionary/' + label, () => rejection(name, container, args, DOMExceptionCtor, 'NotSupportedError'));
      }
      for (const [index, value] of [true, 1, 'string', Symbol('dictionary'), 1n].entries()) {
        await run(name + '/primitive dictionary/' + index, () => rejection(name, container, [value], TypeErrorCtor));
      }
      for (const mediation of ['silent', 'optional', 'conditional', 'required']) {
        await run(name + '/valid mediation/' + mediation, async () => {
          const marker = {}, signal = controller(marker).signal;
          assert(await rejection(name, container, [{mediation, signal}]) === marker, 'Pre-abort reason identity');
        });
      }
      for (const [index, mediation] of ['', 'OPTIONAL', null, Symbol('mediation')].entries()) {
        await run(name + '/invalid mediation/' + index, () => rejection(name, container, [{mediation, signal: controller({}).signal}], TypeErrorCtor));
      }
      for (const [index, reason] of ['reason', {}, [], new Error('reason'), null, 42, 1n, Symbol('reason')].entries()) {
        await run(name + '/pre-abort exact reason/' + index, async () => {
          const signal = controller(reason).signal;
          assert(await rejection(name, container, [{signal}]) === reason, 'Abort reason was replaced');
        });
      }
      await run(name + '/default abort reason and foreign signal', async () => {
        const other = w === globalThis ? foreign : globalThis;
        const abort = new other.AbortController(); abort.abort();
        assert(await rejection(name, container, [{signal: abort.signal}]) === abort.signal.reason, 'Cross-realm native signal/default reason');
      });
      const signal = new w.AbortController().signal;
      const revokedSignal = Proxy.revocable(signal, {}); revokedSignal.revoke();
      for (const [index, invalid] of [null, {}, Object.create(signal), Object.create(w.AbortSignal.prototype), new Proxy(signal, {get: trap, getPrototypeOf: trap}), revokedSignal.proxy, new EventTarget()].entries()) {
        await run(name + '/AbortSignal native brand/' + index, () => rejection(name, container, [{signal: invalid}], TypeErrorCtor));
      }
      await run(name + '/abort state ignores public getters', async () => {
        const marker = {}, signal = controller(marker).signal;
        Object.defineProperties(signal, {aborted: {get: trap}, reason: {get: trap}});
        assert(await rejection(name, container, [{signal}]) === marker, 'Internal abort state/reason');
      });
      await run(name + '/conversion exception before pre-abort', async () => {
        const marker = {}, signal = controller({}).signal;
        assert(await rejection(name, container, [{get mediation() { throw marker; }, signal}]) === marker, 'Conversion exception was replaced');
      });
      await run(name + '/all dictionary getters precede native algorithm', async () => {
        const trace = [], marker = {}, signal = controller(marker).signal;
        const values = name === 'get' ? {federated: undefined, mediation: 'optional', password: false, publicKey: undefined, signal, uiMode: undefined} : {federated: undefined, mediation: 'optional', password: undefined, publicKey: undefined, signal};
        const options = getters(values, trace);
        assert(await rejection(name, container, [options]) === marker, 'Abort must follow conversion');
        same(trace, Object.keys(values)); facts.traces.push({realm, name, trace});
      });
      await run(name + '/missing defaults avoid Object.prototype getters', async () => {
        let reads = 0;
        const get = () => { reads++; throw Error('Inherited default read'); };
        for (const key of ['federated', 'mediation', 'password', 'publicKey', 'signal']) Object.defineProperty(w.Object.prototype, key, {configurable: true, get});
        let error;
        try { error = await rejection(name, container, [], DOMExceptionCtor, 'NotSupportedError'); }
        finally { for (const key of ['federated', 'mediation', 'password', 'publicKey', 'signal']) delete w.Object.prototype[key]; }
        assert(reads === 0 && error.name === 'NotSupportedError', 'Empty default is not an ordinary JS object');
      });
      for (const [index, binary] of [null, [], {}, 'bytes', new Proxy(new Uint8Array([1]), {})].entries()) {
        await run(name + '/native BufferSource/' + index, async () => {
          const publicKey = name === 'create' ? validCreation() : validRequest();
          publicKey.challenge = binary;
          await rejection(name, container, [{publicKey, signal: controller({}).signal}], TypeErrorCtor);
        });
      }
      for (const [index, binary] of [new ArrayBuffer(2), new Uint8Array([1, 2]), new DataView(new ArrayBuffer(3))].entries()) {
        await run(name + '/valid BufferSource before abort/' + index, async () => {
          const marker = {}, publicKey = name === 'create' ? validCreation() : validRequest();
          publicKey.challenge = binary;
          assert(await rejection(name, container, [{publicKey, signal: controller(marker).signal}]) === marker, 'Native ArrayBuffer/view conversion');
        });
      }
      for (const [index, binary] of [new ArrayBuffer(2, {maxByteLength: 4}), new Uint8Array(new ArrayBuffer(2, {maxByteLength: 4})), new DataView(new ArrayBuffer(2, {maxByteLength: 4}))].entries()) {
        await run(name + '/reject resizable BufferSource/' + index, async () => {
          const publicKey = name === 'create' ? validCreation() : validRequest(); publicKey.challenge = binary;
          await rejection(name, container, [{publicKey, signal: controller({}).signal}], TypeErrorCtor);
        });
      }
      if (typeof w.SharedArrayBuffer === 'function') {
        for (const [index, binary] of [new w.SharedArrayBuffer(2), new Uint8Array(new w.SharedArrayBuffer(2)), new DataView(new w.SharedArrayBuffer(2))].entries()) {
          await run(name + '/reject shared BufferSource/' + index, async () => {
            const publicKey = name === 'create' ? validCreation() : validRequest(); publicKey.challenge = binary;
            await rejection(name, container, [{publicKey, signal: controller({}).signal}], TypeErrorCtor);
          });
        }
      }
      for (const extension of [{credBlob: {}}, {largeBlob: {write: {}}}, {prf: {eval: {first: {}}}}, {prf: {evalByCredential: {AA: {first: {}}}}}]) {
        await run(name + '/nested extension buffers/' + JSON.stringify(extension), async () => {
          const publicKey = name === 'create' ? validCreation() : validRequest(); publicKey.extensions = extension;
          await rejection(name, container, [{publicKey, signal: controller({}).signal}], TypeErrorCtor);
        });
      }
      await run(name + '/credential descriptor required members', async () => {
        const publicKey = name === 'create' ? validCreation() : validRequest();
        publicKey[name === 'create' ? 'excludeCredentials' : 'allowCredentials'] = [{type: 'public-key'}];
        await rejection(name, container, [{publicKey, signal: controller({}).signal}], TypeErrorCtor);
      });
      await run(name + '/PRF record key getter exception identity', async () => {
        const marker = {}, publicKey = name === 'create' ? validCreation() : validRequest();
        publicKey.extensions = {prf: {evalByCredential: {get AA() { throw marker; }}}};
        assert(await rejection(name, container, [{publicKey, signal: controller({}).signal}]) === marker, 'Record getter exception identity');
      });
    }
    await run('create/inherited dictionaries convert before derived members', async () => {
      const marker = {}, trace = [], publicKey = validCreation();
      publicKey.rp = getters({name: 'Site', id: location.hostname}, trace, 'rp.');
      publicKey.user = getters({name: 'user', displayName: 'User', id: new Uint8Array([1])}, trace, 'user.');
      assert(await rejection('create', container, [{publicKey, signal: controller(marker).signal}]) === marker, 'Valid nested conversion');
      same(trace, ['rp.name', 'rp.id', 'user.name', 'user.displayName', 'user.id']);
    });
    for (const kind of ['password', 'federated']) {
      await run('create/' + kind + '/inherited dictionary member order', async () => {
        const marker = {}, trace = [];
        const values = kind === 'password' ? {id: 'user', iconURL: '', name: 'User', origin: location.origin, password: 'secret'} : {id: 'user', iconURL: '', name: 'User', origin: location.origin, protocol: undefined, provider: 'https://idp.example'};
        const options = {[kind]: getters(values, trace), signal: controller(marker).signal};
        assert(await rejection('create', container, [options]) === marker, 'Abort after credential conversion');
        same(trace, Object.keys(values));
      });
      await run('create/' + kind + '/required origin', async () => {
        const data = kind === 'password' ? {id: 'user', password: 'secret'} : {id: 'user', provider: 'https://idp.example'};
        await rejection('create', container, [{[kind]: data, signal: controller({}).signal}], TypeErrorCtor);
      });
    }
    await run('create/native form union precedes dictionary getters', async () => {
      const marker = {}, form = document.createElement('form');
      Object.defineProperty(form, 'password', {get: trap});
      assert(await rejection('create', container, [{password: form, signal: controller(marker).signal}]) === marker, 'Native form union branch');
    });
    await run('create/multiple credential types before pre-abort', async () => {
      const password = {id: 'user', origin: location.origin, password: 'secret'}, publicKey = validCreation();
      await rejection('create', container, [{password, publicKey, signal: controller({}).signal}], DOMExceptionCtor, 'NotSupportedError');
    });
    for (const [index, args] of [[], [undefined], [null], [{}], [Object.create(w.Credential.prototype)]].entries()) {
      await run('store/required native Credential/' + index, () => rejection('store', container, args, TypeErrorCtor));
    }
    await run('preventSilentAccess/idempotent undefined without argument conversion', async () => {
      assert(await invoke('preventSilentAccess', container, [new Proxy({}, {get: trap})]) === undefined, 'No extra argument conversion');
      assert(await invoke('preventSilentAccess', container) === undefined, 'Idempotent result');
    });
    await run('native brand remains after prototype replacement', async () => {
      const saved = Object.getPrototypeOf(container);
      Object.setPrototypeOf(container, null);
      try { assert(await invoke('preventSilentAccess', container) === undefined, 'Native brand survives prototype change'); }
      finally { Object.setPrototypeOf(container, saved); }
    });
    await run('native Promise and errors ignore replaced public constructors', async () => {
      const saved = [w.Promise, w.TypeError, w.DOMException, w.CredentialsContainer];
      try {
        w.Promise = w.TypeError = w.DOMException = w.CredentialsContainer = function() { throw Error('Public constructor called'); };
        await rejection('store', container, [{}], TypeErrorCtor);
        await rejection('create', container, [], DOMExceptionCtor, 'NotSupportedError');
        assert(await invoke('preventSilentAccess', container) === undefined, 'Native Promise resolver');
      } finally { [w.Promise, w.TypeError, w.DOMException, w.CredentialsContainer] = saved; }
    });
  }
  for (const ancestor of [false, true]) {
    const name = ancestor ? 'inactive ancestor' : 'removed iframe';
    try {
      const outer = document.createElement('iframe'); document.body.appendChild(outer);
      let frame = outer;
      if (ancestor) { frame = outer.contentDocument.createElement('iframe'); outer.contentDocument.body.appendChild(frame); }
      const w = frame.contentWindow, container = w.navigator.credentials, PromiseCtor = w.Promise, DOMExceptionCtor = w.DOMException, TypeErrorCtor = w.TypeError;
      const marker = {}, abort = new w.AbortController(); abort.abort(marker);
      const get = container.get, create = container.create, prevent = container.preventSilentAccess, store = container.store;
      outer.remove();
      for (const [method, args] of [[get, [{signal: abort.signal}]], [create, [{signal: abort.signal}]], [prevent, []]]) {
        let error; const promise = method.apply(container, args); assert(promise instanceof PromiseCtor, 'Inactive function Promise realm');
        try { await promise; } catch (caught) { error = caught; }
        assert(error instanceof DOMExceptionCtor && error.name === 'InvalidStateError', 'Fully active check precedes pre-abort');
      }
      let error; try { await store.call(container, {}); } catch (caught) { error = caught; }
      assert(error instanceof TypeErrorCtor, 'WebIDL conversion precedes native activity check');
      facts.checks.push({realm: 'lifecycle', name, passed: true});
    } catch (error) { facts.checks.push({realm: 'lifecycle', name, passed: false, error: error.name, message: error.message}); }
  }
  facts.passed = facts.checks.filter(row => row.passed).length; facts.total = facts.checks.length; facts.complete = true;
  return true;
})()
