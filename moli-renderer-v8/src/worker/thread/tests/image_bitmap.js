(async function imageBitmapContract() {
  const checks = [];
  const unhandled = [];
  const onUnhandled = event => { unhandled.push(event.reason); event.preventDefault(); };
  addEventListener('unhandledrejection', onUnhandled);
  const eq = (actual, expected) => {
    if (!Object.is(actual, expected)) throw Error(`expected ${expected}, got ${actual}`);
  };
  const same = (actual, expected) => eq(Array.from(actual).join(), expected.join());
  const check = async (name, run) => {
    try { await run(); checks.push({ name, passed: true }); }
    catch (error) { checks.push({ name, passed: false, error: String(error), stack: error.stack }); }
  };
  const rejects = async (name, run) => {
    const promise = run();
    eq(promise instanceof Promise, true);
    let error;
    try { await promise; } catch (reason) { error = reason; }
    eq(error && error.name, name);
    eq(error instanceof (name === 'TypeError' || name === 'RangeError' ? globalThis[name] : DOMException), true);
    return error;
  };
  const canvas = () => {
    const result = new OffscreenCanvas(2, 2);
    const context = result.getContext('2d');
    const data = new ImageData(new Uint8ClampedArray([
      255, 0, 0, 255, 0, 255, 0, 255,
      0, 0, 255, 255, 255, 255, 255, 255
    ]), 2, 2);
    context.putImageData(data, 0, 0);
    return result;
  };
  const pixel = (image, x = 0, y = 0) => {
    const target = new OffscreenCanvas(image.width || image.displayWidth, image.height || image.displayHeight);
    const context = target.getContext('2d');
    context.drawImage(image, 0, 0);
    return context.getImageData(x, y, 1, 1).data;
  };
  const task = () => new Promise(resolve => setTimeout(resolve, 0));
  const expectsVideoFrame = typeof document !== 'undefined' || typeof DedicatedWorkerGlobalScope === 'function';
  await check('VideoFrame exposure follows the supported global types', () => eq(typeof VideoFrame, expectsVideoFrame ? 'function' : 'undefined'));
  await check('function shape and non-constructor', () => {
    eq(createImageBitmap.name, 'createImageBitmap'); eq(createImageBitmap.length, 1);
    let error; try { new createImageBitmap(canvas()); } catch (reason) { error = reason; }
    eq(error instanceof TypeError, true);
  });
  await check('OffscreenCanvas resolves in a task', async () => {
    let settled = false;
    const promise = createImageBitmap(canvas()); promise.then(() => settled = true, () => {});
    await Promise.resolve(); eq(settled, false);
    const bitmap = await promise;
    eq(bitmap instanceof ImageBitmap, true); eq(bitmap.width, 2); eq(bitmap.height, 2);
    same(pixel(bitmap), [255, 0, 0, 255]); bitmap.close();
  });
  await check('OffscreenCanvas pixels snapshot before later mutation', async () => {
    const source = canvas(), promise = createImageBitmap(source);
    source.getContext('2d').clearRect(0, 0, 2, 2);
    const bitmap = await promise; same(pixel(bitmap), [255, 0, 0, 255]); bitmap.close();
  });
  await check('ImageData pixels snapshot before later mutation', async () => {
    const source = new ImageData(new Uint8ClampedArray([17, 34, 51, 255]), 1, 1);
    const promise = createImageBitmap(source); source.data.fill(0);
    const bitmap = await promise; same(pixel(bitmap), [17, 34, 51, 255]); bitmap.close();
  });
  await check('ImageBitmap source has an independent lifetime', async () => {
    const source = canvas().transferToImageBitmap(), promise = createImageBitmap(source);
    source.close(); const bitmap = await promise;
    same(pixel(bitmap), [255, 0, 0, 255]); bitmap.close(); eq(source.width, 0);
  });
  if (expectsVideoFrame) await check('VideoFrame source snapshots display geometry and lifetime', async () => {
    const source = new VideoFrame(new Uint8Array([17, 34, 51, 255]), {
      format: 'RGBA', codedWidth: 1, codedHeight: 1, displayWidth: 2, displayHeight: 2, timestamp: 7
    });
    const promise = createImageBitmap(source); source.close();
    const bitmap = await promise; eq(bitmap.width, 2); eq(bitmap.height, 2);
    same(pixel(bitmap, 1, 1), [17, 34, 51, 255]); bitmap.close();
  });
  await check('PNG Blob bytes decode asynchronously', async () => {
    const blob = await canvas().convertToBlob(); let settled = false;
    const promise = createImageBitmap(blob); promise.then(() => settled = true, () => {});
    await Promise.resolve(); eq(settled, false);
    const bitmap = await promise; same(pixel(bitmap, 1, 1), [255, 255, 255, 255]); bitmap.close();
  });
  await check('File bytes decode independent of declared MIME', async () => {
    const blob = await canvas().convertToBlob();
    const bitmap = await createImageBitmap(new File([blob], 'image.bin', { type: 'text/plain' }));
    same(pixel(bitmap, 1, 0), [0, 255, 0, 255]); bitmap.close();
  });
  await check('invalid Blob rejects in a task', async () => {
    let settled = false; const promise = createImageBitmap(new Blob());
    promise.then(() => {}, () => settled = true); await Promise.resolve(); eq(settled, false);
    await rejects('InvalidStateError', () => promise);
  });
  await check('corrupted image bytes reject with DOMException', async () => {
    await rejects('InvalidStateError', () => createImageBitmap(new Blob([new Uint8Array([137, 80, 78, 71])], { type: 'image/png' })));
  });
  await check('crop selects the requested pixels', async () => {
    const bitmap = await createImageBitmap(canvas(), 1, 0, 1, 2);
    eq(bitmap.width, 1); eq(bitmap.height, 2);
    same(pixel(bitmap), [0, 255, 0, 255]); same(pixel(bitmap, 0, 1), [255, 255, 255, 255]); bitmap.close();
  });
  await check('negative crop extents move the origin without flipping', async () => {
    const bitmap = await createImageBitmap(canvas(), 2, 2, -1, -1);
    same(pixel(bitmap), [255, 255, 255, 255]); bitmap.close();
  });
  await check('resize and flip use the shared pixel transforms', async () => {
    const bitmap = await createImageBitmap(canvas(), { resizeWidth: 4, resizeHeight: 4, resizeQuality: 'pixelated', imageOrientation: 'flipY' });
    eq(bitmap.width, 4); eq(bitmap.height, 4); same(pixel(bitmap), [0, 0, 255, 255]); bitmap.close();
  });
  await check('dictionary getters precede the source snapshot', async () => {
    const source = canvas(); const seen = [];
    const options = {};
    for (const key of ['colorSpaceConversion', 'imageOrientation', 'premultiplyAlpha', 'resizeHeight', 'resizeQuality', 'resizeWidth']) {
      Object.defineProperty(options, key, { get() { seen.push(key); if (key === 'resizeWidth') source.getContext('2d').clearRect(0, 0, 2, 2); } });
    }
    const bitmap = await createImageBitmap(source, options);
    same(seen, ['colorSpaceConversion', 'imageOrientation', 'premultiplyAlpha', 'resizeHeight', 'resizeQuality', 'resizeWidth']);
    same(pixel(bitmap), [0, 0, 0, 0]); bitmap.close();
  });
  await check('dictionary exception identity propagates', async () => {
    const marker = {}; let error;
    try { await createImageBitmap(canvas(), { get imageOrientation() { throw marker; } }); }
    catch (reason) { error = reason; }
    eq(error, marker);
  });
  await check('zero crop rejects with RangeError', async () => {
    await rejects('RangeError', () => createImageBitmap(canvas(), 0, 0, 0, 1));
  });
  await check('invalid resize and enum values reject', async () => {
    await rejects('InvalidStateError', () => createImageBitmap(canvas(), { resizeWidth: 0 }));
    await rejects('TypeError', () => createImageBitmap(canvas(), { resizeQuality: 'wrong' }));
  });
  await check('missing and invalid sources reject without synchronous throws', async () => {
    await rejects('TypeError', () => createImageBitmap());
    for (const source of [undefined, null, {}, new Uint8Array(4)]) await rejects('TypeError', () => createImageBitmap(source));
  });
  await check('author Proxies and forged source objects reject before options getters', async () => {
    const source = canvas(); let traps = 0, conversions = 0;
    const proxy = new Proxy(source, { get() { traps++; throw Error('source trap'); } });
    const revoked = Proxy.revocable(source, {}); revoked.revoke();
    for (const value of [proxy, revoked.proxy, Object.create(source), Object.create(ImageBitmap.prototype)]) {
      await rejects('TypeError', () => createImageBitmap(value, { get resizeWidth() { conversions++; return 1; } }));
    }
    eq(traps, 0); eq(conversions, 0);
  });
  await check('receiver brand rejects before source and dictionary conversions', async () => {
    let conversions = 0;
    const source = canvas(), options = { get resizeWidth() { conversions++; return 1; } };
    await rejects('TypeError', () => createImageBitmap.call({}, source, options));
    await rejects('TypeError', () => createImageBitmap.call(new Proxy(globalThis, {}), source, options));
    eq(conversions, 0);
  });
  await check('closed bitmap rejects without leaking an inner rejected Promise', async () => {
    const source = canvas().transferToImageBitmap(); source.close();
    await rejects('InvalidStateError', () => createImageBitmap(source));
  });
  if (expectsVideoFrame) await check('closed VideoFrame and getter-closed VideoFrame reject', async () => {
    const source = new VideoFrame(canvas(), { timestamp: 0 }); source.close();
    await rejects('InvalidStateError', () => createImageBitmap(source));
    const late = new VideoFrame(canvas(), { timestamp: 0 });
    await rejects('InvalidStateError', () => createImageBitmap(late, { get resizeWidth() { late.close(); return 1; } }));
  });
  await check('public ImageBitmap replacement does not replace native result identity', async () => {
    const Original = ImageBitmap;
    try {
      globalThis.ImageBitmap = function forged() { throw Error('public constructor'); };
      const bitmap = await createImageBitmap(canvas()); eq(bitmap instanceof Original, true); bitmap.close();
    } finally { globalThis.ImageBitmap = Original; }
  });
  await check('author timer cancellation cannot cancel bitmap completion', async () => {
    const promise = createImageBitmap(canvas());
    for (let id = 0; id < 8; id++) clearTimeout(id);
    const bitmap = await promise; same(pixel(bitmap), [255, 0, 0, 255]); bitmap.close();
  });
  await check('concurrent decode jobs retain their own resolver and transforms', async () => {
    const blob = await canvas().convertToBlob();
    const bitmaps = await Promise.all(Array.from({ length: 6 }, (_, i) => createImageBitmap(blob, i % 2, 0, 1, 2)));
    bitmaps.forEach((bitmap, i) => { same(pixel(bitmap), i % 2 ? [0, 255, 0, 255] : [255, 0, 0, 255]); bitmap.close(); });
  });
  if (typeof document !== 'undefined') {
    await check('loaded HTMLImageElement shares the native CanvasImageSource snapshot', async () => {
      const source = document.createElement('canvas'); source.width = source.height = 1;
      source.getContext('2d').fillStyle = '#ff0000'; source.getContext('2d').fillRect(0, 0, 1, 1);
      const image = new Image(); image.src = source.toDataURL();
      await image.decode(); const bitmap = await createImageBitmap(image); same(pixel(bitmap), [255, 0, 0, 255]); bitmap.close();
    });
    await check('unavailable HTMLImageElement rejects with InvalidStateError', async () => {
      await rejects('InvalidStateError', () => createImageBitmap(new Image()));
    });
  }
  await task(); await task();
  await check('handled failures do not publish unhandled rejections', () => eq(unhandled.length, 0));
  removeEventListener('unhandledrejection', onUnhandled);
  return { complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks };
})
