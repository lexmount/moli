(async () => {
  const checks = [];
  const check = async (name, run) => {
    try {
      const actual = await run();
      checks.push({name, passed: actual === true, actual});
    } catch (error) { checks.push({name, passed: false, error: String(error)}); }
  };
  const throws = (run, Constructor, name) => {
    try { run(); } catch (error) { return error instanceof Constructor && (!name || error.name === name); }
    return false;
  };
  const equal = (actual, expected) => JSON.stringify(actual) === JSON.stringify(expected);
  const realms = [globalThis, document.getElementById('child').contentWindow];
  const surface = globalThis.__bitmapRendererSurface;
  for (let owner = 0; owner < realms.length; owner++) for (let callee = 0; callee < realms.length; callee++) {
    const O = realms[owner], C = realms[callee];
    for (const kind of ['element', 'offscreen']) {
      const prefix = `${owner}/${callee}/${kind}`;
      checks.push(...surface(O, C, kind, prefix));
      const canvas = (width, height) => {
        const target = kind === 'element' ? O.document.createElement('canvas') : new O.OffscreenCanvas(width, height);
        target.width = width; target.height = height; return target;
      };
      const transfer = C.ImageBitmapRenderingContext.prototype.transferFromImageBitmap;
      const pixels = source => {
        const read = new O.OffscreenCanvas(1, 1).getContext('2d');
        read.drawImage(source, 0, 0); return Array.from(read.getImageData(0, 0, 1, 1).data);
      };
      const data = new O.ImageData(new O.Uint8ClampedArray([0, 255, 0, 128, 255, 0, 0, 255]), 2, 1);
      for (const alpha of [true, false]) for (const premultiplyAlpha of ['none', 'premultiply', 'default']) {
        await check(`${prefix}: ownership and pixels alpha=${alpha} premultiply=${premultiplyAlpha}`, async () => {
          const source = await O.createImageBitmap(data, {premultiplyAlpha});
          const target = canvas(4, 3); const ctx = target.getContext('bitmaprenderer', {alpha});
          if (!ctx) return false;
          transfer.call(ctx, source);
          if (source.width !== 0 || source.height !== 0 || !equal(pixels(target), alpha ? [0, 255, 0, 128] : [0, 128, 0, 255])) return false;
          const snapshot = await O.createImageBitmap(target);
          const result = snapshot.width === 2 && snapshot.height === 1 && equal(pixels(snapshot), alpha ? [0, 255, 0, 128] : [0, 128, 0, 255]);
          snapshot.close(); source.close(); source.close();
          return result && equal(pixels(target), alpha ? [0, 255, 0, 128] : [0, 128, 0, 255]);
        });
      }
      await check(`${prefix}: consumed source cannot be reused`, async () => {
        const source = await O.createImageBitmap(data);
        const target = canvas(2, 1), ctx = target.getContext('bitmaprenderer');
        if (!ctx) return false;
        transfer.call(ctx, source);
        const draw = new C.OffscreenCanvas(2, 1).getContext('2d');
        return throws(() => transfer.call(ctx, source), C.DOMException, 'InvalidStateError') &&
          throws(() => draw.drawImage(source, 0, 0), C.DOMException, 'InvalidStateError') &&
          await O.createImageBitmap(source).then(() => false, error => error instanceof O.DOMException && error.name === 'InvalidStateError');
      });
      await check(`${prefix}: author bitmap proxies reject without traps or detaching source`, async () => {
        const source = await O.createImageBitmap(data), ctx = canvas(2, 1).getContext('bitmaprenderer');
        if (!ctx) return false;
        let traps = 0; const trap = () => { traps++; throw Error('bitmap trap'); };
        const revoked = Proxy.revocable(source, {}); revoked.revoke();
        const invalid = [Object.create(source), new Proxy(source, {get: trap, getPrototypeOf: trap}), revoked.proxy];
        const result = invalid.every(value => throws(() => transfer.call(ctx, value), C.TypeError)) && traps === 0 && source.width === 2 && source.height === 1;
        source.close(); return result;
      });
      for (const alpha of [true, false]) {
        await check(`${prefix}: null clears to current attribute size alpha=${alpha}`, async () => {
          const target = canvas(4, 3), ctx = target.getContext('bitmaprenderer', {alpha});
          if (!ctx) return false;
          transfer.call(ctx, await O.createImageBitmap(data));
          target.width = 5; target.height = 4;
          transfer.call(ctx, null);
          const result = await O.createImageBitmap(target);
          const success = result.width === 5 && result.height === 4 && equal(pixels(result), [0, 0, 0, alpha ? 0 : 255]);
          result.close(); return success;
        });
      }
      if (kind === 'element') {
        await check(`${prefix}: HTML attribute resize preserves valid output`, async () => {
          const target = canvas(4, 3), ctx = target.getContext('bitmaprenderer');
          if (!ctx) return false;
          transfer.call(ctx, await O.createImageBitmap(data));
          target.setAttribute('width', '7'); target.height = 8;
          const result = await O.createImageBitmap(target);
          const success = target.width === 7 && target.height === 8 && result.width === 2 && result.height === 1 && equal(pixels(result), [0, 255, 0, 128]);
          result.close(); return success;
        });
        await check(`${prefix}: PNG toDataURL serializes output dimensions`, async () => {
          const target = canvas(4, 3), ctx = target.getContext('bitmaprenderer');
          if (!ctx) return false;
          transfer.call(ctx, await O.createImageBitmap(data));
          const encoded = O.atob(target.toDataURL().split(',')[1]);
          const uint32 = index => [...encoded.slice(index, index + 4)].reduce((value, byte) => value * 256 + byte.charCodeAt(0), 0);
          return encoded.slice(1, 4) === 'PNG' && uint32(16) === 2 && uint32(20) === 1;
        });
        await check(`${prefix}: toBlob snapshots pixels asynchronously before clearing`, async () => {
          const target = canvas(4, 3), ctx = target.getContext('bitmaprenderer');
          if (!ctx) return false;
          transfer.call(ctx, await O.createImageBitmap(data));
          let called = false;
          const encoded = new Promise(resolve => target.toBlob(blob => { called = true; resolve(blob); }));
          transfer.call(ctx, null);
          await Promise.resolve(); if (called) return false;
          const blob = await encoded;
          if (!(blob instanceof O.Blob) || blob.type !== 'image/png' || blob.size === 0) return false;
          const result = await O.createImageBitmap(blob);
          const success = result.width === 2 && result.height === 1 && equal(pixels(result), [0, 255, 0, 128]);
          result.close(); return success;
        });
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(row => row.passed).length, checks};
  return checks.every(row => row.passed);
})()
