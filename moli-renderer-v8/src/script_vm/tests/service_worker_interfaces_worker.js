self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
self.addEventListener('message', event => event.waitUntil((async () => {
  const checks = [];
  const verify = (name, ok) => checks.push({ name, pass: !!ok });
  const reg = self.registration, manager = reg.navigationPreload, active = reg.active;
  verify('worker/registration-prototype', Object.getPrototypeOf(reg) === ServiceWorkerRegistration.prototype && reg instanceof EventTarget);
  verify('worker/registration-shared-method', reg.update === ServiceWorkerRegistration.prototype.update && !Object.hasOwn(reg, 'update'));
  verify('worker/registration-attributes', !Object.hasOwn(reg, 'scope') && reg.updateViaCache === 'imports');
  verify('worker/service-worker-prototype', Object.getPrototypeOf(active) === ServiceWorker.prototype && active instanceof EventTarget);
  verify('worker/service-worker-attributes', !Object.hasOwn(active, 'state') && active.state === 'activated' && active.scriptURL.endsWith('/app/worker.js'));
  verify('worker/preload-prototype', Object.getPrototypeOf(manager) === NavigationPreloadManager.prototype && !Object.hasOwn(manager, 'enable'));
  await NavigationPreloadManager.prototype.setHeaderValue.call(manager, 'from-worker');
  verify('worker/preload-native-state', (await NavigationPreloadManager.prototype.getState.call(manager)).headerValue === 'from-worker');
  let conversions = 0;
  const promise = NavigationPreloadManager.prototype.setHeaderValue.call({}, { toString() { conversions++; return 'x'; } });
  let error; try { await promise; } catch (e) { error = e; }
  verify('worker/preload-brand', promise instanceof Promise && error instanceof TypeError && conversions === 0);
  const order = [], before = () => order.push('before'), after = () => order.push('after');
  reg.addEventListener('updatefound', before);
  reg.onupdatefound = () => order.push('handler');
  reg.addEventListener('updatefound', after);
  reg.dispatchEvent(new Event('updatefound'));
  verify('worker/native-event-target', order.join() === 'before,handler,after' && reg.addEventListener === EventTarget.prototype.addEventListener);
  reg.onupdatefound = null; reg.removeEventListener('updatefound', before); reg.removeEventListener('updatefound', after);
  event.ports[0].postMessage(checks); event.ports[0].close();
})().catch(error => { event.ports[0].postMessage([{ name: 'worker/unexpected', pass: false, message: String(error) }]); event.ports[0].close(); })));
