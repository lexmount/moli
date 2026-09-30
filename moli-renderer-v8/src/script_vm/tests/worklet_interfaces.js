(async () => {
  const rows = [], assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, run) => { try { await run(); rows.push({ name, pass: true }); } catch (error) { rows.push({ name, pass: false, message: String(error) }); } };
  const popup = open(), realms = [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]];
  try {
    for (const [label, w] of realms) {
      if (!w.isSecureContext) {
        await check(label + '/insecure', () => assert(!('Worklet' in w) && !('AudioWorklet' in w) && !('audioWorklet' in w.BaseAudioContext.prototype) && !('audioWorklet' in new w.OfflineAudioContext(1,1,44100)), 'secure globals and attribute hidden'));
        continue;
      }
      for (const name of ['Worklet', 'AudioWorklet']) {
        await check(label + '/' + name, () => {
          const C = w[name], parent = name === 'AudioWorklet' ? w.Worklet : w.Object;
          const d = Object.getOwnPropertyDescriptor(w, name);
          assert(typeof C === 'function' && C.name === name && C.length === 0, 'constructor metadata');
          assert(d.writable && d.configurable && !d.enumerable, 'global descriptor');
          assert(Object.getPrototypeOf(C.prototype) === parent.prototype && Object.getPrototypeOf(C) === (parent === w.Object ? w.Function.prototype : parent), 'native inheritance');
          const tag = Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag);
          assert(tag.value === name && tag.configurable && !tag.writable && !tag.enumerable, 'prototype tag');
          for (const call of [() => C(), () => new C()]) { let error; try { call(); } catch (e) { error = e; } assert(error instanceof w.TypeError, 'illegal constructor'); }
        });
      }
      await check(label + '/receiver', async () => {
        const method = w.Worklet.prototype.addModule, d = Object.getOwnPropertyDescriptor(w.Worklet.prototype, 'addModule');
        assert(method.length === 1 && d.enumerable && d.configurable && d.writable, 'shared operation descriptor');
        let conversions = 0, traps = 0; const trap = () => { traps++; throw Error('author trap'); };
        const url = { toString() { conversions++; return 'data:text/javascript,'; } }, options = { get credentials() { conversions++; return 'same-origin'; } };
        const revoked = Proxy.revocable({}, {}); revoked.revoke();
        for (const receiver of [null, {}, w.Worklet.prototype, w.AudioWorklet.prototype, Object.create(w.Worklet.prototype), new Proxy({}, { get: trap, getPrototypeOf: trap }), revoked.proxy]) {
          const promise = method.call(receiver, url, options); assert(promise instanceof w.Promise, 'callee Promise');
          let error; try { await promise; } catch (e) { error = e; } assert(error instanceof w.TypeError, 'callee TypeError rejection');
        }
        assert(conversions === 0 && traps === 0, 'receiver validation precedes author callbacks');
      });
      let context;
      try {
        await check(label + '/native-factory', () => {
          const C = w.AudioWorklet, descriptor = Object.getOwnPropertyDescriptor(w, 'AudioWorklet'); let reads = 0;
          Object.defineProperty(w, 'AudioWorklet', { configurable: true, get() { reads++; throw Error('author constructor'); } });
          try { context = new w.AudioContext(); } finally { if (descriptor) Object.defineProperty(w, 'AudioWorklet', descriptor); else delete w.AudioWorklet; }
          const worklet = context.audioWorklet;
          const d = Object.getOwnPropertyDescriptor(w.BaseAudioContext.prototype, 'audioWorklet');
          assert(d && typeof d.get === 'function' && d.set === undefined && d.enumerable && d.configurable && !Object.hasOwn(context, 'audioWorklet'), 'shared readonly attribute');
          const offline = new w.OfflineAudioContext(1, 1, 44100);
          assert(Object.getPrototypeOf(offline.audioWorklet) === C.prototype && offline.audioWorklet === offline.audioWorklet, 'offline worklet');
          assert(d.get.call(context) === worklet, 'genuine receiver');
          for (const value of [{}, Object.create(context), new Proxy(context, {})]) { let error; try {d.get.call(value);} catch(e) {error=e;} assert(error instanceof w.TypeError, 'getter receiver'); }
          assert(Object.getPrototypeOf(worklet) === C.prototype && worklet instanceof w.Worklet, 'native derived prototype');
          assert(worklet === context.audioWorklet && !Object.hasOwn(worklet, 'addModule') && worklet.addModule === w.Worklet.prototype.addModule && reads === 0, 'shared operation and intrinsic factory');
        });
        await check(label + '/argument-conversion', async () => {
          const worklet = context.audioWorklet;
          for (const args of [[], [Symbol()], ['data:text/javascript,', 7], ['data:text/javascript,', {credentials:'invalid'}]]) {
            let error; const promise=worklet.addModule(...args); assert(promise instanceof w.Promise, 'conversion rejection is Promise');
            try { await promise; } catch (e) { error=e; } assert(error instanceof w.TypeError, 'invalid arguments');
          }
          const sentinel = {}, order=[]; let error;
          const promise=worklet.addModule({toString(){order.push('url');return 'data:text/javascript,';}},{get credentials(){order.push('credentials');return {toString(){order.push('string');throw sentinel;}};}});
          try { await promise; } catch (e) {error=e;}
          assert(error===sentinel && order.join()==='url,credentials,string', 'dictionary order and original exception');
        });
        await check(label + '/invalid-url', async () => {
          let error;
          try { await context.audioWorklet.addModule('https://['); } catch(e) {error=e;}
          assert(error instanceof w.DOMException && error.name === 'SyntaxError', 'invalid URL DOMException');
          const sentinel = {};
          try { await context.audioWorklet.addModule('https://[', {get credentials(){throw sentinel;}}); } catch(e) {error=e;}
          assert(error === sentinel, 'dictionary conversion before URL parsing');
        });
        await check(label + '/cross-realm', async () => {
          const other = realms.find(([,r]) => r !== w)[1];
          const otherContext = new other.AudioContext();
          try {
            const getter = Object.getOwnPropertyDescriptor(w.BaseAudioContext.prototype, 'audioWorklet').get;
            const receiver = getter.call(otherContext);
            assert(Object.getPrototypeOf(receiver) === other.AudioWorklet.prototype, 'getter uses receiver realm');
            const p = w.Worklet.prototype.addModule.call(receiver, 'https://[');
            let error; try {await p;} catch(e) {error=e;}
            assert(p instanceof other.Promise && error instanceof other.DOMException && error.name === 'SyntaxError', 'valid receiver realm');
            const invalid = w.Worklet.prototype.addModule.call(receiver, Symbol());
            try {await invalid;} catch(e) {error=e;}
            assert(invalid instanceof w.Promise && error instanceof w.TypeError, 'conversion rejection callee realm');
          } finally {await otherContext.close();}
        });
      } finally { if (context) await context.close(); }
    }
  } finally {popup.close();}
  globalThis.__nodeReplacementResults={rows,failures:rows.filter(row=>!row.pass),passed:rows.filter(row=>row.pass).length,total:rows.length};
  return rows.every(row=>row.pass);
})()
