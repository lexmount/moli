(async () => {
  const assert = (ok, label) => { if (!ok) throw Error(label); };
  const frame = document.createElement('iframe'); document.body.appendChild(frame);
  const controller = new AbortController();
  const childDetector = new frame.contentWindow.IdleDetector();
  const rootDetector = new IdleDetector();
  await IdleDetector.prototype.start.call(childDetector, {signal: controller.signal});
  await rootDetector.start({signal: controller.signal});
  assert(childDetector.userState === 'active' && rootDetector.userState === 'active', 'both realms observe');
  frame.remove(); await Promise.resolve();
  assert(childDetector.userState === null && childDetector.screenState === null && rootDetector.userState === 'active', 'teardown stops only the retired owner');
  const marker = {}, aborted = new AbortController(); aborted.abort(marker);
  for (const [options, expected] of [[{threshold: 59999}, 'InvalidStateError'], [{signal: aborted.signal}, 'InvalidStateError'],
    [{get signal() { throw marker; }}, marker], [{threshold: Infinity}, 'TypeError']]) {
    let error;
    await IdleDetector.prototype.start.call(childDetector, options).catch(caught => { error = caught; });
    assert(expected === marker ? error === marker : error && error.name === expected, 'retired owner validation priority');
  }
  controller.abort();
  assert(childDetector.userState === null && rootDetector.userState === null, 'source signal keeps the other realm algorithm after observing realm teardown');
  return true;
})()
