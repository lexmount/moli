(async () => {
  const source = `
    try {
      const check = (ok, message) => {if (!ok) throw new Error(message);};
      check(typeof CSSImageValue === 'function' && CSSImageValue.name === 'CSSImageValue' && CSSImageValue.length === 0, 'worker exposure');
      check(Object.getPrototypeOf(CSSImageValue) === CSSStyleValue && Object.getPrototypeOf(CSSImageValue.prototype) === CSSStyleValue.prototype, 'worker inheritance');
      check(!('parse' in CSSImageValue) && !('parseAll' in CSSImageValue), 'parsing is Window-only');
      let rejected = 0;
      for (const run of [() => CSSImageValue(), () => new CSSImageValue(), () => CSSStyleValue.prototype.toString.call(Object.create(CSSImageValue.prototype))]) {
        try {run();} catch (e) {if (e instanceof TypeError) rejected++; else throw e;}
      }
      check(rejected === 3, 'worker illegal constructor and forged brand');
      postMessage(true);
    } catch (e) {postMessage(String(e));}
  `;
  const url = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
  const worker = new Worker(url);
  let timer;
  try {
    const result = await new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('image worker probe timeout')), 4000);
      worker.onmessage = e => resolve(e.data);
      worker.onerror = e => {e.preventDefault(); reject(new Error(e.message));};
    });
    if (result !== true) throw new Error(result);
    return true;
  } finally {
    clearTimeout(timer);
    worker.terminate();
    URL.revokeObjectURL(url);
  }
})()
