(async () => {
  const failures = []; let checks = 0;
  const check = (value, name) => { checks++; if (!value) failures.push(name); };
  const child = document.querySelector('iframe').contentWindow;
  const method = child.Observable.prototype.catch, marker = {}, source = new Observable(s => s.error(marker));
  child.recoveryValue = 10;
  const callback = child.Function('error', 'globalThis.catcherThis = this; globalThis.catcherArgc = arguments.length; globalThis.catcherError = error; return [recoveryValue];');
  const result = method.call(source, callback);
  check(result instanceof child.Observable && !(result instanceof Observable), 'result uses callee realm');
  check(Object.getPrototypeOf(result) === child.Observable.prototype, 'callee intrinsic prototype');
  check(child.catcherArgc === undefined, 'foreign callback lazy');
  const values = await result.toArray();
  check(values instanceof child.Array && values[0] === 10, 'callee Array result');
  check(child.catcherThis === child && child.catcherArgc === 1, 'callback own realm and argument count');
  check(child.catcherError === marker, 'foreign callback receives original error');
  const local = Observable.prototype.catch.call(new child.Observable(s => s.error(3)), e => [e]);
  check(local instanceof Observable && !(local instanceof child.Observable), 'local method returns local Observable');
  check(Object.getPrototypeOf(local) === Observable.prototype, 'local intrinsic prototype');
  const localValues = await local.toArray();
  check(localValues instanceof Array && localValues[0] === 3, 'local Array result');
  const revoked = Proxy.revocable(source, {}); revoked.revoke();
  for (const receiver of [{}, Object.create(source), new Proxy(source, {}), revoked.proxy]) {
    let error; try { method.call(receiver, () => []); } catch (e) { error = e; }
    check(error instanceof child.TypeError && !(error instanceof TypeError), 'receiver check uses callee TypeError');
  }
  for (const input of [[], [undefined], [null], [1], [{}]]) {
    let error; try { method.apply(source, input); } catch (e) { error = e; }
    check(error instanceof child.TypeError && !(error instanceof TypeError), 'callback conversion uses callee TypeError');
  }
  for (const input of [null, 1, {}, {then() {}}]) {
    const error = await method.call(source, () => input).toArray().catch(e => e);
    check(error instanceof child.TypeError && !(error instanceof TypeError), 'recovery conversion uses callee TypeError');
  }
  const error = new child.Error('recovery'); child.recoveryError = error;
  for (const callback of [child.Function('throw recoveryError'), () => ({get [Symbol.iterator]() { throw error; }}),
    () => new child.Observable(s => s.error(error)), () => child.Promise.reject(error)]) {
    check(await method.call(source, callback).toArray().catch(e => e) === error, 'callback, conversion and replacement error identity');
  }
  for (const phase of ['source', 'recovery']) {
    let outer, inner;
    const foreignSource = new child.Observable(s => { outer = s; });
    const foreignInner = new child.Observable(s => { inner = s; });
    const ac = new AbortController(), reason = {}, log = [];
    const pending = Observable.prototype.catch.call(foreignSource, () => foreignInner).toArray({signal: ac.signal});
    const outcome = pending.catch(e => e);
    outer.addTeardown(() => log.push('source'));
    if (phase === 'recovery') { outer.error(marker); inner.addTeardown(() => log.push('inner')); }
    ac.abort(reason);
    check(await outcome === reason, phase + ' cancellation rejection identity');
    check(!outer.active && (!inner || !inner.active), phase + ' foreign producers close');
    check((inner || outer).signal.reason === reason, phase + ' foreign signal reason identity');
    check(log.join(',') === (phase === 'source' ? 'source' : 'source,inner'), phase + ' foreign cleanup order');
  }
  Object.setPrototypeOf(source, null);
  const branded = method.call(source, () => [1]);
  check(branded instanceof child.Observable && (await branded.toArray())[0] === 1, 'native brand survives prototype replacement');
  return {checks, failures};
})()
