async function blobResponseHeaderProbe() {
  const checks = [];
  const check = (label, actual, wanted) => checks.push({label, actual, wanted,
    pass: JSON.stringify(actual) === JSON.stringify(wanted)});
  const encode = value => Array.from(new TextEncoder().encode(value));
  const cases = [
    ['empty', new Blob([]), [], ''],
    ['empty-typed', new Blob([], {type: 'text/plain'}), [], 'text/plain'],
    ['text', new Blob(['Blob data'], {type: 'TEXT/PLAIN'}), encode('Blob data'), 'text/plain'],
    ['unicode', new Blob(['𝌆é\ud800'], {type: 'text/plain'}), encode('𝌆é\ufffd'), 'text/plain'],
    ['binary', new Blob([new Uint8Array([0, 255, 128, 65])]), [0, 255, 128, 65], ''],
    ['xml-no-sniff', new Blob(['<root/>']), encode('<root/>'), ''],
    ['invalid-type', new Blob(['x'], {type: 'invalid'}), [120], 'invalid'],
    ['invalid-ascii', new Blob(['x'], {type: 'text/\0plain'}), [120], ''],
    ['slice', new Blob([new Uint8Array([0, 128, 255, 65])]).slice(1, 3, 'APPLICATION/OCTET-STREAM'), [128, 255], 'application/octet-stream'],
    ['file', new File(['file'], 'fixture.txt', {type: 'text/plain'}), encode('file'), 'text/plain'],
  ];
  async function inspectFetch(label, input, url, bytes, type) {
    const response = await fetch(input);
    check(label + '/status', [response.status, response.statusText, response.type], [200, 'OK', 'basic']);
    check(label + '/url', response.url === url, true);
    check(label + '/header-names', Array.from(response.headers.keys()), ['content-length', 'content-type']);
    check(label + '/has-type', response.headers.has('content-type'), true);
    check(label + '/type', response.headers.get('content-type'), type);
    check(label + '/length', response.headers.get('content-length'), String(bytes.length));
    let immutable = false;
    try { response.headers.set('content-length', '999'); } catch (error) { immutable = error instanceof TypeError; }
    check(label + '/immutable', immutable, true);
    const clone = response.clone();
    check(label + '/clone-type', clone.headers.get('content-type'), type);
    check(label + '/clone-length', clone.headers.get('content-length'), String(bytes.length));
    check(label + '/bytes', Array.from(new Uint8Array(await clone.arrayBuffer())), bytes);
    const blob = await response.blob();
    check(label + '/consumed-blob', [blob.size, Array.from(new Uint8Array(await blob.arrayBuffer()))], [bytes.length, bytes]);
  }
  async function inspectXhr(label, url, bytes, type, asynchronous, revoke) {
    const xhr = new XMLHttpRequest();
    const buffer = asynchronous || typeof document === 'undefined';
    let ended;
    const done = new Promise(resolve => ended = resolve);
    xhr.onloadend = () => ended();
    xhr.open('GET', url, asynchronous);
    if (buffer) xhr.responseType = 'arraybuffer';
    if (revoke) URL.revokeObjectURL(url);
    xhr.send();
    await done;
    check(label + '/status', [xhr.readyState, xhr.status, xhr.statusText], [4, 200, 'OK']);
    check(label + '/url', xhr.responseURL === url, true);
    check(label + '/type', xhr.getResponseHeader('Content-Type'), type);
    check(label + '/length', xhr.getResponseHeader('Content-Length'), String(bytes.length));
    check(label + '/all-headers', xhr.getAllResponseHeaders(),
      'content-length: ' + bytes.length + '\r\ncontent-type: ' + type + '\r\n');
    check(label + '/body', buffer ? Array.from(new Uint8Array(xhr.response)) : xhr.responseText,
      buffer ? bytes : new TextDecoder().decode(new Uint8Array(bytes)));
  }
  for (const [label, blob, bytes, type] of cases) {
    const url = URL.createObjectURL(blob);
    try {
      await inspectFetch(label + '/fetch', url, url, bytes, type);
      await inspectXhr(label + '/async-xhr', url, bytes, type, true, false);
      await inspectXhr(label + '/sync-xhr', url, bytes, type, false, false);
      const request = new Request(url);
      const clone = request.clone();
      URL.revokeObjectURL(url);
      await inspectFetch(label + '/captured-request', request, url, bytes, type);
      await inspectFetch(label + '/cloned-request', clone, url, bytes, type);
      let revoked = false;
      try { await fetch(url); } catch (error) { revoked = error instanceof TypeError; }
      check(label + '/revoked', revoked, true);
    } finally { URL.revokeObjectURL(url); }
    const xhrUrl = URL.createObjectURL(blob);
    try { await inspectXhr(label + '/captured-xhr', xhrUrl, bytes, type, true, true); }
    finally { URL.revokeObjectURL(xhrUrl); }
    const constructed = new Response(blob);
    check(label + '/constructed-length', constructed.headers.get('content-length'), null);
    check(label + '/constructed-type', constructed.headers.get('content-type'), type || null);
    check(label + '/constructed-body', Array.from(new Uint8Array(await constructed.arrayBuffer())), bytes);
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
