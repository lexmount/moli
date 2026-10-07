(owner, callee, kind, label) => {
  const checks = [];
  const prototype = kind === 'element'
    ? callee.CanvasRenderingContext2D.prototype
    : callee.OffscreenCanvasRenderingContext2D.prototype;
  const fillRect = prototype.fillRect;
  const assert = (value, message) => { if (!value) throw Error(message); };
  const equal = (actual, expected) => assert(JSON.stringify(actual) === JSON.stringify(expected),
    `expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  const fresh = () => {
    const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(64, 64);
    canvas.width = canvas.height = 64;
    const ctx = canvas.getContext('2d');
    return {ctx, pixel: (x, y) => Array.from(ctx.getImageData(x, y, 1, 1).data)};
  };
  const check = (name, run) => {
    try { run(); checks.push({name: `${label}/${name}`, passed: true}); }
    catch (error) { checks.push({name: `${label}/${name}`, passed: false, error: String(error)}); }
  };
  const rejects = (run) => {
    let error;
    try { run(); } catch (caught) { error = caught; }
    assert(error instanceof callee.TypeError, 'expected callee TypeError');
  };
  check('descriptor', () => {
    const descriptor = callee.Object.getOwnPropertyDescriptor(prototype, 'fillRect');
    assert(typeof descriptor.value === 'function' && descriptor.enumerable && descriptor.writable && descriptor.configurable,
      'ordinary enumerable operation');
    equal(fillRect.length, 4);
  });
  for (let arity = 0; arity < 4; arity++) check(`arity-${arity}`, () => {
    rejects(() => fillRect.apply(fresh().ctx, Array(arity).fill(1)));
  });
  for (const forgery of ['prototype', 'inherit-native', 'proxy', 'revoked']) check(`brand-${forgery}`, () => {
    const {ctx} = fresh();
    let reads = 0;
    const value = {valueOf() { reads++; return 1; }};
    let receiver;
    if (forgery === 'prototype') receiver = owner.Object.create(owner.Object.getPrototypeOf(ctx));
    if (forgery === 'inherit-native') receiver = owner.Object.create(ctx);
    if (forgery === 'proxy') receiver = new owner.Proxy(ctx, {get() { reads++; throw Error('proxy trap'); }});
    if (forgery === 'revoked') {
      const pair = owner.Proxy.revocable(ctx, {}); pair.revoke(); receiver = pair.proxy;
    }
    rejects(() => fillRect.call(receiver, value, value, value, value));
    equal(reads, 0);
  });
  check('other-interface', () => {
    const canvas = kind === 'element' ? new owner.OffscreenCanvas(64, 64) : owner.document?.createElement('canvas');
    if (!canvas) return;
    let conversions = 0;
    rejects(() => fillRect.call(canvas.getContext('2d'), {valueOf() { conversions++; return 1; }}, 1, 2, 3));
    equal(conversions, 0);
  });
  for (const [name, transform, rectangle, hit, miss] of [
    ['translate', [1, 0, 0, 1, 20, 10], [2, 2, 8, 8], [24, 14], [4, 4]],
    ['scale', [2, 0, 0, 3, 0, 0], [2, 2, 8, 8], [14, 20], [3, 3]],
    ['rotate', [0, 1, -1, 0, 32, 0], [2, 2, 8, 8], [26, 6], [6, 6]],
    ['shear', [1, 0, 1, 1, 0, 0], [2, 2, 8, 8], [14, 8], [3, 8]],
    ['reflect', [-1, 0, 0, 1, 32, 0], [2, 2, 8, 8], [26, 6], [6, 6]],
    ['negative-size', [1, 0, 0, 1, 16, 12], [10, 10, -8, -8], [22, 18], [6, 6]],
  ]) check(name, () => {
    const {ctx, pixel} = fresh(); ctx.fillStyle = 'red'; ctx.setTransform(...transform);
    fillRect.call(ctx, ...rectangle);
    equal(pixel(...hit), [255, 0, 0, 255]); equal(pixel(...miss), [0, 0, 0, 0]);
  });
  for (const [name, transform] of [['singular', [1, 2, 2, 4, 0, 0]], ['zero-matrix', [0, 0, 0, 0, 0, 0]]])
    check(name, () => {
      const {ctx} = fresh(); ctx.setTransform(...transform); fillRect.call(ctx, 2, 2, 8, 8);
      assert(ctx.getImageData(0, 0, 64, 64).data.every(value => value === 0), 'singular transform drew pixels');
    });
  check('fractional-coordinates', () => {
    const {ctx, pixel} = fresh(); fillRect.call(ctx, 1.75, 1.75, 4.5, 4.5);
    equal(pixel(2, 2), [0, 0, 0, 255]);
    const edge = pixel(1, 1)[3]; assert(edge > 0 && edge < 255, `fractional coverage ${edge}`);
  });
  check('preserve-default-path', () => {
    const {ctx, pixel} = fresh(); ctx.rect(2, 2, 8, 8); fillRect.call(ctx, 30, 2, 8, 8);
    ctx.clearRect(0, 0, 64, 64); ctx.fill();
    equal(pixel(4, 4), [0, 0, 0, 255]); equal(pixel(32, 4), [0, 0, 0, 0]);
  });
  check('global-alpha', () => {
    const {ctx, pixel} = fresh(); ctx.fillStyle = 'red'; ctx.globalAlpha = .5;
    fillRect.call(ctx, 2, 2, 8, 8); equal(pixel(4, 4), [255, 0, 0, 128]);
  });
  check('source-over', () => {
    const {ctx, pixel} = fresh(); ctx.fillStyle = 'blue'; fillRect.call(ctx, 2, 2, 8, 8);
    ctx.fillStyle = 'red'; ctx.globalAlpha = .5; fillRect.call(ctx, 2, 2, 8, 8);
    equal(pixel(4, 4), [128, 0, 127, 255]);
  });
  check('fill-style-alpha', () => {
    const {ctx, pixel} = fresh(); ctx.fillStyle = 'rgba(255, 0, 0, 0.5)';
    fillRect.call(ctx, 2, 2, 8, 8); equal(pixel(4, 4), [255, 0, 0, 128]);
  });
  for (const value of [NaN, Infinity, -Infinity]) for (let index = 0; index < 4; index++)
    check(`nonfinite-${String(value)}-${index}`, () => {
      const {ctx} = fresh(); const args = [2, 2, 8, 8]; args[index] = value;
      fillRect.call(ctx, ...args);
      assert(ctx.getImageData(0, 0, 64, 64).data.every(value => value === 0), 'nonfinite argument drew pixels');
    });
  for (const [name, rectangle] of [['zero-width', [2, 2, 0, 8]], ['zero-height', [2, 2, 8, 0]]])
    check(name, () => {
      const {ctx} = fresh(); fillRect.call(ctx, ...rectangle);
      assert(ctx.getImageData(0, 0, 64, 64).data.every(value => value === 0), 'zero area drew pixels');
    });
  check('conversion-order', () => {
    const {ctx, pixel} = fresh(); const order = [];
    const args = [2, 2, 8, 8].map((value, index) => ({valueOf() { order.push(index); return value; }}));
    fillRect.call(ctx, ...args); equal(order, [0, 1, 2, 3]); equal(pixel(4, 4), [0, 0, 0, 255]);
  });
  check('conversion-exception', () => {
    const {ctx} = fresh(); const marker = {}; let later = false; let caught;
    try { fillRect.call(ctx, 2, {valueOf() { throw marker; }}, {valueOf() { later = true; return 8; }}, 8); }
    catch (error) { caught = error; }
    assert(caught === marker && !later, 'conversion exception or order');
  });
  check('nonfinite-converts-all', () => {
    const {ctx} = fresh(); const order = [];
    const args = [NaN, 2, 8, 8].map((value, index) => ({valueOf() { order.push(index); return value; }}));
    fillRect.call(ctx, ...args); equal(order, [0, 1, 2, 3]);
  });
  check('primitive-conversion', () => {
    const {ctx, pixel} = fresh(); fillRect.call(ctx, null, false, true, '4');
    equal(pixel(0, 1), [0, 0, 0, 255]);
  });
  return checks;
}
