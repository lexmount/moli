(async () => {
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const stopped = new IdleDetector(), controller = new AbortController();
  controller.signal.addEventListener = () => { throw Error('author addEventListener'); };
  let indexedSets = 0, promise;
  const previous = Object.getOwnPropertyDescriptor(Array.prototype, '0');
  Object.defineProperty(Array.prototype, '0', {configurable: true, set() { indexedSets++; throw Error('author index setter'); }});
  try { promise = stopped.start({signal: controller.signal}); }
  finally { if (previous) Object.defineProperty(Array.prototype, '0', previous); else delete Array.prototype[0]; }
  await promise;
  assert(indexedSets === 0, 'observer registry does not execute author indexed setters');
  assert(stopped.userState === 'active' && stopped.screenState === 'unlocked', 'initial idle state');
  const active = new IdleDetector(), activeController = new AbortController();
  await active.start({signal: activeController.signal});
  let error;
  await active.start().catch(caught => { error = caught; });
  assert(error && error.name === 'InvalidStateError', 'single active observation');
  const sentinel = {};
  error = undefined;
  await active.start({get threshold() { throw sentinel; }}).catch(caught => { error = caught; });
  assert(error === sentinel, 'argument conversion precedes started state check');
  controller.abort(sentinel);
  assert(stopped.userState === null && stopped.screenState === null && active.userState === 'active', 'abort only stops its detector');
  const restartedController = new AbortController();
  await stopped.start({signal: restartedController.signal});
  controller.abort();
  assert(stopped.userState === 'active', 'old aborted signal does not stop a new observation');
  restartedController.abort();
  assert(stopped.userState === null && stopped.screenState === null, 'abort resets public state after restart');
  globalThis.idleStopped = stopped;
  globalThis.idleActive = active;
  globalThis.idleActiveController = activeController;
  globalThis.idleStoppedChanges = 0;
  globalThis.idleActiveChanges = 0;
  stopped.addEventListener('change', () => idleStoppedChanges++);
  active.addEventListener('change', () => idleActiveChanges++);
  return true;
})()
