// Run unchanged in the renderer regression and public CDP smoke. These are
// observable GL state contracts, not evidence of GPU-backed rendering.
(() => {
  const results = [];
  for (const kind of ['webgl', 'webgl2']) {
    for (const offscreen of [false, true]) {
      const label = `${offscreen ? 'offscreen' : 'html'}:${kind}`;
      const check = (condition, message) => {
        if (!condition) throw new Error(`${label}: ${message}`);
      };
      const equal = (actual, expected, message) =>
        check(JSON.stringify(actual) === JSON.stringify(expected),
              `${message}: ${JSON.stringify(actual)} != ${JSON.stringify(expected)}`);
      const throwsTypeError = operation => {
        try { operation(); } catch (error) { return error instanceof TypeError; }
        return false;
      };
      const canvas = offscreen ? new OffscreenCanvas(320, 180) : document.createElement('canvas');
      canvas.width = 320;
      canvas.height = 180;
      const gl = canvas.getContext(kind);
      check(gl !== null, 'context acquisition');
      check(typeof gl.viewport === 'function' && gl.viewport.length === 4, 'viewport interface');
      equal([gl.VIEWPORT, gl.NO_ERROR, gl.INVALID_VALUE], [2978, 0, 1281], 'constants');
      const view = () => Array.from(gl.getParameter(gl.VIEWPORT));
      equal(view(), [0, 0, 320, 180], 'initial canvas dimensions');
      check(gl.getParameter(gl.VIEWPORT) instanceof Int32Array, 'query type');
      check(gl.viewport(-5, 8, 90, 70) === undefined, 'return value');
      equal(view(), [-5, 8, 90, 70], 'negative origins are valid');
      const copy = gl.getParameter(gl.VIEWPORT);
      copy.fill(123);
      equal(view(), [-5, 8, 90, 70], 'query is a copy');
      canvas.width = 500;
      canvas.height = 400;
      check(canvas.getContext(kind) === gl, 'reacquisition preserves identity');
      equal(view(), [-5, 8, 90, 70], 'resize preserves viewport');
      for (const other of ['2d', 'webgl', 'webgl2'].filter(value => value !== kind)) {
        check(canvas.getContext(other) === null, 'cannot switch context mode');
      }
      gl.viewport(1, 2, -3, 4);
      equal(view(), [-5, 8, 90, 70], 'invalid width preserves state');
      gl.viewport(0, 0, 10, 20);
      equal([gl.getError(), gl.getError()], [gl.INVALID_VALUE, gl.NO_ERROR],
            'valid operation does not consume a pending GL error');
      gl.viewport(1, 2, 3, -4);
      equal(view(), [0, 0, 10, 20], 'invalid height preserves state');
      equal([gl.getError(), gl.getError()], [gl.INVALID_VALUE, gl.NO_ERROR], 'error consumed once');
      gl.viewport('2.8', undefined, NaN, Infinity);
      equal(view(), [2, 0, 0, 0], 'WebIDL long conversion and zero dimensions');
      check(throwsTypeError(() => gl.viewport(1, 2, 3)), 'required arguments');
      check(throwsTypeError(() => gl.viewport(Symbol(), 0, 1, 1)), 'conversion exception');
      check(throwsTypeError(() => gl.viewport.call({}, 0, 0, 1, 1)), 'viewport receiver');
      check(throwsTypeError(() => gl.getError.call({})), 'getError receiver');
      equal(view(), [2, 0, 0, 0], 'WebIDL errors preserve state');
      check(gl.getError() === gl.NO_ERROR, 'WebIDL errors are not GL errors');
      const maximum = gl.getParameter(gl.MAX_VIEWPORT_DIMS);
      check(maximum instanceof Int32Array && maximum.length === 2, 'maximum dimensions type');
      gl.viewport(0, 0, 2147483647, 2147483647);
      equal(view(), [0, 0, ...maximum], 'dimensions clamp to advertised limits');
      const otherCanvas = offscreen ? new OffscreenCanvas(10, 20) : document.createElement('canvas');
      const other = otherCanvas.getContext(kind);
      gl.viewport(0, 0, -1, 1);
      check(other.getError() === other.NO_ERROR, 'error state is context-local');
      equal(Array.from(other.getParameter(other.VIEWPORT)),
            offscreen ? [0, 0, 10, 20] : [0, 0, 300, 150], 'viewport state is context-local');
      check(gl.getError() === gl.INVALID_VALUE, 'other context does not consume error');
      check(!Object.getOwnPropertyNames(gl).some(name => name.startsWith('__moliWebGl')),
            'GL state stays private');
      results.push(label);
    }
  }
  return JSON.stringify(results);
})()
