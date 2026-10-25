(async () => {
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const child = document.querySelector('iframe').contentWindow;
  const method = child.Observable.prototype.flatMap, source = Observable.from([1, 2]);
  child.mapperCalls = [];
  const callback = child.Function('value', 'index', 'mapperCalls.push([value,index]); globalThis.mapperThis = this; globalThis.mapperArgc = arguments.length; return [value*10];');
  const result = method.call(source, callback);
  check(result instanceof child.Observable && !(result instanceof Observable), 'result uses callee realm');
  check(Object.getPrototypeOf(result) === child.Observable.prototype, 'callee intrinsic prototype');
  check(child.mapperCalls.length === 0, 'foreign mapper lazy');
  const values = await result.toArray();
  check(values instanceof child.Array && values.join(',') === '10,20', 'callee Array result');
  check(child.mapperThis === child && child.mapperArgc === 2, 'callback own realm and argument count');
  check(JSON.stringify(child.mapperCalls) === '[[1,0],[2,1]]', 'foreign mapper indices');
  const local = Observable.prototype.flatMap.call(child.Observable.from([3]), value => [value]);
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
  for (const value of [null, 1, {}, {then() {}}]) {
    const error = await method.call(source, () => value).toArray().catch(e => e);
    check(error instanceof child.TypeError && !(error instanceof TypeError), 'mapper-result conversion error uses callee TypeError');
  }
  const marker = new child.Error('mapper'); child.mapperError = marker;
  for (const mapper of [child.Function('throw mapperError'), () => ({get [Symbol.iterator]() { throw marker; }}),
    () => new child.Observable(s => s.error(marker)), () => child.Promise.reject(marker)]) {
    const error = await method.call(source, mapper).toArray().catch(e => e);
    check(error === marker && error instanceof child.Error, 'callback, conversion and inner errors retain identity');
  }
  const frameSource = new child.Observable(s => { child.pendingOuter = s; });
  const inner = new child.Observable(s => { child.pendingInner = s; });
  const ac = new AbortController(), reason = {}, order = [];
  const pending = Observable.prototype.flatMap.call(frameSource, () => inner).toArray({signal: ac.signal});
  const outcome = pending.catch(e => e);
  child.pendingOuter.next(1);
  child.pendingOuter.addTeardown(() => order.push('outer'));
  child.pendingInner.addTeardown(() => order.push('inner'));
  ac.abort(reason);
  check(await outcome === reason, 'foreign pending graph preserves cancellation reason');
  check(!child.pendingOuter.active && !child.pendingInner.active, 'cancellation reaches both foreign producers');
  check(child.pendingOuter.signal.reason === reason && child.pendingInner.signal.reason === reason, 'foreign producer signals preserve reason');
  check(order.join(',') === 'outer,inner', 'foreign cleanup order');
  Object.setPrototypeOf(source, null);
  const branded = method.call(source, value => [value]);
  check(branded instanceof child.Observable && (await branded.toArray()).join(',') === '1,2', 'native brand survives prototype replacement');
  return {checks, failures};
})()
