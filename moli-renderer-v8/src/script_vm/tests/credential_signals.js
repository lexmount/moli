(async () => {
  const facts = {complete: false, checks: [], traces: [], capabilities: []};
  globalThis.__uiEventResults = facts;
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const same = (actual, expected) => assert(JSON.stringify(actual) === JSON.stringify(expected), JSON.stringify({actual, expected}));
  const worlds = [['main', globalThis], ['iframe', document.getElementById('child').contentWindow]];
  for (const [realm, w] of worlds) {
    const methods = ['signalUnknownCredential', 'signalAllAcceptedCredentials', 'signalCurrentUserDetails'];
    const valid = name => name === methods[0] ? {credentialId: 'AA', rpId: location.hostname} : name === methods[1] ? {allAcceptedCredentialIds: ['AA', 'AQ'], rpId: location.hostname, userId: 'AA'} : {displayName: 'User', name: 'user', rpId: location.hostname, userId: 'AA'};
    const run = async (name, fn) => {
      try { await fn(); facts.checks.push({realm, name, passed: true}); }
      catch (error) { facts.checks.push({realm, name, passed: false, error: error.name, message: error.message}); }
    };
    const invoke = (name, ...args) => {
      let promise;
      try { promise = w.PublicKeyCredential[name](...args); }
      catch (error) { throw Error('Synchronous exception: ' + error); }
      assert(promise instanceof w.Promise, 'Promise belongs to the function realm');
      return promise;
    };
    const rejected = async (name, args, Constructor, exceptionName) => {
      let error;
      try { await invoke(name, ...args); } catch (caught) { error = caught; }
      assert(error instanceof Constructor, 'Wrong rejection constructor or realm: ' + error);
      if (exceptionName) assert(error.name === exceptionName && error.code === 18, 'SecurityError name/code');
      return error;
    };
    for (const name of methods) {
      const required = Object.keys(valid(name)).sort();
      await run(name + '/native descriptor, arity, static receiver and excess args', async () => {
        const descriptor = Object.getOwnPropertyDescriptor(w.PublicKeyCredential, name);
        assert(descriptor && descriptor.enumerable && descriptor.configurable && descriptor.writable, 'Static data property');
        const fn = descriptor.value;
        assert(fn.length === 1 && fn.name === name && Function.prototype.toString.call(fn).includes('[native code]'), 'Native function shape');
        let traps = 0;
        const trap = () => { traps++; throw Error('Unexpected receiver/argument conversion'); };
        const revoked = Proxy.revocable({}, {}); revoked.revoke();
        for (const receiver of [null, {}, revoked.proxy, new Proxy({}, {get: trap, getPrototypeOf: trap})]) {
          const promise = fn.call(receiver, valid(name), {toString: trap});
          assert(promise instanceof w.Promise && await promise === undefined, 'Valid signal resolves undefined');
        }
        assert(traps === 0, 'Static receiver/excess args are ignored');
      });
      for (const [label, args] of [['omitted', []], ['undefined', [undefined]], ['null', [null]], ['empty', [{}]], ['number', [42]], ['symbol', [Symbol('options')]]]) {
        await run(name + '/required dictionary/' + label, () => rejected(name, args, w.TypeError));
      }
      for (const key of required) {
        await run(name + '/missing member/' + key, async () => {
          const options = valid(name); delete options[key];
          await rejected(name, [options], w.TypeError);
        });
        await run(name + '/throwing getter/' + key, async () => {
          const marker = Object.freeze({key}); const options = valid(name);
          Object.defineProperty(options, key, {get() { throw marker; }});
          let caught; try { await invoke(name, options); } catch (error) { caught = error; }
          assert(caught === marker, 'Getter exception identity');
        });
      }
      await run(name + '/lexicographic conversion before base64 validation', async () => {
        const trace = [], options = valid(name);
        const binaryKey = name === methods[0] ? 'credentialId' : 'userId';
        options[binaryKey] = 'invalid padding==';
        for (const key of required) {
          const value = options[key];
          Object.defineProperty(options, key, {get() { trace.push(key); return value; }});
        }
        await rejected(name, [options], w.TypeError);
        same(trace, required); facts.traces.push({realm, name, trace});
      });
      await run(name + '/base64 error precedes RP ID validation', () => rejected(name, [{...valid(name), rpId: 'unrelated.example', [name === methods[0] ? 'credentialId' : 'userId']: 'invalid=='}], w.TypeError));
      await run(name + '/dictionary conversion precedes base64 validation', async () => {
        const marker = Symbol('conversion'), options = valid(name);
        options[name === methods[0] ? 'credentialId' : 'userId'] = 'invalid==';
        options.rpId = {toString() { throw marker; }};
        let caught; try { await invoke(name, options); } catch (error) { caught = error; }
        assert(caught === marker, 'DOMString exception must precede codec error');
      });
      await run(name + '/inherited members and dictionary Proxy', async () => {
        assert(await invoke(name, Object.create(valid(name))) === undefined, 'Inherited dictionary fields');
        const trace = [], options = new Proxy(valid(name), {get(target, key, receiver) { trace.push(key); return Reflect.get(target, key, receiver); }});
        assert(await invoke(name, options) === undefined, 'Proxy dictionary is valid'); same(trace, required);
      });
      for (const encoded of ['', 'AA', 'AB', '-_', '____', 'AQID', '1234']) {
        await run(name + '/base64 accepted/' + JSON.stringify(encoded), async () => {
          const options = valid(name); options[name === methods[0] ? 'credentialId' : 'userId'] = encoded;
          assert(await invoke(name, options) === undefined, 'Valid codec data');
        });
      }
      for (const encoded of ['A', 'AAA==', 'AA=', 'AA\n', 'AA ', '+/', 'é', '\ud800']) {
        await run(name + '/base64 rejected/' + JSON.stringify(encoded), () => rejected(name, [{...valid(name), [name === methods[0] ? 'credentialId' : 'userId']: encoded}], w.TypeError));
      }
      for (const rpId of ['', 'unrelated.example', 'evil' + location.hostname, location.hostname + ':443', 'https://' + location.hostname, location.hostname + '/', location.hostname + '.', '\ud800']) {
        await run(name + '/RP ID rejected/' + JSON.stringify(rpId), () => rejected(name, [{...valid(name), rpId}], w.DOMException, 'SecurityError'));
      }
      await run(name + '/host parser canonicalizes case and percent encoding', async () => {
        for (const rpId of [location.hostname.toUpperCase(), '%'+location.hostname.charCodeAt(0).toString(16)+location.hostname.slice(1)]) {
          assert(await invoke(name, {...valid(name), rpId}) === undefined, 'Canonical host matches');
        }
      });
      await run(name + '/no constructor use or page prototype pollution', async () => {
        const oldPromise = w.Promise, oldTypeError = w.TypeError, oldDOMException = w.DOMException;
        const originalThen = Object.getOwnPropertyDescriptor(w.Object.prototype, 'then');
        let reads = 0;
        try {
          w.Promise = w.TypeError = w.DOMException = function() { throw Error('Page replaced constructor'); };
          Object.defineProperty(w.Object.prototype, 'then', {configurable: true, get() { reads++; throw Error('Unexpected thenable'); }});
          const promise = w.PublicKeyCredential[name](valid(name));
          assert(promise instanceof oldPromise && await promise === undefined, 'Native Promise resolves void');
          let caught; try { await w.PublicKeyCredential[name]({...valid(name), rpId: 'unrelated.example'}); } catch (error) { caught = error; }
          assert(caught instanceof oldDOMException && caught.name === 'SecurityError', 'Native DOMException');
          caught = undefined; try { await w.PublicKeyCredential[name]({}); } catch (error) { caught = error; }
          assert(caught instanceof oldTypeError && reads === 0, 'Native TypeError, no result thenable');
        } finally {
          w.Promise = oldPromise; w.TypeError = oldTypeError; w.DOMException = oldDOMException;
          if (originalThen) Object.defineProperty(w.Object.prototype, 'then', originalThen); else delete w.Object.prototype.then;
        }
      });
    }
    await run('accepted credentials/sequence conversion propagates abrupt completion without IteratorClose', async () => {
      const name = methods[1], trace = [], marker = Symbol('iterator value');
      const iterable = {[Symbol.iterator]() { trace.push('iterator'); return {next() { trace.push('next'); return {done: false, value: {toString() { trace.push('convert'); throw marker; }}}; }, return() { trace.push('close'); return {}; }}; }};
      let caught; try { await invoke(name, {...valid(name), allAcceptedCredentialIds: iterable}); } catch (error) { caught = error; }
      assert(caught === marker, 'Iterable conversion preserves thrown identity'); same(trace, ['iterator', 'next', 'convert']);
      for (const allAcceptedCredentialIds of [[], new Set(['AA', 'AQ']), (function*() { yield 'AA'; yield 'AQ'; })()]) {
        assert(await invoke(name, {...valid(name), allAcceptedCredentialIds}) === undefined, 'Iterable sequence');
      }
      for (const allAcceptedCredentialIds of [null, {}, 'AA', ['AA', 'not valid'], ['AA', 'A']]) {
        await rejected(name, [{...valid(name), allAcceptedCredentialIds}], w.TypeError);
      }
    });
    await run('current user details/DOMString labels preserve lone surrogates and conversion', async () => {
      const trace = [], name = methods[2];
      const options = {...valid(name), displayName: {toString() { trace.push('displayName'); return '\ud800'; }}, name: {toString() { trace.push('name'); return '\udfff'; }}};
      assert(await invoke(name, options) === undefined, 'Labels do not require USVString conversion'); same(trace, ['displayName', 'name']);
      await rejected(name, [{...valid(name), displayName: Symbol('label')}], w.TypeError);
    });
    try { facts.capabilities.push({realm, value: await w.PublicKeyCredential.getClientCapabilities()}); }
    catch (error) { facts.capabilities.push({realm, error: String(error)}); }
  }
  facts.passed = facts.checks.filter(row => row.passed).length; facts.total = facts.checks.length; facts.complete = true;
  return true;
})()
