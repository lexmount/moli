async function runBodyConsumptionStateProbe(scenario, url) {
  const errors = [];
  const check = (value, label) => { if (!value) errors.push(label); };
  const throwsTypeError = (callback, label) => {
    try { callback(); errors.push(label + ': accepted'); }
    catch (error) { check(error instanceof TypeError, label + ': wrong error ' + error); }
  };
  const rejectsTypeError = async (callback, label) => {
    let promise;
    try { promise = callback(); }
    catch (error) { errors.push(label + ': synchronous throw ' + error); return; }
    try { await promise; errors.push(label + ': fulfilled'); }
    catch (error) { check(error instanceof TypeError, label + ': wrong rejection ' + error); }
  };
  const methods = ['text', 'json', 'arrayBuffer', 'bytes', 'blob', 'formData'];
  const streamFrom = value => new ReadableStream({
    start(controller) { controller.enqueue(new TextEncoder().encode(value)); controller.close(); }
  });
  const makeBody = async (source, method = 'text') => {
    const text = method === 'formData' ? 'name=value' : '{"value":1}';
    const headers = {'Content-Type': method === 'formData'
      ? 'application/x-www-form-urlencoded' : 'application/json'};
    if (source === 'request') return new Request(url, {method: 'POST', body: text, headers});
    if (source === 'response') return new Response(text, {headers});
    if (source === 'stream') return new Response(streamFrom(text), {headers});
    return fetch(url);
  };
  const sources = ['request', 'response', 'stream', 'fetch'];

  if (scenario === 'null') {
    const bodies = [new Request(url), new Response(), new Response(null, {status: 204}),
      Response.error(), Response.redirect(url)];
    for (const [index, body] of bodies.entries()) {
      check(body.body === null, index + ': null body');
      for (const method of methods) {
        for (let repeat = 0; repeat < 2; ++repeat) {
          try { await body[method](); } catch (error) {
            check(error instanceof TypeError || error instanceof SyntaxError, index + ': empty conversion error');
          }
          check(!body.bodyUsed, index + ': ' + method + ' must not disturb a null body');
        }
      }
      try { check(await body.text() === '', index + ': null body remains readable'); }
      catch (error) { errors.push(index + ': null body became unusable ' + error); }
      try { check(body.clone().body === null, index + ': null clone'); }
      catch (error) { errors.push(index + ': cannot clone null body ' + error); }
    }
  } else if (scenario === 'locked') {
    for (const source of sources) {
      for (const method of methods) {
        const body = await makeBody(source, method);
        const reader = body.body.getReader();
        check(!body.bodyUsed, source + ': locking alone does not disturb');
        throwsTypeError(() => body.clone(), source + ': locked clone');
        await rejectsTypeError(() => body[method](), source + ': locked ' + method);
        check(!body.bodyUsed, source + ': failed ' + method + ' must not disturb');
        reader.releaseLock();
        try { await body.text(); }
        catch (error) { errors.push(source + ': released unused body cannot be consumed ' + error); }
      }
    }
  } else if (scenario === 'consumed') {
    for (const source of sources) {
      for (const method of methods) {
        const body = await makeBody(source, method);
        const original = body.body;
        const pending = body[method]();
        check(body.bodyUsed, source + ': ' + method + ' disturbs immediately');
        check(original.locked, source + ': ' + method + ' locks immediately');
        throwsTypeError(() => original.getReader(), source + ': reader during ' + method);
        try { await pending; } catch {}
        check(body.body === original && body.bodyUsed, source + ': stable used body');
        check(original.locked, source + ': ' + method + ' keeps the reader lock');
        throwsTypeError(() => body.clone(), source + ': consumed clone');
        await rejectsTypeError(() => body.text(), source + ': repeated consume');
      }
    }
  } else if (scenario === 'disturbed') {
    const controller = new AbortController();
    controller.abort({unexpected: 'abort must not hide unusable body'});
    for (const source of sources) {
      for (const action of ['read', 'cancel']) {
        const body = await makeBody(source);
        const stream = body.body;
        if (action === 'read') {
          const reader = stream.getReader();
          await reader.read();
          reader.releaseLock();
        } else {
          await stream.cancel();
        }
        check(!stream.locked && body.bodyUsed, source + ': ' + action + ' disturbed after release');
        throwsTypeError(() => body.clone(), source + ': disturbed clone');
        throwsTypeError(() => new Response(stream), source + ': disturbed Response input');
        throwsTypeError(() => new Request(url, {method: 'POST', body: stream, duplex: 'half'}),
          source + ': disturbed RequestInit body');
        if (source === 'request') throwsTypeError(() => new Request(body), 'disturbed Request input');
        await rejectsTypeError(() => fetch(url, {method: 'POST', body: stream, duplex: 'half',
          signal: controller.signal}), source + ': disturbed fetch body precedes abort');
        for (const method of methods) {
          await rejectsTypeError(() => body[method](), source + ': disturbed ' + method);
        }
      }
    }
  } else if (scenario === 'errors') {
    const reason = {stream: 'failed'};
    const stream = new ReadableStream({start(controller) { controller.error(reason); }});
    const response = new Response(stream);
    check(!response.bodyUsed, 'unread errored body is not disturbed');
    try { await response.text(); errors.push('errored body fulfilled'); }
    catch (error) { check(error === reason, 'original stream error identity'); }
    check(response.bodyUsed && stream.locked, 'errored read retains used/locked state');
    await rejectsTypeError(() => response.text(), 'errored body cannot be read twice');
    let pulls = 0;
    const invalidMime = new Response(new ReadableStream({pull(controller) {
      ++pulls; controller.enqueue(new Uint8Array([1])); controller.close();
    }}), {headers: {'Content-Type': 'text/plain'}});
    await rejectsTypeError(() => invalidMime.formData(), 'unsupported MIME conversion');
    check(pulls === 1 && invalidMime.bodyUsed && invalidMime.body.locked,
      'conversion failure still fully consumes and locks body');
    const invalidJSON = new Response('{invalid');
    try { await invalidJSON.json(); errors.push('invalid JSON fulfilled'); }
    catch (error) { check(error instanceof SyntaxError, 'JSON parse failure rejects with SyntaxError'); }
    check(invalidJSON.bodyUsed && invalidJSON.body.locked, 'JSON failure consumes body');
  } else if (scenario === 'poison') {
    let reads = 0;
    const poison = () => { ++reads; throw new Error('public stream member consulted'); };
    const stream = streamFrom('intrinsic reader');
    Object.defineProperty(stream, 'locked', {get: poison});
    Object.defineProperty(stream, 'getReader', {get: poison});
    const response = new Response(stream);
    const readDescriptor = Object.getOwnPropertyDescriptor(ReadableStreamDefaultReader.prototype, 'read');
    Object.defineProperty(ReadableStreamDefaultReader.prototype, 'read', {configurable: true, get: poison});
    try { check(await response.text() === 'intrinsic reader', 'intrinsic reader reads body'); }
    finally { Object.defineProperty(ReadableStreamDefaultReader.prototype, 'read', readDescriptor); }
    check(reads === 0, 'body operations ignore public stream member overrides');
    const parse = JSON.parse;
    JSON.parse = poison;
    try {
      for (const source of ['request', 'response', 'stream']) {
        const body = await makeBody(source, 'json');
        check((await body.json()).value === 1, source + ': intrinsic JSON parser');
      }
    } finally { JSON.parse = parse; }
    check(reads === 0, 'body JSON parsing ignores public JSON.parse');
    for (const source of ['request', 'response']) {
      const body = await makeBody(source);
      const reader = body.body.getReader();
      Object.defineProperty(body.body, 'locked', {value: false});
      throwsTypeError(() => body.clone(), source + ': forged unlocked stream cannot clone');
      await rejectsTypeError(() => body.text(), source + ': forged unlocked stream cannot consume');
      check(!body.bodyUsed, source + ': rejected forged lock does not disturb');
      reader.releaseLock();
    }
  } else {
    throw new Error('unknown body scenario ' + scenario);
  }
  return {errors};
}
