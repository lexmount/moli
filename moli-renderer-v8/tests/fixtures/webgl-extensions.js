// Shared Window/Worker probe. Tests the extension contract, not GPU rendering.
(() => {
  const results = [];
  for (const kind of ['webgl', 'webgl2']) {
    for (const offscreen of typeof document === 'undefined' ? [true] : [false, true]) {
      const label = `${offscreen ? 'offscreen' : 'html'}:${kind}`;
      const check = (condition, message) => {
        if (!condition) throw new Error(`${label}: ${message}`);
      };
      const create = () => (offscreen ? new OffscreenCanvas(1, 1) : document.createElement('canvas')).getContext(kind);
      const gl = create();
      const other = create();
      const names = gl.getSupportedExtensions();
      check(Array.isArray(names) && new Set(names).size === names.length, 'unique extension names');
      check(names.includes('WEBGL_debug_renderer_info'), 'debug queries available in both context versions');
      for (const name of names) {
        const extension = gl.getExtension(name);
        check(extension !== null, `listed extension ${name} exists`);
        check(gl.getExtension(name) === extension, `${name} has stable identity`);
        check(gl.getExtension(name.toLowerCase()) === extension, `${name} ignores ASCII case`);
        check(gl.getExtension(name.toUpperCase()) === extension, `${name} uppercase alias`);
        check(other.getExtension(name) !== extension, `${name} belongs to its context`);
        check(gl.getExtension(` ${name}`) === null, `${name} does not trim whitespace`);
      }
      names.length = 0;
      check(gl.getSupportedExtensions().length > 0, 'list is a fresh copy');
      check(gl.getExtension('not-an-extension') === null, 'unknown extension is null');
      const lose = gl.getExtension('WEBGL_lose_context');
      check(typeof lose.loseContext === 'function' && typeof lose.restoreContext === 'function',
            'extension methods exist even without a public Worker constructor');
      for (const method of ['getSupportedExtensions', 'getExtension']) {
        let threw = false;
        try { gl[method].call({}, 'WEBGL_debug_renderer_info'); }
        catch (error) { threw = error instanceof TypeError; }
        check(threw, `${method} rejects an illegal receiver`);
      }
      // Native factories must not invoke mutable public constructors.
      const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'WEBGL_debug_renderer_info');
      try {
        Object.defineProperty(globalThis, 'WEBGL_debug_renderer_info', {
          configurable: true,
          get() { throw new Error('public constructor was read'); }
        });
        check(create().getExtension('WEBGL_debug_renderer_info').UNMASKED_VENDOR_WEBGL === 37445,
              'factory uses the intrinsic prototype');
      } finally {
        if (descriptor) Object.defineProperty(globalThis, 'WEBGL_debug_renderer_info', descriptor);
        else delete globalThis.WEBGL_debug_renderer_info;
      }
      check(gl.getError() === gl.NO_ERROR, 'unsupported extension is not a GL error');
      results.push(label);
    }
  }
  return JSON.stringify(results);
})()
