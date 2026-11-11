async function() {
  const result = {checks: 0, failures: []};
  const check = (name, value) => {
    result.checks++;
    if (!value) result.failures.push(name);
  };
  const url = path => new URL(path, location.href).href;
  const frame = document.createElement('iframe');
  const ready = new Promise(resolve => {
    const listener = event => {
      if (event.data !== 'dynamic-map-csp-ready') return;
      removeEventListener('message', listener);
      resolve();
    };
    addEventListener('message', listener);
  });
  frame.srcdoc = `<!doctype html>
    <meta http-equiv="Content-Security-Policy" content="script-src 'nonce-map'">
    <script type="module" nonce="map">
      window.metaResolve = import.meta.resolve;
      parent.postMessage('dynamic-map-csp-ready', '*');
    </script><body>child`;
  document.body.appendChild(frame);
  await ready;
  const child = frame.contentWindow;
  const resolve = child.metaResolve;
  const probe = (resolve, key) => {
    try { return resolve(key); }
    catch (error) { return 'throws:' + error.name; }
  };
  const blocked = child.document.createElement('script');
  blocked.type = 'importmap';
  blocked.textContent = JSON.stringify({imports: {denied: '/denied.mjs'}});
  child.document.head.appendChild(blocked);
  check('child CSP blocks unnonced map inserted by parent', probe(resolve, 'denied') === 'throws:TypeError');
  blocked.remove();
  blocked.nonce = 'map';
  child.document.head.appendChild(blocked);
  check('blocked preparation cannot retry after reinsertion', probe(resolve, 'denied') === 'throws:TypeError');
  const allowed = child.document.createElement('script');
  allowed.type = 'importmap';
  allowed.nonce = 'map';
  allowed.textContent = JSON.stringify({imports: {allowed: '/allowed.mjs'}});
  child.document.head.appendChild(allowed);
  check('child nonce admits the map', probe(resolve, 'allowed') === url('/allowed.mjs'));
  const mainResolve = (await import('data:text/javascript,export default import.meta.resolve;')).default;
  check('child map does not register in parent', probe(mainResolve, 'allowed') === 'throws:TypeError');
  frame.remove();
  check('allowed map remains available to retained resolver', probe(resolve, 'allowed') === url('/allowed.mjs'));
  result.completed = true;
  return result;
}
