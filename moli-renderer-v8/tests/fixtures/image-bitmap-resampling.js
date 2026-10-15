(async () => {
  const checks = [];
  const globals = [globalThis, document.querySelector('iframe').contentWindow];
  const palette = [255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,255,255];
  const kinds = ['ImageData', 'HTMLCanvasElement', 'OffscreenCanvas', 'ImageBitmap'];
  globalThis.__bitmapSamplingProgress = '';
  function equalPixels(actual, expected, tolerance = 1) {
    if (actual.length !== expected.length) throw new Error('pixel buffer length');
    for (let i = 0; i < actual.length; i++) {
      if (Math.abs(actual[i] - expected[i]) > tolerance)
        throw new Error('channel ' + i + ': expected ' + expected[i] + ', got ' + actual[i]);
    }
  }
  function canvas(owner, width, height, kind = 'OffscreenCanvas') {
    if (kind === 'OffscreenCanvas') return new owner.OffscreenCanvas(width, height);
    const result = owner.document.createElement('canvas');
    result.width = width; result.height = height;
    return result;
  }
  async function source(owner, kind, bytes, width, height) {
    const data = new owner.ImageData(new owner.Uint8ClampedArray(bytes), width, height);
    if (kind === 'ImageData') return data;
    if (kind === 'ImageBitmap') return owner.createImageBitmap(data);
    const result = canvas(owner, width, height, kind);
    result.getContext('2d').putImageData(data, 0, 0);
    return result;
  }
  function read(owner, bitmap) {
    const result = canvas(owner, bitmap.width, bitmap.height);
    const ctx = result.getContext('2d');
    ctx.drawImage(bitmap, 0, 0);
    return Array.from(ctx.getImageData(0, 0, bitmap.width, bitmap.height).data);
  }
  const padded = [0,0,0,0, 0,0,0,0, 0,0,0,0, 0,0,0,0,
    ...palette.slice(0,8), 0,0,0,0, ...palette.slice(8)];
  for (let ownerIndex = 0; ownerIndex < globals.length; ownerIndex++) {
    const owner = globals[ownerIndex];
    for (const kind of kinds) {
      const input = await source(owner, kind, palette, 2, 2);
      const alphaInput = await source(owner, kind, [255,0,0,255, 0,0,255,0], 2, 1);
      for (let calleeIndex = 0; calleeIndex < globals.length; calleeIndex++) {
        const callee = globals[calleeIndex];
        const reference = canvas(owner, 2, 2);
        reference.getContext('2d').putImageData(new owner.ImageData(new owner.Uint8ClampedArray(palette), 2, 2), 0, 0);
        const expanded = canvas(owner, 4, 4);
        expanded.getContext('2d').drawImage(reference, 0, 0, 4, 4);
        const expansion = Array.from(expanded.getContext('2d').getImageData(0, 0, 4, 4).data);
        const cases = [
          ['downscale-two-axes', input, [0,0,2,2], {resizeWidth:1,resizeHeight:1}, [128,128,128,255]],
          ['upscale-centers-and-edges', input, [0,0,2,2], {resizeWidth:4,resizeHeight:4}, expansion],
          ['isolated-crop', input, [0,0,1,1], {resizeWidth:4,resizeHeight:4}, Array(16).fill([255,0,0,255]).flat()],
          ['negative-crop-extents', input, [1,1,-1,-1], {resizeWidth:3,resizeHeight:3}, Array(9).fill([255,0,0,255]).flat()],
          ['transparent-padding', input, [-1,-1,3,3], {}, padded],
          ['flip-after-scaling', input, [0,0,2,2], {resizeWidth:1,resizeHeight:2,imageOrientation:'flipY'}, [128,128,255,255,128,128,0,255]],
          ['nearest-lower-tie', input, [0,0,2,2], {resizeWidth:1,resizeHeight:1,resizeQuality:'pixelated'}, [255,0,0,255]],
          ['alpha-weighted-downscale', alphaInput, [0,0,2,1], {resizeWidth:1,resizeHeight:1}, [255,0,0,128]],
        ];
        for (const [name, value, crop, options, expected] of cases) {
          const label = [ownerIndex,calleeIndex,kind,name].join('/');
          globalThis.__bitmapSamplingProgress = label;
          let bitmap;
          try {
            bitmap = await callee.createImageBitmap(value, ...crop, {resizeQuality:'low', ...options});
            equalPixels(read(owner, bitmap), expected);
            checks.push({label,passed:true});
          } catch (error) {
            checks.push({label,passed:false,error:String(error)});
          } finally { if (bitmap) bitmap.close(); }
        }
      }
      if (kind === 'ImageBitmap') {input.close(); alphaInput.close();}
    }
  }
  globalThis.__uiEventResults = {checks,total:checks.length};
})()
