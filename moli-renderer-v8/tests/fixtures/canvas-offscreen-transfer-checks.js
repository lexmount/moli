(owner, callee, label) => {
  const checks = [], diagnostics = [];
  const check = (name, run) => {
    try {
      const actual = run();
      checks.push({name: label + ': ' + name, passed: actual === true, actual});
    } catch (error) {
      checks.push({name: label + ': ' + name, passed: false, error: String(error)});
    }
  };
  const transfer = callee.OffscreenCanvas.prototype.transferToImageBitmap;
  const pixel = (source, x = 0, y = 0) => {
    const context = new owner.OffscreenCanvas(4, 4).getContext('2d');
    context.drawImage(source, 0, 0);
    return Array.from(context.getImageData(x, y, 1, 1).data).join(',');
  };
  const make = () => {
    const canvas = new owner.OffscreenCanvas(3, 2);
    const context = canvas.getContext('2d');
    context.fillStyle = '#00ff00';
    context.fillRect(0, 0, 3, 2);
    return {canvas, context};
  };
  const throws = (run, Constructor, name) => {
    try { run(); } catch (error) {
      return error instanceof Constructor && (name === undefined || error.name === name);
    }
    return false;
  };
  check('operation descriptor', () => {
    const descriptor = Object.getOwnPropertyDescriptor(callee.OffscreenCanvas.prototype, 'transferToImageBitmap');
    return descriptor?.value === transfer && typeof transfer === 'function' && descriptor.enumerable && descriptor.configurable && descriptor.writable;
  });
  check('operation shape', () => typeof transfer === 'function' && transfer.name === 'transferToImageBitmap' && transfer.length === 0 && !Object.hasOwn(transfer, 'prototype'));
  check('no rendering context throws synchronously in callee realm', () => throws(() => transfer.call(new owner.OffscreenCanvas(3, 2)), callee.DOMException, 'InvalidStateError'));
  check('failed context acquisition does not enable transfer', () => {
    const canvas = new owner.OffscreenCanvas(3, 2);
    const marker = {};
    let caught;
    try { canvas.getContext('bitmaprenderer', {get alpha() { throw marker; }}); } catch (error) { caught = error; }
    return caught === marker && throws(() => transfer.call(canvas), callee.DOMException, 'InvalidStateError');
  });
  check('public context cache cannot enable transfer', () => {
    const canvas = new owner.OffscreenCanvas(3, 2);
    canvas.__moliOffscreenCanvasContext = {};
    canvas.__moliOffscreenCanvasContextKind = '2d';
    return throws(() => transfer.call(canvas), callee.DOMException, 'InvalidStateError');
  });
  check('returns a native ImageBitmap in canvas realm', () => {
    const {canvas} = make();
    const bitmap = transfer.call(canvas);
    const good = Object.getPrototypeOf(bitmap) === owner.ImageBitmap.prototype && Object.prototype.toString.call(bitmap) === '[object ImageBitmap]' && bitmap.width === 3 && bitmap.height === 2 && pixel(bitmap) === '0,255,0,255';
    bitmap.close();
    return good;
  });
  check('replacement is transparent black with stable dimensions', () => {
    const {canvas, context} = make();
    const first = transfer.call(canvas);
    const second = transfer.call(canvas);
    const good = first !== second && first.width === 3 && second.width === 3 && second.height === 2 && canvas.width === 3 && canvas.height === 2 && canvas.getContext('2d') === context && canvas.getContext('bitmaprenderer') === null && pixel(first) === '0,255,0,255' && pixel(second) === '0,0,0,0' && pixel(canvas) === '0,0,0,0';
    first.close(); second.close();
    return good;
  });
  check('future drawing does not modify transferred pixels', () => {
    const {canvas, context} = make();
    const first = transfer.call(canvas);
    context.fillStyle = '#0000ff'; context.fillRect(0, 0, 3, 2);
    const second = transfer.call(canvas);
    const good = pixel(first) === '0,255,0,255' && pixel(second) === '0,0,255,255';
    first.close(); second.close();
    return good;
  });
  check('closing a transferred bitmap does not close canvas storage', () => {
    const {canvas, context} = make();
    const bitmap = transfer.call(canvas); bitmap.close(); bitmap.close();
    context.fillRect(0, 0, 3, 2);
    return bitmap.width === 0 && bitmap.height === 0 && pixel(canvas) === '0,255,0,255';
  });
  check('paint state and transform survive transfer', () => {
    const {canvas, context} = make();
    context.fillStyle = '#ff0000'; context.globalAlpha = 0.5;
    context.translate(1, 0);
    transfer.call(canvas).close();
    const state = context.fillStyle === '#ff0000' && context.globalAlpha === 0.5;
    context.fillRect(0, 0, 1, 1);
    return state && pixel(canvas) === '0,0,0,0' && pixel(canvas, 1, 0) === '255,0,0,128';
  });
  check('transfer preserves existing save/restore behavior', () => {
    const control = make(), transferred = make();
    for (const {context} of [control, transferred]) {
      context.fillStyle = '#ff0000'; context.save(); context.fillStyle = '#0000ff';
    }
    control.context.clearRect(0, 0, 3, 2);
    control.context.restore(); control.context.fillRect(0, 0, 1, 1);
    const expected = {fillStyle: control.context.fillStyle, pixel: pixel(control.canvas)};
    diagnostics.push({name: label + ': independent save/restore conformance', passed: expected.fillStyle === '#ff0000' && expected.pixel === '255,0,0,255', expected: {fillStyle: '#ff0000', pixel: '255,0,0,255'}, actual: expected});
    transfer.call(transferred.canvas).close();
    transferred.context.restore(); transferred.context.fillRect(0, 0, 1, 1);
    return transferred.context.fillStyle === expected.fillStyle && pixel(transferred.canvas) === expected.pixel;
  });
  check('current path survives transfer', () => {
    const {canvas, context} = make();
    context.beginPath(); context.rect(0, 0, 1, 1);
    transfer.call(canvas).close(); context.fillStyle = '#ff0000'; context.fill();
    return pixel(canvas) === '255,0,0,255' && pixel(canvas, 2, 1) === '0,0,0,0';
  });
  check('dimension assignment still resets drawing state after transfer', () => {
    const {canvas, context} = make();
    const bitmap = transfer.call(canvas);
    context.fillStyle = '#ff0000'; canvas.width = 2;
    const good = context.fillStyle === '#000000' && pixel(canvas) === '0,0,0,0' && bitmap.width === 3 && pixel(bitmap) === '0,255,0,255';
    bitmap.close(); return good;
  });
  check('extra arguments have no observable conversion', () => {
    const {canvas} = make();
    const poison = new Proxy({}, {get() { throw Error('extra argument read'); }, getPrototypeOf() { throw Error('extra argument prototype'); }});
    const bitmap = transfer.call(canvas, poison, Symbol(), 1n);
    const good = pixel(bitmap) === '0,255,0,255'; bitmap.close(); return good;
  });
  check('public ImageBitmap replacement cannot affect native creation', () => {
    const saved = owner.ImageBitmap;
    try {
      owner.ImageBitmap = function Replaced() { throw Error('author constructor'); };
      const bitmap = transfer.call(make().canvas);
      const good = Object.getPrototypeOf(bitmap) === saved.prototype && pixel(bitmap) === '0,255,0,255'; bitmap.close(); return good;
    } finally { owner.ImageBitmap = saved; }
  });
  check('no public dimension or context getter runs during transfer', () => {
    const {canvas} = make();
    for (const name of ['width', 'height', 'getContext']) Object.defineProperty(canvas, name, {get() { throw Error('author getter ' + name); }});
    const bitmap = transfer.call(canvas);
    const good = bitmap.width === 3 && bitmap.height === 2 && pixel(bitmap) === '0,255,0,255'; bitmap.close(); return good;
  });
  for (const alpha of [true, false]) {
    const description = 'bitmaprenderer alpha=' + alpha;
    check(description + ' transfers output and retains natural dimensions', () => {
      const source = transfer.call(make().canvas);
      const canvas = new owner.OffscreenCanvas(0, 0);
      const context = canvas.getContext('bitmaprenderer', {alpha});
      context.transferFromImageBitmap(source);
      const first = transfer.call(canvas), second = transfer.call(canvas);
      const actual = {sourceWidth: source.width, canvasWidth: canvas.width, canvasHeight: canvas.height, firstWidth: first.width, firstHeight: first.height, secondWidth: second.width, secondHeight: second.height, firstPixel: pixel(first), secondPixel: pixel(second)};
      const good = source.width === 0 && canvas.width === 0 && canvas.height === 0 && first.width === 3 && first.height === 2 && second.width === 3 && second.height === 2 && actual.firstPixel === '0,255,0,255' && actual.secondPixel === (alpha ? '0,0,0,0' : '0,0,0,255');
      first.close(); second.close(); return good || actual;
    });
    check(description + ' keeps ownership across a second consumer', () => {
      const source = transfer.call(make().canvas);
      const canvas = new owner.OffscreenCanvas(2, 3);
      const context = canvas.getContext('bitmaprenderer', {alpha}); context.transferFromImageBitmap(source);
      const bitmap = transfer.call(canvas);
      const destination = new owner.OffscreenCanvas(1, 1), output = destination.getContext('bitmaprenderer');
      output.transferFromImageBitmap(bitmap);
      context.transferFromImageBitmap(null);
      const actual = {bitmapWidth: bitmap.width, destinationPixel: pixel(destination), clearedPixel: pixel(canvas)};
      return bitmap.width === 0 && actual.destinationPixel === '0,255,0,255' && actual.clearedPixel === (alpha ? '0,0,0,0' : '0,0,0,255') || actual;
    });
  }
  const genuine = make().canvas;
  const revoked = Proxy.revocable(genuine, {}); revoked.revoke();
  let traps = 0;
  const trap = () => { traps++; throw Error('author receiver trap'); };
  for (const [name, receiver] of [
    ['null', null], ['undefined', undefined], ['plain', {}],
    ['prototype forged', Object.create(owner.OffscreenCanvas.prototype)],
    ['inherited genuine', Object.create(genuine)],
    ['author proxy', new Proxy(genuine, {get: trap, getPrototypeOf: trap})],
    ['revoked proxy', revoked.proxy], ['2d context', genuine.getContext('2d')],
  ]) check(name + ' receiver rejects in callee realm', () => typeof transfer === 'function' && throws(() => transfer.call(receiver), callee.TypeError));
  check('invalid receivers did not invoke Proxy traps', () => traps === 0);
  for (const [width, height] of [[0, 2], [2, 0], [0, 0]]) {
    try {
      const canvas = new owner.OffscreenCanvas(width, height); canvas.getContext('2d');
      const bitmap = transfer.call(canvas);
      diagnostics.push({name: label + ': zero ' + width + 'x' + height, width: bitmap.width, height: bitmap.height}); bitmap.close();
    } catch (error) { diagnostics.push({name: label + ': zero ' + width + 'x' + height, error: String(error), exceptionName: error.name}); }
  }
  return {checks, diagnostics};
}
