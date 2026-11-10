// Promise jobs continue after a Window is retired (HTML PR #11441).
// Keep the older pinned WPT assertions unchanged; this guards the current semantics.
async function () {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const checks = [];
  const check = (name, passed) => checks.push({name, passed: !!passed});
  const turn = () => new Promise(resolve => setTimeout(resolve, 0));
  const frame = document.createElement('iframe');
  body.appendChild(frame);
  const child = frame.contentWindow;
  const childArray = child.Array;
  const childPromise = child.Promise;
  const pureFunction = child.Function('return 42');
  const globalFunction = child.Function('return typeof Array === "function" && Array === globalThis.Array');
  const state = child.eval(`(() => {
    const state = {fulfilled: false, rejected: false};
    const fulfilled = Promise.resolve(1);
    const rejected = Promise.reject(1);
    fulfilled.then(() => { state.fulfilled = true; });
    rejected.catch(() => { state.rejected = true; });
    return {state, fulfilled, rejected};
  })()`);
  let parentFulfilled = false;
  let parentRejected = false;
  let parentMicrotask = false;
  let childMicrotask = false;
  const childMicrotaskCallback = child.Function('state', 'return () => { state.called = true; }')({
    set called(value) { childMicrotask = value; }
  });
  queueMicrotask(childMicrotaskCallback);
  child.queueMicrotask(() => { parentMicrotask = true; });
  frame.remove();
  state.fulfilled.then(() => { parentFulfilled = true; });
  state.rejected.catch(() => { parentRejected = true; });
  const assimilated = [];
  const track = (name, promise) => {
    const item = {name, called: false};
    assimilated.push(item);
    promise.then(() => { item.called = true; }, () => { item.called = true; });
  };
  track('resolve fulfilled thenable', Promise.resolve(state.fulfilled));
  track('resolve rejected thenable', Promise.resolve(state.rejected));
  track('fulfillment returns child promise', Promise.resolve(1).then(() => state.fulfilled));
  track('rejection returns child promise', Promise.reject(1).catch(() => state.fulfilled));
  track('Promise.all', Promise.all([state.fulfilled]));
  track('Promise.allSettled', Promise.allSettled([state.fulfilled]));
  track('Promise.any', Promise.any([state.fulfilled]));
  track('Promise.race', Promise.race([state.fulfilled]));
  let rejectFulfilled = false;
  let rejectRejected = false;
  Promise.reject(state.fulfilled).catch(() => { rejectFulfilled = true; });
  Promise.reject(state.rejected).catch(() => { rejectRejected = true; });
  let forwarded = false;
  state.fulfilled.then().then(() => { forwarded = true; });
  await turn();
  check('fulfillment job continues after Window retirement', state.state.fulfilled);
  check('rejection job continues after Window retirement', state.state.rejected);
  for (const item of assimilated) check(item.name + ' continues after Window retirement', item.called);
  check('live fulfillment callback still runs', parentFulfilled);
  check('live rejection callback still runs', parentRejected);
  check('Promise.reject does not assimilate fulfilled promise', rejectFulfilled);
  check('Promise.reject does not assimilate rejected promise', rejectRejected);
  check('null-handler reaction still forwards to a live callback', forwarded);
  check('live queueMicrotask callback survives target retirement', parentMicrotask);
  check('retired queueMicrotask callback is skipped', !childMicrotask);
  try {
    check('retained Window constructors remain readable', child.Array === childArray && child.Promise === childPromise);
    check('retained Window constructor remains callable', new child.Array(2).length === 2);
  } catch (error) {
    check('retained Window constructors remain readable and callable: ' + error, false);
  }
  try {
    check('retained plain function remains callable', pureFunction() === 42);
    check('retained function keeps global lookups', globalFunction());
  } catch (error) {
    check('retained function remains callable with globals: ' + error, false);
  }
  const active = document.createElement('iframe');
  body.appendChild(active);
  const events = [];
  const childCallback = active.contentWindow.Function('events', 'return () => events.push("child")')(events);
  Promise.resolve().then(() => events.push('parent-first'));
  active.contentWindow.Promise.resolve().then(childCallback);
  Promise.resolve().then(() => events.push('parent-last'));
  await turn();
  check('active Window jobs share agent FIFO', JSON.stringify(events) === '["parent-first","child","parent-last"]');
  active.remove();
  return {completed: true, checks, failures: checks.filter(item => !item.passed).map(item => item.name)};
}
