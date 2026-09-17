async function fetchIntegrityProbe(baseUrl, localOnly = false) {
  const resources = [{"name": "text", "bytes": [104, 101, 108, 108, 111, 32, 105, 110, 116, 101, 103, 114, 105, 116, 121], "base64": "aGVsbG8gaW50ZWdyaXR5", "sha256": "sha256-9pyxsrnsacWVDvpoeZHDpEDkdnPx2ySEsclLyHWyL6A=", "sha384": "sha384-CCi8AQnAx6lR9HsMyFzQjILv5ia8wub93wfmdoOS4L7a++MzT++Fu7fSP7kYTmWF", "sha512": "sha512-mXVi95TF+mUwM+gHNM32LCn1SWYf95zOfK1yAG1GIuN0/bZwRhGvOtN/7CHf7whlbimiebfCZwlROrdR5NgQqw=="}, {"name": "empty", "bytes": [], "base64": "", "sha256": "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=", "sha384": "sha384-OLBgp1GsljhM2TJ+sbHjaiH9txEUvgdDTAzHv2P24donTt6/529l+9Ua0vFImLlb", "sha512": "sha512-z4PhNX7vuL3xVChQ1m2AB9Yg5AULVxXcg/SpIdNs6c5H0NE8XYXysP+DGNKHfuwvY7kxvUdBeoGlODJ6+SfaPg=="}, {"name": "binary", "bytes": [0, 255, 128, 1, 2, 65], "base64": "AP+AAQJB", "sha256": "sha256-AUPFNQOnS+WsR2eYg0rb6qoEp35s40QZHzqIjaQb3Cg=", "sha384": "sha384-CTICUISiEC8kw5GcbXJIzxYr8ks0/3gSUBfe8Hg0YLLHkBLQpuiU9J7fDDm3IzDj", "sha512": "sha512-k41a7eyRLajjPtfRMt7ek2wfdA4Zo0wCQbBe/DpXCRMLP9oF8gTJutkQXJOsViyCOtMxhbex8hVEO+IgHdJpHA=="}, {"name": "unicode", "bytes": [240, 157, 140, 134, 195, 169, 239, 191, 189], "base64": "8J2MhsOp77+9", "sha256": "sha256-gwOIYBj9iyRwb5o4SO40WH4GhpQVcsyVoSPCr+AcL+k=", "sha384": "sha384-AlLyVKhsauH8U6ot4I3tmF5IQlBxrn9XKoU83Xk/rYZQlXXqTLA1oW+STAuA0Wbw", "sha512": "sha512-7NrPqei7Xlo2hwgwyz1/Nebsgd7yqgP4Q8DCwXLRl5rpuhvy2YE/7k6g57s8Zp2s9fg1WAbvVmDk0Q8FEfLDhQ=="}];

  const checks = [];
  const check = (label, actual, wanted) => checks.push({label, actual, wanted,
    pass: JSON.stringify(actual) === JSON.stringify(wanted)});
  const wrong256 = 'sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=';
  const wrong512 = 'sha512-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==';
  async function attempt(label, input, init, wanted, expectedBytes, expectedType) {
    try {
      const response = await fetch(input, init);
      check(label + '/accepted', true, wanted);
      if (wanted) {
        check(label + '/bytes', Array.from(new Uint8Array(await response.arrayBuffer())), expectedBytes);
        if (expectedType) check(label + '/type', response.type, expectedType);
      } else if (response.body) { await response.body.cancel(); }
    } catch (error) { check(label + '/accepted', error instanceof TypeError ? false : String(error), wanted); }
  }
  for (const data of resources) {
    const metadata = [
      ['empty', '', true], ['sha256', data.sha256, true], ['sha384', data.sha384, true],
      ['sha512', data.sha512, true], ['unpadded', data.sha512.replace(/=+$/, ''), true],
      ['base64url', data.sha512.replace(/\+/g, '-').replace(/\//g, '_'), true],
      ['invalid', wrong256, false], ['stronger-invalid', data.sha256 + ' ' + wrong512, false],
      ['stronger-valid', wrong256 + ' ' + data.sha384, true],
      ['same-level-valid', wrong512 + ' ' + data.sha512, true],
      ['both-invalid', wrong256 + ' ' + wrong512, false],
      ['unsupported', 'sha1-ignored', true], ['invalid-syntax', 'sha256-***', true],
      ['whitespace', ' \t\n', true], ['leading-trailing', '\t' + data.sha256 + '\n', true],
      ['unicode-prefix', '\u00a0' + wrong256, true], ['unicode-suffix', wrong256 + '\u00a0', true],
    ];
    const blobUrl = URL.createObjectURL(new Blob([new Uint8Array(data.bytes)]));
    try {
      const urls = [['data', 'data:application/octet-stream;base64,' + data.base64], ['blob', blobUrl]];
      if (!localOnly) urls.push(['network', baseUrl + '/body?body=' + data.name]);
      for (const [source, url] of urls) {
        for (const [kind, integrity, wanted] of metadata) {
          await attempt(data.name + '/' + source + '/' + kind, url, {integrity}, wanted, data.bytes);
        }
        const original = new Request(url, {integrity: wrong256});
        await attempt(data.name + '/' + source + '/request', original, undefined, false, []);
        await attempt(data.name + '/' + source + '/clone', original.clone(), undefined, false, []);
        await attempt(data.name + '/' + source + '/override', original, {integrity: data.sha256}, true, data.bytes);
        await attempt(data.name + '/' + source + '/clear', original, {integrity: ''}, true, data.bytes);
      }
    } finally { URL.revokeObjectURL(blobUrl); }
  }
  if (!localOnly) {
    const text = resources[0];
    const cross = baseUrl.replace('127.0.0.1', 'localhost');
    await attempt('cors-valid', cross + '/body?body=text&cors=1', {integrity: text.sha256}, true, text.bytes, 'cors');
    await attempt('cors-invalid', cross + '/body?body=text&cors=1', {integrity: wrong256}, false, []);
    await attempt('cors-denied', cross + '/body?body=text', {integrity: text.sha256}, false, []);
    for (const integrity of ['', text.sha256, 'sha1-ignored', ' ', 'sha256-***']) {
      await attempt('opaque/' + integrity, cross + '/body?body=text', {integrity, mode: 'no-cors'}, !integrity, [], 'opaque');
    }
    for (const [label, method, status] of [['head', 'HEAD', 200], ['204', 'GET', 204], ['205', 'GET', 205], ['304', 'GET', 304]]) {
      for (const [kind, integrity] of [['empty', ''], ['valid-empty', resources[1].sha256], ['unsupported', 'sha1-ignored'], ['whitespace', ' ']]) {
        await attempt('null/' + label + '/' + kind, baseUrl + '/body?body=empty&status=' + status, {method, integrity}, !integrity, []);
      }
    }
    await attempt('http-error-valid', baseUrl + '/body?body=text&status=404', {integrity:text.sha256}, true, text.bytes);
    await attempt('follow-valid', baseUrl + '/redirect', {integrity:text.sha256}, true, text.bytes);
    await attempt('follow-invalid', baseUrl + '/redirect', {integrity:wrong256}, false, []);
    await attempt('manual-valid', baseUrl + '/redirect', {integrity:text.sha256,redirect:'manual'}, false, []);
    await attempt('manual-unsupported', baseUrl + '/redirect', {integrity:'sha1-ignored',redirect:'manual'}, false, []);
    for (const [kind, integrity, wanted, truncate, abort] of [
      ['unvalidated', '', true, false, false],
      ['valid', text.sha256, true, false, false],
      ['invalid', wrong256, false, false, false],
      ['unsupported', 'sha1-ignored', true, false, false],
      ['truncated', text.sha256, false, true, false],
      ['abort', text.sha256, false, false, true],
    ]) {
      const id = Math.random().toString(36).slice(2);
      const controller = new AbortController();
      const reason = {aborted: true};
      let settled = false;
      const outcome = fetch(baseUrl + '/gated?id=' + id + (truncate ? '&truncate=1' : ''), {integrity, signal:controller.signal})
        .then(response => { settled = true; return {response}; }, error => { settled = true; return {error}; });
      try {
        for (let i = 0; i < 100; i++) {
          if (await (await fetch(baseUrl + '/progress?id=' + id)).json()) break;
          if (i === 99) throw new Error('gated server did not start');
          await new Promise(resolve => setTimeout(resolve, 10));
        }
        await new Promise(resolve => setTimeout(resolve, 60));
        check('gated/' + kind + '/before-eof', settled, !integrity);
        if (abort) controller.abort(reason);
      } finally { await fetch(baseUrl + '/release?id=' + id); }
      const result = await outcome;
      if (abort) check('gated/' + kind + '/reason', result.error === reason, true);
      else if (result.error) check('gated/' + kind + '/accepted', result.error instanceof TypeError ? false : String(result.error), wanted);
      else {
        check('gated/' + kind + '/accepted', true, wanted);
        try {
          const bytes = Array.from(new Uint8Array(await result.response.arrayBuffer()));
          if (wanted) check('gated/' + kind + '/bytes', bytes, text.bytes);
        } catch (error) { if (wanted) check('gated/' + kind + '/body-error', String(error), null); }
      }
    }
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
