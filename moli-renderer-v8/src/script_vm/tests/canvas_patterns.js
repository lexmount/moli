function probe(owner, callee, kind, label) {
  const rows = [];
  const check = (name, f) => {
    try { if (f() === false) throw Error('false result'); rows.push({name: label + '/' + name, passed: true}); }
    catch (e) { rows.push({name: label + '/' + name, passed: false, message: String(e)}); }
  };
  const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(8, 8);
  canvas.width = canvas.height = 8;
  const ctx = canvas.getContext('2d');
  const prototype = callee[kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D'].prototype;
  const create = prototype.createPattern;
  const set = callee.CanvasPattern.prototype.setTransform;
  const tile = new owner.OffscreenCanvas(2, 2), tileCtx = tile.getContext('2d');
  tileCtx.fillStyle = 'red'; tileCtx.fillRect(0, 0, 2, 2);
  const pattern = () => create.call(ctx, tile, 'no-repeat');
  const bitmapSource = () => { const source = new owner.OffscreenCanvas(1, 1); source.getContext('2d'); return source.transferToImageBitmap(); };
  const pixel = (x, y) => Array.from(ctx.getImageData(x, y, 1, 1).data).join();
  const paint = p => { ctx.reset(); ctx.imageSmoothingEnabled = false; ctx.fillStyle = p; ctx.fillRect(0, 0, 8, 8); };
  const throws = (ctor, f) => { try { f(); } catch (e) { if (e instanceof ctor) return true; throw e; } throw Error('no exception'); };
  check('factory descriptors', () => create.name === 'createPattern' && create.length === 2 && set.name === 'setTransform' && set.length === 0);
  check('owner realm', () => Object.getPrototypeOf(pattern()) === owner.CanvasPattern.prototype);
  check('constructor illegal', () => throws(callee.TypeError, () => new callee.CanvasPattern()));
  check('arity', () => throws(callee.TypeError, () => create.call(ctx, tile)));
  check('null repetition', () => { paint(create.call(ctx, tile, null)); return pixel(7, 7) === '255,0,0,255'; });
  check('empty repetition', () => { paint(create.call(ctx, tile, '')); return pixel(7, 7) === '255,0,0,255'; });
  for (const [mode, horizontal, vertical] of [['repeat', true, true], ['repeat-x', true, false], ['repeat-y', false, true], ['no-repeat', false, false]]) {
    check(mode, () => { paint(create.call(ctx, tile, mode)); return pixel(4, 0) === (horizontal ? '255,0,0,255' : '0,0,0,0') && pixel(0, 4) === (vertical ? '255,0,0,255' : '0,0,0,0'); });
  }
  check('syntax error callee realm', () => { try { create.call(ctx, tile, 'Repeat'); } catch (e) { return e instanceof callee.DOMException && e.name === 'SyntaxError'; } return false; });
  check('undefined repetition', () => { try { create.call(ctx, tile, undefined); } catch (e) { return e.name === 'SyntaxError'; } return false; });
  check('style identity and no conversion', () => { const p = pattern(); p.toString = () => { throw Error('converted'); }; ctx.fillStyle = p; ctx.strokeStyle = p; return ctx.fillStyle === p && ctx.strokeStyle === p; });
  check('snapshot immutable', () => { const p = pattern(); tileCtx.fillStyle = 'blue'; tileCtx.fillRect(0, 0, 2, 2); paint(p); tileCtx.fillStyle = 'red'; tileCtx.fillRect(0, 0, 2, 2); return pixel(0, 0) === '255,0,0,255'; });
  check('live pattern transform', () => { const p = pattern(); paint(p); set.call(p, {e: 2}); ctx.clearRect(0, 0, 8, 8); ctx.fillRect(0, 0, 8, 8); return pixel(0, 0) === '0,0,0,0' && pixel(2, 0) === '255,0,0,255'; });
  check('drawing CTM', () => { const p = pattern(); paint(p); ctx.clearRect(0, 0, 8, 8); ctx.translate(2, 0); ctx.fillRect(-2, 0, 8, 8); return pixel(0, 0) === '0,0,0,0' && pixel(2, 0) === '255,0,0,255'; });
  check('creation CTM ignored', () => { ctx.reset(); ctx.translate(5, 0); const p = pattern(); paint(p); return pixel(0, 0) === '255,0,0,255' && pixel(5, 0) === '0,0,0,0'; });
  check('reset transform default', () => { const p = pattern(); set.call(p, {e: 4}); set.call(p); paint(p); return pixel(0, 0) === '255,0,0,255'; });
  check('invalid aliases', () => throws(callee.TypeError, () => set.call(pattern(), {a: 1, m11: 2})));
  check('matrix primitive', () => throws(callee.TypeError, () => set.call(pattern(), 1)));
  check('nonfinite transform ignored', () => { const p = pattern(); set.call(p, {e: 2}); set.call(p, {a: Infinity}); paint(p); return pixel(2, 0) === '255,0,0,255' && pixel(0, 0) === '0,0,0,0'; });
  check('conversion precedes finite validation', () => { const sentinel = {}; try { set.call(pattern(), {a: Infinity, get m42() {throw sentinel;}}); } catch (e) {return e === sentinel;} return false; });
  check('singular transform', () => { const p = pattern(); set.call(p, {a: 0, d: 0}); paint(p); return pixel(0, 0) === '0,0,0,0'; });
  check('source checked before repetition', () => { let converted = 0; throws(callee.TypeError, () => create.call(ctx, {}, {toString() {converted++; return 'repeat';}})); return converted === 0; });
  check('zero canvas before syntax validation', () => { const empty = new owner.OffscreenCanvas(0, 1); try { create.call(ctx, empty, 'bad'); } catch (e) {return e instanceof callee.DOMException && e.name === 'InvalidStateError';} return false; });
  check('repetition conversion may resize source', () => { const empty = new owner.OffscreenCanvas(1, 1); try { create.call(ctx, empty, {toString() {empty.width = 0; return 'repeat';}}); } catch (e) {return e.name === 'InvalidStateError';} return false; });
  check('ImageBitmap snapshot', () => { const source = new owner.OffscreenCanvas(2, 2), c = source.getContext('2d'); c.fillStyle = 'lime'; c.fillRect(0, 0, 2, 2); const bitmap = source.transferToImageBitmap(); const p = create.call(ctx, bitmap, 'repeat'); bitmap.close(); paint(p); return pixel(7, 7) === '0,255,0,255'; });
  check('closed ImageBitmap', () => { const bitmap = bitmapSource(); bitmap.close(); try { create.call(ctx, bitmap, 'repeat'); } catch (e) {return e.name === 'InvalidStateError';} return false; });
  check('repetition conversion may close bitmap', () => { const bitmap = bitmapSource(); try { create.call(ctx, bitmap, {toString() {bitmap.close(); return 'repeat';}}); } catch (e) {return e.name === 'InvalidStateError';} return false; });
  check('global alpha', () => { const p = pattern(); ctx.reset(); ctx.globalAlpha = .5; ctx.fillStyle = p; ctx.fillRect(0, 0, 8, 8); return pixel(0, 0) === '255,0,0,128'; });
  check('clip and restore retain pattern identity', () => { const p = pattern(); ctx.reset(); ctx.fillStyle = p; ctx.save(); ctx.beginPath(); ctx.rect(0, 0, 1, 1); ctx.clip(); ctx.fillRect(0, 0, 8, 8); ctx.restore(); return ctx.fillStyle === p && pixel(0, 0) === '255,0,0,255' && pixel(1, 0) === '0,0,0,0'; });
  for (const [name, receiver] of [['plain', {}], ['forged', Object.create(callee.CanvasPattern.prototype)], ['inherited', Object.create(pattern())], ['authorProxy', new owner.Proxy(pattern(), {})]]) {
    check(name + ' receiver brand before conversion', () => { let converted = 0; throws(callee.TypeError, () => set.call(receiver, {get a() {converted++; return 1;}})); return converted === 0; });
  }
  check('revoked receiver before conversion', () => { const p = owner.Proxy.revocable(pattern(), {}); p.revoke(); let converted = 0; throws(callee.TypeError, () => set.call(p.proxy, {get a() {converted++; return 1;}})); return converted === 0; });
  check('author source proxy before conversion', () => { let trapped = 0, converted = 0; const source = new owner.Proxy(tile, {get() {trapped++; throw Error('trap');}}); throws(callee.TypeError, () => create.call(ctx, source, {toString() {converted++; return 'repeat';}})); return trapped === 0 && converted === 0; });
  check('author context proxy before conversion', () => { let converted = 0; throws(callee.TypeError, () => create.call(new owner.Proxy(ctx, {}), tile, {toString() {converted++; return 'repeat';}})); return converted === 0; });
  return rows;
}
