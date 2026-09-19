async function runBodyUtf8Probe(scenario, url) {
  const errors = [];
  const check = (value, label) => { if (!value) errors.push(label); };
  const encode = value => new TextEncoder().encode(value);
  const sameBytes = (actual, expected) => actual.length === expected.length &&
    actual.every((value, index) => value === expected[index]);
  const sources = ['request', 'response', 'request-stream', 'response-stream', 'data', 'network'];
  const make = async (source, bytes, type = 'text/plain;charset=UTF-16') => {
    let input = bytes;
    if (source.endsWith('-stream')) {
      let offset = 0;
      input = new ReadableStream({async pull(controller) {
        await Promise.resolve();
        if (offset === bytes.length) controller.close();
        else { controller.enqueue(bytes.slice(offset, offset + 1)); ++offset; }
      }});
    }
    if (source.startsWith('request')) {
      return new Request(url, {method: 'POST', body: input, duplex: 'half', headers: {'Content-Type': type}});
    }
    if (source.startsWith('response')) return new Response(input, {headers: {'Content-Type': type}});
    const hex = Array.from(bytes, value => value.toString(16).padStart(2, '0'));
    if (source === 'data') return fetch('data:' + type + ',' + hex.map(value => '%' + value).join(''));
    const target = new URL(url);
    target.searchParams.set('hex', hex.join(''));
    target.searchParams.set('type', type);
    return fetch(target);
  };
  const consumeTextual = async (source, bytes, method, expected, label, ready) => {
    const body = await make(source, bytes);
    const clone = body.clone();
    let raw;
    if (ready) raw = await clone.bytes();
    try {
      const value = await body[method]();
      check(expected !== undefined && JSON.stringify(value) === JSON.stringify(expected), label + ': decoded value');
    } catch (error) {
      check(expected === undefined && error instanceof SyntaxError, label + ': rejection ' + error);
    }
    check(body.bodyUsed, label + ': consumed body');
    if (!ready) raw = await clone.bytes();
    check(sameBytes(raw, bytes), label + ': clone preserves exact bytes');
  };

  for (const source of sources) {
    if (scenario === 'text') {
      const cases = [
        [encode('\uFEFFA中'), 'A中'],
        [encode('\uFEFF\uFEFFx'), '\uFEFFx'],
        [encode('x\uFEFF'), 'x\uFEFF'],
        [encode('\uFEFF'), ''],
        [new Uint8Array([0xEF]), '\uFFFD'],
        [new Uint8Array([0xEF, 0xBB]), '\uFFFD'],
        [new Uint8Array([0xEF, 0xBB, 0x41]), '\uFFFDA'],
        [new Uint8Array([0xFF, 0xFE, 0x41, 0]), '\uFFFD\uFFFDA\0'],
        [new Uint8Array([0xEF, 0xBB, 0xBF, 0xF0, 0x9F, 0x41]), '\uFFFDA'],
        [encode('A中'), 'A中'],
        [new Uint8Array(), ''],
      ];
      for (const [index, [bytes, expected]] of cases.entries()) {
        await consumeTextual(source, bytes, 'text', expected, source + '/text/' + index, index % 2 === 0);
      }
    } else if (scenario === 'json') {
      const cases = [
        [encode('\uFEFF{"b":1,"a":2,"b":3}'), {b: 3, a: 2}],
        [encode('\uFEFF"\uFEFFvalue"'), '\uFEFFvalue'],
        [encode('\uFEFF\uFEFF{}'), undefined],
        [encode(' \uFEFF{}'), undefined],
        [encode('\uFEFF'), undefined],
        [new Uint8Array([0xFF, 0xFE, 0x7B, 0, 0x7D, 0]), undefined],
        [new Uint8Array([0xEF, 0xBB, 0xBF, 0x22, 0xFF, 0x22]), '\uFFFD'],
        [encode('0'), 0],
      ];
      for (const [index, [bytes, expected]] of cases.entries()) {
        await consumeTextual(source, bytes, 'json', expected, source + '/json/' + index, index % 2 === 0);
      }
    } else if (scenario === 'bytes') {
      const bytes = encode('\uFEFF\uFEFFvalue\uFEFF');
      for (const method of ['bytes', 'arrayBuffer', 'blob', 'stream']) {
        const body = await make(source, bytes);
        let actual;
        if (method === 'stream') {
          const reader = body.body.getReader();
          const values = [];
          for (;;) {
            const {done, value} = await reader.read();
            if (done) break;
            values.push(...value);
          }
          actual = new Uint8Array(values);
        } else {
          const value = await body[method]();
          actual = method === 'bytes' ? value : new Uint8Array(method === 'blob' ? await value.arrayBuffer() : value);
        }
        check(sameBytes(actual, bytes), source + '/' + method + ': raw BOM bytes');
      }
      const form = await (await make(source, encode('\uFEFFname=\uFEFFvalue'),
        'application/x-www-form-urlencoded')).formData();
      check(form.get('\uFEFFname') === '\uFEFFvalue', source + ': urlencoded BOMs');
      const multipart = '--probe\r\nContent-Disposition: form-data; name="name"\r\n\r\n\uFEFFvalue\uFEFF\r\n--probe--\r\n';
      const fields = await (await make(source, encode(multipart), 'multipart/form-data;boundary=probe')).formData();
      check(fields.get('name') === '\uFEFFvalue\uFEFF', source + ': multipart BOMs');
    }
  }
  return {errors};
}
