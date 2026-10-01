(async () => {
  const rows = [];
  const check = (name, passed) => rows.push({name, passed: Boolean(passed)});
  const finish = () => {
    globalThis.__canvasToBlobResults = {
      rows, passed: rows.filter(row => row.passed).length, total: rows.length
    };
    return rows.every(row => row.passed);
  };
  const canvas = document.createElement('canvas');
  const method = HTMLCanvasElement.prototype.toBlob;
  check('native method exposed', typeof method === 'function');
  if (typeof method !== 'function') return finish();
  check('method length', method.length === 1);
  const descriptor = Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype, 'toBlob');
  check('method descriptor', descriptor.enumerable && descriptor.writable && descriptor.configurable);

  let conversions = 0;
  const type = {toString() { conversions++; return 'image/png'; }};
  for (const callback of [undefined, null, {}, 3, 'callback']) {
    let error;
    try { method.call(canvas, callback, type); } catch (caught) { error = caught; }
    check(`callback rejected before type: ${String(callback)}`, error instanceof TypeError && conversions === 0);
  }
  for (const receiver of [{}, document.createElement('div'),
    Object.create(HTMLCanvasElement.prototype), Object.create(canvas), new Proxy(canvas, {})]) {
    let error;
    try { method.call(receiver, () => {}, type); } catch (caught) { error = caught; }
    check('illegal receiver before conversion', error instanceof TypeError && conversions === 0);
  }
  const revoked = Proxy.revocable(canvas, {}); revoked.revoke();
  let revokedError;
  try { method.call(revoked.proxy, () => {}, type); } catch (caught) { revokedError = caught; }
  check('revoked receiver', revokedError instanceof TypeError && conversions === 0);
  const marker = new Error('type conversion');
  let conversionError;
  try { method.call(canvas, () => {}, {toString() { throw marker; }}); }
  catch (caught) { conversionError = caught; }
  check('type exception preserved', conversionError === marker);
  let symbolError;
  try { method.call(canvas, () => {}, Symbol('png')); } catch (caught) { symbolError = caught; }
  check('symbol type rejected', symbolError instanceof TypeError);

  canvas.width = 2; canvas.height = 1;
  const context = canvas.getContext('2d');
  context.fillStyle = '#00ff00'; context.fillRect(0, 0, 2, 1);
  const order = [];
  let returned;
  const exported = new Promise(resolve => {
    const callback = new Proxy(function(blob) {
      'use strict';
      check('callback undefined receiver', this === undefined);
      order.push('callback');
      resolve(blob);
    }, {apply(target, receiver, args) {
      check('callable Proxy callback', receiver === undefined && args.length === 1);
      return Reflect.apply(target, receiver, args);
    }});
    returned = canvas.toBlob(callback, type, {
      valueOf() { throw new Error('quality converted'); },
      toString() { throw new Error('quality converted'); }
    });
    order.push('return');
    queueMicrotask(() => order.push('microtask'));
  });
  check('returns undefined and callback deferred', returned === undefined && order.join() === 'return');
  check('type converted once', conversions === 1);
  canvas.width = 9; context.fillStyle = '#ff0000'; context.fillRect(0, 0, 9, 1);
  const blob = await exported;
  check('callback follows microtasks', order.join() === 'return,microtask,callback');
  check('PNG Blob output', blob instanceof Blob && blob.type === 'image/png' && blob.size > 33);
  const bytes = new Uint8Array(await blob.arrayBuffer());
  check('PNG signature', Array.from(bytes.slice(0, 8)).join() === '137,80,78,71,13,10,26,10');
  const header = new DataView(bytes.buffer);
  check('snapshot dimensions', header.getUint32(16) === 2 && header.getUint32(20) === 1);
  const bitmap = await createImageBitmap(blob);
  const decoded = document.createElement('canvas'); decoded.width = 2; decoded.height = 1;
  const decodedContext = decoded.getContext('2d'); decodedContext.drawImage(bitmap, 0, 0);
  check('snapshot pixels survive resize and drawing',
    Array.from(decodedContext.getImageData(0, 0, 2, 1).data).join() === '0,255,0,255,0,255,0,255');
  bitmap.close();

  const exportCanvas = (value, ...args) => new Promise(resolve => value.toBlob(resolve, ...args));
  for (const mime of [undefined, 'IMAGE/PNG', 'image/x-unsupported', null]) {
    const fallback = await exportCanvas(canvas, mime);
    check(`PNG format: ${String(mime)}`, fallback instanceof Blob && fallback.type === 'image/png');
  }
  for (const [width, height] of [[0, 4], [3, 0], [0, 0]]) {
    canvas.width = width; canvas.height = height;
    let called = false;
    const zero = new Promise(resolve => canvas.toBlob(blob => { called = true; resolve(blob); }));
    check('zero dimension callback deferred', called === false);
    check('zero dimension result is null', await zero === null);
  }
  const blank = await exportCanvas(document.createElement('canvas'));
  const blankHeader = new DataView(await blank.arrayBuffer());
  check('canvas without context exports default bitmap', blank.type === 'image/png' &&
    blankHeader.getUint32(16) === 300 && blankHeader.getUint32(20) === 150);
  const jpegCanvas = document.createElement('canvas'); jpegCanvas.width = 16; jpegCanvas.height = 8;
  const jpeg = await exportCanvas(jpegCanvas, 'IMAGE/JPEG', 0.9);
  check('JPEG MIME selection', jpeg instanceof Blob && jpeg.type === 'image/jpeg');
  const jpegBytes = new Uint8Array(await jpeg.arrayBuffer());
  check('JPEG signature', jpegBytes[0] === 255 && jpegBytes[1] === 216 && jpegBytes[2] === 255);
  const jpegBitmap = await createImageBitmap(jpeg);
  check('JPEG dimensions', jpegBitmap.width === 16 && jpegBitmap.height === 8);
  decodedContext.clearRect(0, 0, 2, 1); decodedContext.drawImage(jpegBitmap, 0, 0);
  check('JPEG composites transparency on black', Array.from(decodedContext.getImageData(0, 0, 1, 1).data).join() === '0,0,0,255');
  jpegBitmap.close();
  for (const quality of [0, 1, NaN, Infinity, '0.2', {
    valueOf() { throw new Error('quality converted'); }
  }]) {
    check('JPEG quality accepts IDL any without coercion',
      (await exportCanvas(jpegCanvas, 'image/jpeg', quality)).type === 'image/jpeg');
  }
  const inert = document.implementation.createHTMLDocument('');
  const inertCanvas = inert.createElement('canvas'); inertCanvas.width = 1; inertCanvas.height = 1;
  check('windowless canvas exports', (await exportCanvas(inertCanvas)) instanceof Blob);

  const other = document.getElementById('child').contentWindow;
  const childCanvas = other.document.createElement('canvas'); childCanvas.width = 1; childCanvas.height = 1;
  const childBlob = await new Promise(resolve => method.call(childCanvas, resolve));
  check('Blob uses canvas realm', childBlob instanceof other.Blob && !(childBlob instanceof Blob));
  const parentBlob = await new Promise(resolve => {
    other.__canvasResolve = resolve;
    canvas.width = 1; canvas.height = 1;
    method.call(canvas, new other.Function('blob', 'parent.__canvasCallbackRealm = globalThis === parent.document.getElementById("child").contentWindow; __canvasResolve(blob);'));
  });
  check('callback realm remains independent', parent.__canvasCallbackRealm === true && parentBlob instanceof Blob && !(parentBlob instanceof other.Blob));
  delete other.__canvasResolve; delete globalThis.__canvasCallbackRealm;
  return finish();
})()
