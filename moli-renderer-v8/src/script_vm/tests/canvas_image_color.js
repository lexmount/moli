async function probe(bytes) {
  const rows = [];
  const blob = new Blob([new Uint8Array(bytes)], {type: 'image/png'});
  const image = new Image();
  const ready = new Promise((resolve, reject) => { image.onload = resolve; image.onerror = reject; });
  image.src = 'data:image/png;base64,' + btoa(String.fromCharCode(...bytes));
  await ready;
  const seed = await createImageBitmap(image);
  for (const [sourceName, source] of [['image', image], ['blob', blob], ['bitmap', seed]]) {
    for (const conversion of ['default', 'none']) {
      const bitmap = await createImageBitmap(source, {colorSpaceConversion: conversion});
      for (const kind of ['element', 'offscreen']) {
        for (const colorSpace of ['srgb', 'display-p3']) {
          for (const paint of ['drawImage', 'pattern']) {
            const canvas = kind === 'element' ? document.createElement('canvas') : new OffscreenCanvas(2, 2);
            canvas.width = canvas.height = 2;
            const context = canvas.getContext('2d', {colorSpace});
            context.imageSmoothingEnabled = false;
            if (paint === 'drawImage') context.drawImage(bitmap, 0, 0);
            else { context.fillStyle = context.createPattern(bitmap, 'repeat'); context.fillRect(0, 0, 2, 2); }
            const converted = conversion === 'default' || sourceName === 'bitmap';
            const expected = converted ? (colorSpace === 'srgb' ? [205, 0, 0, 204] : [187, 0, 0, 204])
              : (colorSpace === 'srgb' ? [187, 0, 0, 204] : [171, 35, 23, 204]);
            const actual = Array.from(context.getImageData(0, 0, 1, 1, {colorSpace}).data);
            rows.push({name: [sourceName, conversion, kind, colorSpace, paint].join('/'), expected, actual,
              passed: actual.every((value, i) => Math.abs(value - expected[i]) <= 2)});
          }
        }
      }
      bitmap.close();
    }
  }
  seed.close();
  return rows;
}
