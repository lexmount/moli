(() => {
  const results = [];
  const check = (value, message) => { if (!value) throw new Error(message); };
  const throws = (callback, message) => {
    let name; try { callback(); } catch (error) { name = error.name; }
    check(name === 'TypeError', message);
  };
  const make = offscreen => offscreen ? new OffscreenCanvas(300, 150) : document.createElement('canvas');
  for (const kind of ['webgl', 'webgl2']) {
    for (const offscreen of typeof document === 'object' ? [false, true] : [true]) {
      const label = `${offscreen ? 'offscreen' : 'html'}:${kind}`;
      const canvas = make(offscreen), gl = canvas.getContext(kind);
      check(!!gl, `${label}: context available`);
      const proto = Object.getPrototypeOf(gl);
      for (const field of ['drawingBufferWidth', 'drawingBufferHeight']) {
        const d = Object.getOwnPropertyDescriptor(proto, field);
        check(d && typeof d.get === 'function' && d.set === undefined && d.enumerable && d.configurable,
          `${label}: native readonly ${field}`);
        throws(() => d.get.call({}), `${label}: ${field} receiver`);
        throws(() => d.get.call(Object.create(proto)), `${label}: ${field} forged receiver`);
      }
      check(gl.drawingBufferWidth === 300 && gl.drawingBufferHeight === 150, `${label}: default extent`);
      for (const [width, height] of [[319, 173], [0, 173], [0, 0], [23, 17]]) {
        canvas.width = width; canvas.height = height;
        check(gl.drawingBufferWidth === Math.max(1, width) && gl.drawingBufferHeight === Math.max(1, height),
          `${label}: resize extent`);
      }
      Object.defineProperty(canvas, 'width', {configurable: true, get(){ throw new Error('page width getter'); }});
      Object.defineProperty(canvas, 'height', {configurable: true, get(){ throw new Error('page height getter'); }});
      Object.defineProperty(gl, 'canvas', {configurable: true, value: {width: 999, height: 999}});
      check(gl.drawingBufferWidth === 23 && gl.drawingBufferHeight === 17, `${label}: extent uses native owner and size`);
      delete canvas.width; delete canvas.height; delete gl.canvas;
      // A minimal real shader pair keeps uniform locations valid on Chromium.
      const program = gl.createProgram();
      for (const [type, source] of [
        [gl.VERTEX_SHADER, 'attribute vec4 p; uniform mat2 m2; uniform mat3 m3; uniform mat4 m4; void main(){ gl_Position=m4*vec4(m3*vec3(m2*p.xy,p.z),p.w); }'],
        [gl.FRAGMENT_SHADER, 'precision mediump float; void main(){gl_FragColor=vec4(1.0);}'],
      ]) {
        const shader = gl.createShader(type);
        gl.shaderSource(shader, source); gl.compileShader(shader);
        check(gl.getShaderParameter(shader, gl.COMPILE_STATUS), `${label}: shader compile`);
        gl.attachShader(program, shader);
      }
      gl.linkProgram(program); gl.useProgram(program);
      check(gl.getProgramParameter(program, gl.LINK_STATUS), `${label}: program linked`);
      const other = make(offscreen).getContext(kind);
      for (const size of [2, 3, 4]) {
        const name = `uniformMatrix${size}fv`, method = gl[name];
        const d = Object.getOwnPropertyDescriptor(proto, name), count = size * size;
        check(d?.value === method && method.name === name && method.length === 3 && d.enumerable && d.configurable && d.writable,
          `${label}: matrix method descriptor`);
        const location = gl.getUniformLocation(program, `m${size}`);
        check(location !== null, `${label}: live matrix location`);
        other[name](location, false, new Float32Array(count));
        check(other.getError() === 0x0502, `${label}: location belongs to its context (INVALID_OPERATION)`);
        throws(() => method.call({}, null, false, []), `${label}: matrix receiver`);
        throws(() => method.call(Object.create(proto), null, false, []), `${label}: forged matrix receiver`);
        throws(() => method.call(gl), `${label}: required arguments`);
        let iterated = 0;
        const values = {[Symbol.iterator](){iterated++; return Array(count).fill(0)[Symbol.iterator]();}};
        throws(() => method.call(gl, {}, false, values), `${label}: invalid location`);
        check(iterated === 0, `${label}: validate location before data conversion`);
        check(method.call(gl, location, false, values) === undefined && gl.getError() === gl.NO_ERROR && iterated === 1,
          `${label}: iterable values`);
        const typed = new Float32Array(count);
        Object.defineProperty(typed, Symbol.iterator, {get(){throw new Error('typed-array iterator must not be consulted');}});
        method.call(gl, location, false, typed);
        check(gl.getError() === gl.NO_ERROR, `${label}: typed-array union branch`);
        method.call(gl, location, false, new Float64Array(count));
        check(gl.getError() === gl.NO_ERROR, `${label}: other typed array uses iterable branch`);
        throws(() => method.call(gl, location, false, null), `${label}: data cannot be null`);
        throws(() => method.call(gl, location, false, [1n]), `${label}: numeric element conversion`);
        const sentinel = new Error('iterator failure');
        let caught;
        try {method.call(gl, location, false, {[Symbol.iterator](){throw sentinel;}});}
        catch (error) {caught = error;}
        check(caught === sentinel, `${label}: preserve iterator exception`);
        method.call(gl, location, false, []);
        check(gl.getError() === gl.INVALID_VALUE, `${label}: empty matrix error`);
        method.call(gl, null, false, new Float32Array(0));
        check(gl.getError() === gl.INVALID_VALUE, `${label}: empty data precedes null-location no-op`);
        method.call(gl, null, true, [1]);
        check(gl.getError() === gl.NO_ERROR, `${label}: converted nonempty null-location call ignored`);
        method.call(gl, location, false, Array(count - 1).fill(0));
        check(gl.getError() === gl.INVALID_VALUE, `${label}: incomplete matrix`);
        method.call(gl, location, true, Array(count).fill(0));
        check(gl.getError() === (kind === 'webgl2' ? gl.NO_ERROR : gl.INVALID_VALUE), `${label}: transpose version policy`);
        method.call(gl, location, false, Array(count + 1).fill(0), 1);
        check(gl.getError() === (kind === 'webgl2' ? gl.NO_ERROR : gl.INVALID_VALUE), `${label}: srcOffset overload`);
        method.call(gl, location, false, Array(count + 2).fill(0), 1, count);
        check(gl.getError() === (kind === 'webgl2' ? gl.NO_ERROR : gl.INVALID_VALUE), `${label}: srcLength overload`);
        method.call(gl, location, false, Array(count).fill(0), count);
        check(gl.getError() === (kind === 'webgl2' ? gl.INVALID_VALUE : gl.NO_ERROR), `${label}: offset past end`);
        const offset = {valueOf(){throw new TypeError('offset conversion');}};
        if (kind === 'webgl2') throws(() => method.call(gl, null, false, typed, offset), `${label}: offset WebIDL`);
        else method.call(gl, null, false, typed, offset);
        check(gl.getError() === gl.NO_ERROR, `${label}: WebIDL exceptions are not GL errors`);
      }
      gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
      check(Array.from(gl.getParameter(gl.VIEWPORT)).join(',') === '0,0,23,17', `${label}: native extents feed viewport`);
      results.push(label);
    }
  }
  return JSON.stringify(results);
})()
