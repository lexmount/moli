(async registration => {
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const members = ['actions','badge','body','data','dir','icon','image','lang',
    'renotify','requireInteraction','silent','tag','timestamp','vibrate'];
  const rejection = async (operation, expected, label) => {
    let promise;
    try { promise = operation(); } catch (error) { throw Error(label + ' threw synchronously'); }
    assert(promise instanceof Promise, label + ' Promise');
    let rejected = false, error;
    await promise.then(() => {}, caught => { rejected = true; error = caught; });
    assert(rejected && Object.is(error, expected), label + ' original rejection');
  };
  for (const stop of members) for (const sentinel of [undefined, null, false, 0, 'sentinel', {}]) {
    const trace = [];
    const options = {};
    for (const key of members) Object.defineProperty(options, key, {get() {
      trace.push(key);
      if (key === stop) throw sentinel;
    }});
    await rejection(() => registration.showNotification('audit', options), sentinel, stop);
    assert(trace.join() === members.slice(0, members.indexOf(stop) + 1).join(), stop + ' short circuit: ' + trace);
  }
  const sentinel = {};
  let index = 0, late = 0;
  const actions = {[Symbol.iterator]() { return {next() {
    if (index++ < 2) return {value: {action: 'a', title: 'A'}};
    if (index === 3) return {value: {action: 'bad', get title() { throw sentinel; }}};
    throw Error('must not read after failed item');
  }}; }};
  await rejection(() => registration.showNotification('audit', {actions, get data() { late++; }}), sentinel, 'third action');
  assert(index === 3 && late === 0, 'validate actions beyond display limit before data');
  await rejection(() => registration.getNotifications({get tag() { throw sentinel; }}), sentinel, 'getNotifications tag');
  for (const value of [1, true, 'options', 1n, Symbol('options')]) {
    for (const operation of [() => registration.showNotification('audit', value), () => registration.getNotifications(value)]) {
      let promise;
      try { promise = operation(); } catch (error) { throw Error('primitive options threw synchronously'); }
      let error;
      await promise.catch(caught => { error = caught; });
      assert(error instanceof TypeError, 'primitive dictionary must reject TypeError');
    }
  }
  let titleReads = 0;
  const title = {toString() { titleReads++; throw sentinel; }};
  await rejection(() => registration.showNotification(title, {get actions() { throw Error('late options'); }}), sentinel, 'title');
  assert(titleReads === 1, 'title converted before options');
  return true;
})
