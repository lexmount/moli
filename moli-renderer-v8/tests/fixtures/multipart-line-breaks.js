(async () => {
  async function multipartChecks(realms) {
    realms ??= [['worker', globalThis]];
    const cases = [
      ['plain', 'field', 'field', 'field', 'field'],
      ['lf', '\n', '\r\n', '%0D%0A', '%0A'],
      ['cr', '\r', '\r\n', '%0D%0A', '%0D'],
      ['crlf', '\r\n', '\r\n', '%0D%0A', '%0D%0A'],
      ['lf-cr', '\n\r', '\r\n\r\n', '%0D%0A%0D%0A', '%0A%0D'],
      ['cr-crlf', '\r\r\n', '\r\n\r\n', '%0D%0A%0D%0A', '%0D%0D%0A'],
      ['lf-lf', '\n\n', '\r\n\r\n', '%0D%0A%0D%0A', '%0A%0A'],
      ['unicode', '星🌟\u2028\r終\n', '星🌟\u2028\r\n終\r\n', '星🌟\u2028%0D%0A終%0D%0A', '星🌟\u2028%0D終%0A'],
      ['controls', 'a\0\t"\\%0A', 'a\0\t"\\%0A', 'a\0\t%22\\%0A', 'a\0\t%22\\%0A'],
      ['empty', '', '', '', ''],
    ];
    const checks = [], payload = 'binary\rbare\nline\r\n\0end';
    const check = (name, passed, details) => checks.push({name, passed: !!passed, ...(passed ? {} : {details})});
    const expected = (boundary, fieldName, filename, text) => [
      '--' + boundary,
      `Content-Disposition: form-data; name="${fieldName}"`,
      '', text,
      '--' + boundary,
      `Content-Disposition: form-data; name="file${fieldName}"; filename="${filename}.bin"`,
      'Content-Type: application/octet-stream',
      '', payload,
      '--' + boundary + '--', '',
    ].join('\r\n');
    for (const [producerName, producer] of realms) for (const [consumerName, consumer] of realms) {
      for (const kind of ['Request', 'Response']) for (const [label, raw, normalized, fieldName, filename] of cases) {
        const prefix = `${producerName}/${consumerName}/${kind}/${label}`;
        const fd = new producer.FormData();
        const file = new producer.File([payload], raw + '.bin', {type: 'application/octet-stream'});
        fd.append(raw, raw); fd.append('file' + raw, file);
        const makeBody = () => kind === 'Request' ? new consumer.Request('https://multipart-body.test/', {method: 'POST', body: fd}) : new consumer.Response(fd);
        try {
          const body = makeBody(), clone = body.clone();
          const boundary = body.headers.get('content-type').split('boundary=')[1];
          check(prefix + '/stored-values', fd.get(raw) === raw && fd.get('file' + raw) === file && file.name === raw + '.bin', [...fd.keys()]);
          fd.set(raw, 'changed\nvalue');
          const actual = await body.text();
          check(prefix + '/wire', actual === expected(boundary, fieldName, filename, normalized), actual);
          const bytes = await clone.arrayBuffer();
          const decoded = new consumer.TextDecoder().decode(bytes);
          check(prefix + '/clone-and-realm', Object.getPrototypeOf(bytes) === consumer.ArrayBuffer.prototype && decoded === actual && body.bodyUsed && clone.bodyUsed, {decoded, actual});
          const next = makeBody(), nextBoundary = next.headers.get('content-type').split('boundary=')[1];
          const nextText = await next.text();
          check(prefix + '/repeat-after-mutation', nextText === expected(nextBoundary, fieldName, filename, 'changed\r\nvalue') && fd.get(raw) === 'changed\nvalue' && file.name === raw + '.bin', nextText);
        } catch (error) {
          check(prefix + '/unexpected-error', false, String(error));
        }
      }
    }
    return checks;
  }
  const checks = await multipartChecks([['main', globalThis], ['child', document.querySelector('#child').contentWindow]]);
  const source = `(${multipartChecks.toString()})().then(checks=>postMessage(JSON.stringify({checks})),error=>postMessage(JSON.stringify({error:String(error)})));`;
  const workerURL = URL.createObjectURL(new Blob([source], {type: 'text/javascript'}));
  const worker = new Worker(workerURL);
  try {
    const message = await new Promise((resolve, reject) => {
      worker.onmessage = event => resolve(JSON.parse(event.data));
      worker.onerror = event => reject(new Error(event.message));
    });
    if (message.error) throw new Error(message.error);
    checks.push(...message.checks);
  } finally {worker.terminate(); URL.revokeObjectURL(workerURL);}
  globalThis.__uiEventResults = {complete: true, total: checks.length, passed: checks.filter(c => c.passed).length, checks};
  return checks.every(c => c.passed);
})()
