async function xhrBlobMimeProbe(extraCases = []) {
  const checks = [];
  const cases = [
    ['default data MIME', 'data:,%00%FF%80', undefined, 'text/plain;charset=US-ASCII', 'text/plain;charset=US-ASCII'],
    ['response parameters', 'data:Text/Plain;Charset=GBK;Title=MiXeD,%00%FF%80', undefined, 'text/plain;charset=GBK;title=MiXeD', 'text/plain;charset=GBK;title=MiXeD'],
    ['override parameters', 'data:text/plain,%00%FF%80', 'TEXT/HTML;CHARSET=GBK', 'text/html;charset=GBK', 'text/plain'],
    ['quoted parameter', 'data:text/plain,%00%FF%80', 'text/plain;TITLE="AbC ; dEf"', 'text/plain;title="AbC ; dEf"', 'text/plain'],
    ['escaped parameter', 'data:text/plain,%00%FF%80', 'text/plain;title="AbC\\\"DeF"', 'text/plain;title="AbC\\\"DeF"', 'text/plain'],
    ['empty parameter', 'data:text/plain,%00%FF%80', 'text/plain;charset=""', 'text/plain;charset=""', 'text/plain'],
    ['duplicate parameter', 'data:text/plain,%00%FF%80', 'text/plain;CHARSET=GBK;charset=utf-8', 'text/plain;charset=GBK', 'text/plain'],
    ['non-ASCII parameter', 'data:text/plain,%00%FF%80', 'text/plain;title=ÿ', 'text/plain;title="ÿ"', 'text/plain'],
    ['tab parameter', 'data:text/plain,%00%FF%80', 'text/plain;title="A\tB"', 'text/plain;title="A\tB"', 'text/plain'],
    ['invalid override', 'data:text/plain,%00%FF%80', 'not-a-mime-type', 'application/octet-stream', 'text/plain'],
    ['empty override', 'data:text/plain,%00%FF%80', '', 'application/octet-stream', 'text/plain'],
    ['wildcard override', 'data:text/plain,%00%FF%80', '*/*', '*/*', 'text/plain'],
    ...extraCases
  ];
  for (const asynchronous of typeof document === 'undefined' ? [true, false] : [true]) {
    for (const [index, [label, url, override, expected, header]] of cases.entries()) {
      const check = (name, pass, actual = '') => checks.push({label: `${asynchronous}/${label}/${name}`, pass, actual: String(actual)});
      const xhr = new XMLHttpRequest();
      if (override !== undefined && index % 2 === 0) xhr.overrideMimeType(override);
      xhr.open('GET', url, asynchronous);
      xhr.responseType = 'blob';
      if (override !== undefined && index % 2 !== 0) xhr.overrideMimeType(override);
      if (asynchronous) {
        await new Promise((resolve, reject) => {
          xhr.onload = resolve;
          xhr.onerror = () => reject(new Error(label + ': request failed'));
          xhr.send();
        });
      } else xhr.send();
      const blob = xhr.response;
      check('response Blob and identity', blob instanceof Blob && blob === xhr.response);
      check('final MIME', blob.type === expected, blob.type);
      check('original response header', xhr.getResponseHeader('content-type') === header, xhr.getResponseHeader('content-type'));
      check('response bytes', blob.size === 3 && String(new Uint8Array(await blob.arrayBuffer())) === '0,255,128', blob.size);
      const normalized = /^[\x20-\x7e]*$/.test(expected) ? expected.toLowerCase() : '';
      check('Blob constructor normalization', new Blob([], {type: expected}).type === normalized);
    }
  }
  return {state: checks.every(check => check.pass) ? 'pass' : 'fail', checks};
}
