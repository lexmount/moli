// Moli's declared Windows/D3D11 compatibility profile, not a physical GPU probe.
(() => {
  const results = [];
  for (const kind of ['webgl', 'webgl2']) {
    for (const offscreen of typeof document === 'undefined' ? [true] : [false, true]) {
      const label = `${offscreen ? 'offscreen' : 'html'}:${kind}`;
      const check = (condition, message) => {
        if (!condition) throw new Error(`${label}: ${message}`);
      };
      const gl = (offscreen ? new OffscreenCanvas(1, 1) : document.createElement('canvas')).getContext(kind);
      check(gl.getParameter(gl.VENDOR) === 'WebKit', 'masked vendor is unchanged');
      check(gl.getParameter(gl.RENDERER) === 'WebKit WebGL', 'masked renderer is unchanged');
      const debug = gl.getExtension('WEBGL_debug_renderer_info');
      check(gl.getParameter(debug.UNMASKED_VENDOR_WEBGL) === 'Google Inc. (NVIDIA)', 'compatibility vendor');
      check(gl.getParameter(debug.UNMASKED_RENDERER_WEBGL) ===
        'ANGLE (NVIDIA, NVIDIA Quadro P1000 Direct3D11 vs_5_0 ps_5_0, D3D11-23.21.13.9077)',
        'compatibility renderer');
      // These are capability declarations, not a GPU allocation/rendering test.
      for (const [name, expected] of [
        ['MAX_TEXTURE_SIZE', 16384], ['MAX_CUBE_MAP_TEXTURE_SIZE', 16384],
        ['MAX_RENDERBUFFER_SIZE', 16384], ['MAX_TEXTURE_IMAGE_UNITS', 16],
        ['MAX_VERTEX_TEXTURE_IMAGE_UNITS', 16], ['MAX_COMBINED_TEXTURE_IMAGE_UNITS', 32],
      ]) {
        check(gl.getParameter(gl[name]) === expected, `${name} resource limit`);
      }
      for (const [name, type, expected] of [
        ['MAX_VIEWPORT_DIMS', Int32Array, [32767, 32767]],
        ['ALIASED_POINT_SIZE_RANGE', Float32Array, [1, 1024]],
        ['ALIASED_LINE_WIDTH_RANGE', Float32Array, [1, 1]],
      ]) {
        const value = gl.getParameter(gl[name]);
        check(value instanceof type && Array.from(value).join('|') === expected.join('|'),
          `${name} range type and limits`);
        value.fill(0);
        check(Array.from(gl.getParameter(gl[name])).join('|') === expected.join('|'),
          `${name} returns an independent copy`);
      }
      gl.viewport(0, 0, 2147483647, 2147483647);
      check(Array.from(gl.getParameter(gl.VIEWPORT)).join('|') === '0|0|32767|32767',
        'viewport clamps to the declared maximum');
      // ANGLE D3D11 uses IEEE floats and 32-bit integers for every precision class.
      for (const shader of [gl.VERTEX_SHADER, gl.FRAGMENT_SHADER]) {
        for (const [names, expected] of [
          [['LOW_FLOAT', 'MEDIUM_FLOAT', 'HIGH_FLOAT'], [127, 127, 23]],
          [['LOW_INT', 'MEDIUM_INT', 'HIGH_INT'], [31, 30, 0]],
        ]) {
          for (const name of names) {
            const value = gl.getShaderPrecisionFormat(shader, gl[name]);
            check([value.rangeMin, value.rangeMax, value.precision].join('|') === expected.join('|'),
              `${shader}:${name} precision matches the declared backend`);
          }
        }
      }
      check(gl.getError() === gl.NO_ERROR, 'profile queries do not generate GL errors');
      results.push(label);
    }
  }
  return JSON.stringify(results);
})()
