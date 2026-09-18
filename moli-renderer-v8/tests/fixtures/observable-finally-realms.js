(async () => {
  const failures = [];
  let checks = 0;
  const check = (value, label) => { checks++; if (!value) failures.push(label); };
  const frame = document.querySelector('iframe'), child = frame.contentWindow;
  const method = child.Observable.prototype.finally, source = Observable.from([1]);
  child.finallyCalls = 0;
  const callback = child.Function('finallyCalls++; globalThis.finallyThis = this; globalThis.finallyArguments = arguments.length;');
  const result = method.call(source, callback);
  check(result instanceof child.Observable && !(result instanceof Observable), 'result belongs to callee realm');
  check(Object.getPrototypeOf(result) === child.Observable.prototype, 'callee intrinsic prototype');
  check(child.finallyCalls === 0, 'foreign callback lazy');
  const values = await result.toArray();
  check(values instanceof child.Array && values[0] === 1, 'callee Array result');
  check(child.finallyThis === child && child.finallyArguments === 0 && child.finallyCalls === 1, 'callback executes in its own realm');
  const local = Observable.prototype.finally.call(child.Observable.from([2]), () => {});
  check(local instanceof Observable && !(local instanceof child.Observable), 'borrowed local method creates local result');
  check(Object.getPrototypeOf(local) === Observable.prototype, 'local intrinsic prototype');
  const localValues = await local.toArray();
  check(localValues instanceof Array && localValues[0] === 2, 'local Array result');
  const revoked = Proxy.revocable(source, {}); revoked.revoke();
  for (const receiver of [{}, Object.create(source), new Proxy(source, {}), revoked.proxy]) {
    let error; try { method.call(receiver, () => {}); } catch (e) { error = e; }
    check(error instanceof child.TypeError && !(error instanceof TypeError), 'invalid receiver uses callee TypeError');
  }
  for (const input of [[], [undefined], [null], [1], [{}]]) {
    let error; try { method.apply(source, input); } catch (e) { error = e; }
    check(error instanceof child.TypeError && !(error instanceof TypeError), 'invalid callback uses callee TypeError');
  }
  const marker = new child.Error('finally callback'); child.finallyError = marker;
  const fail = child.Function('throw finallyError');
  const mainReports = [], childReports = [];
  const onmain = e => { mainReports.push(e.error); e.preventDefault(); };
  const onchild = e => { childReports.push(e.error); e.preventDefault(); };
  addEventListener('error', onmain); child.addEventListener('error', onchild);
  try {
    for (const mode of ['complete', 'error', 'abort', 'pre-abort']) {
      const ac = new AbortController(), reason = {};
      let s, error, complete = 0;
      if (mode === 'pre-abort') ac.abort(reason);
      new Observable(subscriber => { s = subscriber; }).finally(fail)
        .subscribe({error: e => { error = e; }, complete: () => complete++}, {signal: ac.signal});
      if (mode === 'complete') s.complete();
      if (mode === 'error') s.error(reason);
      if (mode === 'abort') ac.abort(reason);
      check(mainReports.length === 0, mode + ' does not report in subscription realm');
      check(childReports.length === 1 && childReports[0] === marker, mode + ' reports original error in callback realm');
      check(error === (mode === 'error' ? reason : undefined) && complete === (mode === 'complete' ? 1 : 0), mode + ' retains original terminal outcome');
      check(!s.active, mode + ' source closed');
      childReports.length = 0;
    }
    let localThis, localCalls = 0;
    await method.call(source, function() { 'use strict'; localThis = this; localCalls++; }).toArray();
    check(localThis === undefined && localCalls === 1, 'foreign binding preserves local strict callback receiver');
  } finally { removeEventListener('error', onmain); child.removeEventListener('error', onchild); }
  Object.setPrototypeOf(source, null);
  const branded = method.call(source, callback);
  check(branded instanceof child.Observable && (await branded.toArray())[0] === 1, 'native brand independent of prototype');
  let s, completed = false;
  new Observable(subscriber => { s = subscriber; }).finally(callback).subscribe({complete: () => { completed = true; }});
  const before = child.finallyCalls;
  frame.remove();
  s.complete();
  check(child.finallyCalls === before && completed, 'retired callback realm is skipped without dropping local completion');
  return {checks, failures};
})()
