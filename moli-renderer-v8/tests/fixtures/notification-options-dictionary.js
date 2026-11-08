(() => {
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const members = ['actions','badge','body','data','dir','icon','image','lang',
    'renotify','requireInteraction','silent','tag','timestamp','vibrate'];
  const trace = [], sentinel = {};
  const options = {};
  for (const key of members) Object.defineProperty(options, key, {
    configurable: true, get() { trace.push(key); }
  });
  const notification = new Notification('audit', options);
  notification.close();
  assert(trace.join() === members.join(), 'NotificationOptions lexical order: ' + trace);
  for (const stop of members) {
    trace.length = 0;
    const bad = Object.create(options);
    Object.defineProperty(bad, stop, {get() { trace.push(stop); throw sentinel; }});
    let error;
    try { new Notification('audit', bad); } catch (caught) { error = caught; }
    assert(error === sentinel, stop + ' original exception');
    assert(trace.join() === members.slice(0, members.indexOf(stop) + 1).join(), stop + ' short circuit');
  }
  for (const value of [1, true, 'options', 1n, Symbol('options')]) {
    let error;
    try { new Notification('audit', value); } catch (caught) { error = caught; }
    assert(error instanceof TypeError, 'primitive dictionary rejected');
  }
  let iterations = 0, nexts = 0;
  const empty = {[Symbol.iterator]() {
    iterations++;
    return {next() { nexts++; return {done: true}; }};
  }};
  const emptyActions = new Notification('empty', {actions: empty});
  emptyActions.close();
  assert(iterations === 1 && nexts === 1, 'empty actions iterable must actually be consumed');
  let index = 0, late = 0, error;
  const actions = {[Symbol.iterator]() { return {next() {
    if (index++ < 2) return {value: {action: 'a', title: 'A'}};
    if (index === 3) return {value: {action: 'bad', get title() { throw sentinel; }}};
    throw Error('must not read after failed item');
  }}; }};
  try { new Notification('bad', {actions, get badge() { late++; }}); } catch (caught) { error = caught; }
  assert(error === sentinel && index === 3 && late === 0, 'convert every action before limits, stop at failing item');
  for (const actions of [null, [{}], [{action: 'a'}]]) {
    let error;
    try { new Notification('bad', {actions}); } catch (caught) { error = caught; }
    assert(error instanceof TypeError, 'actions sequence and required dictionary members');
  }
  const data = {value: 'before'};
  const cloned = new Notification('clone', {data, get tag() { data.value = 'after'; return 'tag'; }});
  assert(cloned.data !== data && cloned.data.value === 'after', 'clone data after complete dictionary conversion');
  cloned.close();
  const deferred = {get poison() { throw Error('clone must wait'); }};
  error = undefined;
  try { new Notification('clone', {data: deferred, get tag() { throw sentinel; }}); } catch (caught) { error = caught; }
  assert(error === sentinel, 'later conversion error precedes data cloning');
  return true;
})()
