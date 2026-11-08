(async () => {
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const rejection = async (detector, options) => {
    let promise;
    try { promise = detector.start(options); } catch (error) { throw Error('start threw synchronously'); }
    assert(promise instanceof Promise, 'start Promise');
    let rejected = false, error;
    await promise.then(() => {}, caught => { rejected = true; error = caught; });
    assert(rejected, 'start must reject');
    return error;
  };
  const members = ['signal','threshold'];
  for (const stop of members) for (const sentinel of [undefined, null, false, 0, 'sentinel', {}]) {
    const trace = [], options = {};
    for (const key of members) Object.defineProperty(options, key, {get() {
      trace.push(key); if (key === stop) throw sentinel;
    }});
    const detector = new IdleDetector();
    const error = await rejection(detector, options);
    assert(Object.is(error, sentinel) && trace.join() === members.slice(0, members.indexOf(stop) + 1).join(), stop + ' original rejection and short circuit');
    assert(detector.userState === null && detector.screenState === null, 'failed conversion does not start observation');
  }
  for (const signal of [null, {}, new Proxy(new AbortController().signal, {})]) {
    let late = 0;
    const error = await rejection(new IdleDetector(), {signal, get threshold() { late++; }});
    assert(error instanceof TypeError && late === 0, 'native signal conversion before threshold');
  }
  for (const threshold of [-1, Infinity, NaN, 1n, Symbol('threshold'), 59999]) {
    assert(await rejection(new IdleDetector(), {threshold}) instanceof TypeError, 'threshold range and algorithm minimum');
  }
  for (const options of [1, true, 'options', 1n, Symbol('options')]) {
    assert(await rejection(new IdleDetector(), options) instanceof TypeError, 'primitive dictionary');
  }
  const reason = {}, controller = new AbortController(); controller.abort(reason);
  const detector = new IdleDetector();
  assert(await rejection(detector, {signal: controller.signal}) === reason, 'abort reason before permission check');
  const conversionError = {};
  assert(await rejection(detector, {signal: controller.signal, get threshold() { throw conversionError; }}) === conversionError,
    'complete dictionary conversion before abort algorithm');
  let reads = 0, promise;
  try { promise = IdleDetector.prototype.start.call({}, {get signal() { reads++; }}); } catch (error) { throw Error('receiver failure threw synchronously'); }
  let error;
  await promise.catch(caught => { error = caught; });
  assert(error instanceof TypeError && reads === 0, 'receiver checked before options in rejected Promise');
  return true;
})()
