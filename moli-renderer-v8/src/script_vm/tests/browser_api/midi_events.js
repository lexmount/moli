(() => {
  const checks = [];
  const assert = (ok, message = 'assertion failed') => { if (!ok) throw Error(message); };
  const check = (name, run) => {
    try { run(); checks.push({name, passed: true}); }
    catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const throws = (run, prototype) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    assert(error && Object.getPrototypeOf(error) === prototype, 'callee TypeError required');
  };
  const realms = [window, document.querySelector('iframe').contentWindow];
  for (const [realmIndex, realm] of realms.entries()) {
    for (const [kind, member] of [['MIDIMessageEvent', 'data'], ['MIDIConnectionEvent', 'port']]) {
      const Ctor = realm[kind];
      const tag = `${realmIndex}:${kind}`;
      const getter = () => {
        const d = Object.getOwnPropertyDescriptor(Ctor.prototype, member);
        assert(d && typeof d.get === 'function' && !d.set && d.enumerable && d.configurable, 'payload descriptor');
        return d.get;
      };
      check(tag + ':metadata', () => {
        assert(Ctor.name === kind && Ctor.length === 1 && Ctor.prototype.constructor === Ctor);
        assert(Object.getPrototypeOf(Ctor) === realm.Event);
        assert(Object.getPrototypeOf(Ctor.prototype) === realm.Event.prototype);
      });
      check(tag + ':getter metadata', () => { assert(getter().length === 0); });
      check(tag + ':no new', () => throws(() => Ctor('test'), realm.TypeError.prototype));
      check(tag + ':no arguments', () => throws(() => new Ctor(), realm.TypeError.prototype));
      for (const [index, init] of [undefined, null, {}, [], function init() {}, Object(7), {[member]: undefined}].entries()) {
        check(tag + ':empty dictionary ' + index, () => {
          const event = new Ctor('test', init);
          assert(event[member] === null && event.type === 'test');
          assert(!event.bubbles && !event.cancelable && !event.composed && !event.isTrusted);
          assert(event instanceof realm.Event && Object.getPrototypeOf(event) === Ctor.prototype);
          assert(Object.prototype.toString.call(event) === `[object ${kind}]`);
        });
      }
      for (const [index, init] of [false, 1, 'str', 1n, Symbol('dict')].entries()) {
        check(tag + ':primitive dictionary ' + index, () => throws(() => new Ctor('test', init), realm.TypeError.prototype));
      }
      for (const [index, type] of [undefined, null, 7, 'type\ud800\udfff'].entries()) {
        check(tag + ':type conversion ' + index, () => assert(new Ctor(type).type === String(type)));
      }
      check(tag + ':symbol type', () => throws(() => new Ctor(Symbol()), realm.TypeError.prototype));
      check(tag + ':type exception precedes dictionary', () => {
        let reads = 0; const sentinel = {};
        const init = new Proxy({}, {get() { reads++; throw Error('unexpected dictionary read'); }});
        let error;
        try { new Ctor({toString() { throw sentinel; }}, init); } catch (caught) { error = caught; }
        assert(error === sentinel && reads === 0);
      });
      check(tag + ':conversion order', () => {
        const order = [];
        const init = new Proxy({}, {get(target, key) { order.push(key); return key === member ? undefined : true; }});
        const event = new Ctor({toString() { order.push('type'); return 'test'; }}, init);
        assert(order.join() === `type,bubbles,cancelable,composed,${member}`, String(order));
        assert(event[member] === null && event.bubbles && event.cancelable && event.composed);
      });
      for (const key of ['bubbles', 'cancelable', 'composed', member]) {
        check(tag + ':getter exception ' + key, () => {
          const order = []; const sentinel = {};
          const init = new Proxy({}, {get(target, name) { order.push(name); if (name === key) throw sentinel; return undefined; }});
          let error;
          try { new Ctor('test', init); } catch (caught) { error = caught; }
          assert(error === sentinel);
          const members = ['bubbles', 'cancelable', 'composed', member];
          assert(order.join() === members.slice(0, members.indexOf(key) + 1).join());
        });
      }
      const invalid = member === 'data'
        ? [null, false, 0, 1n, Symbol(), '', {}, [], new ArrayBuffer(2), new DataView(new ArrayBuffer(2)), new Int8Array(2), new Uint8ClampedArray(2), new Uint16Array(2), Object.create(Uint8Array.prototype), Object.create(new Uint8Array(2))]
        : [null, false, 0, 1n, Symbol(), '', {}, [], new realm.Event('test'), Object.create(realm.MIDIPort.prototype), Object.create(realm.MIDIInput.prototype), Object.create(realm.MIDIOutput.prototype)];
      for (const [index, value] of invalid.entries()) {
        check(tag + ':invalid payload ' + index, () => throws(() => new Ctor('test', {[member]: value}), realm.TypeError.prototype));
      }
      let traps = 0;
      const target = member === 'data' ? new realm.Uint8Array(2) : Object.create(realm.MIDIPort.prototype);
      const author = new Proxy(target, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 43;}});
      const revoked = Proxy.revocable(target, {}); revoked.revoke();
      for (const [index, value] of [author, revoked.proxy].entries()) {
        check(tag + ':payload proxy ' + index, () => {
          throws(() => new Ctor('test', {[member]: value}), realm.TypeError.prototype);
          assert(traps === 0);
        });
      }
      for (let index = 0; index < 9; index++) {
        check(tag + ':getter receiver ' + index, () => {
          const event = new Ctor('test');
          const eventAuthor = new Proxy(event, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 43;}});
          const eventRevoked = Proxy.revocable(event, {}); eventRevoked.revoke();
          const value = [undefined, null, {}, Ctor.prototype, Object.create(event), Object.create(Ctor.prototype), new realm.Event('test'), eventAuthor, eventRevoked.proxy][index];
          throws(() => getter().call(value), realm.TypeError.prototype); assert(traps === 0);
        });
      }
      for (const [ownerIndex, owner] of realms.entries()) {
        check(tag + ':cross realm getter ' + ownerIndex, () => assert(getter().call(new owner[kind]('test')) === null));
        if (member === 'data') {
          check(tag + ':payload identity ' + ownerIndex, () => {
            const data = new owner.Uint8Array(new owner.ArrayBuffer(8), 2, 3); data.set([0x90, 60, 127]);
            const value = new Ctor('test', {data, bubbles: true, cancelable: true, composed: true});
            assert(value.data === data && getter().call(value) === data);
            assert(value.bubbles && value.cancelable && value.composed && !value.isTrusted);
            assert(!Object.hasOwn(value, 'data') && Array.from(value.data).join() === '144,60,127');
            data[1] = 61; assert(value.data[1] === 61);
            value.initEvent('again', false, false); assert(value.data === data);
          });
          for (const detached of [false, true]) {
            check(tag + ':fixed empty/detached ' + ownerIndex + ':' + detached, () => {
              const buffer = new owner.ArrayBuffer(detached ? 2 : 0); const data = new owner.Uint8Array(buffer);
              if (detached) buffer.transfer();
              assert(new Ctor('test', {data}).data === data);
            });
          }
          for (const [label, make] of [
            ['resizable empty', () => new owner.Uint8Array(new owner.ArrayBuffer(0, {maxByteLength: 4}))],
            ['resizable fixed view', () => new owner.Uint8Array(new owner.ArrayBuffer(2, {maxByteLength: 4}), 0, 2)],
            ['resizable tracking view', () => new owner.Uint8Array(new owner.ArrayBuffer(2, {maxByteLength: 4}))],
            ['resizable detached', () => {const buffer = new owner.ArrayBuffer(2, {maxByteLength: 4}); const data = new owner.Uint8Array(buffer); buffer.transfer(); return data;}],
            ['shared', () => new owner.Uint8Array(new owner.SharedArrayBuffer(2))],
            ['shared empty', () => new owner.Uint8Array(new owner.SharedArrayBuffer(0))],
            ['growable shared', () => new owner.Uint8Array(new owner.SharedArrayBuffer(2, {maxByteLength: 4}))],
            ['growable empty', () => new owner.Uint8Array(new owner.SharedArrayBuffer(0, {maxByteLength: 4}))],
          ]) {
            check(tag + ':buffer policy ' + ownerIndex + ':' + label, () => throws(() => new Ctor('test', {data: make()}), realm.TypeError.prototype));
          }
          check(tag + ':native buffer ignores author properties ' + ownerIndex, () => {
            const data = new owner.Uint8Array(2);
            Object.defineProperty(data, 'buffer', {get() {throw 42;}});
            assert(new Ctor('test', {data}).data === data);
          });
        }
      }
      check(tag + ':dispatch', () => {
        const source = member === 'data' ? new realm.Uint8Array([0x90, 60, 127]) : undefined;
        const value = new Ctor('midi', {[member]: source, cancelable: true});
        const target = new realm.EventTarget(); let count = 0;
        target.addEventListener('midi', e => { assert(e === value && e[member] === (source ?? null)); count++; e.preventDefault(); });
        assert(target.dispatchEvent(value) === false && value.defaultPrevented && count === 1);
      });
    }
    const encoder = new realm.TextEncoder();
    for (const [ownerIndex, owner] of realms.entries()) {
      for (const shared of [false, true]) {
        for (const [input, length, expectedRead, bytes] of [['', 0, 0, []], ['A', 0, 0, []], ['A', 1, 1, [65]], ['\u{1D306}', 3, 0, []], ['\u{1D306}', 4, 2, [240, 157, 140, 134]], ['\u{1D306}A', 5, 3, [240, 157, 140, 134, 65]], ['\ud800', 3, 1, [239, 191, 189]]]) {
          check(`${realmIndex}:encodeInto:${ownerIndex}:${shared}:${input}:${length}`, () => {
            const buffer = new owner[shared ? 'SharedArrayBuffer' : 'ArrayBuffer'](length + 4);
            const whole = new owner.Uint8Array(buffer); whole.fill(0x80);
            const dest = new owner.Uint8Array(buffer, 2, length);
            const result = encoder.encodeInto(input, dest);
            assert(result.read === expectedRead && result.written === bytes.length);
            assert(Array.from(whole).join() === [128, 128, ...bytes, ...Array(length - bytes.length + 2).fill(128)].join());
          });
        }
        check(`${realmIndex}:encodeInto:empty:${ownerIndex}:${shared}`, () => {
          const dest = new owner.Uint8Array(new owner[shared ? 'SharedArrayBuffer' : 'ArrayBuffer'](0));
          const result = encoder.encodeInto('A', dest); assert(result.read === 0 && result.written === 0);
        });
        check(`${realmIndex}:encodeInto:resizable:${ownerIndex}:${shared}`, () => {
          const dest = new owner.Uint8Array(new owner[shared ? 'SharedArrayBuffer' : 'ArrayBuffer'](0, {maxByteLength: 4}));
          throws(() => encoder.encodeInto('A', dest), realm.TypeError.prototype);
        });
      }
      check(`${realmIndex}:encodeInto:detached:${ownerIndex}`, () => {
        const buffer = new owner.ArrayBuffer(2); const dest = new owner.Uint8Array(buffer); buffer.transfer();
        const result = encoder.encodeInto('A', dest); assert(result.read === 0 && result.written === 0);
      });
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return true;
})()
