use super::*;

async fn drain_canvas_image_load_event_tasks(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) -> usize {
    let mut count = 0;
    while page
        .run_one_dom_manipulation_task_executor_turn(
            PageDomManipulationTestFamily::ImageLoadEvent,
            loader,
        )
        .await
        .expect("canvas/image DOM-manipulation task should run")
    {
        count += 1;
    }
    count
}

async fn run_one_canvas_image_load_event_task(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
) {
    assert_eq!(
        drain_canvas_image_load_event_tasks(page, loader).await,
        1,
        "the fixture should enqueue exactly one DOM-manipulation task"
    );
}

#[test]
fn create_image_bitmap_matches_chromium_offscreen_canvas_and_close_contract() {
    let mut vm = new_storage_test_vm("https://image-bitmap-surface.test/");

    vm.exec(
        r#"
        (() => {
          const caughtName = callback => {
            try {
              callback();
              return "none";
            } catch (error) {
              return error.name;
            }
          };
          const descriptor = Object.getOwnPropertyDescriptor(window, "createImageBitmap");
          const blank = new OffscreenCanvas(16, 9);
          const drawn = new OffscreenCanvas(16, 9);
          drawn.getContext("2d").fillRect(0, 0, 1, 1);
          globalThis.__imageBitmapProbe = {
            functionShape: [
              typeof createImageBitmap,
              createImageBitmap.name,
              createImageBitmap.length,
              Object.prototype.hasOwnProperty.call(createImageBitmap, "prototype"),
              descriptor.enumerable,
              descriptor.configurable,
              descriptor.writable,
              caughtName(() => new createImageBitmap(drawn)),
              caughtName(() => createImageBitmap()),
            ],
            constructorShape: [
              typeof ImageBitmap,
              ImageBitmap.name,
              ImageBitmap.length,
              Object.prototype.toString.call(ImageBitmap.prototype),
              Object.getPrototypeOf(ImageBitmap.prototype) === Object.prototype,
              caughtName(() => new ImageBitmap()),
            ],
            settled: false,
          };
          Promise.all([
            createImageBitmap(blank).then(
              () => "resolved",
              error => `rejected:${error.name}`,
            ),
            createImageBitmap(drawn).then(bitmap => {
              const before = [
                Object.prototype.toString.call(bitmap),
                bitmap instanceof ImageBitmap,
                Object.getPrototypeOf(bitmap) === ImageBitmap.prototype,
                Object.getOwnPropertyNames(bitmap).length,
                bitmap.width,
                bitmap.height,
                typeof bitmap.close,
              ];
              const closeResult = bitmap.close();
              return [before, typeof closeResult, bitmap.width, bitmap.height];
            }),
          ]).then(([blankOutcome, bitmapOutcome]) => {
            __imageBitmapProbe.blankOutcome = blankOutcome;
            __imageBitmapProbe.bitmapOutcome = bitmapOutcome;
            __imageBitmapProbe.settled = true;
          });
        })()
        "#,
        None,
    )
    .expect("createImageBitmap probe should execute");

    let result = vm
        .eval("JSON.stringify(globalThis.__imageBitmapProbe)")
        .expect("createImageBitmap probe should be readable");
    assert_eq!(
        result,
        r#"{"functionShape":["function","createImageBitmap",1,false,true,true,true,"TypeError","TypeError"],"constructorShape":["function","ImageBitmap",0,"[object ImageBitmap]",true,"TypeError"],"settled":true,"blankOutcome":"rejected:InvalidStateError","bitmapOutcome":[["[object ImageBitmap]",true,true,0,16,9,"function"],"undefined",0,0]}"#
    );
}

#[test]
fn webgl_context_exposes_context_attributes_and_loss_state() {
    let mut vm = new_storage_test_vm("https://webgl-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl');
  if (!gl) {
    return 'missing';
  }
  const attrs = gl.getContextAttributes();
  return [
    typeof gl.getContextAttributes,
    typeof gl.isContextLost,
    typeof attrs,
    attrs && attrs.alpha,
    attrs && attrs.antialias,
    attrs && attrs.powerPreference,
    gl.isContextLost(),
  ].join('|');
})()
"#,
        )
        .expect("webgl compatibility surface should be readable");

    assert_eq!(result, "function|function|object|true|true|default|false");
}

#[test]
fn webgl2_context_matches_the_chromium_interface_and_canvas_acquisition_shape() {
    let mut vm = new_storage_test_vm("https://webgl2-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const caughtName = callback => {
    try {
      callback();
      return 'none';
    } catch (error) {
      return error.name;
    }
  };
  const constructor = WebGL2RenderingContext;
  const descriptor = Object.getOwnPropertyDescriptor(window, 'WebGL2RenderingContext');
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('webgl2');
  return JSON.stringify({
    global: [
      typeof constructor,
      constructor.name,
      constructor.length,
      descriptor.enumerable,
      descriptor.configurable,
      descriptor.writable,
      caughtName(() => constructor()),
      caughtName(() => new constructor()),
    ],
    context: [
      Object.prototype.toString.call(context),
      context.constructor.name,
      context instanceof WebGL2RenderingContext,
      context instanceof WebGLRenderingContext,
      Object.getPrototypeOf(context) === WebGL2RenderingContext.prototype,
      canvas.getContext('webgl2') === context,
      canvas.getContext('webgl') === null,
      document.createElement('canvas').getContext('WebGL2') === null,
    ],
    prototypeMethods: [
      'bufferData',
      'getExtension',
      'getParameter',
      'getShaderPrecisionFormat',
      'getSupportedExtensions',
      'readPixels',
    ].map(name => [
      name,
      typeof WebGL2RenderingContext.prototype[name],
      Object.hasOwn(WebGL2RenderingContext.prototype, name),
    ]),
  });
})()
"#,
        )
        .expect("WebGL2 interface and canvas acquisition surface should evaluate");

    assert_eq!(
        result,
        r#"{"global":["function","WebGL2RenderingContext",0,false,true,true,"TypeError","TypeError"],"context":["[object WebGL2RenderingContext]","WebGL2RenderingContext",true,false,true,true,true,true],"prototypeMethods":[["bufferData","function",true],["getExtension","function",true],["getParameter","function",true],["getShaderPrecisionFormat","function",true],["getSupportedExtensions","function",true],["readPixels","function",true]]}"#
    );
}

#[test]
fn webgl2_exposes_query_and_framebuffer_compatibility_without_a_rendering_backend() {
    let mut vm = new_storage_test_vm("https://webgl2-query-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const gl = document.createElement('canvas').getContext('webgl2');
  const samples = gl.getInternalformatParameter(gl.RENDERBUFFER, gl.RGBA8, gl.SAMPLES);
  const framebuffer = gl.createFramebuffer();
  const renderbuffer = gl.createRenderbuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  gl.bindRenderbuffer(gl.RENDERBUFFER, renderbuffer);
  gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, 4, 4);
  gl.framebufferRenderbuffer(
    gl.FRAMEBUFFER,
    gl.COLOR_ATTACHMENT0,
    gl.RENDERBUFFER,
    renderbuffer,
  );
  const framebufferStatus = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
  gl.deleteRenderbuffer(renderbuffer);
  gl.deleteFramebuffer(framebuffer);
  const loseContext = gl.getExtension('WEBGL_lose_context');

  return JSON.stringify({
    constants: [
      gl.IMPLEMENTATION_COLOR_READ_FORMAT,
      gl.IMPLEMENTATION_COLOR_READ_TYPE,
      gl.RENDERBUFFER,
      gl.SAMPLES,
      gl.RGBA8,
      gl.FRAMEBUFFER,
      gl.COLOR_ATTACHMENT0,
      gl.FRAMEBUFFER_COMPLETE,
    ],
    parameters: [
      gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_FORMAT),
      gl.getParameter(gl.IMPLEMENTATION_COLOR_READ_TYPE),
      gl.getParameter(gl.MAX_3D_TEXTURE_SIZE),
      gl.getParameter(gl.VERSION),
      gl.getParameter(gl.SHADING_LANGUAGE_VERSION),
    ],
    internalFormat: [
      Object.prototype.toString.call(samples),
      Array.from(samples),
    ],
    colors: [gl.drawingBufferColorSpace, gl.unpackColorSpace],
    framebuffer: [
      Object.prototype.toString.call(framebuffer),
      Object.prototype.toString.call(renderbuffer),
      framebufferStatus,
    ],
    extensions: [
      gl.getSupportedExtensions().includes('WEBGL_lose_context'),
      typeof loseContext.loseContext,
      typeof loseContext.restoreContext,
    ],
  });
})()
"#,
        )
        .expect("WebGL2 query and framebuffer compatibility surface should evaluate");

    assert_eq!(
        result,
        r#"{"constants":[35739,35738,36161,32937,32856,36160,36064,36053],"parameters":[6408,5121,2048,"WebGL 2.0 (OpenGL ES 3.0 Chromium)","WebGL GLSL ES 3.00 (OpenGL ES GLSL ES 3.0 Chromium)"],"internalFormat":["[object Int32Array]",[4]],"colors":["srgb","srgb"],"framebuffer":["[object WebGLFramebuffer]","[object WebGLRenderbuffer]",36053],"extensions":[true,"function","function"]}"#
    );
}

#[test]
fn webgl_handle_objects_use_chromium_illegal_constructors_and_prototype_hierarchy() {
    let mut vm = new_storage_test_vm("https://webgl-handle-constructors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const gl = document.createElement('canvas').getContext('webgl');
  const gl2 = document.createElement('canvas').getContext('webgl2');
  const handles = [
    ['WebGLBuffer', gl.createBuffer(), 'WebGLObject'],
    ['WebGLFramebuffer', gl2.createFramebuffer(), 'WebGLObject'],
    ['WebGLProgram', gl.createProgram(), 'WebGLObject'],
    ['WebGLRenderbuffer', gl2.createRenderbuffer(), 'WebGLObject'],
    ['WebGLShader', gl.createShader(gl.VERTEX_SHADER), 'WebGLObject'],
    ['WebGLUniformLocation', gl.getUniformLocation(gl.createProgram(), 'value'), 'Object'],
  ];
  const caughtName = callback => {
    try {
      callback();
      return 'none';
    } catch (error) {
      return error.name;
    }
  };
  return JSON.stringify({
    webglObject: [
      typeof WebGLObject,
      WebGLObject.name,
      WebGLObject.length,
      caughtName(() => WebGLObject()),
      caughtName(() => new WebGLObject()),
      Object.prototype.toString.call(WebGLObject.prototype),
    ],
    handles: handles.map(([name, handle, parent]) => {
      const constructor = globalThis[name];
      return [
        name,
        typeof constructor,
        constructor.name,
        constructor.length,
        caughtName(() => constructor()),
        caughtName(() => new constructor()),
        handle instanceof constructor,
        Object.prototype.toString.call(handle),
        Object.getPrototypeOf(constructor.prototype) === globalThis[parent].prototype,
      ];
    }),
  });
})()
"#,
        )
        .expect("WebGL handle constructor surface should evaluate");

    assert_eq!(
        result,
        r#"{"webglObject":["function","WebGLObject",0,"TypeError","TypeError","[object WebGLObject]"],"handles":[["WebGLBuffer","function","WebGLBuffer",0,"TypeError","TypeError",true,"[object WebGLBuffer]",true],["WebGLFramebuffer","function","WebGLFramebuffer",0,"TypeError","TypeError",true,"[object WebGLFramebuffer]",true],["WebGLProgram","function","WebGLProgram",0,"TypeError","TypeError",true,"[object WebGLProgram]",true],["WebGLRenderbuffer","function","WebGLRenderbuffer",0,"TypeError","TypeError",true,"[object WebGLRenderbuffer]",true],["WebGLShader","function","WebGLShader",0,"TypeError","TypeError",true,"[object WebGLShader]",true],["WebGLUniformLocation","function","WebGLUniformLocation",0,"TypeError","TypeError",true,"[object WebGLUniformLocation]",true]]}"#
    );
}

#[test]
fn webgl_fingerprint_pipeline_methods_are_available() {
    let mut vm = new_storage_test_vm("https://webgl-fingerprint-pipeline.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl');
  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-.2, -.9, 0, .4, -.26, 0]), gl.STATIC_DRAW);
  buffer.itemSize = 3;
  buffer.numItems = 2;

  const program = gl.createProgram();
  const vertex = gl.createShader(gl.VERTEX_SHADER);
  const fragment = gl.createShader(gl.FRAGMENT_SHADER);
  gl.shaderSource(vertex, 'attribute vec2 attrVertex; void main(){ gl_Position=vec4(attrVertex,0,1); }');
  gl.shaderSource(fragment, 'precision mediump float; void main(){ gl_FragColor=vec4(0,0,0,1); }');
  gl.compileShader(vertex);
  gl.compileShader(fragment);
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  gl.useProgram(program);

  const attrib = gl.getAttribLocation(program, 'attrVertex');
  const uniform = gl.getUniformLocation(program, 'uniformOffset');
  gl.enableVertexAttribArray(attrib);
  gl.vertexAttribPointer(attrib, buffer.itemSize, gl.FLOAT, false, 0, 0);
  gl.uniform2f(uniform, 1, 1);
  gl.clearColor(0, 0, 0, 1);
  gl.enable(gl.DEPTH_TEST);
  gl.depthFunc(gl.LEQUAL);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  gl.drawArrays(gl.TRIANGLE_STRIP, 0, buffer.numItems);

  const line = gl.getParameter(gl.ALIASED_LINE_WIDTH_RANGE);
  const viewport = gl.getParameter(gl.MAX_VIEWPORT_DIMS);
  const precision = gl.getShaderPrecisionFormat(gl.VERTEX_SHADER, gl.HIGH_FLOAT);
  const desc = Object.getOwnPropertyDescriptor(WebGLRenderingContext.prototype, 'ARRAY_BUFFER');
  const ownerDataUrl = gl.canvas && gl.canvas.toDataURL();

  return JSON.stringify({
    methods: [
      typeof gl.createBuffer,
      typeof gl.bufferData,
      typeof gl.createProgram,
      typeof gl.createShader,
      typeof gl.getShaderPrecisionFormat
    ],
    handles: [
      typeof buffer,
      typeof program,
      typeof vertex,
      typeof uniform
    ],
    attrib,
    constants: [
      gl.ARRAY_BUFFER,
      gl.STATIC_DRAW,
      gl.VERTEX_SHADER,
      gl.FRAGMENT_SHADER,
      gl.TRIANGLE_STRIP
    ],
    descriptor: [desc.value, desc.enumerable, desc.writable, desc.configurable],
    ownerCanvas: {
      same: gl.canvas === canvas,
      ownEnumerable: Object.getOwnPropertyDescriptor(gl, 'canvas').enumerable,
      dataUrl: typeof ownerDataUrl,
      hashTail: ownerDataUrl.substr(ownerDataUrl.length - 6, 6)
    },
    parameters: {
      line: [line[0], line[1]],
      viewport: [viewport[0], viewport[1]],
      redBits: gl.getParameter(gl.RED_BITS),
      vendor: gl.getParameter(gl.VENDOR),
      version: gl.getParameter(gl.VERSION),
      missing: gl.getParameter(0xffffffff)
    },
    precision: {
      precision: precision.precision,
      rangeMin: precision.rangeMin,
      rangeMax: precision.rangeMax
    },
    shaderStatus: gl.getShaderParameter(vertex, 0),
    programStatus: gl.getProgramParameter(program, 0),
    shaderLog: gl.getShaderInfoLog(vertex),
    error: gl.getError()
  });
})()
"#,
        )
        .expect("WebGL fingerprint pipeline should evaluate");

    assert_eq!(
        result,
        r#"{"methods":["function","function","function","function","function"],"handles":["object","object","object","object"],"attrib":0,"constants":[34962,35044,35633,35632,5],"descriptor":[34962,true,false,false],"ownerCanvas":{"same":true,"ownEnumerable":false,"dataUrl":"string","hashTail":"SuQmCC"},"parameters":{"line":[1,1],"viewport":[300,150],"redBits":8,"vendor":"","version":"WebGL 1.0","missing":null},"precision":{"precision":23,"rangeMin":127,"rangeMax":127},"shaderStatus":true,"programStatus":true,"shaderLog":"","error":0}"#
    );
}

#[test]
fn webgl_shader_pipeline_cleanup_and_uniform_methods_are_available() {
    let mut vm = new_storage_test_vm("https://webgl-shader-pipeline.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl');
  const program = gl.createProgram();
  const vertex = gl.createShader(gl.VERTEX_SHADER);
  const fragment = gl.createShader(gl.FRAGMENT_SHADER);
  gl.shaderSource(vertex, 'attribute vec2 position; void main(){ gl_Position=vec4(position,0,1); }');
  gl.shaderSource(fragment, 'precision mediump float; void main(){ gl_FragColor=vec4(0,0,0,1); }');
  gl.compileShader(vertex);
  gl.compileShader(fragment);
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);

  const returns = [
    gl.deleteShader(vertex),
    gl.deleteShader(fragment)
  ];
  gl.linkProgram(program);

  const uniform = gl.getUniformLocation(program, 'resolution');
  returns.push(gl.uniform2fv(uniform, new Float32Array([300, 150])));
  returns.push(gl.uniform1f(uniform, 1));

  const status = [
    gl.getShaderParameter(vertex, gl.COMPILE_STATUS),
    gl.getProgramParameter(program, gl.LINK_STATUS)
  ];
  returns.push(gl.deleteProgram(program));

  const methodNames = ['deleteShader', 'deleteProgram', 'uniform2fv', 'uniform1f'];
  return JSON.stringify({
    methods: methodNames.map(name => [
      name,
      typeof gl[name],
      gl[name].length,
      Object.hasOwn(WebGLRenderingContext.prototype, name)
    ]),
    constants: [gl.COMPILE_STATUS, gl.LINK_STATUS],
    status,
    returns: returns.map(value => value === undefined)
  });
})()
"#,
        )
        .expect("WebGL shader pipeline cleanup and uniform methods should evaluate");

    assert_eq!(
        result,
        r#"{"methods":[["deleteShader","function",1,true],["deleteProgram","function",1,true],["uniform2fv","function",2,true],["uniform1f","function",2,true]],"constants":[35713,35714],"status":[true,true],"returns":[true,true,true,true,true]}"#
    );
}

#[test]
fn webgl_handle_placeholders_use_declared_private_state() {
    let mut vm = new_storage_test_vm("https://webgl-handle-placeholders.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  const gl = canvas.getContext('webgl');
  const buffer = gl.createBuffer();
  const program = gl.createProgram();
  const shader = gl.createShader(gl.VERTEX_SHADER);
  const uniform = gl.getUniformLocation(program, 'uniformOffset');
  const handles = [buffer, program, shader, uniform];

  const internalNames = value => Object.getOwnPropertyNames(value)
    .filter(name => name.startsWith('__moli'))
    .sort();
  const namesBefore = handles.map(internalNames);
  Object.prototype.__moliWebGlHandleKind = 'prototype-spoof';
  for (const handle of handles) {
    handle.__moliWebGlHandleKind = 'own-spoof';
  }
  const tags = handles.map(handle => Object.prototype.toString.call(handle));
  const namesAfter = handles.map(internalNames);
  delete Object.prototype.__moliWebGlHandleKind;

  return JSON.stringify({ tags, namesBefore, namesAfter });
})()
"#,
        )
        .expect("WebGL handle declarations should evaluate");

    assert_eq!(
        result,
        r#"{"tags":["[object WebGLBuffer]","[object WebGLProgram]","[object WebGLShader]","[object WebGLUniformLocation]"],"namesBefore":[[],[],[],[]],"namesAfter":[["__moliWebGlHandleKind"],["__moliWebGlHandleKind"],["__moliWebGlHandleKind"],["__moliWebGlHandleKind"]]}"#
    );
}

#[test]
fn html_canvas_2d_text_methods_are_available_for_fingerprinting_scripts() {
    let mut vm = new_storage_test_vm("https://canvas-2d-text-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  const ctx = canvas.getContext('2d');
  ctx.font = '16px Arial';
  ctx.fillText('Moli', 2, 3);
  ctx.strokeText('Moli', 2, 3);
  const metrics = ctx.measureText('Moli');
  return [
    typeof ctx.fillText,
    typeof ctx.strokeText,
    typeof ctx.clearRect,
    typeof ctx.measureText,
    metrics.width > 0,
    canvas.toDataURL().startsWith('data:image/png;base64,')
  ].join('|');
})()
"#,
        )
        .expect("canvas 2d text surface should evaluate");

    assert_eq!(result, "function|function|function|function|true|true");
}

#[test]
fn html_canvas_linear_gradient_surface_is_available_for_fingerprinting_scripts() {
    let mut vm = new_storage_test_vm("https://canvas-linear-gradient-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const ctx = document.createElement('canvas').getContext('2d');
  const gradient = ctx.createLinearGradient(0, 0, 10, 10);
  const probe = fn => {
    try {
      fn();
      return 'ok';
    } catch (error) {
      return error.name;
    }
  };
  const addStops = probe(() => {
    gradient.addColorStop(0, '#fff');
    gradient.addColorStop(1, 'rgba(0, 0, 0, 0.5)');
  });
  const badOffset = probe(() => gradient.addColorStop(2, '#fff'));
  const badCoordinate = probe(() => ctx.createLinearGradient(0, 0, Infinity, 10));
  return JSON.stringify({
    createType: typeof ctx.createLinearGradient,
    addType: typeof gradient.addColorStop,
    instance: gradient instanceof CanvasGradient,
    tag: Object.prototype.toString.call(gradient),
    addStops,
    badOffset,
    badCoordinate
  });
})()
"#,
        )
        .expect("canvas linear gradient surface should evaluate");

    assert_eq!(
        result,
        r##"{"createType":"function","addType":"function","instance":true,"tag":"[object CanvasGradient]","addStops":"ok","badOffset":"IndexSizeError","badCoordinate":"NotSupportedError"}"##
    );
}

#[test]
fn html_canvas_to_data_url_tracks_canvas_dimensions() {
    let mut vm = new_storage_test_vm("https://canvas-data-url-dimensions.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const base = document.createElement('canvas');
  const small = document.createElement('canvas');
  small.width = 17;
  small.height = 9;
  const large = document.createElement('canvas');
  large.width = 32;
  large.height = 18;
  return JSON.stringify({
    base: base.toDataURL(),
    small: small.toDataURL(),
    large: large.toDataURL(),
    empty: (() => {
      const zero = document.createElement('canvas');
      zero.width = 0;
      zero.height = 0;
      return zero.toDataURL();
    })()
  });
})()
"#,
        )
        .expect("canvas data urls should evaluate");

    let value: serde_json::Value =
        serde_json::from_str(&result).expect("canvas data urls should be valid json");
    let base = value["base"].as_str().expect("base data url");
    let small = value["small"].as_str().expect("small data url");
    let large = value["large"].as_str().expect("large data url");
    let empty = value["empty"].as_str().expect("empty data url");

    assert_eq!(decode_png_dimensions_from_data_url(base), (300, 150));
    assert_ne!(small, large);
    assert_eq!(decode_png_dimensions_from_data_url(small), (17, 9));
    assert_eq!(decode_png_dimensions_from_data_url(large), (32, 18));
    assert_eq!(empty, "data:,");
}

#[test]
fn html_canvas_fill_rect_mutates_pixels_and_export() {
    let mut vm = new_storage_test_vm("https://canvas-fill-rect.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 4;
  canvas.height = 4;
  const blank = canvas.toDataURL();
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = 'red';
  ctx.fillRect(1, 1, 2, 1);
  const data = Array.from(ctx.getImageData(0, 0, 4, 4).data);
  const hit = data.slice((1 * 4 + 1) * 4, (1 * 4 + 1) * 4 + 4);
  const miss = data.slice(0, 4);
  return JSON.stringify({
    changed: blank !== canvas.toDataURL(),
    hit,
    miss
  });
})()
"#,
        )
        .expect("canvas fillRect should evaluate");

    assert_eq!(
        result,
        r#"{"changed":true,"hit":[255,0,0,255],"miss":[0,0,0,0]}"#
    );
}

#[test]
fn html_canvas_clear_rect_only_clears_target_region() {
    let mut vm = new_storage_test_vm("https://canvas-clear-rect.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 3;
  canvas.height = 2;
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = 'red';
  ctx.fillRect(0, 0, 3, 2);
  ctx.clearRect(1, 0, 1, 2);
  const data = Array.from(ctx.getImageData(0, 0, 3, 2).data);
  return JSON.stringify({
    left: data.slice(0, 4),
    clearedTop: data.slice(4, 8),
    clearedBottom: data.slice(16, 20),
    right: data.slice(8, 12)
  });
})()
"#,
        )
        .expect("canvas clearRect should evaluate");

    assert_eq!(
        result,
        r#"{"left":[255,0,0,255],"clearedTop":[0,0,0,0],"clearedBottom":[0,0,0,0],"right":[255,0,0,255]}"#
    );
}

#[test]
fn canvas_context_rect_path_method_is_available_for_fingerprinting_scripts() {
    let mut vm = new_storage_test_vm("https://canvas-rect-path-method.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  const canvas = document.createElement('canvas');
  const html = canvas.getContext('2d');
  const offscreen = new OffscreenCanvas(2, 2).getContext('2d');
  const before = canvas.toDataURL();
  const htmlCall = probe(() => {
    html.rect(0, 0, 1, 1);
    html.beginPath();
    html.moveTo(0, 0);
    html.lineTo(1, 1);
    html.quadraticCurveTo(0, 1, 1, 1);
    html.bezierCurveTo(0, 0, 1, 1, 2, 2);
    html.arcTo(0, 0, 1, 1, 1);
    html.arc(1, 1, 1, 0, Math.PI, true);
    html.ellipse(1, 1, 1, 1, 0, 0, Math.PI, false);
    html.closePath();
    html.fill('evenodd');
    html.stroke();
    html.clip();
  });
  const offscreenCall = probe(() => {
    offscreen.rect("0", { valueOf() { return 0; } }, 1, 1);
    offscreen.beginPath();
    offscreen.arc(1, 1, 1, 0, Math.PI, true);
    offscreen.closePath();
    offscreen.fill();
  });
  return JSON.stringify({
    htmlType: typeof html.rect,
    offscreenType: typeof offscreen.rect,
    arcType: typeof html.arc,
    pointType: typeof html.isPointInPath,
    htmlCall,
    offscreenCall,
    pointInPath: html.isPointInPath(5, 5, 'evenodd'),
    pointInStroke: html.isPointInStroke(5, 5),
    changed: before !== canvas.toDataURL()
  });
})()
"#,
        )
        .expect("canvas rect path surface should evaluate");

    assert_eq!(
        result,
        r#"{"htmlType":"function","offscreenType":"function","arcType":"function","pointType":"function","htmlCall":"ok","offscreenCall":"ok","pointInPath":false,"pointInStroke":false,"changed":false}"#
    );
}

#[test]
fn html_canvas_fill_style_getter_canonicalizes_supported_values() {
    let mut vm = new_storage_test_vm("https://canvas-fill-style-canonicalization.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const ctx = document.createElement('canvas').getContext('2d');
  const values = [];
  ctx.fillStyle = 'rebeccapurple';
  values.push(ctx.fillStyle);
  ctx.fillStyle = '#abc';
  values.push(ctx.fillStyle);
  ctx.fillStyle = '#ff000080';
  values.push(ctx.fillStyle);
  return JSON.stringify(values);
})()
"#,
        )
        .expect("canvas fillStyle canonicalization should evaluate");

    assert_eq!(result, r##"["#663399","#aabbcc","rgba(255, 0, 0, 0.50)"]"##);
}

#[test]
fn canvas_context_constructors_preserve_declared_defaults() {
    let mut vm = new_storage_test_vm("https://canvas-context-defaults.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.createElement('canvas').getContext('2d');
  const offscreen = new OffscreenCanvas(2, 2).getContext('2d');
  const snapshot = ctx => ({
    fillStyle: ctx.fillStyle,
    font: ctx.font,
    imageSmoothingEnabled: ctx.imageSmoothingEnabled,
    imageSmoothingQuality: ctx.imageSmoothingQuality,
    globalAlpha: ctx.globalAlpha,
    globalCompositeOperation: ctx.globalCompositeOperation,
    ownPublic: [
      'fillStyle',
      'font',
      'imageSmoothingEnabled',
      'imageSmoothingQuality',
      'globalAlpha',
      'globalCompositeOperation'
    ].some(name => Object.prototype.hasOwnProperty.call(ctx, name))
  });
  return JSON.stringify({
    html: snapshot(html),
    offscreen: snapshot(offscreen),
    htmlInstance: html instanceof CanvasRenderingContext2D,
    offscreenInstance: offscreen instanceof OffscreenCanvasRenderingContext2D
  });
})()
"#,
        )
        .expect("canvas context default declarations should evaluate");

    assert_eq!(
        result,
        r##"{"html":{"fillStyle":"#000000","font":"10px sans-serif","imageSmoothingEnabled":true,"imageSmoothingQuality":"low","globalAlpha":1,"globalCompositeOperation":"source-over","ownPublic":false},"offscreen":{"fillStyle":"#000000","font":"10px sans-serif","imageSmoothingEnabled":true,"imageSmoothingQuality":"low","globalAlpha":1,"globalCompositeOperation":"source-over","ownPublic":false},"htmlInstance":true,"offscreenInstance":true}"##
    );
}

#[test]
fn canvas_line_dash_uses_webidl_sequence_conversion_and_private_state() {
    let mut vm = new_storage_test_vm("https://canvas-line-dash-sequence.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const ctx = document.createElement('canvas').getContext('2d');
  const initial = ctx.getLineDash();
  ctx.setLineDash([1, 2]);
  const array = ctx.getLineDash();

  function* generated() {
    yield 4;
    yield 5;
  }
  ctx.setLineDash(generated());
  const generator = ctx.getLineDash();

  let iteratorGets = 0;
  const overridden = [10, 11];
  Object.defineProperty(overridden, Symbol.iterator, {
    get() {
      iteratorGets += 1;
      return function* () {
        yield 6;
        yield 7;
      };
    }
  });
  ctx.setLineDash(overridden);
  const customIterator = ctx.getLineDash();

  ctx.setLineDash([3]);
  const odd = ctx.getLineDash();
  const copy = ctx.getLineDash();
  copy[0] = 99;
  ctx.setLineDash([-1, 2]);

  return JSON.stringify({
    initial,
    array,
    generator,
    customIterator,
    iteratorGets,
    odd,
    independentCopy: ctx.getLineDash(),
    ownVisibleState: Object.getOwnPropertyNames(ctx)
      .some(name => name === '__moliCanvasContextLineDash')
  });
})()
"#,
        )
        .expect("Canvas line dash sequence probe should evaluate");

    assert_eq!(
        result,
        r#"{"initial":[],"array":[1,2],"generator":[4,5],"customIterator":[6,7],"iteratorGets":1,"odd":[3,3],"independentCopy":[3,3],"ownVisibleState":false}"#
    );
}

#[test]
fn canvas_context_state_uses_private_slots_for_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://canvas-context-private-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 2;
  canvas.height = 1;
  const ctx = canvas.getContext('2d');
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliCanvasContext'))
    .sort()
    .join(',');
  const snapshot = () => [
    ctx.fillStyle,
    ctx.font,
    ctx.imageSmoothingEnabled,
    ctx.imageSmoothingQuality,
    ctx.globalAlpha,
    ctx.globalCompositeOperation
  ].join('|');

  const initialOwnSlots = internalNames(ctx);
  const defaults = snapshot();
  CanvasRenderingContext2D.prototype.__moliCanvasContextFillStyle = '#00ff00';
  CanvasRenderingContext2D.prototype.__moliCanvasContextFont = '1px Arial';
  CanvasRenderingContext2D.prototype.__moliCanvasContextImageSmoothingEnabled = true;
  CanvasRenderingContext2D.prototype.__moliCanvasContextImageSmoothingQuality = 'low';
  CanvasRenderingContext2D.prototype.__moliCanvasContextGlobalAlpha = 0;
  CanvasRenderingContext2D.prototype.__moliCanvasContextGlobalCompositeOperation = 'copy';

  ctx.fillStyle = '#ff0000';
  ctx.font = '20px Arial';
  ctx.imageSmoothingEnabled = false;
  ctx.imageSmoothingQuality = 'high';
  ctx.globalAlpha = 0.5;
  ctx.globalCompositeOperation = 'source-over';
  const beforeSpoof = snapshot();
  const ownSlotsAfterSetter = internalNames(ctx);
  ctx.fillRect(0, 0, 1, 1);
  const beforeSpoofPixel = Array.from(ctx.getImageData(0, 0, 1, 1).data).join(',');
  const widthBeforeSpoof = ctx.measureText('Hi').width;

  ctx.__moliCanvasContextFillStyle = '#0000ff';
  ctx.__moliCanvasContextFont = '1px Arial';
  ctx.__moliCanvasContextImageSmoothingEnabled = true;
  ctx.__moliCanvasContextImageSmoothingQuality = 'low';
  ctx.__moliCanvasContextGlobalAlpha = 0;
  ctx.__moliCanvasContextGlobalCompositeOperation = 'copy';

  const afterSpoof = snapshot();
  ctx.fillRect(1, 0, 1, 1);
  const afterSpoofPixel = Array.from(ctx.getImageData(1, 0, 1, 1).data).join(',');
  const widthSame = ctx.measureText('Hi').width === widthBeforeSpoof;
  const fillStyleGetter = Object.getOwnPropertyDescriptor(
    CanvasRenderingContext2D.prototype,
    'fillStyle'
  ).get;
  let fakeFillStyleError = null;
  try {
    fillStyleGetter.call({});
  } catch (error) {
    fakeFillStyleError = error.name;
  }

  return JSON.stringify({
    initialOwnSlots,
    defaults,
    beforeSpoof,
    ownSlotsAfterSetter,
    beforeSpoofPixel,
    afterSpoof,
    afterSpoofPixel,
    ownSlotsAfterSpoof: internalNames(ctx),
    widthSame,
    fakeFillStyleError,
    instance: ctx instanceof CanvasRenderingContext2D
  });
})()
"#,
        )
        .expect("canvas context state should resist reflection and spoofing");

    assert_eq!(
        result,
        r##"{"initialOwnSlots":"","defaults":"#000000|10px sans-serif|true|low|1|source-over","beforeSpoof":"#ff0000|20px Arial|false|high|0.5|source-over","ownSlotsAfterSetter":"","beforeSpoofPixel":"255,0,0,255","afterSpoof":"#ff0000|20px Arial|false|high|0.5|source-over","afterSpoofPixel":"255,0,0,255","ownSlotsAfterSpoof":"__moliCanvasContextFillStyle,__moliCanvasContextFont,__moliCanvasContextGlobalAlpha,__moliCanvasContextGlobalCompositeOperation,__moliCanvasContextImageSmoothingEnabled,__moliCanvasContextImageSmoothingQuality","widthSame":true,"fakeFillStyleError":"TypeError","instance":true}"##
    );
}

#[test]
fn canvas_owner_and_backing_store_use_private_slots_for_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://canvas-owner-backing-store-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 2;
  canvas.height = 1;
  const other = document.createElement('canvas');
  other.width = 2;
  other.height = 1;
  const ctx = canvas.getContext('2d');
  const otherCtx = other.getContext('2d');
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => (
      name === '__moliCanvasOwner' ||
      name === '__moliCanvasBackingStore'
    ))
    .sort()
    .join(',');

  const initialContextSlots = internalNames(ctx);
  const initialCanvasSlots = internalNames(canvas);
  const canvasDescriptor = Object.getOwnPropertyDescriptor(ctx, 'canvas');

  CanvasRenderingContext2D.prototype.__moliCanvasOwner = other;
  HTMLCanvasElement.prototype.__moliCanvasBackingStore =
    new Uint8ClampedArray([1, 2, 3, 4]);
  ctx.fillStyle = '#ff0000';
  ctx.fillRect(0, 0, 1, 1);
  const afterPrototypeSpoof = [
    Array.from(ctx.getImageData(0, 0, 1, 1).data).join(','),
    Array.from(otherCtx.getImageData(0, 0, 1, 1).data).join(','),
    internalNames(ctx),
    internalNames(canvas)
  ].join('|');

  ctx.__moliCanvasOwner = other;
  canvas.__moliCanvasBackingStore =
    new Uint8ClampedArray([0, 0, 255, 255, 0, 0, 255, 255]);
  ctx.fillStyle = '#00ff00';
  ctx.fillRect(1, 0, 1, 1);
  const afterOwnSpoof = [
    Array.from(ctx.getImageData(0, 0, 2, 1).data).join(','),
    Array.from(otherCtx.getImageData(0, 0, 2, 1).data).join(','),
    internalNames(ctx),
    internalNames(canvas)
  ].join('|');

  return JSON.stringify({
    initialContextSlots,
    initialCanvasSlots,
    canvasProperty: [
      ctx.canvas === canvas,
      canvasDescriptor.enumerable
    ].join(':'),
    afterPrototypeSpoof,
    afterOwnSpoof
  });
})()
"#,
        )
        .expect("canvas owner and backing store should resist reflection and spoofing");

    assert_eq!(
        result,
        r#"{"initialContextSlots":"","initialCanvasSlots":"","canvasProperty":"true:false","afterPrototypeSpoof":"255,0,0,255|0,0,0,0||","afterOwnSpoof":"255,0,0,255,0,255,0,255|0,0,0,0,0,0,0,0|__moliCanvasOwner|__moliCanvasBackingStore"}"#
    );
}

#[test]
fn canvas_context_string_setters_parse_webidl_domstring() {
    let mut vm = new_storage_test_vm("https://canvas-context-string-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };

  const ctx = document.createElement('canvas').getContext('2d');
  ctx.font = null;
  const fontNull = ctx.font;
  ctx.font = undefined;
  const fontUndefined = ctx.font;
  ctx.font = { toString() { return '16px serif'; } };
  const fontObject = ctx.font;
  const fontSymbol = probe(() => { ctx.font = Symbol('font'); });
  const fontAfterSymbol = ctx.font;
  const fontThrow = probe(() => {
    ctx.font = { toString() { throw new RangeError('font'); } };
  });
  const fontAfterThrow = ctx.font;

  ctx.fillStyle = { toString() { return 'red'; } };
  const fillObject = ctx.fillStyle;
  const fillSymbol = probe(() => { ctx.fillStyle = Symbol('fill'); });
  const fillAfterSymbol = ctx.fillStyle;
  const fillThrow = probe(() => {
    ctx.fillStyle = { toString() { throw new RangeError('fill'); } };
  });
  const fillAfterThrow = ctx.fillStyle;

  const offscreen = new OffscreenCanvas(1, 1).getContext('2d');
  offscreen.font = { toString() { return '20px sans-serif'; } };
  offscreen.fillStyle = { toString() { return '#00ff00'; } };

  return JSON.stringify({
    fontNull,
    fontUndefined,
    fontObject,
    fontSymbol,
    fontAfterSymbol,
    fontThrow,
    fontAfterThrow,
    fillObject,
    fillSymbol,
    fillAfterSymbol,
    fillThrow,
    fillAfterThrow,
    offscreenFont: offscreen.font,
    offscreenFill: offscreen.fillStyle
  });
})()
"#,
        )
        .expect("canvas context string setter WebIDL boundary should evaluate");

    assert_eq!(
        result,
        r##"{"fontNull":"null","fontUndefined":"undefined","fontObject":"16px serif","fontSymbol":"TypeError","fontAfterSymbol":"16px serif","fontThrow":"RangeError","fontAfterThrow":"16px serif","fillObject":"#ff0000","fillSymbol":"TypeError","fillAfterSymbol":"#ff0000","fillThrow":"RangeError","fillAfterThrow":"#ff0000","offscreenFont":"20px sans-serif","offscreenFill":"#00ff00"}"##
    );
}

#[test]
fn canvas_context_text_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://canvas-context-text-methods-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value && typeof value === 'object' && 'width' in value ? 'ok' : String(value);
    } catch (error) {
      return error && error.name;
    }
  };

  const canvas = document.createElement('canvas');
  canvas.width = 48;
  canvas.height = 24;
  const ctx = canvas.getContext('2d');
  ctx.font = '12px Arial';
  ctx.fillStyle = 'red';
  const blank = canvas.toDataURL();

  const textObject = { toString() { return 'Hi'; } };
  const fillObject = probe(() => ctx.fillText(textObject, { valueOf() { return 2; } }, 12));
  const afterFillObject = canvas.toDataURL() !== blank;
  const afterGoodFill = canvas.toDataURL();
  const fillMissing = probe(() => ctx.fillText());
  const fillSymbol = probe(() => ctx.fillText(Symbol('text'), 2, 12));
  const fillBadNumber = probe(() => ctx.fillText('Hi', Symbol('x'), 12));
  const afterBadFill = canvas.toDataURL() === afterGoodFill;

  const strokeObject = probe(() => ctx.strokeText({ toString() { return 'Ok'; } }, 4, { valueOf() { return 12; } }, 100));
  const afterGoodStroke = canvas.toDataURL();
  const strokeThrow = probe(() => ctx.strokeText({ toString() { throw new RangeError('stroke'); } }, 4, 12));
  const afterBadStroke = canvas.toDataURL() === afterGoodStroke;

  const measureObject = probe(() => ctx.measureText({ toString() { return 'Hi'; } }));
  const measured = ctx.measureText('Hi').width > 0;
  const measureMissing = probe(() => ctx.measureText());
  const measureSymbol = probe(() => ctx.measureText(Symbol('measure')));

  const offscreen = new OffscreenCanvas(48, 24).getContext('2d');
  offscreen.font = '12px Arial';
  const offscreenMeasure = offscreen.measureText({ toString() { return 'Hi'; } }).width > 0;
  const offscreenFill = probe(() => offscreen.fillText({ toString() { return 'Hi'; } }, 2, 12));

  return JSON.stringify({
    fillObject,
    afterFillObject,
    fillMissing,
    fillSymbol,
    fillBadNumber,
    afterBadFill,
    strokeObject,
    strokeThrow,
    afterBadStroke,
    measureObject,
    measured,
    measureMissing,
    measureSymbol,
    offscreenMeasure,
    offscreenFill
  });
})()
"#,
        )
        .expect("canvas context text method WebIDL boundary should evaluate");

    assert_eq!(
        result,
        r#"{"fillObject":"undefined","afterFillObject":true,"fillMissing":"TypeError","fillSymbol":"TypeError","fillBadNumber":"TypeError","afterBadFill":true,"strokeObject":"undefined","strokeThrow":"RangeError","afterBadStroke":true,"measureObject":"ok","measured":true,"measureMissing":"TypeError","measureSymbol":"TypeError","offscreenMeasure":true,"offscreenFill":"undefined"}"#
    );
}

#[test]
fn html_canvas_fill_text_changes_export_and_measure_text_scales_with_font() {
    let mut vm = new_storage_test_vm("https://canvas-fill-text.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 96;
  canvas.height = 48;
  const ctx = canvas.getContext('2d');
  const blank = canvas.toDataURL();
  ctx.font = '16px Arial';
  const small = ctx.measureText('Hi').width;
  ctx.fillText('Hi', 2, 18);
  ctx.font = '32px Arial';
  const large = ctx.measureText('Hi').width;
  return JSON.stringify({
    changed: blank !== canvas.toDataURL(),
    small,
    large
  });
})()
"#,
        )
        .expect("canvas fillText should evaluate");

    let value: serde_json::Value =
        serde_json::from_str(&result).expect("canvas fillText result should be valid json");
    assert_eq!(value["changed"], true);
    assert!(value["small"].as_f64().unwrap_or_default() > 0.0);
    assert!(
        value["large"].as_f64().unwrap_or_default() > value["small"].as_f64().unwrap_or_default()
    );
}

#[test]
fn html_canvas_put_and_get_image_data_round_trip_pixels() {
    let mut vm = new_storage_test_vm("https://canvas-image-data-round-trip.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 2;
  canvas.height = 2;
  const blank = canvas.toDataURL();
  const ctx = canvas.getContext('2d');
  const data = new Uint8ClampedArray([
    255, 0, 0, 255,
    0, 255, 0, 255,
    0, 0, 255, 255,
    255, 255, 0, 255
  ]);
  ctx.putImageData(new ImageData(data, 2, 2), 0, 0);
  return JSON.stringify({
    changed: blank !== canvas.toDataURL(),
    pixels: Array.from(ctx.getImageData(0, 0, 2, 2).data)
  });
})()
"#,
        )
        .expect("canvas imageData round trip should evaluate");

    assert_eq!(
        result,
        r#"{"changed":true,"pixels":[255,0,0,255,0,255,0,255,0,0,255,255,255,255,0,255]}"#
    );
}

#[test]
fn html_image_data_private_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://canvas-image-data-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const image = new ImageData(new Uint8ClampedArray([
    1, 2, 3, 4,
    5, 6, 7, 8
  ]), 2, 1, { colorSpace: 'display-p3' });
  const initialOwnSlots = Object.getOwnPropertyNames(image)
    .filter(name => name.startsWith('__moliImageData'))
    .sort();

  ImageData.prototype.__moliImageDataWidth = 1;
  ImageData.prototype.__moliImageDataHeight = 1;
  ImageData.prototype.__moliImageDataColorSpace = 'srgb';
  ImageData.prototype.__moliImageDataPixelFormat = 'rgba-unorm8';
  ImageData.prototype.__moliImageDataData = new Uint8ClampedArray([9, 9, 9, 9]);
  ImageData.prototype.__moliImageDataBrand = true;
  Object.defineProperties(image, {
    __moliImageDataWidth: { value: 99, configurable: true },
    __moliImageDataHeight: { value: 99, configurable: true },
    __moliImageDataColorSpace: { value: 'srgb', configurable: true },
    __moliImageDataPixelFormat: { value: 'bad-format', configurable: true },
    __moliImageDataData: {
      value: new Uint8ClampedArray([9, 9, 9, 9]),
      configurable: true
    },
    __moliImageDataBrand: { value: true, configurable: true }
  });
  const pollutedOwnSlots = Object.getOwnPropertyNames(image)
    .filter(name => name.startsWith('__moliImageData'))
    .sort();
  const fake = Object.create(ImageData.prototype);
  Object.assign(fake, {
    __moliImageDataWidth: 1,
    __moliImageDataHeight: 1,
    __moliImageDataColorSpace: 'srgb',
    __moliImageDataPixelFormat: 'rgba-unorm8',
    __moliImageDataData: new Uint8ClampedArray([9, 9, 9, 9]),
    __moliImageDataBrand: true
  });
  const fakeGetterResults = [
    'width',
    'height',
    'data',
    'colorSpace',
    'pixelFormat'
  ].map(name => probe(() =>
    Object.getOwnPropertyDescriptor(ImageData.prototype, name).get.call(fake)
  ));
  function descriptorSummary(name) {
    const descriptor = Object.getOwnPropertyDescriptor(ImageData.prototype, name);
    return [
      name,
      typeof descriptor.get,
      descriptor.get ? descriptor.get.name : "",
      descriptor.get ? descriptor.get.length : -1,
      typeof descriptor.set,
      descriptor.set ? descriptor.set.name : "",
      descriptor.set ? descriptor.set.length : -1,
      descriptor.enumerable,
      descriptor.configurable,
      Object.hasOwn(image, name)
    ].join(':');
  }

  const canvas = document.createElement('canvas');
  canvas.width = 1;
  canvas.height = 1;
  const ctx = canvas.getContext('2d');

  return JSON.stringify({
    initialOwnSlots,
    pollutedOwnSlots,
    real: [
      image.width,
      image.height,
      image.colorSpace,
      image.pixelFormat,
      Array.from(image.data).join(',')
    ].join('|'),
    descriptors: [
      descriptorSummary('width'),
      descriptorSummary('height'),
      descriptorSummary('data'),
      descriptorSummary('colorSpace'),
      descriptorSummary('pixelFormat')
    ],
    fakeGetterResults,
    fakePut: probe(() => ctx.putImageData(fake, 0, 0)),
    realPut: probe(() => ctx.putImageData(image, 0, 0))
  });
})()
"#,
        )
        .expect("ImageData private slots should evaluate");

    assert_eq!(
        result,
        r#"{"initialOwnSlots":[],"pollutedOwnSlots":["__moliImageDataBrand","__moliImageDataColorSpace","__moliImageDataData","__moliImageDataHeight","__moliImageDataPixelFormat","__moliImageDataWidth"],"real":"2|1|display-p3|rgba-unorm8|1,2,3,4,5,6,7,8","descriptors":["width:function:get width:0:undefined::-1:true:true:false","height:function:get height:0:undefined::-1:true:true:false","data:function:get data:0:undefined::-1:true:true:false","colorSpace:function:get colorSpace:0:undefined::-1:true:true:false","pixelFormat:function:get pixelFormat:0:undefined::-1:true:true:false"],"fakeGetterResults":["TypeError","TypeError","TypeError","TypeError","TypeError"],"fakePut":"TypeError","realPut":"ok"}"#
    );
}

#[test]
fn html_structured_clone_image_data_preserves_interface_and_pixels() {
    let mut vm = new_storage_test_vm("https://canvas-image-data-structured-clone.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = new ImageData(new Uint8ClampedArray([1, 2, 3, 4]), 1, 1);
  const clone = structuredClone(source);
  clone.data[0] = 128;
  return [
    '' + clone,
    clone.width,
    clone.height,
    clone.colorSpace,
    Array.from(source.data).join(','),
    Array.from(clone.data).join(',')
  ].join('|');
})()
"#,
        )
        .expect("ImageData structuredClone should evaluate");

    assert_eq!(result, "[object ImageData]|1|1|srgb|1,2,3,4|128,2,3,4");
}

#[test]
fn html_canvas_put_image_data_honors_dirty_rect() {
    let mut vm = new_storage_test_vm("https://canvas-image-data-dirty-rect.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 2;
  canvas.height = 2;
  const ctx = canvas.getContext('2d');
  const data = new Uint8ClampedArray([
    255, 0, 0, 255,
    0, 255, 0, 255,
    0, 0, 255, 255,
    255, 255, 0, 255
  ]);
  ctx.putImageData(new ImageData(data, 2, 2), 0, 0, 1, 0, 1, 2);
  return JSON.stringify(Array.from(ctx.getImageData(0, 0, 2, 2).data));
})()
"#,
        )
        .expect("canvas putImageData dirty rect should evaluate");

    assert_eq!(result, "[0,255,0,255,0,0,0,0,255,255,0,255,0,0,0,0]");
}

#[test]
fn html_canvas_image_data_methods_parse_webidl_enforce_range_long() {
    let mut vm = new_storage_test_vm("https://canvas-image-data-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return callback();
    } catch (error) {
      return error && error.name;
    }
  };

  const canvas = document.createElement('canvas');
  canvas.width = 3;
  canvas.height = 3;
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = 'red';
  ctx.fillRect(0, 0, 3, 3);

  const created = probe(() => {
    const value = ctx.createImageData({ valueOf() { return -2.8; } }, "3.9");
    return `${value.width}:${value.height}`;
  });
  const createZero = probe(() => ctx.createImageData(0, 1));
  const createSymbol = probe(() => ctx.createImageData(Symbol('width'), 1));
  const getNegative = probe(() => ctx.getImageData(2, 2, -2, -2));
  const getMissing = probe(() => ctx.getImageData(0, 0, 1));
  const getNan = probe(() => ctx.getImageData(NaN, 0, 1, 1));
  const getZero = probe(() => ctx.getImageData(0, 0, 0, 1));
  const constructorOversized = probe(() => new ImageData(23171, 23171));
  const getOversized = probe(() => ctx.getImageData(0, 0, 23171, 23171));

  const putCanvas = document.createElement('canvas');
  putCanvas.width = 3;
  putCanvas.height = 1;
  const putCtx = putCanvas.getContext('2d');
  const source = new ImageData(new Uint8ClampedArray([
    255, 0, 0, 255,
    0, 255, 0, 255,
    0, 0, 255, 255
  ]), 3, 1);
  putCtx.putImageData(source, { valueOf() { return 1.8; } }, "0.9");
  const putCoerced = Array.from(putCtx.getImageData(0, 0, 3, 1).data).join(',');
  const beforeBadPut = putCoerced;
  const putSymbol = probe(() => putCtx.putImageData(source, Symbol('dx'), 0));
  const afterBadPut = Array.from(putCtx.getImageData(0, 0, 3, 1).data).join(',') === beforeBadPut;

  const dirtyCanvas = document.createElement('canvas');
  dirtyCanvas.width = 2;
  dirtyCanvas.height = 1;
  const dirtyCtx = dirtyCanvas.getContext('2d');
  dirtyCtx.putImageData(new ImageData(new Uint8ClampedArray([
    255, 0, 0, 255,
    0, 255, 0, 255
  ]), 2, 1), 0, 0, 1, 0, -1, 1);
  const dirtyNegative = Array.from(dirtyCtx.getImageData(0, 0, 2, 1).data).join(',');
  const putPlainObject = probe(() => putCtx.putImageData({}, 0, 0));

  return JSON.stringify({
    created,
    createZero,
    createSymbol,
    getNegative,
    getMissing,
    getNan,
    getZero,
    constructorOversized,
    getOversized,
    putCoerced,
    putSymbol,
    afterBadPut,
    dirtyNegative,
    putPlainObject
  });
})()
"#,
        )
        .expect("canvas ImageData WebIDL boundaries should evaluate");

    assert_eq!(
        result,
        r#"{"created":"2:3","createZero":"IndexSizeError","createSymbol":"TypeError","getNegative":"IndexSizeError","getMissing":"TypeError","getNan":"TypeError","getZero":"IndexSizeError","constructorOversized":"IndexSizeError","getOversized":"IndexSizeError","putCoerced":"0,0,0,0,255,0,0,255,0,255,0,255","putSymbol":"TypeError","afterBadPut":true,"dirtyNegative":"255,0,0,255,0,0,0,0","putPlainObject":"TypeError"}"#
    );
}

#[test]
fn html_canvas_get_image_data_out_of_bounds_returns_transparent_black() {
    let mut vm = new_storage_test_vm("https://canvas-image-data-out-of-bounds.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 1;
  canvas.height = 1;
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = 'red';
  ctx.fillRect(0, 0, 1, 1);
  return JSON.stringify(Array.from(ctx.getImageData(-1, -1, 2, 2).data));
})()
"#,
        )
        .expect("canvas getImageData out-of-bounds should evaluate");

    assert_eq!(result, "[0,0,0,0,0,0,0,0,0,0,0,0,255,0,0,255]");
}

#[test]
fn html_canvas_dimension_reset_clears_backing_store() {
    let mut vm = new_storage_test_vm("https://canvas-dimension-reset.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 4;
  canvas.height = 4;
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = 'red';
  ctx.fillRect(0, 0, 4, 4);
  canvas.width = 4;
  return JSON.stringify(Array.from(ctx.getImageData(0, 0, 1, 1).data));
})()
"#,
        )
        .expect("canvas dimension reset should evaluate");

    assert_eq!(result, "[0,0,0,0]");
}

#[test]
fn html_canvas_dimension_setters_parse_webidl_unsigned_long() {
    let mut vm = new_storage_test_vm("https://canvas-dimension-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  const canvas = document.createElement('canvas');
  canvas.width = null;
  const widthNull = `${canvas.width}:${canvas.getAttribute('width')}`;
  canvas.height = undefined;
  const heightUndefined = `${canvas.height}:${canvas.getAttribute('height')}`;
  canvas.width = { valueOf() { return 7.9; } };
  const widthObject = `${canvas.width}:${canvas.getAttribute('width')}`;
  canvas.height = { toString() { return '5.9'; } };
  const heightObject = `${canvas.height}:${canvas.getAttribute('height')}`;

  const ctx = canvas.getContext('2d');
  ctx.fillStyle = 'red';
  ctx.fillRect(0, 0, 1, 1);
  const beforeSymbol = canvas.toDataURL();
  const widthSymbol = probe(() => { canvas.width = Symbol('width'); });
  const afterWidthSymbol = `${canvas.width}:${canvas.toDataURL() === beforeSymbol}`;
  const heightThrow = probe(() => {
    canvas.height = { valueOf() { throw new RangeError('height'); } };
  });
  const afterHeightThrow = `${canvas.height}:${canvas.toDataURL() === beforeSymbol}`;

  return JSON.stringify({
    widthNull,
    heightUndefined,
    widthObject,
    heightObject,
    widthSymbol,
    afterWidthSymbol,
    heightThrow,
    afterHeightThrow
  });
})()
"#,
        )
        .expect("canvas dimension WebIDL boundary should evaluate");

    assert_eq!(
        result,
        r#"{"widthNull":"0:0","heightUndefined":"0:0","widthObject":"7:7","heightObject":"5:5","widthSymbol":"TypeError","afterWidthSymbol":"7:true","heightThrow":"RangeError","afterHeightThrow":"5:true"}"#
    );
}

#[test]
fn offscreen_canvas_2d_pixels_round_trip_and_reset() {
    let mut vm = new_storage_test_vm("https://offscreen-canvas-2d-round-trip.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = new OffscreenCanvas(2, 2);
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = '#00ff00';
  ctx.fillRect(0, 0, 2, 2);
  const before = Array.from(ctx.getImageData(0, 0, 1, 1).data);
  canvas.width = 2;
  const after = Array.from(ctx.getImageData(0, 0, 1, 1).data);
  return JSON.stringify({
    shape: [
      canvas instanceof OffscreenCanvas,
      typeof ctx.fillRect,
      typeof ctx.getImageData
    ],
    before,
    after
  });
})()
"#,
        )
        .expect("offscreen canvas 2d surface should evaluate");

    assert_eq!(
        result,
        r#"{"shape":[true,"function","function"],"before":[0,255,0,255],"after":[0,0,0,0]}"#
    );
}

#[test]
fn offscreen_canvas_dimensions_use_private_slots_for_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://offscreen-canvas-private-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const canvas = new OffscreenCanvas(2, 2);
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith('__moliOffscreenCanvas'))
    .sort()
    .join(',');

  const initialOwnSlots = internalNames(canvas);
  OffscreenCanvas.prototype.__moliOffscreenCanvasWidth = -100;
  OffscreenCanvas.prototype.__moliOffscreenCanvasHeight = -100;
  OffscreenCanvas.prototype.__moliOffscreenCanvasBrand = true;
  const before = `${canvas.width}:${canvas.height}`;
  const descriptorShape = name => {
    const descriptor = Object.getOwnPropertyDescriptor(OffscreenCanvas.prototype, name);
    return [
      typeof descriptor.get,
      descriptor.get.name,
      descriptor.get.length,
      typeof descriptor.set,
      descriptor.set.name,
      descriptor.set.length,
      descriptor.enumerable,
      descriptor.configurable,
      Object.prototype.hasOwnProperty.call(canvas, name)
    ].join(':');
  };
  const widthDescriptor = descriptorShape('width');
  const heightDescriptor = descriptorShape('height');

  canvas.width = 3;
  canvas.height = 4;
  const afterSetter = `${canvas.width}:${canvas.height}`;
  const ownSlotsAfterSetter = internalNames(canvas);

  const ctx = canvas.getContext('2d');
  ctx.fillStyle = '#00ff00';
  ctx.fillRect(0, 0, 3, 4);
  const beforeSpoofPixel = Array.from(ctx.getImageData(2, 3, 1, 1).data).join(',');

  canvas.__moliOffscreenCanvasWidth = -200;
  canvas.__moliOffscreenCanvasHeight = -200;
  canvas.__moliOffscreenCanvasBrand = true;
  const afterSpoof = [
    canvas.width,
    canvas.height,
    Array.from(ctx.getImageData(2, 3, 1, 1).data).join(','),
    internalNames(canvas)
  ].join(':');

  const widthDescriptorObject = Object.getOwnPropertyDescriptor(OffscreenCanvas.prototype, 'width');
  const fake = Object.assign(Object.create(OffscreenCanvas.prototype), {
    __moliOffscreenCanvasWidth: 1,
    __moliOffscreenCanvasHeight: 1,
    __moliOffscreenCanvasBrand: true
  });
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return error.constructor.name;
    }
  };
  const fakeResults = [
    probe(() => widthDescriptorObject.get.call(fake)),
    probe(() => widthDescriptorObject.set.call(fake, 5)),
    probe(() => OffscreenCanvas.prototype.getContext.call(fake, '2d')),
    probe(() => OffscreenCanvas.prototype.convertToBlob.call(fake))
  ].join(',');

  return JSON.stringify({
    initialOwnSlots,
    before,
    afterSetter,
    ownSlotsAfterSetter,
    beforeSpoofPixel,
    afterSpoof,
    widthDescriptor,
    heightDescriptor,
    fakeResults,
    fakeSlots: internalNames(fake),
    instance: canvas instanceof OffscreenCanvas
  });
})()
"#,
        )
        .expect("OffscreenCanvas dimensions should resist reflection and spoofing");

    assert_eq!(
        result,
        r#"{"initialOwnSlots":"","before":"2:2","afterSetter":"3:4","ownSlotsAfterSetter":"","beforeSpoofPixel":"0,255,0,255","afterSpoof":"3:4:0,255,0,255:__moliOffscreenCanvasBrand,__moliOffscreenCanvasHeight,__moliOffscreenCanvasWidth","widthDescriptor":"function:get width:0:function:set width:1:true:true:false","heightDescriptor":"function:get height:0:function:set height:1:true:true:false","fakeResults":"TypeError,TypeError,TypeError,TypeError","fakeSlots":"__moliOffscreenCanvasBrand,__moliOffscreenCanvasHeight,__moliOffscreenCanvasWidth","instance":true}"#
    );
}

#[test]
fn offscreen_canvas_dimensions_parse_webidl_enforce_range() {
    let mut vm = new_storage_test_vm("https://offscreen-canvas-dimensions-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };

  const constructed = new OffscreenCanvas({ valueOf() { return 7.8; } }, "5.9");
  const constructorMissing = probe(() => new OffscreenCanvas());
  const constructorSymbol = probe(() => new OffscreenCanvas(Symbol('width'), 1));
  const constructorNegative = probe(() => new OffscreenCanvas(-1, 1));
  const constructorNan = probe(() => new OffscreenCanvas(NaN, 1));

  const canvas = new OffscreenCanvas(2, 2);
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = '#00ff00';
  ctx.fillRect(0, 0, 1, 1);
  const beforeResize = Array.from(ctx.getImageData(0, 0, 1, 1).data).join(',');
  canvas.width = { valueOf() { return 3.8; } };
  const afterGoodWidth = [
    canvas.width,
    Array.from(ctx.getImageData(0, 0, 1, 1).data).join(',')
  ].join(':');

  ctx.fillStyle = '#00ff00';
  ctx.fillRect(0, 0, 1, 1);
  const beforeBad = Array.from(ctx.getImageData(0, 0, 1, 1).data).join(',');
  const widthSymbol = probe(() => { canvas.width = Symbol('width'); });
  const afterWidthSymbol = [
    canvas.width,
    Array.from(ctx.getImageData(0, 0, 1, 1).data).join(',') === beforeBad
  ].join(':');
  const heightThrow = probe(() => {
    canvas.height = { valueOf() { throw new RangeError('height'); } };
  });
  const afterHeightThrow = [
    canvas.height,
    Array.from(ctx.getImageData(0, 0, 1, 1).data).join(',') === beforeBad
  ].join(':');
  const widthUndefined = probe(() => { canvas.width = undefined; });
  const widthOutOfRange = probe(() => { canvas.width = 4294967296; });

  canvas.height = "4.9";

  return JSON.stringify({
    constructed: `${constructed.width}:${constructed.height}`,
    constructorMissing,
    constructorSymbol,
    constructorNegative,
    constructorNan,
    beforeResize,
    afterGoodWidth,
    widthSymbol,
    afterWidthSymbol,
    heightThrow,
    afterHeightThrow,
    widthUndefined,
    widthOutOfRange,
    heightAfterString: canvas.height
  });
})()
"#,
        )
        .expect("OffscreenCanvas dimension WebIDL boundary should evaluate");

    assert_eq!(
        result,
        r#"{"constructed":"7:5","constructorMissing":"TypeError","constructorSymbol":"TypeError","constructorNegative":"TypeError","constructorNan":"TypeError","beforeResize":"0,255,0,255","afterGoodWidth":"3:0,0,0,0","widthSymbol":"TypeError","afterWidthSymbol":"3:true","heightThrow":"RangeError","afterHeightThrow":"2:true","widthUndefined":"TypeError","widthOutOfRange":"TypeError","heightAfterString":4}"#
    );
}

#[test]
fn html_canvas_transform_methods_exist_and_draw_image_scales_canvas_sources() {
    let mut vm = new_storage_test_vm("https://canvas-transform-and-draw-image.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = document.createElement('canvas');
  source.width = 1;
  source.height = 1;
  const sourceCtx = source.getContext('2d');
  sourceCtx.fillStyle = '#00ff00';
  sourceCtx.fillRect(0, 0, 1, 1);

  const canvas = document.createElement('canvas');
  canvas.width = 4;
  canvas.height = 4;
  const ctx = canvas.getContext('2d');
  ctx.save();
  ctx.scale(2, 2);
  ctx.translate(1, 1);
  ctx.rotate(Math.PI / 4);
  ctx.restore();
  ctx.fill();
  ctx.drawImage(source, 1, 1, 2, 2);

  return JSON.stringify({
    surface: [
      typeof ctx.save,
      typeof ctx.restore,
      typeof ctx.scale,
      typeof ctx.translate,
      typeof ctx.rotate,
      typeof ctx.transform,
      typeof ctx.setTransform,
      typeof ctx.resetTransform,
      typeof ctx.fill,
      typeof ctx.drawImage
    ],
    pixels: Array.from(ctx.getImageData(0, 0, 4, 4).data)
  });
})()
"#,
        )
        .expect("canvas transform and drawImage surface should evaluate");

    let value: serde_json::Value =
        serde_json::from_str(&result).expect("canvas transform result should be valid json");
    assert_eq!(
        value["surface"],
        serde_json::json!([
            "function", "function", "function", "function", "function", "function", "function",
            "function", "function", "function"
        ])
    );
    assert_eq!(
        value["pixels"],
        serde_json::json!([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 255, 0, 255, 0,
            255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ])
    );
}

#[test]
fn html_canvas_draw_image_supports_nine_argument_source_cropping() {
    let mut vm = new_storage_test_vm("https://canvas-draw-image-crop.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = document.createElement('canvas');
  source.width = 2;
  source.height = 2;
  const sctx = source.getContext('2d');
  sctx.putImageData(new ImageData(new Uint8ClampedArray([
    255, 0, 0, 255,
    0, 255, 0, 255,
    0, 0, 255, 255,
    255, 255, 0, 255
  ]), 2, 2), 0, 0);

  const canvas = document.createElement('canvas');
  canvas.width = 2;
  canvas.height = 2;
  const ctx = canvas.getContext('2d');
  ctx.drawImage(source, 1, 0, 1, 2, 0, 0, 1, 2);
  return JSON.stringify(Array.from(ctx.getImageData(0, 0, 2, 2).data));
})()
"#,
        )
        .expect("canvas drawImage crop should evaluate");

    assert_eq!(result, "[0,255,0,255,0,0,0,0,255,255,0,255,0,0,0,0]");
}

#[tokio::test]
async fn html_canvas_draw_image_reads_data_url_image_pixels() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://canvas-draw-image-data-url.test/");

    vm.eval(
        r#"
(() => {
  const canvas = document.createElement('canvas');
  canvas.width = 2;
  canvas.height = 2;
  const context = canvas.getContext('2d');
  const image = new Image();
  globalThis.__lmDataImageDraw = { canvas, context, image, result: 'pending' };
  image.onload = () => {
    context.drawImage(image, 0, 0);
    globalThis.__lmDataImageDraw.result = JSON.stringify({
      srcData: image.src.startsWith('data:image/png'),
      currentData: image.currentSrc.startsWith('data:image/png'),
      size: [image.naturalWidth, image.naturalHeight],
      first: Array.from(context.getImageData(0, 0, 1, 1).data),
      second: Array.from(context.getImageData(1, 0, 1, 1).data)
    });
  };
  image.onerror = () => {
    globalThis.__lmDataImageDraw.result = 'error';
  };
  image.src = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAGQAAABkCAYAAABw4pVUAAAA+UlEQVR4nO3RoRHAQBDEsOu/6YR+B2sgIO4Z3919pMwDMCRtHoAhafMADEmbB2BI2jwAQ9LmARiSNg/AkLR5AIakzQMwJG0egCFp8wAMSZsHYEjaPABD0uYBGJI2D8CQtHkAhqTNAzAkbR6AIWnzAAxJmwdgSNo8AEPS5gEYkjYPwJC0eQCGpM0DMCRtHoAhafMADEmbB2BI2jwAQ9LmARiSNg/AkLR5AIakzQMwJG0egCFp8wAMSZsHYEjaPABD0uYBGJI2D8CQtHkAhqTNAzAkbR6AIWnzAAxJmwdgSNo8AEPS5gEYkjYPwJC0eQCGpM0DMCRtHsDjB5K06yueJFXJAAAAAElFTkSuQmCC';
})()
"#,
    )
    .expect("data URL image draw setup should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "data URL image load should dispatch"
    );

    let result = vm
        .eval("globalThis.__lmDataImageDraw.result")
        .expect("data URL image draw result should be readable");

    assert_eq!(
        result,
        r#"{"srcData":true,"currentData":true,"size":[100,100],"first":[0,0,0,255],"second":[0,0,0,255]}"#
    );
}

#[test]
fn html_canvas_image_smoothing_controls_draw_image_filter() {
    let mut vm = new_storage_test_vm("https://canvas-image-smoothing.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const source = document.createElement('canvas');
  source.width = 2;
  source.height = 1;
  source.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray([
    255, 0, 0, 255,
    0, 255, 0, 255
  ]), 2, 1), 0, 0);

  const smoothCanvas = document.createElement('canvas');
  smoothCanvas.width = 3;
  smoothCanvas.height = 1;
  const smooth = smoothCanvas.getContext('2d');
  const defaults = [smooth.imageSmoothingEnabled, smooth.imageSmoothingQuality].join(':');
  smooth.imageSmoothingQuality = 'high';
  const high = smooth.imageSmoothingQuality;
  const invalidQuality = (() => {
    try {
      smooth.imageSmoothingQuality = 'invalid';
      return 'no-throw';
    } catch (error) {
      return error.name;
    }
  })();
  smooth.drawImage(source, 0, 0, 3, 1);
  const smoothPixels = Array.from(smooth.getImageData(0, 0, 3, 1).data);

  const nearestCanvas = document.createElement('canvas');
  nearestCanvas.width = 3;
  nearestCanvas.height = 1;
  const nearest = nearestCanvas.getContext('2d');
  nearest.imageSmoothingEnabled = 0;
  const disabled = nearest.imageSmoothingEnabled;
  nearest.drawImage(source, 0, 0, 3, 1);
  const nearestPixels = Array.from(nearest.getImageData(0, 0, 3, 1).data);

  const offscreen = new OffscreenCanvas(1, 1).getContext('2d');
  offscreen.imageSmoothingEnabled = { valueOf() { return 0; } };
  offscreen.imageSmoothingQuality = 'medium';

  return JSON.stringify({
    defaults,
    high,
    invalidQuality,
    disabled,
    smoothPixels,
    nearestPixels,
    offscreen: [offscreen.imageSmoothingEnabled, offscreen.imageSmoothingQuality].join(':')
  });
})()
"#,
        )
        .expect("canvas image smoothing controls should evaluate");

    assert_eq!(
        result,
        r#"{"defaults":"true:low","high":"high","invalidQuality":"TypeError","disabled":false,"smoothPixels":[255,0,0,255,85,170,0,255,0,255,0,255],"nearestPixels":[255,0,0,255,255,0,0,255,0,255,0,255],"offscreen":"true:medium"}"#
    );
}

#[test]
fn html_image_data_gif_exposes_intrinsic_dimensions() {
    let mut vm = new_storage_test_vm("https://image-data-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const img = new Image(4, 5);
  img.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
  return [
    img.complete,
    img.width,
    img.height,
    img.naturalWidth,
    img.naturalHeight
  ].join('|');
})()
"#,
        )
        .expect("data image dimensions should be readable");

    assert_eq!(result, "true|4|5|1|1");
}

#[test]
fn html_image_unfetched_urls_do_not_invent_intrinsic_dimensions_from_filenames() {
    let mut vm = new_storage_test_vm(
        "https://example.test/html/semantics/embedded-content/the-img-element/page.html",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const cat = new Image();
  cat.src = 'resources/cat.jpg';

  const explicit = new Image(12, 13);
  explicit.src = 'resources/cat.jpg';

  const green = new Image();
  green.src = '/images/green.png';

  const srcset = new Image();
  srcset.srcset = '/images/green-256x256.png 1x';

  const broken = new Image();
  broken.src = 'non-existent.jpg';

  return [
    [cat.width, cat.height, cat.naturalWidth, cat.naturalHeight].join('x'),
    [explicit.width, explicit.height, explicit.naturalWidth, explicit.naturalHeight].join('x'),
    [green.naturalWidth, green.naturalHeight].join('x'),
    [srcset.naturalWidth, srcset.naturalHeight].join('x'),
    [broken.naturalWidth, broken.naturalHeight].join('x')
  ].join('|');
})()
"#,
        )
        .expect("unfetched image dimensions should be readable");

    assert_eq!(result, "0x0x0x0|12x13x0x0|0x0|0x0|0x0");
}

#[test]
fn html_image_data_svg_exposes_intrinsic_dimensions() {
    let mut vm = new_storage_test_vm("https://image-data-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const sized = new Image();
  sized.src = 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" width="500" height="400"/>';

  const ratioWidth = new Image();
  ratioWidth.src = 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" width="400" viewBox="0 0 800 600"/>';

  const noSize = new Image();
  noSize.src = 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg"/>';

  const fractionalConcreteHeight = new Image();
  fractionalConcreteHeight.src = 'data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 12"/>';

  return [
    [sized.naturalWidth, sized.naturalHeight].join('x'),
    [ratioWidth.naturalWidth, ratioWidth.naturalHeight].join('x'),
    [noSize.naturalWidth, noSize.naturalHeight].join('x'),
    [fractionalConcreteHeight.naturalWidth, fractionalConcreteHeight.naturalHeight].join('x')
  ].join('|');
})()
"#,
        )
        .expect("data SVG image dimensions should be readable");

    // Blink resolves an external SVG with neither dimensions nor a viewBox
    // against the replaced-element default object size.
    // Its DOM natural dimensions remain integers even when layout retains the
    // precise fractional concrete object size for aspect-ratio calculations.
    assert_eq!(result, "500x400|400x300|300x150|300x38");
}

#[test]
fn html_legacy_factory_constructors_use_element_interface_prototypes() {
    let mut vm = new_storage_test_vm("https://legacy-factory-constructor.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function check(label, ctor, iface, instance) {
    const descriptor = Object.getOwnPropertyDescriptor(ctor, 'prototype');
    return [
      label,
      ctor.name,
      Object.getPrototypeOf(ctor) === Function.prototype,
      ctor.prototype === iface.prototype,
      Object.getPrototypeOf(instance) === iface.prototype,
      descriptor.configurable,
      descriptor.enumerable,
      descriptor.writable,
      instance.localName,
      instance.namespaceURI
    ].join(':');
  }
  return [
    check('Audio', Audio, HTMLAudioElement, new Audio()),
    check('Image', Image, HTMLImageElement, new Image()),
    check('Option', Option, HTMLOptionElement, new Option())
  ].join('|');
})()
"#,
        )
        .expect("legacy factory constructor shapes should evaluate");

    assert_eq!(
        result,
        concat!(
            "Audio:Audio:true:true:true:false:false:false:audio:http://www.w3.org/1999/xhtml|",
            "Image:Image:true:true:true:false:false:false:img:http://www.w3.org/1999/xhtml|",
            "Option:Option:true:true:true:false:false:false:option:http://www.w3.org/1999/xhtml"
        )
    );
}

#[tokio::test]
async fn html_image_load_runs_on_its_dom_task_not_window_load_dispatch() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-load-order.test/");

    let before_load = vm
        .eval(
            r#"
(() => {
  const img = document.createElement('img');
  img.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
  (document.body || document.documentElement || document).appendChild(img);
  img.onload = () => {
    globalThis.__lmImageLoadOrder = [
      img.width,
      img.height,
      img.naturalWidth,
      img.naturalHeight
    ].join('x');
  };
  return [
    img.width,
    img.height,
    img.naturalWidth,
    img.naturalHeight,
    globalThis.__lmImageLoadOrder || 'pending'
  ].join('|');
})()
"#,
        )
        .expect("image setup should evaluate");

    assert_eq!(before_load, "1|1|1|1|pending");

    vm.dispatch_window_load_event()
        .expect("window load should not dispatch pending image loads");
    assert_eq!(
        vm.eval("globalThis.__lmImageLoadOrder || 'pending'")
            .expect("pre-image-task state should be readable"),
        "pending"
    );
    run_one_canvas_image_load_event_task(&mut vm, &loader).await;

    let after_load = vm
        .eval("globalThis.__lmImageLoadOrder")
        .expect("image load handler result should be readable");

    assert_eq!(after_load, "1x1x1x1");
}

#[tokio::test]
async fn html_image_load_records_resource_performance_entry() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://image-resource-timing.test/base/page.html");

    vm.eval(
        r#"
(() => {
  globalThis.__lmImageResourceTiming = 'pending';
  const img = document.createElement('img');
  img.src = 'hero.png';
  new PerformanceObserver(list => {
    const entries = list.getEntriesByName(img.src);
    if (entries.length) {
      const entry = entries[0];
      globalThis.__lmImageResourceTiming = [
        entry.name,
        entry.entryType,
        entry.initiatorType,
        entry.domainLookupStart === entry.domainLookupEnd,
        entry.domainLookupStart === entry.connectStart,
        entry.domainLookupStart === entry.connectEnd
      ].join('|');
    }
  }).observe({type: 'resource'});
  (document.body || document.documentElement || document).appendChild(img);
})()
"#,
    )
    .expect("image resource timing setup should evaluate");

    run_one_canvas_image_load_event_task(&mut vm, &loader).await;
    vm.run_next_timeout_for_test()
        .expect("performance observer delivery should run");

    let result = vm
        .eval("globalThis.__lmImageResourceTiming")
        .expect("image resource timing result should be readable");

    assert_eq!(
        result,
        "https://image-resource-timing.test/base/hero.png|resource|img|true|true|true"
    );
}

#[tokio::test]
async fn html_image_load_does_not_reach_window_capture_listener() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-load-window-path.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__lmLoadPath = [];
  const body = document.body || document.documentElement || document;
  const img = document.createElement('img');
  img.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
  body.appendChild(img);

  window.addEventListener('load', event => {
    __lmLoadPath.push(`window:${event.target === document ? 'document' : event.target.tagName}:${event.eventPhase}`);
  }, true);
  document.addEventListener('load', event => {
    __lmLoadPath.push(`document:${event.target === img ? 'img' : event.target.nodeName}:${event.eventPhase}`);
  }, true);
  body.addEventListener('load', event => {
    __lmLoadPath.push(`body:${event.target === img ? 'img' : event.target.nodeName}:${event.eventPhase}`);
  }, true);
  img.addEventListener('load', event => {
    __lmLoadPath.push(`img:${event.eventPhase}`);
  });
  return 'ready';
})()
"#,
    )
    .expect("image load path setup should evaluate");

    run_one_canvas_image_load_event_task(&mut vm, &loader).await;
    vm.dispatch_window_load_event()
        .expect("window load should remain a separate lifecycle action");

    let events = vm
        .eval("globalThis.__lmLoadPath.join('|')")
        .expect("load path should be readable");

    assert_eq!(events, "document:img:1|body:img:1|img:2|window:document:2");
}

#[tokio::test]
async fn html_image_complete_tracks_pending_src_and_srcset_loads() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-complete.test/");

    let before_load = vm
        .eval(
            r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  const srcImg = document.createElement('img');
  const srcsetImg = document.createElement('img');
  const emptyImg = document.createElement('img');
  const removedImg = document.createElement('img');
  (document.body || document.documentElement || document).append(srcImg, srcsetImg, emptyImg, removedImg);

  srcImg.src = '/images/green-256x256.png';
  srcsetImg.srcset = '/images/green-256x256.png 1x';
  emptyImg.setAttribute('src', '');
  removedImg.src = '/images/green-256x256.png';
  const removedBefore = removedImg.complete;
  removedImg.removeAttribute('src');

  return [
    srcImg.complete,
    srcsetImg.complete,
    emptyImg.complete,
    removedBefore,
    removedImg.complete
  ].join('|');
})()
"#,
        )
        .expect("image complete setup should evaluate");

    assert_eq!(before_load, "false|false|true|false|true");

    assert_eq!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await,
        4,
        "src, srcset, empty-src, and retired-source work must each settle one queued task"
    );

    let after_load = vm
        .eval(
            r#"
(() => {
  const [srcImg, srcsetImg] = document.querySelectorAll('img');
  return [srcImg.complete, srcsetImg.complete].join('|');
})()
"#,
        )
        .expect("image complete post-load state should evaluate");

    assert_eq!(after_load, "true|true");
}

#[test]
fn html_image_complete_uses_shared_data_url_image_mime_classification() {
    let mut vm = new_storage_test_vm("https://image-data-url-complete.test/");

    let complete = vm
        .eval(
            r#"
(() => {
  const upper = new Image();
  upper.src = 'data:Image/PNG;base64,AA==';
  const parameterized = new Image();
  parameterized.src = 'data:image/svg+xml;charset=utf-8,<svg xmlns="http://www.w3.org/2000/svg"/>';
  const nonImage = new Image();
  nonImage.src = 'data:text/plain,not-an-image';
  return [upper.complete, parameterized.complete, nonImage.complete].join('|');
})()
"#,
        )
        .expect("data URL image complete state should evaluate");

    assert_eq!(complete, "true|true|false");
}

#[tokio::test]
async fn html_image_source_mutations_queue_coalesced_async_events() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-complete.test/page.html");

    let before = vm
        .eval(
            r#"
(() => {
  const img = new Image();
  const srcset = new Image();
  const broken = new Image();
  globalThis.__lmImageEvents = { img, srcset, broken, loadHits: 0, srcsetHits: 0, errorHits: 0 };
  img.onload = () => { globalThis.__lmImageEvents.loadHits += 1; };
  srcset.onload = () => {
    globalThis.__lmImageEvents.srcsetCompleteInHandler = srcset.complete;
    srcset.removeAttribute('srcset');
    globalThis.__lmImageEvents.srcsetCompleteAfterRemoval = srcset.complete;
    globalThis.__lmImageEvents.srcsetHits += 1;
  };
  broken.onerror = () => { globalThis.__lmImageEvents.errorHits += 1; };
  img.src = 'first.jpg';
  img.src = '3.jpg?nocache=1';
  srcset.srcset = '3.jpg?nocache=srcset 1x';
  broken.src = 'data:text/plain,not-an-image';
  return [
    img.complete,
    img.currentSrc,
    srcset.complete,
    broken.complete,
    globalThis.__lmImageEvents.loadHits,
    globalThis.__lmImageEvents.srcsetHits,
    globalThis.__lmImageEvents.errorHits
  ].join('|');
})()
"#,
        )
        .expect("image mutation setup should evaluate");

    assert_eq!(before, "false||false|false|0|0|0");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "queued image events should drain"
    );

    let after = vm
        .eval(
            r#"
(() => {
  const { img, srcset, broken, loadHits, srcsetHits, errorHits } = globalThis.__lmImageEvents;
  return [
    img.complete,
    new URL(img.currentSrc).pathname,
    srcset.complete,
    globalThis.__lmImageEvents.srcsetCompleteInHandler,
    globalThis.__lmImageEvents.srcsetCompleteAfterRemoval,
    broken.complete,
    loadHits,
    srcsetHits,
    errorHits
  ].join('|');
})()
"#,
        )
        .expect("queued image event state should be readable");

    assert_eq!(after, "true|/3.jpg|true|true|true|true|1|1|1");
}

#[tokio::test]
async fn html_image_load_honors_img_src_with_unknown_csp_directives() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-csp.test/page.html");
    vm.set_response_content_security_policies(&["img-src 'none'; aaa;".to_owned()]);
    vm.set_response_content_security_report_only_policies(&["img-src 'none'".to_owned()]);

    vm.eval(
        r#"
(() => {
  globalThis.__lmImageCspEvents = [];
  document.addEventListener('securitypolicyviolation', event => {
    __lmImageCspEvents.push(`csp:${event.disposition}:${event.effectiveDirective}:${event.blockedURI}`);
  });
  const image = new Image();
  image.onload = () => __lmImageCspEvents.push('load');
  image.onerror = () => __lmImageCspEvents.push('error');
  image.src = 'asset.png';
  (document.body || document.documentElement || document).appendChild(image);
})()
"#,
    )
    .expect("image CSP setup should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "image CSP event should drain"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );
    assert_eq!(
        vm.eval("__lmImageCspEvents.slice().sort().join('|')")
            .expect("image CSP events should evaluate"),
        "csp:enforce:img-src:https://image-csp.test/asset.png|csp:report:img-src:https://image-csp.test/asset.png|error"
    );
}

#[tokio::test]
async fn html_image_report_only_csp_reports_without_blocking_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://image-csp-report-only.test/page.html");
    vm.set_response_content_security_report_only_policies(&["img-src 'none'".to_owned()]);

    vm.eval(
        r#"
(() => {
  globalThis.__lmImageReportOnlyEvents = [];
  document.addEventListener('securitypolicyviolation', event => {
    __lmImageReportOnlyEvents.push(`csp:${event.disposition}:${event.effectiveDirective}`);
  });
  const image = new Image();
  image.onload = () => __lmImageReportOnlyEvents.push('load');
  image.onerror = () => __lmImageReportOnlyEvents.push('error');
  image.src = 'asset.png';
  (document.body || document.documentElement || document).appendChild(image);
})()
"#,
    )
    .expect("image report-only CSP setup should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "image report-only CSP event should drain"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("__lmImageReportOnlyEvents.slice().sort().join('|')")
            .expect("image report-only CSP events should evaluate"),
        "csp:report:img-src|load"
    );
}

#[tokio::test]
async fn html_image_invalid_base_url_fails_before_csp_check() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://image-csp-fallback.test/path/page.html");
    vm.set_response_content_security_policies(&["img-src 'none'".to_owned()]);

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const base = document.createElement('base');
  base.href = 'about:blank';
  root.prepend(base);

  globalThis.__lmImageFallbackCspEvents = [];
  document.addEventListener('securitypolicyviolation', event => {
    __lmImageFallbackCspEvents.push(`csp:${event.effectiveDirective}:${event.blockedURI}`);
  });
  const image = new Image();
  image.onload = () => __lmImageFallbackCspEvents.push('load');
  image.onerror = () => __lmImageFallbackCspEvents.push('error');
  image.src = 'asset.png';
  root.appendChild(image);
})()
"#,
    )
    .expect("image fallback CSP setup should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "image fallback event should drain"
    );
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        0,
        "an invalid base URL must fail before CSP queues a violation task"
    );
    assert_eq!(
        vm.eval("__lmImageFallbackCspEvents.slice().sort().join('|')")
            .expect("image fallback CSP events should evaluate"),
        "error"
    );
}

#[tokio::test]
async fn html_image_invalid_request_url_fails_before_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-invalid-url.test/page.html");

    vm.eval(
        r#"
(() => {
  globalThis.__lmImageInvalidUrlEvents = [];
  const image = new Image();
  image.onload = () => __lmImageInvalidUrlEvents.push('load');
  image.onerror = () => __lmImageInvalidUrlEvents.push('error');
  image.src = 'http://[';
  (document.body || document.documentElement || document).appendChild(image);
})()
"#,
    )
    .expect("invalid image URL setup should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "invalid image URL event should drain"
    );
    assert_eq!(
        vm.eval("__lmImageInvalidUrlEvents.join('|')")
            .expect("invalid image URL events should evaluate"),
        "error"
    );
}

#[tokio::test]
async fn parser_image_load_honors_meta_img_src_csp() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_parsed_page_task_executor_test_vm(
        "https://parser-image-csp.test/page.html",
        r#"<!doctype html>
<meta http-equiv="Content-Security-Policy" content="img-src 'none'">
<img id="target" src="asset.png">"#,
        &loader,
    );
    vm.eval(
        r#"
(() => {
  globalThis.__lmParserImageCspEvents = [];
  document.addEventListener('securitypolicyviolation', event => {
    __lmParserImageCspEvents.push(`csp:${event.effectiveDirective}:${event.blockedURI}`);
  });
  const image = document.getElementById('target');
  image.onload = () => __lmParserImageCspEvents.push('load');
  image.onerror = () => __lmParserImageCspEvents.push('error');
})()
"#,
    )
    .expect("parser image CSP handlers should install");

    let owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    let interactive = vm
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    vm.apply_main_document_interactive_lifecycle_action(interactive)
        .expect("interactive transition should register the parser image");
    run_one_canvas_image_load_event_task(&mut vm, &loader).await;
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("__lmParserImageCspEvents.slice().sort().join('|')")
            .expect("parser image CSP events should evaluate"),
        "csp:img-src:https://parser-image-csp.test/asset.png|error"
    );
}

#[tokio::test]
async fn html_image_adoption_reselects_detached_subtree_sources() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-adoption.test/page.html");

    let before = vm
        .eval(
            r#"
(() => {
  const parsed = new DOMParser().parseFromString(`
    <img id="direct" src="/images/green-1x1.png">
    <picture id="picture">
      <source srcset="/images/green-1x1.png">
      <img id="picture-image" src="/images/green-2x2.png">
    </picture>
  `, 'text/html');
  const direct = parsed.getElementById('direct');
  const picture = parsed.getElementById('picture');
  const pictureImage = parsed.getElementById('picture-image');
  globalThis.__lmAdoptedImageEvents = [];
  direct.addEventListener('load', () => {
    __lmAdoptedImageEvents.push(`direct:${new URL(direct.currentSrc).pathname}`);
  });
  pictureImage.addEventListener('load', () => {
    __lmAdoptedImageEvents.push(`picture:${new URL(pictureImage.currentSrc).pathname}`);
  });
  document.adoptNode(direct);
  document.adoptNode(picture);
  return [
    direct.ownerDocument === document,
    pictureImage.ownerDocument === document,
    direct.parentNode === null,
    picture.parentNode === null,
    pictureImage.parentNode === picture,
    __lmAdoptedImageEvents.length
  ].join('|');
})()
"#,
        )
        .expect("image adoption setup should evaluate");

    assert_eq!(before, "true|true|true|true|true|0");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "adopted image events should drain"
    );

    let after = vm
        .eval("globalThis.__lmAdoptedImageEvents.sort().join('|')")
        .expect("adopted image events should be readable");

    assert_eq!(
        after,
        "direct:/images/green-1x1.png|picture:/images/green-1x1.png"
    );
}

#[tokio::test]
async fn html_picture_relevant_tree_mutations_reload_images() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://picture-relevant-mutations.test/page.html");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  globalThis.__lmPictureFirstLoads = 0;
  globalThis.__lmPictureSecondLoads = 0;

  const firstPicture = document.createElement('picture');
  const firstSource = document.createElement('source');
  const firstImage = document.createElement('img');
  firstSource.srcset = '/images/green.png';
  firstImage.src = '/images/red.png';
  firstImage.onload = () => { globalThis.__lmPictureFirstLoads += 1; };
  firstPicture.append(firstSource, firstImage);

  const secondPicture = document.createElement('picture');
  const secondSource = document.createElement('source');
  const secondImage = document.createElement('img');
  secondSource.srcset = '/images/green.png';
  secondImage.src = '/images/red.png';
  secondImage.onload = () => { globalThis.__lmPictureSecondLoads += 1; };
  secondPicture.appendChild(secondSource);

  document.body.append(firstPicture, secondPicture, secondImage);
})()
"#,
    )
    .expect("picture relevant mutation setup should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "initial image events should drain"
    );

    let initially_loaded = vm
        .eval("[globalThis.__lmPictureFirstLoads, globalThis.__lmPictureSecondLoads].join('|')")
        .expect("initial picture load counts should be readable");
    assert_eq!(initially_loaded, "1|1");

    vm.eval(
        r#"
(() => {
  document.body.moveBefore(document.querySelector('source'), null);
  document.querySelectorAll('picture')[1].moveBefore(
    document.querySelectorAll('img')[1],
    null
  );
})()
"#,
    )
    .expect("picture relevant mutations should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "relevant mutation image events should drain"
    );

    let reloaded = vm
        .eval("[globalThis.__lmPictureFirstLoads, globalThis.__lmPictureSecondLoads].join('|')")
        .expect("reloaded picture load counts should be readable");
    assert_eq!(reloaded, "2|2");
}

#[tokio::test]
async fn html_image_available_resource_makes_complete_synchronous_after_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-cache.test/page.html");

    vm.eval(
        r#"
(() => {
  const preload = new Image();
  globalThis.__lmImageCache = { preload, events: [] };
  preload.onload = () => {
    const cached = new Image();
    globalThis.__lmImageCache.cached = cached;
    cached.onload = () => { globalThis.__lmImageCache.events.push('cached-load'); };
    cached.src = preload.src;
    globalThis.__lmImageCache.completeImmediately = cached.complete;
    globalThis.__lmImageCache.currentSrcImmediately = cached.currentSrc;
  };
  preload.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
})()
"#,
    )
    .expect("image cache setup should evaluate");

    assert_eq!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await,
        2,
        "the preload handler should append one cached-image task to the same FIFO"
    );

    let after_preload = vm
        .eval(
            r#"
(() => [
  globalThis.__lmImageCache.completeImmediately,
  globalThis.__lmImageCache.currentSrcImmediately === globalThis.__lmImageCache.preload.src,
  globalThis.__lmImageCache.events.join(',')
].join('|'))()
"#,
        )
        .expect("image cache state should be readable");

    assert_eq!(after_preload, "true|true|cached-load");

    assert_eq!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await,
        0,
        "the initial family drain should already consume the cached-image tail task"
    );

    let after_cached_event = vm
        .eval("globalThis.__lmImageCache.events.join(',')")
        .expect("cached image load event should be readable");

    assert_eq!(after_cached_event, "cached-load");
}

#[tokio::test]
async fn html_image_decode_uses_owned_resource_state_and_tracks_source_changes() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-decode.test/page.html");

    vm.eval(
        r#"
(() => {
  const outcome = (promise) => promise.then(
    (value) => `resolve:${String(value)}`,
    (error) => `reject:${error && error.name}:${error instanceof DOMException}`
  );
  const imageDecodeSlotNames = [
    "__lmImageDecodeHandle",
    "__lmImageDecodeId",
    "__lmImageDecodeSource",
    "__lmImageDecodePictureParent",
    "__lmImageDecodeCacheHit"
  ];
  const imageDecodeOwnNames = object => Object.getOwnPropertyNames(object)
    .filter(name => name.startsWith("__lmImageDecode"))
    .sort()
    .join(",");
  const spoofImageDecodeSlots = image => {
    for (const name of imageDecodeSlotNames) {
      const value = name.endsWith("Source")
        ? "/non/existent/spoof.png"
        : name.endsWith("PictureParent") || name.endsWith("CacheHit")
          ? true
          : 999n;
      HTMLImageElement.prototype[name] = value;
      image[name] = value;
    }
  };

  const png = new Image();
  png.src = "/images/green.png";

  const data = new Image();
  data.src = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAACXBIWXMAAAsTAAALEwEAmpwYAAAAB3RJTUUH4QUSEioKsyAgywAAABl0RVh0Q29tbWVudABDcmVhdGVkIHdpdGggR0lNUFeBDhcAAAAWSURBVAjXY9y3bx8DAwPL58+fGRgYACktBRltLfebAAAAAElFTkSuQmCC";

  const svg = new Image();
  svg.src = "/images/green.svg";

  const srcset = new Image();
  srcset.srcset = "/images/green.png 100w";

  const wptResizeObserverPng = new Image();
  wptResizeObserverPng.src = "/resize-observer/resources/image.png";

  const missing = new Image();

  const bad = new Image();
  bad.src = "/non/existent/path.png";

  const corrupt = new Image();
  corrupt.src = "data:image/png;base64,iVBO00PDR0BADBEEF00KGg";

  const changed = new Image();
  changed.src = "/images/green.png?decode-change";

  const inactiveDoc = document.implementation.createHTMLDocument();
  const inactive = inactiveDoc.createElement("img");
  inactive.src = "/images/green.png";

  const adoptedInactive = inactiveDoc.createElement("img");
  adoptedInactive.src = "/images/green.png";
  (document.body || document.documentElement || document).appendChild(adoptedInactive);

  const picture = document.createElement("picture");
  const source = document.createElement("source");
  const pictureImg = document.createElement("img");
  source.srcset = "/images/green.png";
  picture.append(source, pictureImg);

  const quick = new Image();
  quick.src = "/images/green.png?quick-picture";
  const initialOwnNames = imageDecodeOwnNames(png);
  for (const image of [
    png,
    data,
    svg,
    srcset,
    wptResizeObserverPng,
    missing,
    bad,
    corrupt,
    changed,
    inactive,
    adoptedInactive,
    pictureImg,
    quick
  ]) {
    spoofImageDecodeSlots(image);
  }
  const spoofedOwnNames = imageDecodeOwnNames(png);

  const firstChanged = changed.decode();
  changed.src = "/images/blue.png?decode-change";

  const quickPromise = quick.decode();
  document.createElement("picture").appendChild(quick);

  globalThis.__lmImageDecode = { initialOwnNames, spoofedOwnNames, values: [] };
  Promise.all([
    outcome(png.decode()),
    outcome(data.decode()),
    outcome(svg.decode()),
    outcome(srcset.decode()),
    outcome(wptResizeObserverPng.decode()),
    outcome(missing.decode()),
    outcome(bad.decode()),
    outcome(corrupt.decode()),
    outcome(firstChanged),
    outcome(changed.decode()),
    outcome(inactive.decode()),
    outcome(adoptedInactive.decode()),
    outcome(pictureImg.decode()),
    outcome(quickPromise)
  ]).then(values => {
    globalThis.__lmImageDecode.values = values;
  });
})()
"#,
    )
    .expect("image decode setup should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "image decode DOM-manipulation tasks should run"
    );

    let result = vm
        .eval("JSON.stringify(globalThis.__lmImageDecode)")
        .expect("image decode outcomes should be readable");

    assert_eq!(
        result,
        r#"{"initialOwnNames":"","spoofedOwnNames":"__lmImageDecodeCacheHit,__lmImageDecodeHandle,__lmImageDecodeId,__lmImageDecodePictureParent,__lmImageDecodeSource","values":["reject:EncodingError:true","resolve:undefined","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true","reject:EncodingError:true"]}"#,
        "decode() must consult owned resource state: the real data PNG is ready, while disabled-fetch URL guesses and spoofed JS slots cannot manufacture decoded resources"
    );
}

#[tokio::test]
async fn html_image_decode_rejects_old_document_request_on_document_open() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://image-decode-document-open.test/page.html");

    vm.eval(
        r#"
(() => {
  const image = new Image();
  image.src = "/images/green.png?decode-document-open";
  globalThis.__lmImageDecodeReplacement = "pending";
  image.decode().then(
    () => { globalThis.__lmImageDecodeReplacement = "resolved"; },
    error => {
      globalThis.__lmImageDecodeReplacement =
        `rejected:${error && error.name}:${error instanceof DOMException}`;
    }
  );
})()
"#,
    )
    .expect("image decode request should register");

    let original_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_image_decode_request_owners_for_test(),
        vec![(
            original_owner,
            crate::native_bridge::WindowExecutionContextOwner::Frame(
                original_owner.local_window_id,
            ),
        )],
        "decode request must bind its element and Promise realm before resource completion"
    );

    vm.eval("document.open(); document.write('<p>replacement</p>'); document.close();")
        .expect("main document replacement should evaluate");

    assert!(
        vm._context_host
            .borrow()
            .pending_image_decode_request_owners_for_test()
            .is_empty(),
        "document replacement must actively retire the old element-owned decode request"
    );
    assert_eq!(
        vm.eval("globalThis.__lmImageDecodeReplacement")
            .expect("decode replacement result should evaluate"),
        "rejected:EncodingError:true"
    );

    let _ = drain_canvas_image_load_event_tasks(&mut vm, &loader).await;
    assert_eq!(
        vm.eval("globalThis.__lmImageDecodeReplacement")
            .expect("retired decode result should remain stable"),
        "rejected:EncodingError:true",
        "old resource completion must not settle against the replacement document"
    );
}

#[tokio::test]
async fn html_image_decode_keeps_child_element_owner_separate_from_parent_relevant_realm() {
    let mut vm = new_storage_test_vm("https://image-decode-cross-realm.test/page.html");

    vm.eval(
        r#"
(() => {
  const frame = document.createElement("iframe");
  frame.id = "decode-owner-frame";
  frame.srcdoc = "<body><p>original</p></body>";
  (document.body || document.documentElement || document).appendChild(frame);
})()
"#,
    )
    .expect("child frame should be created");
    vm.drain_pending_child_frame_work_for_test();

    let (child_handle, child_owner) = {
        let host = vm._context_host.borrow();
        let child_handle = host
            .child_browsing_context_handles_in_document_order()
            .into_iter()
            .next()
            .expect("child frame handle");
        let child_owner = host
            .current_child_document_task_owner(child_handle)
            .expect("child document owner");
        (child_handle, child_owner)
    };
    let main_owner = vm
        .current_main_document_task_owner()
        .expect("main document owner");

    vm.eval(
        r#"
(() => {
  const frame = document.getElementById("decode-owner-frame");
  const image = frame.contentDocument.createElement("img");
  image.src = "/images/green.png?decode-cross-realm";
  frame.contentDocument.body.appendChild(image);
  const promise = HTMLImageElement.prototype.decode.call(image);
  globalThis.__lmCrossRealmDecode = {
    promiseUsesParentPrototype: Object.getPrototypeOf(promise) === Promise.prototype,
    outcome: "pending"
  };
  promise.then(
    () => { globalThis.__lmCrossRealmDecode.outcome = "resolved"; },
    error => {
      globalThis.__lmCrossRealmDecode.outcome =
        `rejected:${error && error.name}:${error instanceof DOMException}`;
    }
  );
})()
"#,
    )
    .expect("cross-realm image decode request should register");

    assert_eq!(
        vm._context_host
            .borrow()
            .pending_image_decode_request_owners_for_test(),
        vec![(
            child_owner,
            crate::native_bridge::WindowExecutionContextOwner::Frame(main_owner.local_window_id),
        )],
        "element document ownership and Promise relevant realm must remain independent"
    );

    vm.eval(
        r#"
document.getElementById("decode-owner-frame").srcdoc = "<body><p>replacement</p></body>";
"#,
    )
    .expect("child replacement should enqueue");
    run_realm_prerequisite_then_expected_child_frame_semantic_turn_for_test(
        &mut vm,
        ChildFrameSemanticTurnKind::NavigationCommit,
        "child replacement must retire the old image decode owner",
    )
    .await;

    assert!(
        vm._context_host
            .borrow()
            .pending_image_decode_request_owners_for_test()
            .is_empty(),
        "child owner transaction must retire the old element-owned request"
    );
    assert_eq!(
        vm.eval(
            "`${globalThis.__lmCrossRealmDecode.promiseUsesParentPrototype}|${globalThis.__lmCrossRealmDecode.outcome}`",
        )
        .expect("cross-realm decode result should evaluate"),
        "true|pending",
        "owner retirement must not run user Promise reactions inline in the navigation turn"
    );
    vm.eval("void 0")
        .expect("a later owner turn should run the rejection reaction");
    assert_eq!(
        vm.eval("globalThis.__lmCrossRealmDecode.outcome")
            .expect("cross-realm decode rejection should evaluate"),
        "rejected:EncodingError:true",
        "retirement must reject in the still-live parent relevant realm"
    );

    assert!(
        vm.apply_next_image_load_event_body_for_test()
            .expect("stale child image body should retire"),
        "child replacement should leave one stale image body"
    );
    assert_ne!(
        vm._context_host
            .borrow()
            .current_child_document_task_owner(child_handle),
        Some(child_owner),
        "test must observe a real child document replacement"
    );
    assert_eq!(
        vm.eval("globalThis.__lmCrossRealmDecode.outcome")
            .expect("cross-realm decode result should remain stable"),
        "rejected:EncodingError:true"
    );
}

#[tokio::test]
async fn html_image_lazy_detached_sources_wait_until_inserted() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://image-lazy-detached.test/page.html");

    let before_insert = vm
        .eval(
            r#"
(() => {
  const empty = new Image();
  const srcset = new Image();
  globalThis.__lmLazyImageEvents = { empty, srcset, events: [] };

  empty.src = '';
  empty.loading = 'lazy';
  empty.onload = () => globalThis.__lmLazyImageEvents.events.push('empty-load');
  empty.onerror = () => globalThis.__lmLazyImageEvents.events.push('empty-error');

  srcset.src = '';
  srcset.srcset = '/images/green.png';
  srcset.loading = 'lazy';
  srcset.onload = () => globalThis.__lmLazyImageEvents.events.push('srcset-load');
  srcset.onerror = () => globalThis.__lmLazyImageEvents.events.push('srcset-error');

  return globalThis.__lmLazyImageEvents.events.join(',');
})()
"#,
        )
        .expect("lazy detached image setup should evaluate");

    assert_eq!(before_insert, "");

    let _ = drain_canvas_image_load_event_tasks(&mut vm, &loader).await;

    let after_detached_drain = vm
        .eval("globalThis.__lmLazyImageEvents.events.join(',')")
        .expect("lazy detached image state should be readable");

    assert_eq!(after_detached_drain, "");

    vm.eval(
        r#"
(() => {
  const { empty, srcset } = globalThis.__lmLazyImageEvents;
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  document.documentElement.appendChild(empty);
  document.documentElement.appendChild(srcset);
})()
"#,
    )
    .expect("lazy images should append");

    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("inserted lazy-image layout refresh should succeed")
    );
    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "inserted lazy image events should dispatch"
    );

    let after_insert = vm
        .eval("globalThis.__lmLazyImageEvents.events.join(',')")
        .expect("inserted lazy image events should be readable");

    assert_eq!(after_insert, "empty-error,srcset-load");
}

#[tokio::test]
async fn html_image_disconnected_lazy_defers_while_auto_and_eager_load() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm =
        new_storage_page_task_executor_test_vm("https://image-disconnected-lazy.test/page.html");

    vm.eval(
        r#"
(() => {
  globalThis.__lmDisconnectedLazy = [];
  x = new Image();
  x.loading = 'auto';
  x.onload = () => globalThis.__lmDisconnectedLazy.push('auto-load');
  x.onerror = () => globalThis.__lmDisconnectedLazy.push('auto-error');
  x.src = 'resources/image.png?auto';

  x = new Image();
  x.loading = 'eager';
  x.onload = () => globalThis.__lmDisconnectedLazy.push('eager-load');
  x.onerror = () => globalThis.__lmDisconnectedLazy.push('eager-error');
  x.src = 'resources/image.png?eager';

  x = new Image();
  x.loading = 'lazy';
  x.onload = () => globalThis.__lmDisconnectedLazy.push('lazy-load');
  x.onerror = () => globalThis.__lmDisconnectedLazy.push('lazy-error');
  x.src = 'resources/image.png?lazy';
})()
"#,
    )
    .expect("disconnected image setup should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "disconnected image events should drain"
    );

    let result = vm
        .eval("globalThis.__lmDisconnectedLazy.join(',')")
        .expect("disconnected image event state should be readable");

    assert_eq!(result, "auto-load,eager-load");
}

#[tokio::test]
async fn html_image_below_viewport_lazy_waits_until_scroll_reveal() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-lazy-scroll.test/page.html");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.body.innerHTML = `
    <img id="top" loading="lazy" src="resources/image.png?top">
    <div style="height:3000px"></div>
    <img id="below" loading="lazy" src="resources/image.png?below">`;
  globalThis.__lmLazyScrollEvents = [];
  const topImage = document.getElementById('top');
  const belowImage = document.getElementById('below');
  topImage.onload = () => globalThis.__lmLazyScrollEvents.push('top-load:' + topImage.complete);
  belowImage.onload = () => globalThis.__lmLazyScrollEvents.push('below-load:' + belowImage.complete);
})()
"#,
    )
    .expect("lazy scroll setup should evaluate");

    assert_eq!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await,
        0,
        "lazy requests wait for a real layout sample"
    );
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("initial lazy-image layout refresh should succeed")
    );
    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "the near-viewport lazy image should be admitted"
    );

    let after_load = vm
        .eval("globalThis.__lmLazyScrollEvents.join('|')")
        .expect("lazy scroll state after load should be readable");

    assert_eq!(after_load, "top-load:true");

    vm.eval("document.getElementById('below').scrollIntoView()")
        .expect("scrollIntoView should reveal lazy image");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "live scroll offsets should reveal the far lazy image from the latest snapshot"
    );

    let after_scroll = vm
        .eval("globalThis.__lmLazyScrollEvents.join('|')")
        .expect("lazy scroll state after reveal should be readable");

    assert_eq!(after_scroll, "top-load:true|below-load:true");
}

#[tokio::test]
async fn html_image_loading_change_to_eager_reveals_deferred_lazy_image() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-lazy-eager.test/page.html");

    vm.eval(
        r#"
(() => {
  if (!document.documentElement) {
    document.appendChild(document.createElement('html'));
  }
  if (!document.body) {
    document.documentElement.appendChild(document.createElement('body'));
  }
  document.body.innerHTML = `
    <div style="height:3000px"></div>
    <img id="below" loading="lazy" src="resources/image.png?below-eager">`;
  globalThis.__lmLazyEagerEvents = [];
  const belowImage = document.getElementById('below');
  belowImage.onload = () => globalThis.__lmLazyEagerEvents.push('load:' + belowImage.complete);
})()
"#,
    )
    .expect("lazy eager setup should evaluate");

    assert_eq!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await,
        0,
        "the lazy image should not start before a layout sample"
    );
    assert!(
        vm.refresh_layout_snapshot_for_test(moli_layout::LayoutViewport::new(800, 600, 1.0,))
            .expect("far lazy-image layout refresh should succeed")
    );
    assert_eq!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await,
        0,
        "a real far-below fragment must remain deferred"
    );

    let after_load = vm
        .eval("globalThis.__lmLazyEagerEvents.join('|')")
        .expect("lazy eager state after load should be readable");

    assert_eq!(after_load, "");

    vm.eval("document.getElementById('below').loading = 'eager'")
        .expect("loading eager mutation should evaluate");

    assert!(
        drain_canvas_image_load_event_tasks(&mut vm, &loader).await > 0,
        "changing loading to eager should start the exact request"
    );

    let after_eager = vm
        .eval("globalThis.__lmLazyEagerEvents.join('|')")
        .expect("lazy eager state after mutation should be readable");

    assert_eq!(after_eager, "load:true");
}

#[tokio::test]
async fn html_image_lifecycle_dispatch_uses_error_event_for_non_image_data_source() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm("https://image-load-order.test/");

    let before_load = vm
        .eval(
            r#"
(() => {
  const img = document.createElement('img');
  (document.body || document.documentElement || document).appendChild(img);
  globalThis.__lmBrokenImageEvents = [];
  img.onload = () => { globalThis.__lmBrokenImageEvents.push('load:' + img.complete); };
  img.onerror = () => { globalThis.__lmBrokenImageEvents.push('error:' + img.complete); };
  img.src = 'data:text/plain,not-an-image';
  return img.complete;
})()
"#,
        )
        .expect("broken image setup should evaluate");

    assert_eq!(before_load, "false");

    run_one_canvas_image_load_event_task(&mut vm, &loader).await;

    let after_load = vm
        .eval("globalThis.__lmBrokenImageEvents.join('|')")
        .expect("broken image events should be readable");

    assert_eq!(after_load, "error:true");
}

#[tokio::test]
async fn html_image_invalid_data_source_dispatches_plain_error_event() {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).expect("loader");
    let mut vm = new_storage_page_task_executor_test_vm(
        "https://image-load-order.test/html/semantics/embedded-content/the-img-element/",
    );

    let before_load = vm
        .eval(
            r#"
(() => {
  const img = document.createElement('img');
  (document.body || document.documentElement || document).appendChild(img);
  globalThis.__lmMissingImageEvents = [];
  img.onload = () => { globalThis.__lmMissingImageEvents.push('load:' + img.complete); };
  img.onerror = (event) => {
    globalThis.__lmMissingImageEvents.push([
      event.type,
      img.complete,
      event instanceof Event,
      event instanceof Error,
      event instanceof DOMException
    ].join(':'));
  };
  img.src = 'data:text/plain,not-an-image';
  return img.complete;
})()
"#,
        )
        .expect("missing image setup should evaluate");

    assert_eq!(before_load, "false");

    run_one_canvas_image_load_event_task(&mut vm, &loader).await;

    let after_load = vm
        .eval("globalThis.__lmMissingImageEvents.join('|')")
        .expect("missing image events should be readable");

    assert_eq!(after_load, "error:true:true:false:false");
}

#[test]
fn detached_html_image_src_does_not_synchronously_dispatch_load() {
    let mut vm = new_storage_test_vm("https://image-load-order.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const img = new Image();
  let hits = 0;
  img.onload = () => { hits += 1; };
  img.src = 'https://metrics.example/pixel.gif';
  return String(hits);
})()
"#,
        )
        .expect("detached image setup should evaluate");

    assert_eq!(result, "0");
}
