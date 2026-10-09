async function(owner, callee, label) {
  const checks = [];
  const check = (name, passed, detail) => checks.push({name:label + ': ' + name, passed:!!passed, ...(passed ? {} : {detail:String(detail ?? '')})});
  const method = callee.OffscreenCanvas.prototype.convertToBlob;
  const bytes = async blob => blob ? new Uint8Array(await blob.arrayBuffer()) : new Uint8Array();
  const png = data => data.length > 24 && [137,80,78,71,13,10,26,10].every((value,index) => data[index] === value);
  const dimension = (data, offset) => data.length > offset + 3 ? data[offset]*16777216 + data[offset+1]*65536 + data[offset+2]*256 + data[offset+3] : 0;
  async function blob(name, canvas, options) {
    try { return await method.call(canvas, options); }
    catch(error) { check(name + ' exports', false, error); return null; }
  }
  async function rejects(name, receiver, options, ErrorClass, expected) {
    let promise, caught, synchronous = false;
    try { promise = method.call(receiver, options); }
    catch(error) { caught = error; synchronous = true; }
    check(name + ' returns callee Promise', !synchronous && promise instanceof callee.Promise, caught);
    if (!synchronous) {
      try { await promise; } catch(error) { caught = error; }
    }
    check(name + ' rejection', !synchronous && (ErrorClass ? caught instanceof ErrorClass && caught.name === expected : caught === expected), caught);
  }

  const descriptor = Object.getOwnPropertyDescriptor(callee.OffscreenCanvas.prototype, 'convertToBlob');
  check('descriptor', descriptor.enumerable && descriptor.configurable && descriptor.writable && method.name === 'convertToBlob' && method.length === 0);
  const canvas = new owner.OffscreenCanvas(2, 1);
  const context = canvas.getContext('2d');
  context.fillStyle = '#00ff00';
  context.fillRect(0, 0, 2, 1);
  let settled = false;
  const pending = method.call(canvas);
  check('export Promise realm', pending instanceof callee.Promise);
  pending.then(() => { settled = true; }, () => { settled = true; });
  await callee.Promise.resolve();
  check('settlement waits for a task', !settled);
  canvas.width = 3;
  context.fillStyle = '#ff0000';
  context.fillRect(0, 0, 3, 1);
  const snapshot = await pending;
  const snapshotBytes = await bytes(snapshot);
  check('snapshot Blob realm', snapshot instanceof owner.Blob);
  check('snapshot PNG', snapshot.type === 'image/png' && png(snapshotBytes));
  check('snapshot dimensions before resize', dimension(snapshotBytes,16) === 2 && dimension(snapshotBytes,20) === 1);

  for (const [name, options] of [
    ['undefined',undefined], ['null',null], ['empty',{}], ['array',[]], ['callable',function(){}],
    ['uppercase',{type:'IMAGE/PNG'}], ['unsupported',{type:'image/not-supported'}],
    ['parameters',{type:'image/jpeg;quality=1'}],
  ]) {
    const exported = await blob(name, canvas, options);
    check(name + ' MIME fallback', exported?.type === 'image/png');
    check(name + ' relevant Blob realm', exported instanceof owner.Blob);
    check(name + ' PNG file', png(await bytes(exported)));
  }
  for (const [name, quality] of [
    ['absent',undefined], ['null',null], ['string','0.2'], ['object',{valueOf(){return 0.5;}}],
    ['NaN',NaN], ['Infinity',Infinity], ['negative',-1], ['above range',2],
  ]) {
    const exported = await blob('JPEG '+name, canvas, {type:'IMAGE/JPEG',quality});
    const data = await bytes(exported);
    check('JPEG '+name+' MIME', exported?.type === 'image/jpeg');
    check('JPEG '+name+' realm', exported instanceof owner.Blob);
    check('JPEG '+name+' file', data[0] === 255 && data[1] === 216 && data[data.length-2] === 255 && data[data.length-1] === 217);
  }
  for (const [name, options] of [['boolean',false], ['number',1], ['string','png'], ['symbol',Symbol('x')], ['bigint',1n]]) {
    await rejects('dictionary '+name, canvas, options, callee.TypeError, 'TypeError');
  }
  for (const member of ['quality','type']) {
    const marker = new owner.Error(member);
    await rejects(member+' getter', canvas, {[member]:undefined, get [member](){throw marker;}}, null, marker);
  }
  await rejects('quality Symbol', canvas, {quality:Symbol('quality')}, callee.TypeError, 'TypeError');
  await rejects('type Symbol', canvas, {type:Symbol('type')}, callee.TypeError, 'TypeError');
  let conversions = 0, traps = 0;
  const options = {get quality(){conversions++;return 0.5;},get type(){conversions++;return 'image/png';}};
  const proxy = new owner.Proxy(canvas, {get(){traps++;},getPrototypeOf(){traps++;}});
  const revoked = owner.Proxy.revocable(canvas, {});
  revoked.revoke();
  for (const [name, receiver] of [['plain',{}], ['inherited',owner.Object.create(canvas)], ['prototype',owner.OffscreenCanvas.prototype], ['proxy',proxy], ['revoked',revoked.proxy], ['null',null]]) {
    await rejects('receiver '+name, receiver, options, callee.TypeError, 'TypeError');
  }
  check('receiver check precedes conversion', conversions === 0, conversions);
  check('receiver check does not execute Proxy traps', traps === 0, traps);
  for (const [width,height] of [[0,1],[1,0],[0,0]]) {
    await rejects('empty '+width+'x'+height, new owner.OffscreenCanvas(width,height), {}, callee.DOMException, 'IndexSizeError');
  }

  const converted = new owner.OffscreenCanvas(0,0);
  const convertedContext = converted.getContext('2d');
  const order = [];
  const convertedBlob = await blob('converted', converted, {
    get quality(){order.push('get quality');return {valueOf(){order.push('number');converted.width=2;converted.height=1;return 0.2;}};},
    get type(){order.push('get type');return {toString(){order.push('string');convertedContext.fillStyle='#0000ff';convertedContext.fillRect(0,0,2,1);return 'IMAGE/PNG';}};},
  });
  check('dictionary member conversion order', order.join(',') === 'get quality,number,get type,string', order);
  check('state checked after conversion', png(await bytes(convertedBlob)));
  const changedPrototype = new owner.OffscreenCanvas(1,1);
  owner.Object.setPrototypeOf(changedPrototype, null);
  check('native identity survives prototype change', (await blob('native identity',changedPrototype,{})) instanceof owner.Blob);
  return {checks, snapshot, converted:convertedBlob};
}
