self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
self.addEventListener('message', event => event.waitUntil((async () => {
  const rows = [], check = (name, pass) => rows.push({ name: 'worker/' + name, pass: !!pass });
  const manager = self.registration.pushManager;
  check('manager-prototype', Object.getPrototypeOf(manager) === PushManager.prototype && manager.subscribe === PushManager.prototype.subscribe);
  check('secure-interfaces', typeof PushSubscription === 'function' && typeof PushSubscriptionOptions === 'function');
  check('encodings', Object.isFrozen(PushManager.supportedContentEncodings) && PushManager.supportedContentEncodings === PushManager.supportedContentEncodings);
  check('native-store', await manager.getSubscription() === null);
  const sentinel = {}, order = []; let error;
  try { await manager.subscribe({ get applicationServerKey() { order.push('key'); return null; }, get userVisibleOnly() { order.push('visible'); throw sentinel; } }); } catch (e) { error = e; }
  check('dictionary-exception', error === sentinel && order.join() === 'key,visible');
  let conversions = 0;
  const rejected = PushManager.prototype.permissionState.call(new Proxy(manager, {}), { get userVisibleOnly() { conversions++; return false; } });
  error = undefined; try { await rejected; } catch (e) { error = e; }
  check('brand-before-conversion', rejected instanceof Promise && error instanceof TypeError && conversions === 0);
  event.ports[0].postMessage(rows); event.ports[0].close();
})().catch(error => { event.ports[0].postMessage([{ name: 'worker/unexpected', pass: false, message: String(error) }]); event.ports[0].close(); })));
