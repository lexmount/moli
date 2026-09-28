(async () => {
  const url = URL.createObjectURL(new Blob([`
    try {
      const fallback = new CSSUnparsedValue(['red']);
      const reference = new CSSVariableReferenceValue('--color', fallback);
      const value = new CSSUnparsedValue([reference]);
      fallback[0] = 'blue';
      value[1] = 'suffix';
      if (!(value instanceof CSSStyleValue) || reference.fallback !== fallback ||
          value.length !== 2 || Array.from(value)[0] !== reference ||
          String(value) !== 'var(--color,blue)suffix') throw new Error('worker native values');
      let rejected = false;
      try { new Proxy(value, {}).length; } catch (e) { rejected = e instanceof TypeError; }
      if (!rejected) throw new Error('worker receiver brand');
      fallback[0] = reference;
      if (String(value) !== '') throw new Error('worker fallback cycle');
      postMessage(true);
    } catch (e) { postMessage(String(e)); }
  `], {type: 'text/javascript'}));
  const worker = new Worker(url);
  let timer;
  try {
    const result = await new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('worker probe timeout')), 4000);
      worker.onmessage = e => resolve(e.data);
      worker.onerror = e => { e.preventDefault(); reject(new Error(e.message)); };
    });
    if (result !== true) throw new Error(result);
    return true;
  } finally {
    clearTimeout(timer);
    worker.terminate();
    URL.revokeObjectURL(url);
  }
})()
