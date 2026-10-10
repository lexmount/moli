(async function webLocksContract() {
  const assert = (value, message) => { if (!value) throw Error(message); };
  const rejection = async (promise, expected) => {
    try { await promise; } catch (error) {
      assert(error === expected || error.name === expected, 'rejection identity');
      return;
    }
    throw Error('expected rejection');
  };
  const gate = () => {
    let resolve;
    const promise = new Promise(r => { resolve = r; });
    return {promise, resolve};
  };
  const manager = navigator.locks;
  assert(manager instanceof LockManager && manager === navigator.locks, 'same native manager');
  assert(LockManager.prototype.request.length === 2, 'request arity');
  let called = false;
  const acquired = manager.request('task', lock => {
    called = true;
    assert(lock instanceof Lock && lock.name === 'task' && lock.mode === 'exclusive', 'native lock');
    return 42;
  });
  assert(!called, 'callback must be a task');
  assert(await acquired === 42, 'callback value');
  assert((await manager.query()).held.length === 0, 'release before request settles');
  const error = {then() { throw Error('thrown thenable must not be assimilated'); }};
  await rejection(manager.request('throw', () => { throw error; }), error);
  await rejection(manager.request('-reserved', () => {}), 'NotSupportedError');
  await rejection(manager.request('invalid', {steal:true, mode:'shared'}, () => {}), 'NotSupportedError');
  let conversions = 0;
  await rejection(LockManager.prototype.request.call(new Proxy(manager, {}), {
    toString() { conversions++; return 'proxy'; }
  }, () => {}), 'TypeError');
  assert(conversions === 0, 'receiver check before conversion');
  const first = gate(), writer = gate(), firstEntered = gate(), writerEntered = gate();
  const order = [];
  const a = manager.request('fifo', {mode:'shared'}, async () => {
    order.push('first'); firstEntered.resolve(); await first.promise;
  });
  await firstEntered.promise;
  const b = manager.request('fifo', async () => {
    order.push('writer'); writerEntered.resolve(); await writer.promise;
  });
  const c = manager.request('fifo', {mode:'shared'}, () => { order.push('last'); });
  const unavailable = await manager.request('fifo', {mode:'shared', ifAvailable:true}, lock => {
    assert(lock === null, 'ifAvailable cannot overtake queued writer'); return 'unavailable';
  });
  assert(unavailable === 'unavailable', 'null callback value');
  const queued = await manager.query();
  assert(queued.held.length === 1 && queued.pending.length === 2, 'queue snapshot');
  assert(queued.held[0].clientId && queued.pending.every(lock => lock.clientId === queued.held[0].clientId), 'environment ID');
  first.resolve(); await writerEntered.promise;
  assert(order.join() === 'first,writer', 'writer before later reader');
  writer.resolve(); await Promise.all([a,b,c]);
  assert(order.join() === 'first,writer,last', 'FIFO completion');
  const controller = new AbortController(), reason = {reason:'exact'};
  let abortedCallback = false;
  const aborted = manager.request('abort-before-task', {signal:controller.signal}, () => { abortedCallback = true; });
  controller.abort(reason);
  await rejection(aborted, reason);
  assert(!abortedCallback, 'abort before grant task');
  const holding = gate(), entered = gate();
  const old = manager.request('steal', () => { entered.resolve(); return holding.promise; });
  const oldRejected = rejection(old, 'AbortError');
  await entered.promise;
  const replacement = gate(), replacementEntered = gate();
  const stealing = manager.request('steal', {steal:true}, () => { replacementEntered.resolve(); return replacement.promise; });
  await replacementEntered.promise; await oldRejected;
  holding.resolve();
  await Promise.resolve(); await Promise.resolve();
  assert((await manager.query()).held.length === 1, 'late old completion cannot release replacement');
  replacement.resolve(); await stealing;
  const surrogateGate = gate(), surrogateEntered = gate();
  const surrogate = manager.request('\ud800', () => { surrogateEntered.resolve(); return surrogateGate.promise; });
  await surrogateEntered.promise;
  assert(await manager.request('\ufffd', {ifAvailable:true}, lock => lock.name) === '\ufffd', 'UTF16 names are distinct');
  assert((await manager.query()).held[0].name === '\ud800', 'UTF16 snapshot');
  surrogateGate.resolve(); await surrogate;
  const final = await manager.query();
  assert(final.held.length === 0 && final.pending.length === 0, 'final empty state');
  return 'complete';
})
