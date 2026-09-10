(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const throws = (fn, name = 'TypeError') => {
    let caught;
    try { fn(); } catch (error) { caught = error; }
    assert(caught && caught.name === name, `expected ${name}`);
  };
  for (const ctor of [WebGLRenderingContext, WebGL2RenderingContext,
      WebGLBuffer, WebGLShader, WebGLProgram, WebGLFramebuffer,
      WebGLRenderbuffer, WebGLUniformLocation]) {
    throws(() => new ctor());
    throws(() => ctor());
    assert(typeof ctor === 'function' && ctor.length === 0, 'interface constructor shape');
  }
  assert(Object.getPrototypeOf(WebGL2RenderingContext.prototype) === Object.prototype, 'WebGL2 is not a WebGL1 subclass');
  for (const ctor of [WebGLBuffer, WebGLShader, WebGLProgram, WebGLFramebuffer, WebGLRenderbuffer]) {
    assert(Object.getPrototypeOf(ctor.prototype) === WebGLObject.prototype, 'WebGL handle inheritance');
  }
  let coerced = false;
  const argument = { [Symbol.toPrimitive]() { coerced = true; return 0; } };
  for (const ctor of [WebGLRenderingContext, WebGL2RenderingContext]) {
    for (const receiver of [{}, Object.create(ctor.prototype), ctor.prototype]) {
      for (const [name, descriptor] of Object.entries(Object.getOwnPropertyDescriptors(ctor.prototype))) {
        if (name === 'constructor') continue;
        // Chromium's makeXRCompatible rejects a Promise rather than throwing.
        // Moli does not expose that optional XR entry point; this matrix covers
        // the synchronous WebGL methods and accessors it currently exposes.
        if (name === 'makeXRCompatible') continue;
        if (typeof descriptor.value === 'function') {
          throws(() => descriptor.value.call(receiver, argument, argument, argument, argument));
        }
        if (descriptor.get) throws(() => descriptor.get.call(receiver));
        if (descriptor.set) throws(() => descriptor.set.call(receiver, argument));
      }
    }
  }
  assert(!coerced, 'invalid receiver must fail before argument conversion');

  throws(() => WebGLContextEvent('test'));
  throws(() => new WebGLContextEvent());
  const event = new WebGLContextEvent('example', {
    statusMessage: 123, bubbles: true, cancelable: true, composed: true
  });
  assert(event instanceof Event && event instanceof WebGLContextEvent, 'event inheritance');
  assert(Object.prototype.toString.call(event) === '[object WebGLContextEvent]', 'event tag');
  assert(event.statusMessage === '123' && !event.isTrusted, 'constructor state');
  assert(event.bubbles && event.cancelable && event.composed, 'EventInit members');
  assert(new WebGLContextEvent('test').statusMessage === '', 'default status message');
  const descriptor = Object.getOwnPropertyDescriptor(WebGLContextEvent.prototype, 'statusMessage');
  assert(descriptor.enumerable && descriptor.configurable && !descriptor.set, 'readonly prototype accessor');
  assert(!Object.hasOwn(event, 'statusMessage'), 'state is not an own data property');
  throws(() => descriptor.get.call({}));
  throws(() => descriptor.get.call(Object.create(WebGLContextEvent.prototype)));
  throws(() => descriptor.get.call(new Proxy(event, {})));
  throws(() => new WebGLContextEvent('test', { statusMessage: Symbol() }));
  throws(() => new WebGLContextEvent('test', { get statusMessage() { throw new RangeError('fixture'); } }), 'RangeError');

  const labels = [];
  const owners = typeof document === 'undefined' ? ['offscreen'] : ['html', 'offscreen'];
  for (const owner of owners) {
    for (const kind of owner === 'html' ? ['webgl', 'experimental-webgl', 'webgl2'] : ['webgl', 'webgl2']) {
      const canvas = owner === 'html' ? document.createElement('canvas') : new OffscreenCanvas(2, 2);
      assert(canvas instanceof EventTarget, `${owner} must be an EventTarget`);
      const events = [];
      const listener = e => {
        assert(e instanceof WebGLContextEvent, 'native failure event type');
        assert(e.isTrusted && e.cancelable && !e.bubbles, 'failure event flags');
        assert(e.target === canvas && e.currentTarget === canvas, 'failure event target');
        assert(typeof e.statusMessage === 'string' && e.statusMessage.length > 0, 'failure reason');
        e.preventDefault();
        assert(e.defaultPrevented, 'cancelable failure notification');
        events.push(e);
      };
      canvas.addEventListener('webglcontextcreationerror', listener);
      assert(canvas.getContext(kind) === null, `${owner}:${kind} must report unavailable`);
      assert(events.length === 1, 'failure notification must be synchronous');
      assert(canvas.getContext(kind) === null && events.length === 2, 'repeat failure is not cached');
      const ctx = canvas.getContext('2d');
      assert(ctx !== null, 'failed creation must allow 2D fallback');
      assert(canvas.getContext('2d') === ctx, 'successful context is cached');
      ctx.fillStyle = '#ff0000'; ctx.fillRect(0, 0, 1, 1);
      assert(Array.from(ctx.getImageData(0, 0, 1, 1).data).join() === '255,0,0,255', 'fallback actually renders');
      assert(canvas.getContext(kind) === null, 'cannot change an existing context type');
      assert(events.length === (owner === 'html' ? 3 : 2), 'host-specific context conflict notification');
      assert(events.every(e => e.currentTarget === null), 'dispatch state is cleared');
      canvas.removeEventListener('webglcontextcreationerror', listener);
      labels.push(`${owner}:${kind}`);
    }
  }
  const offscreen = new OffscreenCanvas(1, 1);
  throws(() => offscreen.getContext('experimental-webgl'));
  throws(() => offscreen.getContext('unknown'));
  throws(() => offscreen.getContext(Symbol()));
  assert(offscreen.getContext('2d') !== null, 'invalid Offscreen enum must not lock context');
  if (typeof document !== 'undefined') {
    const canvas = document.createElement('canvas');
    assert(canvas.getContext('unknown') === null, 'unknown HTML context is null, not an enum error');
    throws(() => canvas.getContext(Symbol()));
    canvas.__moliCanvasContextKind = 'webgl';
    canvas.__moliCanvasContext2D = {};
    assert(canvas.getContext('webgl') === null, 'author properties must not manufacture a native context');
    const ctx = canvas.getContext('2d');
    canvas.__moliCanvasContextKind = 'webgl2';
    assert(canvas.getContext('2d') === ctx, 'author properties must not replace the context cache');
  }
  return JSON.stringify(labels);
})()
