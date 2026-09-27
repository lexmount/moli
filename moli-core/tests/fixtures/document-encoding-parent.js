(() => {
  const q = new URLSearchParams(location.search);
  const token = q.get('token');
  addEventListener('error', event => top.postMessage({token, error: event.message}, '*'));
  top.postMessage({token: token + ':parent', charset: document.characterSet}, '*');
  const child = new URL('/encoding-child', q.get('childOrigin') || location.href);
  for (const name of ['token', 'header', 'meta', 'csp', 'bom']) {
    if (q.has(name)) child.searchParams.set(name, q.get(name));
  }
  const report = '<script>addEventListener("load",()=>{' +
    'const target=opener?opener.top:top;' +
    'target.postMessage({token:' + JSON.stringify(token) +
    ',charset:document.characterSet,text:document.querySelector("p").textContent},"*")})</script>';
  const markup = '<!doctype html><body><p>\xa2</p>' + report;
  let target = child.href;
  if (q.has('redirectOrigin')) {
    target = q.get('redirectOrigin') + '/redirect?to=' + encodeURIComponent(target);
  }
  const mode = q.get('mode');
  if (mode === 'data') {
    target = 'data:text/html,' + Array.from(markup, c => '%' + c.charCodeAt(0).toString(16).padStart(2, '0')).join('');
  } else if (mode === 'blob') {
    target = URL.createObjectURL(new Blob([Uint8Array.from(markup, c => c.charCodeAt(0))], {type: 'text/html'}));
  }
  if (q.has('popup')) {
    const popup = open(target, '_blank');
    addEventListener('message', event => { if (event.data === 'cleanup') popup.close(); });
  } else {
    const frame = document.createElement('iframe');
    if (q.has('sandbox')) frame.setAttribute('sandbox', q.get('sandbox'));
    if (mode === 'srcdoc') frame.srcdoc = markup;
    else frame.src = target;
    document.body.append(frame);
  }
  addEventListener('message', event => {
    if (event.data === 'cleanup' && mode === 'blob') URL.revokeObjectURL(target);
  });
})()
