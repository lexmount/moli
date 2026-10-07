(function canvasContract(owner, callee, kind, tag) {
  const checks = [];
  const check = (name, run) => {
    try { checks.push({name: 'canvas/' + tag + '/' + name, passed: run() === true}); }
    catch (error) { checks.push({name: 'canvas/' + tag + '/' + name, passed: false, error: String(error)}); }
  };
  const interfaceName = kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D';
  const makeCanvas = () => kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(96, 96);
  const canvas = makeCanvas(); canvas.width = canvas.height = 96;
  const ctx = canvas.getContext('2d');
  const prototype = callee[interfaceName].prototype;
  const method = prototype.getTransform;
  const Matrix = callee.DOMMatrix;
  const matrixPrototype = Matrix.prototype;
  const get = (receiver = ctx, ...args) => {
    if (typeof method !== 'function') throw new Error('getTransform missing');
    return method.call(receiver, ...args);
  };
  const components = matrix => ['a', 'b', 'c', 'd', 'e', 'f'].map(name => matrix[name]);
  const equal = (actual, expected) => actual.length === expected.length && actual.every((value, index) =>
    value === expected[index] || Math.abs(value - expected[index]) <= 1e-12 * Math.max(1, Math.abs(expected[index])));
  const matches = expected => equal(components(get()), expected);
  const identity = [1, 0, 0, 1, 0, 0];
  const throwsTypeError = run => { try { run(); return false; } catch (error) { return error instanceof callee.TypeError; } };

  check('own descriptor', () => {
    const d = Object.getOwnPropertyDescriptor(prototype, 'getTransform');
    return !!d && d.value === method && d.enumerable && d.writable && d.configurable;
  });
  check('function metadata', () => typeof method === 'function' && method.length === 0 && method.name === 'getTransform' && method instanceof callee.Function);
  check('initial identity', () => matches(identity));
  check('DOMMatrix prototype in callee realm', () => Object.getPrototypeOf(get()) === matrixPrototype);
  check('mutable and readonly brands', () => get() instanceof Matrix && get() instanceof callee.DOMMatrixReadOnly);
  check('initial 2D and identity flags', () => get().is2D && get().isIdentity);
  check('fresh objects', () => get() !== get());
  check('no own expando components', () => !Object.hasOwn(get(), 'a') && !Object.hasOwn(get(), 'm11'));
  check('Float64 mapping and realm', () => {
    const array = get().toFloat64Array();
    return array instanceof callee.Float64Array && equal(Array.from(array), [1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]);
  });
  check('Float32 mapping and realm', () => get().toFloat32Array() instanceof callee.Float32Array);
  check('JSON mapping', () => get().toJSON().m44 === 1 && get().toJSON().is2D === true);
  check('ignored extra arguments', () => {
    let traps = 0;
    const extra = new Proxy({}, {get() { traps++; throw new Error('argument converted'); }});
    return equal(components(get(ctx, extra)), identity) && traps === 0;
  });

  for (const [name, receiver] of [
    ['null', null], ['undefined', undefined], ['number', 0], ['string', ''], ['boolean', false],
    ['plain', {}], ['prototype', prototype], ['forged prototype', Object.create(prototype)],
    ['inherited real object', Object.create(ctx)], ['author Proxy', new Proxy(ctx, {})]
  ]) check('receiver ' + name, () => throwsTypeError(() => {
    if (typeof method !== 'function') throw new Error('getTransform missing');
    method.call(receiver);
  }));
  const revoked = Proxy.revocable(ctx, {}); revoked.revoke();
  check('revoked Proxy receiver', () => throwsTypeError(() => get(revoked.proxy)));
  check('receiver validation without author traps', () => {
    let traps = 0;
    const fake = new Proxy(ctx, {get() { traps++; throw new Error('receiver trap'); }, getPrototypeOf() { traps++; return prototype; }});
    return throwsTypeError(() => get(fake)) && traps === 0;
  });
  if (owner.document) check('other context interface rejected', () => {
    const other = kind === 'element' ? new owner.OffscreenCanvas(96,96) : owner.document.createElement('canvas');
    return throwsTypeError(() => get(other.getContext('2d')));
  });

  check('six scalar coefficients', () => { ctx.setTransform(1, 2, 3, 4, 5, 6); return matches([1,2,3,4,5,6]); });
  check('six scalar 4D mapping', () => equal(Array.from(get().toFloat64Array()), [1,2,0,0,3,4,0,0,0,0,1,0,5,6,0,1]));
  check('nonidentity flag', () => !get().isIdentity && get().is2D);
  check('snapshot mutation leaves context unchanged', () => { const m = get(); m.a = 8; m.e = 30; return matches([1,2,3,4,5,6]); });
  check('snapshot mutator leaves context unchanged', () => { get().translateSelf(20,30).scaleSelf(2); return matches([1,2,3,4,5,6]); });
  check('context mutation leaves snapshot unchanged', () => { const m = get(); ctx.setTransform(2,0,0,3,10,20); return equal(components(m),[1,2,3,4,5,6]) && matches([2,0,0,3,10,20]); });
  check('postmultiply translation', () => { ctx.translate(4,5); return matches([2,0,0,3,18,35]); });
  check('postmultiply scale', () => { ctx.scale(4,5); return matches([8,0,0,15,18,35]); });
  check('postmultiply rotation', () => { ctx.rotate(Math.PI/2); return matches([0,15,-8,0,18,35]); });
  check('reset transform', () => { ctx.resetTransform(); return matches(identity); });
  check('noncommutative multiplication', () => { ctx.setTransform(1,2,3,4,5,6); ctx.transform(7,8,9,10,11,12); return matches([31,46,39,58,52,76]); });
  check('fractional double state', () => { const a=[1.125,0.0625,-0.03125,2.25,5.125,-7.75]; ctx.setTransform(...a); return matches(a); });
  check('singular matrix', () => { ctx.setTransform(0,0,0,0,5,6); return matches([0,0,0,0,5,6]); });

  const names = ['a','b','c','d','e','f'];
  const aliases = ['m11','m12','m21','m22','m41','m42'];
  for (let index=0; index<6; index++) {
    check('dictionary component ' + names[index], () => { ctx.setTransform({[names[index]]:2}); const a=identity.slice(); a[index]=2; return matches(a); });
    check('dictionary alias ' + aliases[index], () => { ctx.setTransform({[aliases[index]]:3}); const a=identity.slice(); a[index]=3; return matches(a); });
    check('inconsistent aliases atomic ' + names[index], () => { ctx.setTransform(1,2,3,4,5,6); let failed=false; try { ctx.setTransform({[names[index]]:1,[aliases[index]]:2}); } catch (e) { failed=e instanceof owner.TypeError; } return failed && matches([1,2,3,4,5,6]); });
  }
  for (const [label,value] of [['empty',{}],['undefined',undefined],['null',null]])
    check('dictionary identity ' + label, () => { ctx.translate(20,30); ctx.setTransform(value); return matches(identity); });
  check('no argument identity', () => { ctx.translate(20,30); ctx.setTransform(); return matches(identity); });
  check('dictionary from snapshot', () => { ctx.setTransform(1,2,3,4,5,6); const m=get(); ctx.resetTransform(); ctx.setTransform(m); return matches([1,2,3,4,5,6]); });
  for (const [label,value] of [['NaN',NaN],['positive Infinity',Infinity],['negative Infinity',-Infinity]]) {
    check('nonfinite scalar ignored ' + label, () => { ctx.setTransform(1,2,3,4,5,6); ctx.setTransform(value,0,0,1,0,0); return matches([1,2,3,4,5,6]); });
    check('nonfinite dictionary ignored ' + label, () => { ctx.setTransform(1,2,3,4,5,6); ctx.setTransform({a:value,m11:value}); return matches([1,2,3,4,5,6]); });
  }
  check('getter exception preserves state and identity', () => {
    ctx.setTransform(1,2,3,4,5,6); const sentinel={}; let same=false;
    try { ctx.setTransform({a:8,get e(){throw sentinel;}}); } catch(e) {same=e===sentinel;}
    return same && matches([1,2,3,4,5,6]);
  });
  check('beginPath preserves transform', () => { ctx.beginPath(); return matches([1,2,3,4,5,6]); });
  check('fresh independent context', () => matches([1,2,3,4,5,6]) && equal(components(get(makeCanvas().getContext('2d'))),identity));
  check('repeated getContext preserves state', () => canvas.getContext('2d') === ctx && matches([1,2,3,4,5,6]));
  check('width reset', () => { canvas.width = canvas.width; return matches(identity); });
  check('height reset', () => { ctx.translate(20,30); canvas.height = canvas.height; return matches(identity); });
  check('snapshot survives bitmap reset', () => { ctx.setTransform(1,2,3,4,5,6); const m=get(); canvas.width=canvas.width+1; return equal(components(m),[1,2,3,4,5,6]) && matches(identity); });
  check('global constructor spoofing', () => {
    const original=callee.DOMMatrix; let calls=0;
    try { callee.DOMMatrix=function(){calls++; throw new Error('author constructor');}; return Object.getPrototypeOf(get()) === matrixPrototype && calls === 0; }
    finally {callee.DOMMatrix=original;}
  });
  return checks;
})
