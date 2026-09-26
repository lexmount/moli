(async () => {
  const rows = [];
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const test = async (name, run) => {
    try { await run(); rows.push({name, passed: true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const encode = text => new TextEncoder().encode(text);
  const stream = text => new ReadableStream({start(c) { c.enqueue(encode(text)); c.close(); }});
  const make = (kind, body, values) => {
    const headers = values.map(value => ['Content-Type', value]);
    return kind === 'Request'
      ? new Request('https://body-mime.test/', {method: 'POST', body, headers, duplex: 'half'})
      : new Response(body, {headers});
  };
  const blobCases = [
    [[], ''],
    [['', 'text/plain'], 'text/plain'],
    [['text/plain', ''], 'text/plain'],
    [['text/plain', 'text/html'], 'text/html'],
    [['TEXT/PLAIN;Charset=GBK', 'text/plain'], 'text/plain;charset=GBK'],
    [['text/html', '*/*'], 'text/html'],
    [['text/html; x="A,b"'], 'text/html;x="A,b"'],
    [['text/plain; p=ÿ'], 'text/plain;p="ÿ"'],
    [['invalid'], ''],
    [['*/*'], ''],
    [['application/javascript'], 'application/javascript'],
    [['text/html', 'application/private'], 'application/private'],
    [['text/html;charset=gbk', 'text/html;x=",text/plain'], 'text/html;x=",text/plain";charset=gbk'],
    [['text/plain;charset=utf-8', 'text/plain;charset=Shift_JIS'], 'text/plain;charset=Shift_JIS'],
  ];
  const multipart = '--UpperCase\r\nContent-Disposition: form-data; name="a"\r\n\r\nvalue\r\n--UpperCase--\r\n';
  const formCases = [
    [['invalid', 'application/x-www-form-urlencoded'], 'a=value', true],
    [['application/x-www-form-urlencoded', 'text/plain'], 'a=value', false],
    [['multipart/form-data; boundary=UpperCase', '*/*'], multipart, true],
    [['multipart/form-data; boundary=Wrong', 'multipart/form-data; boundary="UpperCase"'], multipart, true],
    [['application/x-www-form-urlencoded', 'invalid'], 'a=value', true],
  ];
  for (const kind of ['Request', 'Response']) {
    for (const mode of ['null', 'bytes', 'stream']) for (const [values, expected] of blobCases) {
      await test(`${kind} ${mode} Blob ${JSON.stringify(values)}`, async () => {
        const body = make(kind, mode === 'null' ? null : mode === 'bytes' ? encode('payload') : stream('payload'), values);
        const originalHeader = body.headers.get('Content-Type');
        const blob = await body.blob();
        assert(blob.type === expected, `expected ${JSON.stringify(expected)}, got ${JSON.stringify(blob.type)}`);
        assert(blob.size === (mode === 'null' ? 0 : 7), 'Blob size');
        assert(await blob.text() === (mode === 'null' ? '' : 'payload'), 'Blob bytes');
        assert(body.bodyUsed === (mode !== 'null'), 'bodyUsed');
        assert(body.headers.get('Content-Type') === originalHeader, 'header list is not rewritten');
      });
    }
    for (const mode of ['bytes', 'stream']) for (const [values, text, succeeds] of formCases) {
      await test(`${kind} ${mode} FormData ${JSON.stringify(values)}`, async () => {
        const body = make(kind, mode === 'bytes' ? encode(text) : stream(text), values);
        let form, error;
        try { form = await body.formData(); } catch (caught) { error = caught; }
        if (succeeds) assert(form instanceof FormData && form.get('a') === 'value', `form parse failed: ${error}`);
        else assert(error instanceof TypeError, 'last MIME does not select form parsing');
      });
    }
    for (const action of ['set', 'delete']) await test(`${kind} pending Blob header ${action}`, async () => {
      let controller;
      const body = make(kind, new ReadableStream({start(c) { controller = c; }}), ['text/plain']);
      const pending = body.blob();
      await Promise.resolve();
      if (action === 'set') body.headers.set('Content-Type', 'text/plain; Label=AfTeR');
      else body.headers.delete('Content-Type');
      controller.enqueue(encode('payload')); controller.close();
      const blob = await pending;
      assert(blob.type === (action === 'set' ? 'text/plain;label=AfTeR' : ''), 'MIME extracted at byte conversion');
    });
    for (const succeeds of [true, false]) await test(`${kind} pending FormData header ${succeeds}`, async () => {
      let controller;
      const body = make(kind, new ReadableStream({start(c) { controller = c; }}), ['text/plain']);
      const pending = body.formData();
      await Promise.resolve();
      body.headers.set('Content-Type', succeeds ? 'application/x-www-form-urlencoded' : 'application/private');
      controller.enqueue(encode('a=value')); controller.close();
      let form, error;
      try { form = await pending; } catch (caught) { error = caught; }
      if (succeeds) assert(form instanceof FormData && form.get('a') === 'value', `late type ignored: ${error}`);
      else assert(error instanceof TypeError, 'unsupported final MIME rejects');
    });
    await test(`${kind} clone has its own live headers`, async () => {
      let controller;
      const original = make(kind, new ReadableStream({start(c) { controller = c; }}), ['text/plain']);
      const copy = original.clone();
      const first = original.blob(), second = copy.blob();
      original.headers.set('Content-Type', 'text/html; x=Original');
      copy.headers.set('Content-Type', 'application/json; x=Copy');
      controller.enqueue(encode('payload')); controller.close();
      const blobs = await Promise.all([first, second]);
      assert(blobs[0].type === 'text/html;x=Original' && blobs[1].type === 'application/json;x=Copy', 'clone MIME metadata is independent');
    });
    await test(`${kind} MIME reads only native headers`, async () => {
      const body = make(kind, encode('payload'), ['TEXT/PLAIN; p=MiXeD']);
      body.headers.get = () => { throw new Error('author Headers.get'); };
      body.headers[Symbol.iterator] = () => { throw new Error('author Headers iterator'); };
      Object.defineProperty(body, 'headers', {get() { throw new Error('author headers getter'); }});
      assert((await body.blob()).type === 'text/plain;p=MiXeD', 'native MIME extraction');
    });
    await test(`${kind} stream rejection wins MIME conversion`, async () => {
      let controller;
      const body = make(kind, new ReadableStream({start(c) { controller = c; }}), ['invalid']);
      const reason = {}, pending = body.blob();
      controller.error(reason);
      let caught;
      try { await pending; } catch (error) { caught = error; }
      assert(caught === reason, 'original stream rejection is preserved');
    });
  }
  await test('Blob constructor retains its distinct type normalization', async () => {
    assert(new Blob([], {type: 'TEXT/PLAIN; p=MiXeD'}).type === 'text/plain; p=mixed', 'Blob constructor normalization');
  });
  return JSON.stringify({total: rows.length, failures: rows.filter(row => !row.passed)});
})();
