(() => {
  const rows = [];
  const methods = ['addEventListener', 'removeEventListener', 'dispatchEvent'];
  const check = (condition, label) => {
    rows.push([label, !!condition]);
    if (!condition) throw Error(label);
  };
  const inspect = (target, label, realm = globalThis) => {
    const {EventTarget, Event} = realm;
    check(target instanceof EventTarget, label + ': native inheritance');
    for (const name of methods) {
      check(!Object.hasOwn(target, name), label + ': no own ' + name);
      check(target[name] === EventTarget.prototype[name], label + ': inherited ' + name);
    }
    const calls = [];
    function listener(event) {
      calls.push(this === target && event.target === target && event.currentTarget === target &&
        event.composedPath()[0] === target);
      event.preventDefault();
    }
    target.addEventListener('inheritance-probe', listener, {once: true});
    const event = new Event('inheritance-probe', {cancelable: true});
    check(!target.dispatchEvent(event) && calls.length === 1 && calls[0], label + ': dispatch identity');
    check(target.dispatchEvent(new Event('inheritance-probe')) && calls.length === 1, label + ': once');
    check(event.currentTarget === null && event.composedPath().length === 0, label + ': cleanup');
    const order = [];
    target.addEventListener('order-probe', () => order.push('normal'), {once: true});
    target.addEventListener('order-probe', event => {
      order.push('capture');
      event.stopPropagation();
      target.addEventListener('order-probe', () => order.push('late'), {once: true});
    }, {capture: true, once: true});
    target.addEventListener('order-probe', () => order.push('last'), {once: true});
    target.dispatchEvent(new Event('order-probe'));
    check(order.join(',') === 'normal,capture,last', label + ': registration order and stopPropagation');
    order.length = 0;
    target.dispatchEvent(new Event('order-probe'));
    check(order.join(',') === 'late', label + ': list snapshot and stop flag cleanup');
    const original = EventTarget.prototype.addEventListener;
    let inheritedCalls = 0;
    try {
      EventTarget.prototype.addEventListener = function(...args) {
        inheritedCalls++;
        return Reflect.apply(original, this, args);
      };
      target.addEventListener('inheritance-probe', listener);
      check(inheritedCalls === 1, label + ': prototype replacement');
    } finally {
      EventTarget.prototype.addEventListener = original;
      target.removeEventListener('inheritance-probe', listener);
    }
  };
  const prototypes = ['EventTarget', 'Performance', 'BroadcastChannel', 'Worker', 'SharedWorker',
    'Notification', 'WebSocket', 'OfflineAudioContext', 'BatteryManager', 'NetworkInformation',
    'ServiceWorkerContainer', 'ServiceWorkerRegistration', 'ServiceWorker', 'Screen',
    'ScreenOrientation', 'Navigation', 'NavigationHistoryEntry', 'IDBRequest', 'IDBDatabase', 'IDBTransaction'];
  for (const name of prototypes) {
    if (typeof globalThis[name] !== 'function') continue;
    const prototype = globalThis[name].prototype;
    for (const method of methods) {
      check(prototype[method] === EventTarget.prototype[method], name + ': canonical ' + method);
      if (name !== 'EventTarget') check(!Object.hasOwn(prototype, method), name + ': no duplicate ' + method);
    }
  }
  inspect(globalThis, 'global');
  inspect(performance, 'performance');
  inspect(new EventTarget(), 'EventTarget');
  inspect(new FileReader(), 'FileReader');
  inspect(new AbortController().signal, 'AbortSignal');
  const channel = new MessageChannel();
  try { inspect(channel.port1, 'MessagePort'); } finally { channel.port1.close(); channel.port2.close(); }
  const broadcast = new BroadcastChannel('inheritance-probe');
  try { inspect(broadcast, 'BroadcastChannel'); } finally { broadcast.close(); }
  if (typeof window !== 'undefined') {
    inspect(navigation, 'navigation');
    if (navigation.currentEntry) inspect(navigation.currentEntry, 'NavigationHistoryEntry');
    inspect(screen, 'screen');
    inspect(screen.orientation, 'screen.orientation');
    inspect(navigator.connection, 'navigator.connection');
    if (navigator.serviceWorker) inspect(navigator.serviceWorker, 'navigator.serviceWorker');
  }
  globalThis.__eventTargetInheritance = {rows, pending: false, inspect};
  if (typeof navigator.getBattery === 'function') {
    __eventTargetInheritance.pending = true;
    navigator.getBattery().then(battery => {
      try { inspect(battery, 'battery'); } finally { __eventTargetInheritance.pending = false; }
    }, () => { rows.push(['battery: resolved', false]); __eventTargetInheritance.pending = false; });
  }
  return true;
})()
