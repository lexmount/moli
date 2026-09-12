// No GPU-specific identity or numeric limit is assumed by this contract probe.
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
      check(gl.getParameter(gl.VENDOR) === 'WebKit', 'masked vendor');
      check(gl.getParameter(gl.RENDERER) === 'WebKit WebGL', 'masked renderer');
      check(gl.getParameter(gl.VERSION).startsWith(kind === 'webgl' ? 'WebGL 1.0 (' : 'WebGL 2.0 ('), 'version format');
      check(gl.getParameter(gl.SHADING_LANGUAGE_VERSION).startsWith('WebGL GLSL ES '), 'shading language version');
      gl.getSupportedExtensions();
      check(gl.getParameter(37445) === null, 'enumerating extensions does not enable debug queries');
      check(gl.getError() === gl.INVALID_ENUM, 'disabled debug query signals an error');
      const debug = gl.getExtension('webgl_debug_renderer_info');
      for (const pname of [debug.UNMASKED_VENDOR_WEBGL, debug.UNMASKED_RENDERER_WEBGL]) {
        check(typeof gl.getParameter(pname) === 'string', 'enabled debug query returns a string');
      }
      const other = create();
      check(other.getParameter(37446) === null && other.getError() === other.INVALID_ENUM, 'activation is context-local');
      for (const pname of [gl.ALIASED_LINE_WIDTH_RANGE, gl.ALIASED_POINT_SIZE_RANGE]) {
        const range = gl.getParameter(pname);
        check(range instanceof Float32Array && range.length === 2, 'range query is Float32Array');
        const first = range[0];
        range[0] = NaN;
        check(gl.getParameter(pname)[0] === first, 'range query returns a copy');
      }
      check(gl.getParameter(gl.COMPRESSED_TEXTURE_FORMATS) instanceof Uint32Array, 'compressed formats query type');
      check(gl.getParameter(gl.MAX_VIEWPORT_DIMS) instanceof Int32Array, 'viewport limits query type');
      for (const shader of [gl.VERTEX_SHADER, gl.FRAGMENT_SHADER]) {
        for (const name of ['LOW_INT', 'MEDIUM_INT', 'HIGH_INT']) {
          const precision = gl.getShaderPrecisionFormat(shader, gl[name]);
          check(precision.precision === 0 && precision.rangeMin > 0 && precision.rangeMax > 0, 'integer precision');
        }
        check(gl.getShaderPrecisionFormat(shader, gl.HIGH_FLOAT).precision > 0, 'floating-point precision');
      }
      check(gl.getError() === gl.NO_ERROR, 'valid queries leave no error');
      for (const call of [() => gl.getParameter(), () => gl.getParameter.call({}, gl.VENDOR),
                         () => gl.getShaderPrecisionFormat(gl.VERTEX_SHADER),
                         () => gl.getShaderPrecisionFormat.call({}, gl.VERTEX_SHADER, gl.HIGH_FLOAT)]) {
        let threw = false;
        try { call(); } catch (error) { threw = error instanceof TypeError; }
        check(threw, 'query checks receiver and required arguments');
      }
      check(gl.getError() === gl.NO_ERROR, 'WebIDL exceptions do not create GL errors');
      check(gl.getParameter(0xffffffff) === null, 'unknown enum returns null');
      check(gl.getParameter(0xffffffff) === null, 'repeat invalid query returns null');
      gl.viewport(0, 0, -1, 1);
      check(gl.getError() === gl.INVALID_ENUM, 'first pending error');
      check(gl.getError() === gl.INVALID_VALUE, 'distinct error is preserved');
      check(gl.getError() === gl.NO_ERROR, 'repeated pending errors coalesce');
      check(gl.getShaderPrecisionFormat(0, gl.HIGH_FLOAT) === null, 'invalid shader enum');
      check(gl.getError() === gl.INVALID_ENUM, 'invalid shader enum error');
      check(gl.getShaderPrecisionFormat(gl.VERTEX_SHADER, 0) === null, 'invalid precision enum');
      check(gl.getError() === gl.INVALID_ENUM, 'invalid precision enum error');
      results.push(label);
    }
  }
  return JSON.stringify(results);
})()
