(async () => {
  const rows = [], assert = (ok, message) => { if (!ok) throw Error(message); };
  const check = async (name, run) => { try { await run(); rows.push({ name, pass: true }); } catch (error) { rows.push({ name, pass: false, message: String(error) }); } };
  const inWindow = typeof window !== 'undefined';
  const popup = inWindow ? open() : null;
  const realms = inWindow ? [['main', window], ['child', document.getElementById('child').contentWindow], ['popup', popup]] : [['worker', self]];
  const resources = ['WebGLTexture', 'WebGLQuery', 'WebGLSampler', 'WebGLSync', 'WebGLTransformFeedback', 'WebGLVertexArrayObject'];
  const attributes = { WebGLActiveInfo: ['size', 'type', 'name'], WebGLShaderPrecisionFormat: ['rangeMin', 'rangeMax', 'precision'], WebGLContextEvent: ['statusMessage'] };
  const names = [...resources, ...Object.keys(attributes)];
  try {
    for (const [label, w] of realms) {
      for (const name of names) {
        await check(label + '/' + name + '/constructor', () => {
          const C = w[name], parent = resources.includes(name) ? w.WebGLObject : name === 'WebGLContextEvent' ? w.Event : w.Object;
          const d = Object.getOwnPropertyDescriptor(w, name);
          assert(typeof C === 'function' && C.name === name && C.length === (name === 'WebGLContextEvent' ? 1 : 0), 'constructor metadata');
          assert(d.writable && d.configurable && !d.enumerable, 'global descriptor');
          assert(Object.getPrototypeOf(C.prototype) === parent.prototype && Object.getPrototypeOf(C) === (parent === w.Object ? w.Function.prototype : parent), 'native inheritance');
          assert(C.prototype.constructor === C, 'prototype constructor');
          const tag = Object.getOwnPropertyDescriptor(C.prototype, Symbol.toStringTag);
          assert(tag.value === name && tag.configurable && !tag.writable && !tag.enumerable, 'prototype tag');
          for (const call of [() => C(), () => new C()]) {
            let error; try { call(); } catch (e) { error = e; }
            assert(error instanceof w.TypeError, 'constructor restrictions and error realm');
          }
        });
        for (const attribute of attributes[name] || []) {
          await check(label + '/' + name + '/' + attribute, () => {
            const C = w[name], d = Object.getOwnPropertyDescriptor(C.prototype, attribute);
            assert(d.get.length === 0 && d.set === undefined && d.enumerable && d.configurable, 'readonly shared getter');
            let traps = 0; const trap = () => { traps++; throw Error('author trap'); };
            const revoked = Proxy.revocable({}, {}); revoked.revoke();
            for (const receiver of [{}, C.prototype, Object.create(C.prototype), new Proxy({}, { get: trap, getPrototypeOf: trap }), revoked.proxy]) {
              let error; try { d.get.call(receiver); } catch (e) { error = e; }
              assert(error instanceof w.TypeError, 'native brand required');
            }
            assert(traps === 0, 'brand check does not invoke Proxy traps');
          });
        }
      }
      await check(label + '/event-payload', () => {
        const C = w.WebGLContextEvent, d = Object.getOwnPropertyDescriptor(C.prototype, 'statusMessage');
        const defaultEvent = new C('webglcontextlost');
        assert(defaultEvent instanceof w.Event && defaultEvent.statusMessage === '' && !defaultEvent.isTrusted, 'default event state');
        const order = [], value = '\ud800\0state\udfff';
        const event = new C({ toString() { order.push('type'); return 'loss'; } }, {
          get bubbles() { order.push('bubbles'); return true; },
          get cancelable() { order.push('cancelable'); return true; },
          get composed() { order.push('composed'); return true; },
          get statusMessage() { order.push('statusMessage'); return { toString() { order.push('string'); return value; } }; }
        });
        assert(order.join() === 'type,bubbles,cancelable,composed,statusMessage,string', 'EventInit conversion order');
        assert(event.statusMessage === value && event.bubbles && event.cancelable && event.composed, 'payload preserves DOMString code units');
        assert(!Object.hasOwn(event, 'statusMessage'), 'prototype payload');
        const target = new w.EventTarget(); let calls = 0;
        target.addEventListener('loss', e => { assert(e === event && e.target === target && e.statusMessage === value, 'native dispatch state'); calls++; e.preventDefault(); });
        assert(target.dispatchEvent(event) === false && calls === 1, 'Event cancellation');
        Object.setPrototypeOf(event, null);
        assert(d.get.call(event) === value, 'brand independent of author prototype');
        const sentinel = {}; let error;
        try { new C('loss', { get statusMessage() { throw sentinel; } }); } catch (e) { error = e; }
        assert(error === sentinel, 'getter exception preserved');
        assert(new C('loss', { statusMessage: null }).statusMessage === 'null', 'DOMString null conversion');
        class Derived extends C {}
        const derived = new Derived('loss', { statusMessage: 'subclass' });
        assert(derived instanceof Derived && derived instanceof C && derived.statusMessage === 'subclass', 'subclass identity');
      });
      await check(label + '/precision-native-factory', () => {
        const gl = new w.OffscreenCanvas(1, 1).getContext('webgl');
        assert(gl, 'WebGL context available');
        const C = w.WebGLShaderPrecisionFormat, globalDescriptor = Object.getOwnPropertyDescriptor(w, 'WebGLShaderPrecisionFormat');
        let reads = 0, value;
        Object.defineProperty(w, 'WebGLShaderPrecisionFormat', { configurable: true, get() { reads++; throw Error('author constructor'); } });
        try { value = gl.getShaderPrecisionFormat(gl.VERTEX_SHADER, gl.HIGH_FLOAT); }
        finally { if (globalDescriptor) Object.defineProperty(w, 'WebGLShaderPrecisionFormat', globalDescriptor); else delete w.WebGLShaderPrecisionFormat; }
        assert(reads === 0 && Object.getPrototypeOf(value) === C.prototype, 'intrinsic value prototype');
        for (const key of ['precision', 'rangeMin', 'rangeMax']) {
          const d = Object.getOwnPropertyDescriptor(C.prototype, key), number = value[key];
          assert(!Object.hasOwn(value, key) && Number.isInteger(number) && number >= 0, 'readonly numeric value');
          assert(d.get.call(value) === number, 'genuine receiver');
          for (const receiver of [Object.create(value), new Proxy(value, {})]) {
            let error; try { d.get.call(receiver); } catch (e) { error = e; }
            assert(error instanceof w.TypeError, 'forged receiver rejected');
          }
        }
        const getter = Object.getOwnPropertyDescriptor(C.prototype, 'precision').get, precision = value.precision;
        Object.setPrototypeOf(value, null);
        assert(getter.call(value) === precision, 'native state survives author prototype change');
      });
    }
  } finally { if (popup) popup.close(); }
  globalThis.__nodeReplacementResults = { rows, failures: rows.filter(row => !row.pass), passed: rows.filter(row => row.pass).length, total: rows.length };
  return rows.every(row => row.pass);
})()
