async function blobRangeProbe() {
  const checks = [];
  const check = (label, actual, wanted) => checks.push({label, actual, wanted,
    pass: JSON.stringify(actual) === JSON.stringify(wanted)});
  const binary = [0, 255, 128, 65, 66, 67, 68, 69, 70, 71];
  const type = 'application/example';
  const blob = new Blob([new Uint8Array(binary)], {type});
  const url = URL.createObjectURL(blob);
  const huge = '9'.repeat(80);

  async function readFetch(label, input, init, bytes, mime, range, expectedUrl = url) {
    let actual, statusText = null;
    try {
      const response = await fetch(input, init);
      statusText = response.statusText;
      actual = {status: response.status, type: response.type,
        url: response.url === expectedUrl, mime: response.headers.get('content-type'),
        length: response.headers.get('content-length'), range: response.headers.get('content-range'),
        bytes: Array.from(new Uint8Array(await response.arrayBuffer()))};
    } catch (error) { actual = error.name; }
    check(label + '/response', actual, {status: range === null ? 200 : 206, type: 'basic',
      url: true, mime, length: String(bytes.length), range, bytes});
    check(label + '/status-text', statusText, range === null ? 'OK' : 'Partial Content');
  }
  async function rejectFetch(label, input, init) {
    let actual;
    try { actual = 'fulfilled:' + (await fetch(input, init)).status; }
    catch (error) { actual = error.name; }
    check(label, actual, 'TypeError');
  }
  async function readXhr(label, requestUrl, headers, bytes, mime, range, asynchronous, revoke = false) {
    const xhr = new XMLHttpRequest();
    const buffer = asynchronous || typeof document === 'undefined';
    let finish, actual, statusText = null, sending = true, completedInsideSend = null;
    const done = new Promise(resolve => finish = resolve);
    xhr.onloadend = () => { completedInsideSend = sending; finish(); };
    xhr.open('GET', requestUrl, asynchronous);
    if (buffer) xhr.responseType = 'arraybuffer';
    for (const [name, value] of headers) xhr.setRequestHeader(name, value);
    if (revoke) URL.revokeObjectURL(requestUrl);
    try {
      xhr.send();
      sending = false;
      if (asynchronous) await done;
      if (xhr.status === 0) actual = 'NetworkError';
      else {
        statusText = xhr.statusText;
        actual = {state: xhr.readyState, status: xhr.status, url: xhr.responseURL === requestUrl,
          mime: xhr.getResponseHeader('content-type'), length: xhr.getResponseHeader('content-length'),
          range: xhr.getResponseHeader('content-range'),
          body: buffer ? Array.from(new Uint8Array(xhr.response)) : xhr.responseText};
      }
    } catch (error) { sending = false; actual = error.name; }
    const rejected = bytes === null;
    check(label + '/response', actual, rejected ? 'NetworkError' : {
      state: 4, status: range === null ? 200 : 206, url: true, mime,
      length: String(bytes.length), range,
      body: buffer ? bytes : new TextDecoder().decode(new Uint8Array(bytes))});
    if (!rejected) check(label + '/status-text', statusText, range === null ? 'OK' : 'Partial Content');
    if (asynchronous) check(label + '/async-completion', completedInsideSend, false);
  }
  try {
    const valid = [
      ['closed', 'bytes=2-5', 2, 6],
      ['open', 'bytes=4-', 4, 10],
      ['suffix', 'bytes=-3', 7, 10],
      ['full-suffix', 'bytes=-10', 0, 10],
      ['long-suffix', 'bytes=-12', 0, 10],
      ['long-end', 'bytes=4-100000000000', 4, 10],
      ['end-at-length', 'bytes=4-10', 4, 10],
      ['whitespace', 'bytes \t= \t1 \t- \t3', 1, 4],
      ['zero-padding', 'bytes=0001-0003', 1, 4],
      ['huge-end', 'bytes=1-' + huge, 1, 10],
      ['huge-suffix', 'bytes=-' + huge, 0, 10],
      ['zero-suffix', 'bytes=-0', 10, 10],
    ];
    for (const [label, value, start, end] of valid) {
      const headers = [['rAnGe', value]];
      const bytes = binary.slice(start, end);
      const range = 'bytes ' + start + '-' + (end - 1) + '/10';
      await readFetch(label + '/fetch', url, {headers}, bytes, type, range);
      await readXhr(label + '/async-xhr', url, headers, bytes, type, range, true);
      await readXhr(label + '/sync-xhr', url, headers, bytes, type, range, false);
    }
    const invalid = [
      ['empty', ''], ['unit', 'byte=0-'], ['uppercase', 'BYTES=0-1'],
      ['equals', 'bytes 0-1'], ['dash', 'bytes=1'], ['missing', 'bytes=-'],
      ['descending', 'bytes=8-4'], ['start-digit', 'bytes=x-4'], ['end-digit', 'bytes=1-x'],
      ['plus-start', 'bytes=+1-4'], ['plus-end', 'bytes=1-+4'],
      ['embedded-space', 'bytes=1 2-4'], ['nbsp', 'bytes=\u00a01-4'],
      ['comma', 'bytes=1-4,'], ['multi', 'bytes=1-4,6-8'],
      ['out-of-bounds', 'bytes=10-'], ['huge-start', 'bytes=' + huge + '-'],
    ];
    for (const [label, value] of invalid) {
      const headers = [['Range', value]];
      await rejectFetch(label + '/fetch', url, {headers});
      await readXhr(label + '/async-xhr', url, headers, null, null, null, true);
      await readXhr(label + '/sync-xhr', url, headers, null, null, null, false);
    }
    const repeated = [['Range', 'bytes=1-4'], ['range', 'bytes=6-8']];
    await rejectFetch('repeated/fetch', url, {headers: repeated});
    await readXhr('repeated/async-xhr', url, repeated, null, null, null, true);
    await readXhr('repeated/sync-xhr', url, repeated, null, null, null, false);
    await readFetch('no-range', url, {}, binary, type, null);
    await readFetch('no-cors-removes-range', url, {mode: 'no-cors', headers: {Range: 'bytes=1-3'}}, binary, type, null);
    await rejectFetch('non-get', url, {method: 'POST', headers: {Range: 'bytes=1-3'}});

    const request = new Request(url, {headers: {Range: 'bytes=1-3'}});
    await readFetch('request-clone', request.clone(), {}, binary.slice(1, 4), type, 'bytes 1-3/10');
    await readFetch('request-override', request.clone(), {headers: {Range: 'bytes=6-8'}}, binary.slice(6, 9), type, 'bytes 6-8/10');
    await readFetch('request-clear', request.clone(), {headers: {}}, binary, type, null);
    const modified = request.clone();
    modified.headers.set('Range', 'bytes=4-5');
    await readFetch('request-mutation', modified, {}, binary.slice(4, 6), type, 'bytes 4-5/10');
    const capturedClone = request.clone();
    URL.revokeObjectURL(url);
    await readFetch('captured-request', request, {}, binary.slice(1, 4), type, 'bytes 1-3/10');
    await readFetch('captured-clone', capturedClone, {}, binary.slice(1, 4), type, 'bytes 1-3/10');
    await rejectFetch('revoked-url', url, {headers: {Range: 'bytes=1-3'}});
    for (const asynchronous of [false, true]) {
      const capturedUrl = URL.createObjectURL(blob);
      await readXhr('captured-xhr/' + asynchronous, capturedUrl, [['Range', 'bytes=1-3']],
        binary.slice(1, 4), type, 'bytes 1-3/10', asynchronous, true);
    }

    const bodies = [
      ['unicode', new Blob(['é𝌆z'], {type: 'text/plain'}), [169, 240, 157], 'text/plain', 7],
      ['untyped', new Blob([new Uint8Array(binary)]), binary.slice(1, 4), '', 10],
      ['file', new File([new Uint8Array(binary)], 'bytes.bin', {type}), binary.slice(1, 4), type, 10],
      ['sliced-blob', blob.slice(1, 6, type), binary.slice(2, 5), type, 5],
    ];
    for (const [label, value, bytes, mime, size] of bodies) {
      const bodyUrl = URL.createObjectURL(value);
      try {
        await readFetch(label, bodyUrl, {headers: {Range: 'bytes=1-3'}}, bytes, mime,
          'bytes 1-3/' + size, bodyUrl);
      } finally { URL.revokeObjectURL(bodyUrl); }
    }
    const emptyUrl = URL.createObjectURL(new Blob([]));
    try {
      await readFetch('empty/no-range', emptyUrl, {}, [], '', null, emptyUrl);
      await readFetch('empty/suffix', emptyUrl, {headers: {Range: 'bytes=-1'}}, [], '', 'bytes 0--1/0', emptyUrl);
      await rejectFetch('empty/start', emptyUrl, {headers: {Range: 'bytes=0-'}});
    } finally { URL.revokeObjectURL(emptyUrl); }

    const integrityUrl = URL.createObjectURL(new Blob(['0123456789']));
    try {
      // SHA-256 of selected bytes "2345", distinct from the complete Blob.
      const integrity = 'sha256-OAg8fukSHhdAGINWahSKpcLi1V3FO8SpSgJlF9v/PGs=';
      await readFetch('integrity/partial', integrityUrl,
        {headers: {Range: 'bytes=2-5'}, integrity}, [50, 51, 52, 53], '', 'bytes 2-5/10', integrityUrl);
      await rejectFetch('integrity/full-digest', integrityUrl, {headers: {Range: 'bytes=2-5'},
        integrity: 'sha256-hNiYd/DUBB77a/kaFvAkjy/Vc+avBcGflr7bn4gveII='});
      const response = await fetch(integrityUrl, {headers: {Range: 'bytes=2-5'}});
      const clone = response.clone();
      check('clone/header', clone.headers.get('content-range'), 'bytes 2-5/10');
      check('clone/body', await clone.text(), '2345');
      const body = await response.blob();
      check('consumed-blob', [body.size, body.type, await body.text()], [4, '', '2345']);
      let immutable = false;
      try { response.headers.set('Content-Range', 'wrong'); } catch (error) { immutable = error instanceof TypeError; }
      check('immutable-headers', immutable, true);
      const originalSlice = Blob.prototype.slice;
      Blob.prototype.slice = () => { throw new Error('must use captured native bytes'); };
      try {
        await readFetch('native-slice', integrityUrl, {headers: {Range: 'bytes=2-5'}},
          [50, 51, 52, 53], '', 'bytes 2-5/10', integrityUrl);
      } finally { Blob.prototype.slice = originalSlice; }
    } finally { URL.revokeObjectURL(integrityUrl); }
    for (const value of ['bytes=1-3', 'invalid']) {
      const response = await fetch('data:,0123456789', {headers: {Range: value}});
      check('data-ignores/' + value, [response.status, response.headers.get('content-range'), await response.text()],
        [200, null, '0123456789']);
    }
  } finally { URL.revokeObjectURL(url); }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
