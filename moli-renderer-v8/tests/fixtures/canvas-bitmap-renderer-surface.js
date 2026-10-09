(owner, callee, kind, label) => {
  const checks = [];
  const check = (name, run) => {
    try {
      const actual = run();
      checks.push({name: `${label}: ${name}`, passed: actual === true, actual});
    } catch (error) {
      checks.push({name: `${label}: ${name}`, passed: false, error: String(error)});
    }
  };
  const throws = (run, Constructor) => {
    try { run(); } catch (error) { return error instanceof Constructor; }
    return false;
  };
  const canvas = () => {
    const value = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(2, 3);
    value.width = 2; value.height = 3;
    return value;
  };
  const getContext = (kind === 'element' ? callee.HTMLCanvasElement : callee.OffscreenCanvas).prototype.getContext;
  const C = owner.ImageBitmapRenderingContext;
  const prototype = callee.ImageBitmapRenderingContext.prototype;
  const descriptor = Object.getOwnPropertyDescriptor(prototype, 'canvas');
  const transfer = prototype.transferFromImageBitmap;
  const value = canvas();
  const context = getContext.call(value, 'bitmaprenderer');
  check('context creation and owner realm', () => context !== null && Object.getPrototypeOf(context) === C.prototype);
  check('illegal constructor', () => C.length === 0 && throws(() => new C(), owner.TypeError));
  check('readonly canvas accessor shape', () => descriptor?.enumerable && descriptor.configurable && typeof descriptor.get === 'function' && descriptor.get.length === 0 && descriptor.set === undefined);
  check('operation shape', () => typeof transfer === 'function' && transfer.length === 1 && transfer.name === 'transferFromImageBitmap' && !Object.hasOwn(transfer, 'prototype'));
  check('canvas identity', () => descriptor.get.call(context) === value && !Object.hasOwn(context, 'canvas'));
  check('stable context without reading options again', () => getContext.call(value, 'bitmaprenderer', {get alpha() { throw Error('reacquired options'); }}) === context);
  check('other modes cannot read options or replace context', () => getContext.call(value, '2d', {get alpha() { throw Error('mode switch'); }}) === null && getContext.call(value, 'webgpu') === null && getContext.call(value, 'bitmaprenderer') === context);
  check('2d mode excludes bitmaprenderer', () => getContext.call(canvasWith2d(), 'bitmaprenderer') === null);
  function canvasWith2d() { const result = canvas(); getContext.call(result, '2d'); return result; }
  check('context ID conversion preserves exception identity', () => {
    const marker = {}; try { getContext.call(canvas(), {toString() { throw marker; }}); } catch (error) { return error === marker; }
    return false;
  });
  check('required context ID and Symbol conversion', () => throws(() => getContext.call(canvas()), callee.TypeError) && throws(() => getContext.call(canvas(), Symbol()), callee.TypeError));
  check('primitive settings are ignored', () => [null, undefined, true, 42, 'value', Symbol(), 1n].every(options => getContext.call(canvas(), 'bitmaprenderer', options) !== null));
  check('settings getter once and inherited', () => {
    let reads = 0;
    const options = Object.create({get alpha() { reads++; return true; }});
    return getContext.call(canvas(), 'bitmaprenderer', options) !== null && reads === 1;
  });
  check('settings failure keeps canvas uninitialized', () => {
    const marker = {}; const target = canvas(); let caught;
    try { getContext.call(target, 'bitmaprenderer', {get alpha() { throw marker; }}); } catch (error) { caught = error; }
    return caught === marker && getContext.call(target, '2d') !== null;
  });
  check('public cache names cannot replace the context', () => {
    value.__moliCanvasContextKind = '2d'; value.__moliCanvasContextBitmapRenderer = {};
    value.__moliOffscreenCanvasContextKind = '2d'; value.__moliOffscreenCanvasContext = {};
    return getContext.call(value, 'bitmaprenderer') === context && getContext.call(value, '2d') === null;
  });
  check('null and undefined clear output', () => transfer.call(context, null) === undefined && transfer.call(context, undefined) === undefined);
  check('missing bitmap argument', () => typeof transfer === 'function' && context !== null && throws(() => transfer.call(context), callee.TypeError));
  const revoked = Proxy.revocable(context || {}, {}); revoked.revoke();
  let traps = 0;
  const trap = () => { traps++; throw Error('author trap'); };
  const invalid = [null, {}, Object.create(C.prototype), Object.create(context || {}), new Proxy(context || {}, {get: trap, getPrototypeOf: trap}), revoked.proxy];
  invalid.forEach((receiver, index) => {
    check(`canvas receiver ${index}`, () => typeof descriptor?.get === 'function' && throws(() => descriptor.get.call(receiver), callee.TypeError));
    check(`transfer receiver ${index} before argument inspection`, () => typeof transfer === 'function' && throws(() => transfer.call(receiver, new Proxy({}, {get: trap, getPrototypeOf: trap})), callee.TypeError));
  });
  check('invalid bitmap values', () => typeof transfer === 'function' && context !== null && [0, false, Symbol(), {}, value, new Proxy({}, {get: trap, getPrototypeOf: trap})].every(bitmap => throws(() => transfer.call(context, bitmap), callee.TypeError)));
  check('author proxy traps were not invoked', () => traps === 0);
  check('public constructor replacement keeps intrinsic prototype', () => {
    const saved = owner.ImageBitmapRenderingContext;
    try {
      owner.ImageBitmapRenderingContext = function Replaced() {};
      return Object.getPrototypeOf(getContext.call(canvas(), 'bitmaprenderer')) === saved.prototype;
    } finally { owner.ImageBitmapRenderingContext = saved; }
  });
  return checks;
}
