(async () => {
  const rows = [], errors = [];
  const source = `postMessage({secure: isSecureContext,
    register: 'registerProtocolHandler' in navigator,
    unregister: 'unregisterProtocolHandler' in navigator,
    registerPrototype: Object.hasOwn(WorkerNavigator.prototype, 'registerProtocolHandler'),
    unregisterPrototype: Object.hasOwn(WorkerNavigator.prototype, 'unregisterProtocolHandler')});`;
  const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
  const worker = new Worker(url);
  try {
    const facts = await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(event.data);
      worker.onerror = event => reject(new Error(event.message));
    });
    rows.push({label: 'worker-navigator', checks: {
      absent: !facts.register && !facts.unregister,
      absentPrototype: !facts.registerPrototype && !facts.unregisterPrototype,
    }, observed: facts});
  } catch (error) {errors.push(String(error.stack || error));}
  finally {worker.terminate(); URL.revokeObjectURL(url);}
  globalThis.__uiEventResults = {rows, errors};
  return errors.length === 0 && rows.every(row => Object.values(row.checks).every(value => value === true));
})()
