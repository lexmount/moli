function startAuxiliaryModuleProbe(base, mode) {
  globalThis.auxiliaryModuleResult = {done: false};
  (async () => {
    const popup = open(mode === 'blank' ? 'about:blank' : base + '/popup-' + mode);
    const originalFunction = popup.Function;
    try {
      if (mode !== 'blank') {
        await new Promise(resolve => popup.addEventListener('load', resolve, {once: true}));
      }
      const retainedWindow = popup.Function === originalFunction;
      const checks = await popup.Function('base', 'mode', `
        return (async () => {
          const checks = [];
          const check = (name, value) => {
            if (!value) throw Error(name);
            checks.push(name);
          };
          if (mode === 'blank' && document.URL !== 'about:blank') {
            throw Error('initial Document URL must remain about:blank');
          }
          if (mode === 'blocked') {
            const events = [];
            addEventListener('securitypolicyviolation', event => events.push(event));
            let failure;
            try { await import(base + '/blocked.js'); } catch (error) { failure = error; }
            check('blocked', failure instanceof TypeError);
            await new Promise(resolve => setTimeout(resolve, 100));
            check('violation', events.length === 1 &&
              events[0] instanceof SecurityPolicyViolationEvent &&
              events[0].isTrusted && events[0].target === document &&
              events[0].effectiveDirective === 'script-src-elem');
          } else {
            check('parser', mode === 'blank' || globalThis.parserModuleAnswer === 42);
            await new Promise((resolve, reject) => {
              const script = document.createElement('script');
              script.type = 'module';
              script.src = base + '/runtime.js';
              script.onload = resolve;
              script.onerror = () => reject(Error('runtime module failed'));
              document.body.appendChild(script);
            });
            check('runtime', globalThis.runtimeModuleAnswer === 42);
            check('javascript', (await import(base + '/module.js')).default === 42);
            check('graph', (await import(base + '/graph.js')).default === 42);
            check('json', (await import(base + '/module.json', {with: {type: 'json'}})).default.answer === 42);
            check('text', (await import(base + '/module.txt', {with: {type: 'text'}})).default === 'module-text');
            const sheet = (await import(base + '/module.css', {with: {type: 'css'}})).default;
            check('css', sheet instanceof CSSStyleSheet && sheet.cssRules[0].style.color === 'green');
            let failure;
            try { await import(base + '/missing.js'); } catch (error) { failure = error; }
            check('fetch-error', failure instanceof TypeError);
          }
          return checks;
        })();
      `)(base, mode);
      return {retainedWindow, checks};
    } finally {
      popup.close();
    }
  })().then(
    value => { globalThis.auxiliaryModuleResult = {done: true, ...value}; },
    error => { globalThis.auxiliaryModuleResult = {done: true, error: String(error.stack || error)}; }
  );
}
