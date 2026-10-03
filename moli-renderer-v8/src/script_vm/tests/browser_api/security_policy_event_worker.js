(async () => {
  const source = 'globalThis.__cspEventProbe = ' + globalThis.__cspEventProbe.toString() + '; postMessage(__cspEventProbe([self]));';
  const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
  const worker = new Worker(url);
  let result;
  try { result = await new Promise((resolve, reject) => { worker.onmessage = e => resolve(e.data); worker.onerror = e => reject(new Error(e.message)); }); }
  finally { worker.terminate(); URL.revokeObjectURL(url); }
  globalThis.__uiEventResults = result;
  return result.errors.length === 0 && result.rows.every(row => Object.values(row.checks).every(value => value === true));
})()
