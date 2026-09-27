(async () => {
  const [origin, other] = globalThis.encodingOrigins || globalThis.__fetchOrigins;
  const cases = [
    ['same-origin container', {}, 'windows-1253', 'Ά'],
    ['different nested encoding', {charset: 'windows-1251'}, 'windows-1251', 'ў'],
    ['cross-origin child', {childOrigin: other}, 'windows-1252', '¢'],
    ['cross-origin parent with same-origin child', {parentOrigin: other, childOrigin: other}, 'windows-1253', 'Ά'],
    ['cross-origin parent with top-origin child', {parentOrigin: other, childOrigin: origin}, 'windows-1252', '¢'],
    ['redirect to other origin', {childOrigin: other, redirectOrigin: origin}, 'windows-1252', '¢'],
    ['redirect to parent origin', {redirectOrigin: other}, 'windows-1253', 'Ά'],
    ['opaque child', {sandbox: 'allow-scripts'}, 'windows-1252', '¢'],
    ['same-origin sandbox child', {sandbox: 'allow-scripts allow-same-origin'}, 'windows-1253', 'Ά'],
    ['CSP opaque child', {csp: 'sandbox allow-scripts'}, 'windows-1252', '¢'],
    ['opaque parent', {parentSandbox: 'allow-scripts'}, 'windows-1252', '¢'],
    ['UTF-16LE parent', {charset: 'UTF-16LE'}, 'windows-1252', '¢'],
    ['UTF-16BE parent', {charset: 'UTF-16BE'}, 'windows-1252', '¢'],
    ['transport wins', {header: 'windows-1251'}, 'windows-1251', 'ў'],
    ['meta wins', {meta: 'windows-1251'}, 'windows-1251', 'ў'],
    ['invalid transport inherits', {header: 'not-an-encoding'}, 'windows-1253', 'Ά'],
    ['cross-origin invalid transport', {header: 'not-an-encoding', childOrigin: other}, 'windows-1252', '¢'],
    ['BOM wins', {bom: 'utf8'}, 'UTF-8', '�'],
    ['data URL is opaque', {mode: 'data'}, 'windows-1252', '¢'],
    ['blob URL shares parent origin', {mode: 'blob'}, 'windows-1253', 'Ά'],
    ['sandboxed blob', {mode: 'blob', sandbox: 'allow-scripts'}, 'windows-1252', '¢'],
    ['srcdoc uses UTF-8', {mode: 'srcdoc'}, 'UTF-8', '¢'],
    ['sandboxed srcdoc uses UTF-8', {mode: 'srcdoc', sandbox: 'allow-scripts'}, 'UTF-8', '¢'],
    ['srcdoc from UTF-16 parent uses UTF-8', {mode: 'srcdoc', charset: 'UTF-16LE'}, 'UTF-8', '¢'],
    ['popup has no container', {popup: '1'}, 'windows-1252', '¢'],
    ['popup transport wins', {popup: '1', header: 'windows-1251'}, 'windows-1251', 'ў'],
    ['popup blob has no container', {popup: '1', mode: 'blob'}, 'windows-1252', '¢'],
  ];
  const errors = [], observations = [];
  for (const [name, options, charset, text] of cases) {
    const token = 'encoding-' + observations.length;
    const frame = document.createElement('iframe');
    const url = new URL('/encoding-parent', options.parentOrigin || origin);
    url.searchParams.set('token', token);
    url.searchParams.set('charset', options.charset || 'windows-1253');
    for (const [key, value] of Object.entries(options)) url.searchParams.set(key, value);
    if (options.parentSandbox) frame.setAttribute('sandbox', options.parentSandbox);
    let listener, timer, parent;
    try {
      const observed = await new Promise((resolve, reject) => {
        listener = event => {
          if (event.data && event.data.token === token + ':parent') parent = event.data.charset;
          if (event.data && event.data.token === token) resolve(event.data);
        };
        addEventListener('message', listener);
        timer = setTimeout(() => reject(new Error('child did not report')), 5000);
        frame.src = url.href;
        document.body.append(frame);
      });
      observations.push({name, parent, observed});
      if (parent !== (options.charset || 'windows-1253') || observed.charset !== charset || observed.text !== text) {
        errors.push({name, parent, observed, expected: {charset, text}});
      }
    } catch (error) {
      observations.push({name, parent, error: String(error)});
      errors.push({name, parent, error: String(error)});
    } finally {
      clearTimeout(timer);
      removeEventListener('message', listener);
      if (frame.contentWindow) frame.contentWindow.postMessage('cleanup', '*');
      await new Promise(resolve => setTimeout(resolve, 0));
      frame.remove();
    }
  }
  return JSON.stringify({errors, observations});
})()
