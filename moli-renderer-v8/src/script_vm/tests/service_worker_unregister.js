globalThis.unregisteredReplacementProbe = {phase: 'running', events: [], result: {}};
(async () => {
  const probe = globalThis.unregisteredReplacementProbe;
  const sw = navigator.serviceWorker;
  const activated = async registration => {
    const worker = registration.installing || registration.waiting || registration.active;
    if (!worker) throw new Error('registration has no worker');
    if (worker.state !== 'activated') await new Promise((resolve, reject) => {
      worker.addEventListener('statechange', () => {
        if (worker.state === 'activated') resolve();
        if (worker.state === 'redundant') reject(new Error('worker became redundant'));
      });
    });
    return worker;
  };
  const oldRegistration = await sw.register('/controlled/sw.js', {scope: '/controlled/'});
  const oldWorker = await activated(oldRegistration);
  probe.oldRegistration = oldRegistration;
  probe.oldWorker = oldWorker;
  const frame = document.createElement('iframe');
  const ready = new Promise(resolve => {
    const listener = event => {
      if (event.source === frame.contentWindow && event.data === 'controlled-ready') {
        removeEventListener('message', listener);
        resolve();
      }
    };
    addEventListener('message', listener);
  });
  frame.src = '/controlled/client.html';
  document.body.appendChild(frame);
  await ready;
  const controller = frame.contentWindow.navigator.serviceWorker.controller;
  if (!controller || controller.state !== 'activated') throw new Error('initial iframe is not controlled');
  let oldUpdateFound = 0;
  oldRegistration.addEventListener('updatefound', () => oldUpdateFound++);
  oldWorker.addEventListener('statechange', () => {
    if (oldWorker.state === 'redundant') {
      probe.events.push('event:' + (oldRegistration.active === oldWorker));
      Promise.resolve().then(() => {
        probe.events.push('microtask:' + (oldRegistration.active === oldWorker));
      });
    }
  });
  const result = probe.result;
  result.unregistered = await oldRegistration.unregister();
  result.oldWorkerRetained = oldRegistration.active === oldWorker && oldWorker.state === 'activated';
  result.oldControllerRetained = frame.contentWindow.navigator.serviceWorker.controller === controller;
  result.lookupAbsent = await sw.getRegistration('/controlled/client.html') === undefined;
  let rejected = false;
  try {
    await sw.register('/controlled/invalid.js', {scope: '/controlled/'});
  } catch (error) {
    rejected = error instanceof TypeError;
  }
  result.failedReplacementRejected = rejected;
  result.failedReplacementKeepsOldWorker = oldRegistration.active === oldWorker && oldWorker.state === 'activated';
  result.failedReplacementStaysUnregistered = await sw.getRegistration('/controlled/client.html') === undefined;
  const replacement = await sw.register('/controlled/sw.js', {scope: '/controlled/'});
  probe.replacement = replacement;
  const replacementWorker = replacement.installing || replacement.waiting || replacement.active;
  result.newRegistration = replacement !== oldRegistration;
  result.newWorker = !!replacementWorker && replacementWorker !== oldWorker;
  await activated(replacement);
  result.oldWorkerStillActive = oldRegistration.active === oldWorker && oldWorker.state === 'activated';
  result.oldRegistrationHasNoInstallingWorker = oldRegistration.installing === null;
  result.oldRegistrationHasNoWaitingWorker = oldRegistration.waiting === null;
  result.oldRegistrationReceivedNoUpdateFound = oldUpdateFound === 0;
  result.controllerRetainedAfterReplacement = frame.contentWindow.navigator.serviceWorker.controller === controller;
  result.oldControllerStillHandlesFetch = await frame.contentWindow.fetch('/controlled/echo').then(response => response.text()) === 'controlled-fetch';
  frame.remove();
  result.replacementSurvivesOldClientRemoval = await sw.getRegistration('/controlled/client.html') === replacement;
  probe.phase = 'registered';
})().catch(error => {
  unregisteredReplacementProbe.error = String(error);
  unregisteredReplacementProbe.phase = 'error';
});
