(() => {
  const checks = [];
  const check = (name, run, available = true) => {
    try {
      checks.push({name, passed: available && run() === true});
    } catch (error) {
      checks.push({name, passed: false, error: String(error)});
    }
  };
  const throws = (run, ctor) => {
    try { run(); return false; } catch (error) { return error instanceof ctor; }
  };
  const throwsExactly = (run, expected) => {
    try { run(); return false; } catch (error) { return error === expected; }
  };
  const child = document.getElementById('child').contentWindow;
  for (const [realmName, realm] of [['main', globalThis], ['iframe', child]]) {
    for (const interfaceName of ['RTCRtpSender', 'RTCRtpReceiver']) {
      const ctor = realm[interfaceName];
      const method = ctor?.getCapabilities;
      const available = typeof method === 'function';
      const prefix = realmName + '/' + interfaceName + '/';
      const run = (name, body) => check(prefix + name, body, available);
      const call = (...args) => Reflect.apply(method, null, args);
      const marker = new realm.RangeError('conversion sentinel');
      check(prefix + 'available', () => available);
      run('descriptor', () => {
        const d = realm.Object.getOwnPropertyDescriptor(ctor, 'getCapabilities');
        return d.enumerable && d.configurable && d.writable && d.value === method &&
          method.name === 'getCapabilities' && method.length === 1;
      });
      run('method is not a constructor', () => {
        let conversions = 0;
        const value = {toString() { conversions++; return 'audio'; }};
        return throws(() => realm.Reflect.construct(method, [value]), realm.TypeError) && conversions === 0;
      });
      run('required argument', () => throws(() => call(), realm.TypeError));
      run('symbol', () => throws(() => call(realm.Symbol('audio')), realm.TypeError));
      run('boxed symbol', () => throws(() => call(realm.Object(realm.Symbol())), realm.TypeError));
      for (const [name, value] of [['undefined', undefined], ['null', null], ['false', false],
        ['number', 1], ['bigint', 1n], ['empty', ''], ['upper-audio', 'Audio'],
        ['upper-video', 'VIDEO'], ['space', ' audio'], ['trailing-space', 'video '],
        ['unknown', 'dummy'], ['nul', 'audio\0'], ['surrogate', 'video\ud800']]) {
        run('unknown/' + name, () => call(value) === null);
      }
      for (const kind of ['audio', 'video']) {
        run(kind + '/dictionary', () => {
          const value = call(kind);
          return value !== null && Object.getPrototypeOf(value) === realm.Object.prototype &&
            realm.Array.isArray(value.codecs) && realm.Array.isArray(value.headerExtensions) &&
            Object.getPrototypeOf(value.codecs) === realm.Array.prototype &&
            Object.getPrototypeOf(value.headerExtensions) === realm.Array.prototype &&
            value.codecs.every(codec => Object.getPrototypeOf(codec) === realm.Object.prototype &&
              typeof codec.mimeType === 'string' && codec.mimeType.startsWith(kind + '/') &&
              Number.isInteger(codec.clockRate) && codec.clockRate > 0 &&
              (codec.channels === undefined || Number.isInteger(codec.channels) && codec.channels > 0) &&
              (codec.sdpFmtpLine === undefined || typeof codec.sdpFmtpLine === 'string')) &&
            value.headerExtensions.every(ext => Object.getPrototypeOf(ext) === realm.Object.prototype &&
              typeof ext.uri === 'string');
        });
        run(kind + '/fresh nested dictionaries', () => {
          const first = call(kind), second = call(kind);
          const original = JSON.stringify(second);
          if (first.codecs.length) first.codecs[0].mimeType = 'changed';
          if (first.headerExtensions.length) first.headerExtensions[0].uri = 'changed';
          first.codecs.push({mimeType: 'changed'});
          first.headerExtensions.push({uri: 'changed'});
          return first !== second && first.codecs !== second.codecs &&
            first.headerExtensions !== second.headerExtensions &&
            JSON.stringify(second) === original && JSON.stringify(call(kind)) === original;
        });
        run(kind + '/DOMString conversion', () => {
          const log = [];
          const value = {get [Symbol.toPrimitive]() {
            log.push('get');
            return hint => { log.push(hint); return kind; };
          }};
          return call(value) !== null && log.join() === 'get,string';
        });
        run(kind + '/static receiver ignored', () => [null, undefined, {}, child.navigator,
          new Proxy(ctor, {})].every(receiver => Reflect.apply(method, receiver, [kind]) !== null));
        run(kind + '/author function Proxy forwards', () => Reflect.apply(new Proxy(method, {}), {}, [kind]) !== null);
        run(kind + '/extra arguments ignored', () => call(kind, {
          get [Symbol.toPrimitive]() { throw marker; }
        }) !== null);
        run(kind + '/public JSON ignored', () => {
          const original = Object.getOwnPropertyDescriptor(realm.JSON, 'parse');
          Object.defineProperty(realm.JSON, 'parse', {configurable: true, get() { throw marker; }});
          try { return call(kind) !== null; }
          finally { Object.defineProperty(realm.JSON, 'parse', original); }
        });
        run(kind + '/output bypasses inherited setters', () => {
          const keys = ['codecs', 'headerExtensions', 'mimeType', 'clockRate', 'channels', 'sdpFmtpLine', 'uri'];
          const old = keys.map(key => Object.getOwnPropertyDescriptor(realm.Object.prototype, key));
          let writes = 0;
          for (const key of keys) Object.defineProperty(realm.Object.prototype, key,
            {configurable: true, set() { writes++; }});
          try {
            const value = call(kind);
            return writes === 0 && Object.hasOwn(value, 'codecs') && Object.hasOwn(value, 'headerExtensions');
          } finally {
            keys.forEach((key, index) => old[index]
              ? Object.defineProperty(realm.Object.prototype, key, old[index])
              : Reflect.deleteProperty(realm.Object.prototype, key));
          }
        });
      }
      run('ordinary conversion order', () => {
        const log = [];
        const value = {
          get toString() { log.push('get-toString'); return () => {log.push('toString'); return {};}; },
          get valueOf() { log.push('get-valueOf'); return () => {log.push('valueOf'); return 'audio';}; }
        };
        return call(value) !== null && log.join() === 'get-toString,toString,get-valueOf,valueOf';
      });
      run('toPrimitive getter exception', () => throwsExactly(() => call({
        get [Symbol.toPrimitive]() { throw marker; }
      }), marker));
      run('toString exception identity', () => throwsExactly(() => call({toString() { throw marker; }}), marker));
      run('conversion rejects nonprimitive', () => throws(() => call({
        [Symbol.toPrimitive]() { return {}; }
      }), realm.TypeError));
      run('callee error realm', () => {
        const other = realm === child ? globalThis : child;
        for (const args of [[], [Symbol()], [{[Symbol.toPrimitive]() {return {};}}]]) {
          try { other.Reflect.apply(method, {}, args); return false; }
          catch (error) { if (!(error instanceof realm.TypeError) || error instanceof other.TypeError) return false; }
        }
        return true;
      });
      run('public interface getter ignored', () => {
        const original = Object.getOwnPropertyDescriptor(realm, interfaceName);
        Object.defineProperty(realm, interfaceName, {configurable: true, get() { throw marker; }});
        try { return call('audio') !== null; }
        finally { Object.defineProperty(realm, interfaceName, original); }
      });
    }
  }
  const passed = checks.filter(row => row.passed).length;
  globalThis.__uiEventResults = {complete: true, checks, total: checks.length, passed};
  if (passed !== checks.length) throw new Error(JSON.stringify(checks.filter(row => !row.passed)));
  return true;
})()
