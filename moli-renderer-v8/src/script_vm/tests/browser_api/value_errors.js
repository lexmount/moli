(() => {
  const checks = [];
  const check = (name, action) => {
    try { checks.push({name, passed: action() === true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const throws = (action, realm, expected) => {
    try { action(); } catch (error) {
      return expected === undefined ? Object.getPrototypeOf(error) === realm.TypeError.prototype : error === expected;
    }
    return false;
  };
  const utf16 = 'a\u0000\ud800b\udfff\ud83d\ude00';
  function run(realm, label) {
    const test = (name, action) => check(label + ': ' + name, action);
    const WT = realm.WebTransportError;
    const OC = realm.OverconstrainedError;
    const createWT = (message, options = {}) => new WT(message, options);
    test('WebTransportError inheritance and defaults', () => {
      const e = new WT();
      return e instanceof WT && e instanceof realm.DOMException && e instanceof realm.Error &&
        Object.getPrototypeOf(WT.prototype) === realm.DOMException.prototype && e.name === 'WebTransportError' &&
        e.message === '' && e.code === 0 && e.source === 'stream' && e.streamErrorCode === null;
    });
    test('WebTransportError length and tag', () => WT.length === 0 &&
      Object.prototype.toString.call(new WT()) === '[object WebTransportError]');
    test('WebTransportError new before conversion', () => {
      let reads = 0;
      return throws(() => WT({toString() { reads++; return 'x'; }}), realm) && reads === 0;
    });
    for (const [index, [value, expected]] of [[undefined, ''], [null, 'null'], [true, 'true'],
        [0, '0'], [-0, '0'], [1n, '1'], [utf16, utf16]].entries()) {
      test('WebTransportError message ' + index + ': ' + String(value), () => createWT(value).message === expected);
    }
    test('WebTransportError Symbol message before dictionary', () => {
      let reads = 0;
      return throws(() => new WT(Symbol(), {get source() {reads++; return 'stream';}}), realm) && reads === 0;
    });
    for (const options of [undefined, null, {}, []]) {
      test('WebTransportError empty options ' + String(options), () => {
        const e = new WT(utf16, options);
        return e.message === utf16 && e.source === 'stream' && e.streamErrorCode === null;
      });
    }
    for (const options of [false, 0, '', 1n, Symbol('dictionary')]) {
      test('WebTransportError invalid dictionary ' + String(options), () => throws(() => new WT('', options), realm));
    }
    for (const source of ['stream', 'session']) {
      test('WebTransportError source ' + source, () => createWT(utf16, {source, streamErrorCode: 42}).source === source);
    }
    for (const source of [null, '', 'STREAM', 'other', 0, Symbol('source')]) {
      test('WebTransportError rejects source ' + String(source), () => {
        let reads = 0;
        return throws(() => createWT('', {source, get streamErrorCode() {reads++; return 1;}}), realm) && reads === 0;
      });
    }
    test('WebTransportError undefined source default', () => createWT('', {source: undefined}).source === 'stream');
    const codes = [[undefined, null], [null, null], [NaN, 0], [Infinity, 4294967295], [-Infinity, 0],
      [-4294967297, 0], [-1, 0], [-0, 0], [0, 0], [0.5, 0], [1.5, 2], [2.5, 2], [3.5, 4],
      [42.4, 42], [42.6, 43], [65536, 65536], [4294967294.5, 4294967294],
      [4294967295, 4294967295], [4294967295.5, 4294967295], [4294967296, 4294967295],
      [Number.MAX_VALUE, 4294967295], [false, 0], [true, 1], ['3.5', 4], ['', 0], ['x', 0]];
    for (const [index, [value, expected]] of codes.entries()) {
      test('WebTransportError clamp ' + index + ': ' + String(value), () => Object.is(createWT('', {streamErrorCode: value}).streamErrorCode, expected));
    }
    for (const value of [1n, Symbol('code')]) {
      test('WebTransportError rejects code ' + String(value), () => throws(() => createWT('', {streamErrorCode: value}), realm));
    }
    test('WebTransportError one numeric conversion', () => {
      let reads = 0;
      const e = createWT('', {streamErrorCode: {[Symbol.toPrimitive](hint) {reads++; if(hint !== 'number') throw 42; return 2.5;}}});
      return e.streamErrorCode === 2 && reads === 1;
    });
    const sentinel = {};
    for (const member of ['message', 'source', 'streamErrorCode']) {
      test('WebTransportError exception ' + member, () => {
        const order = [];
        const message = {toString() {order.push('message'); if(member === 'message') throw sentinel; return utf16;}};
        const options = new Proxy({}, {get(target, name) {order.push(name); if(name === member) throw sentinel; return undefined;}});
        const expected = ['message', 'source', 'streamErrorCode'].slice(0, ['message', 'source', 'streamErrorCode'].indexOf(member)+1);
        return throws(() => new WT(message, options), realm, sentinel) && JSON.stringify(order) === JSON.stringify(expected);
      });
    }
    test('WebTransportError conversion order', () => {
      const order = [];
      const e = new WT({toString() {order.push('message'); return utf16;}}, new Proxy({}, {get(target, name) {
        order.push(name); return name === 'source' ? {toString() {order.push('source string'); return 'session';}} : {valueOf() {order.push('code number'); return 4.5;}};
      }}));
      return e.message === utf16 && e.source === 'session' && e.streamErrorCode === 4 &&
        JSON.stringify(order) === JSON.stringify(['message', 'source', 'source string', 'streamErrorCode', 'code number']);
    });
    test('WebTransportError null options avoid prototype getters', () => {
      let reads = 0;
      Object.defineProperty(realm.Object.prototype, 'source', {configurable:true, get() {reads++; throw sentinel;}});
      try { return new WT('', null).source === 'stream' && new WT('', undefined).source === 'stream' && reads === 0; }
      finally { delete realm.Object.prototype.source; }
    });
    for (const name of ['source', 'streamErrorCode']) {
      test('WebTransportError descriptor ' + name, () => {
        const d = Object.getOwnPropertyDescriptor(WT.prototype, name);
        const e = createWT('', {streamErrorCode:42});
        return d.get.length === 0 && d.get.name === 'get ' + name && d.set === undefined && d.enumerable && d.configurable &&
          !Object.hasOwn(e, name) && !Reflect.set(e, name, 99);
      });
      for (const kind of ['plain', 'prototype', 'inherit', 'author proxy', 'revoked proxy', 'other error']) {
        test('WebTransportError receiver ' + name + ' ' + kind, () => {
          const e = createWT(); let traps = 0;
          const proxy = new Proxy(e, {get() {traps++; throw sentinel;}, getPrototypeOf() {traps++; throw sentinel;}});
          const revoked = Proxy.revocable(e, {}); revoked.revoke();
          const bad = {'plain':{}, 'prototype':WT.prototype, 'inherit':Object.create(e), 'author proxy':proxy,
            'revoked proxy':revoked.proxy, 'other error':new realm.DOMException()}[kind];
          return throws(() => Object.getOwnPropertyDescriptor(WT.prototype, name).get.call(bad), realm) && traps === 0;
        });
      }
    }
    test('WebTransportError brand survives prototype removal', () => {
      const e = createWT(utf16, {source:'session', streamErrorCode:3});
      Object.setPrototypeOf(e, null);
      return Object.getOwnPropertyDescriptor(WT.prototype, 'source').get.call(e) === 'session' &&
        Object.getOwnPropertyDescriptor(realm.DOMException.prototype, 'message').get.call(e) === utf16;
    });
    test('WebTransportError subclass and Reflect.construct', () => {
      class Derived extends WT {}
      const e = new Derived(utf16, {streamErrorCode:1.5});
      function NewTarget() {}
      const other = Reflect.construct(WT, [utf16, {source:'session'}], NewTarget);
      return e instanceof Derived && e.streamErrorCode === 2 && Object.getPrototypeOf(other) === NewTarget.prototype &&
        Object.getOwnPropertyDescriptor(WT.prototype, 'source').get.call(other) === 'session';
    });
    for (const source of ['stream', 'session']) for (const code of [null, 0, 42, 4294967295]) {
      test('WebTransportError structured clone ' + source + ' ' + code, () => {
        const e = createWT(utf16, {source, streamErrorCode:code});
        const sentinel = {};
        for (const name of ['name','message','source','streamErrorCode']) Object.defineProperty(e, name, {enumerable:true, get() {throw sentinel;}});
        const cloned = realm.structuredClone({a:e, b:e});
        return cloned.a === cloned.b && cloned.a !== e && Object.getPrototypeOf(cloned.a) === realm.WebTransportError.prototype &&
          cloned.a.name === 'WebTransportError' && cloned.a.message === utf16 && cloned.a.source === source &&
          cloned.a.streamErrorCode === code && cloned.a.code === 0 && !Object.hasOwn(cloned.a, 'name');
      });
    }
    test('WebTransportError author Proxy clone rejects without traps', () => {
      let traps = 0, error;
      const proxy = new Proxy(createWT(), {ownKeys() {traps++; throw sentinel;}, get() {traps++; throw sentinel;}});
      try { realm.structuredClone(proxy); } catch(caught) {error = caught;}
      return error instanceof realm.DOMException && error.name === 'DataCloneError' && traps === 0;
    });
    test('OverconstrainedError Window exposure', () => typeof OC === (typeof document === 'undefined' ? 'undefined' : 'function'));
    if (typeof document !== 'undefined') {
      test('OverconstrainedError inheritance and defaults', () => {
        const e = new OC('width');
        return e instanceof OC && e instanceof realm.DOMException && e instanceof realm.Error &&
          Object.getPrototypeOf(OC.prototype) === realm.DOMException.prototype && e.name === 'OverconstrainedError' &&
          e.constraint === 'width' && e.message === '' && e.code === 0;
      });
      test('OverconstrainedError length and tag', () => OC.length === 1 && Object.prototype.toString.call(new OC('')) === '[object OverconstrainedError]');
      test('OverconstrainedError missing arity', () => throws(() => new OC(), realm));
      test('OverconstrainedError new before conversion', () => {
        let reads = 0; return throws(() => OC({toString(){reads++; return '';}}), realm) && reads === 0;
      });
      for (const [value, expected] of [[undefined,'undefined'], [null,'null'], [true,'true'], [0,'0'], [1n,'1'], [utf16,utf16]]) {
        test('OverconstrainedError constraint ' + String(value), () => new OC(value).constraint === expected);
      }
      for (const [value, expected] of [[undefined,''], [null,'null'], [true,'true'], [0,'0'], [1n,'1'], [utf16,utf16]]) {
        test('OverconstrainedError message ' + String(value), () => new OC(utf16,value).message === expected);
      }
      test('OverconstrainedError conversion order', () => {
        const order=[]; const e=new OC({toString(){order.push('constraint');return utf16;}},{toString(){order.push('message');return utf16;}});
        return e.constraint === utf16 && e.message === utf16 && JSON.stringify(order) === '["constraint","message"]';
      });
      test('OverconstrainedError Symbol constraint before message', () => {
        let reads=0; return throws(() => new OC(Symbol(), {toString(){reads++;return '';}}),realm) && reads===0;
      });
      test('OverconstrainedError Symbol message', () => throws(() => new OC('',Symbol()),realm));
      for(const member of ['constraint','message']) test('OverconstrainedError exception '+member, () => {
        let reads=0;
        return throws(() => new OC({toString(){reads++;if(member==='constraint')throw sentinel;return utf16;}},
          {toString(){reads++;throw sentinel;}}),realm,sentinel) && reads===(member==='constraint'?1:2);
      });
      test('OverconstrainedError descriptor', () => {
        const d=Object.getOwnPropertyDescriptor(OC.prototype,'constraint');const e=new OC(utf16);
        return d.get.name === 'get constraint' && d.get.length === 0 && d.set === undefined && d.enumerable && d.configurable &&
          !Object.hasOwn(e,'constraint') && !Reflect.set(e,'constraint','changed');
      });
      for(const kind of ['plain','prototype','inherit','author proxy','revoked proxy','other error']) test('OverconstrainedError receiver '+kind,() => {
        const e=new OC(utf16);let traps=0;
        const proxy=new Proxy(e,{get(){traps++;throw sentinel;},getPrototypeOf(){traps++;throw sentinel;}});
        const revoked=Proxy.revocable(e,{});revoked.revoke();
        const bad={'plain':{},'prototype':OC.prototype,'inherit':Object.create(e),'author proxy':proxy,'revoked proxy':revoked.proxy,'other error':new realm.DOMException()}[kind];
        return throws(()=>Object.getOwnPropertyDescriptor(OC.prototype,'constraint').get.call(bad),realm)&&traps===0;
      });
      test('OverconstrainedError subclass and native brand', () => {
        class Derived extends OC {} const e=new Derived(utf16,utf16); Object.setPrototypeOf(e,null);
        return Object.getOwnPropertyDescriptor(OC.prototype,'constraint').get.call(e)===utf16 &&
          Object.getOwnPropertyDescriptor(realm.DOMException.prototype,'message').get.call(e)===utf16;
      });
      test('OverconstrainedError nonserializable subclass', () => {
        let error; try {realm.structuredClone(new OC('width'));}catch(caught){error=caught;}
        return error instanceof realm.DOMException && error.name==='DataCloneError';
      });
    }
  }
  if (typeof document === 'undefined') run(globalThis, 'worker');
  else {
    run(globalThis, 'main');
    run(document.querySelector('iframe').contentWindow, 'iframe');
    check('cross realm error getters and clone', () => {
      const child=document.querySelector('iframe').contentWindow;
      const wt=new child.WebTransportError(utf16,{source:'session',streamErrorCode:4.5});
      const oc=new child.OverconstrainedError(utf16,utf16);
      return Object.getOwnPropertyDescriptor(WebTransportError.prototype,'streamErrorCode').get.call(wt)===4 &&
        Object.getOwnPropertyDescriptor(OverconstrainedError.prototype,'constraint').get.call(oc)===utf16 &&
        Object.getOwnPropertyDescriptor(DOMException.prototype,'message').get.call(wt)===utf16 &&
        Object.getPrototypeOf(structuredClone(wt))===WebTransportError.prototype &&
        Object.getPrototypeOf(child.structuredClone(new WebTransportError()))===child.WebTransportError.prototype;
    });
  }
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  if(typeof document==='undefined') postMessage(globalThis.__uiEventResults);
  return true;
})()
