async function runRequestStreamBodyProbe(scenario, url) {
  const errors = [];
  const check = (value, label) => { if (!value) errors.push(label); };
  const throwsTypeError = (callback, label) => {
    try { callback(); errors.push(label + ': accepted'); }
    catch (error) { check(error instanceof TypeError, label + ': ' + error); }
  };
  const makeStream = text => {
    const bytes = new TextEncoder().encode(text);
    let offset = 0;
    return new ReadableStream({async pull(controller) {
      await Promise.resolve();
      if (offset === bytes.length) controller.close();
      else { controller.enqueue(bytes.slice(offset, offset + 1)); ++offset; }
    }});
  };
  const makeRequest = (stream, init = {}) => new Request(url,
    {method: 'POST', body: stream, duplex: 'half', ...init});

  if (scenario === 'input') {
    for (const method of ['text', 'json', 'bytes', 'arrayBuffer', 'blob', 'formData']) {
      const text = method === 'formData' ? 'name=value' : '"delayed bytes"';
      const stream = makeStream(text);
      const headers = method === 'formData' ? {'Content-Type': 'application/x-www-form-urlencoded'} : {};
      const request = makeRequest(stream, {headers});
      check(request.body === stream && !stream.locked && !request.bodyUsed, method + ': retains input stream');
      if (method !== 'formData') check(!request.headers.has('Content-Type'), method + ': no inferred MIME');
      const value = await request[method]();
      if (method === 'text') check(value === text, method + ': bytes');
      else if (method === 'json') check(value === 'delayed bytes', method + ': bytes');
      else if (method === 'formData') check(value.get('name') === 'value', method + ': bytes');
      else {
        const bytes = method === 'bytes' ? value : new Uint8Array(method === 'blob' ? await value.arrayBuffer() : value);
        check(new TextDecoder().decode(bytes) === text, method + ': bytes');
      }
      check(request.bodyUsed && stream.locked, method + ': consumed state');
      throwsTypeError(() => request.clone(), method + ': consumed clone');
    }
    const stream = makeStream('validation');
    throwsTypeError(() => new Request(url, {method: 'POST', body: stream}), 'missing duplex');
    throwsTypeError(() => makeRequest(stream, {mode: 'no-cors'}), 'no-cors stream');
    throwsTypeError(() => makeRequest(stream, {keepalive: true}), 'keepalive stream');
    for (const method of ['GET', 'HEAD']) {
      throwsTypeError(() => makeRequest(stream, {method}), method + ': stream body');
      throwsTypeError(() => new Request(url, {method, body: ''}), method + ': empty body');
    }
    check(!stream.locked, 'failed construction keeps source unlocked');
    check(await makeRequest(stream).text() === 'validation', 'failed construction preserves bytes');
    const locked = makeStream('locked');
    const reader = locked.getReader();
    throwsTypeError(() => makeRequest(locked), 'locked source');
    await reader.read();
    reader.releaseLock();
    throwsTypeError(() => makeRequest(locked), 'disturbed source with released lock');
  } else if (scenario === 'clone') {
    const controller = new AbortController();
    const stream = makeStream('{"value":"AB"}');
    const request = makeRequest(stream, {signal: controller.signal, headers: {'X-Value': 'original'}});
    stream.tee = () => { throw new Error('public tee called'); };
    const clone = request.clone();
    const second = clone.clone();
    check(request.body !== stream && stream.locked, 'clone tees original stream');
    check(request.body !== clone.body && !request.bodyUsed && !clone.bodyUsed, 'clone has independent unused stream');
    clone.headers.set('X-Value', 'changed');
    check(request.headers.get('X-Value') === 'original', 'clone has independent headers');
    const reason = {abort: 'request signal'};
    controller.abort(reason);
    check(clone.signal !== request.signal && clone.signal.aborted && clone.signal.reason === reason,
      'clone follows signal without sharing signal object');
    check(await request.text() === '{"value":"AB"}', 'original consumes late bytes');
    check(!clone.bodyUsed && !second.bodyUsed, 'reading original leaves clones unused');
    check((await clone.json()).value === 'AB' && await second.text() === '{"value":"AB"}', 'clones consume late bytes');

    let cancelled;
    const source = new ReadableStream({cancel(reason) { cancelled = reason; }});
    const first = makeRequest(source);
    const other = first.clone();
    const cancellation = first.body.cancel('first');
    check(cancelled === undefined, 'one branch cancellation preserves source');
    await Promise.all([cancellation, other.body.cancel('second')]);
    check(Array.isArray(cancelled) && cancelled[0] === 'first' && cancelled[1] === 'second', 'both cancellations reach source');
    const error = {stream: 'failed'};
    const failed = makeRequest(new ReadableStream({start(c) { c.error(error); }}));
    const failedClone = failed.clone();
    for (const body of [failed, failedClone]) {
      try { await body.text(); errors.push('errored clone fulfilled'); }
      catch (reason) { check(reason === error, 'errored clone preserves reason'); }
    }
    const chunk = new Uint8Array([0, 65, 66, 0]).subarray(1, 3);
    const isolated = makeRequest(new ReadableStream({start(c) { c.enqueue(chunk); c.close(); }}));
    const isolatedClone = isolated.clone();
    const originalChunk = (await isolated.body.getReader().read()).value;
    originalChunk[0] = 90;
    check(await isolatedClone.text() === 'AB', 'clone copies chunks before original can mutate them');
    const shared = new ReadableStream({start(c) { c.enqueue(chunk); c.close(); }}).tee();
    check((await shared[0].getReader().read()).value === (await shared[1].getReader().read()).value,
      'public tee keeps sharing default stream chunks');
    let cloneFailure;
    const uncloneable = makeRequest(new ReadableStream({
      start(c) { c.enqueue(() => {}); }, cancel(error) { cloneFailure = error; }
    }));
    const uncloneableClone = uncloneable.clone();
    const outcomes = await Promise.allSettled([
      uncloneable.body.getReader().read(), uncloneableClone.body.getReader().read()
    ]);
    check(outcomes.every(result => result.status === 'rejected' && result.reason.name === 'DataCloneError'),
      'uncloneable chunks error both branches');
    check(cloneFailure === outcomes[0].reason && cloneFailure === outcomes[1].reason,
      'clone failure cancels source with the same error');
  } else if (scenario === 'inherit') {
    const source = makeStream('proxy bytes');
    const original = makeRequest(source);
    source.pipeThrough = source.pipeTo = () => { throw new Error('public pipe called'); };
    const proxy = new Request(original);
    check(proxy.body !== source && original.bodyUsed && source.locked, 'inherited stream becomes a proxy');
    check(!proxy.bodyUsed && await proxy.text() === 'proxy bytes', 'proxy preserves late bytes');
    throwsTypeError(() => original.clone(), 'transferred original is unusable');
    const closed = makeRequest(new ReadableStream({start(c) { c.close(); }}));
    const closedProxy = new Request(closed);
    check(closed.bodyUsed && !closedProxy.bodyUsed && await closedProxy.text() === '',
      'proxy immediately disturbs an already closed source');
    const retainedSource = makeStream('retained');
    const retained = makeRequest(retainedSource);
    const replacement = makeStream('replacement');
    const replaced = new Request(retained, {body: replacement, duplex: 'half'});
    check(replaced.body === replacement && !retained.bodyUsed && !retainedSource.locked, 'override preserves original');
    check(await retained.text() === 'retained' && await replaced.text() === 'replacement', 'override bytes');
    const keepalive = new Request(makeRequest(makeStream('inherited')), {keepalive: true});
    check(keepalive.keepalive && await keepalive.text() === 'inherited', 'keepalive validation applies to extraction');
    const error = {cancel: 'proxy'};
    let complete;
    const cancelled = new Promise(resolve => { complete = resolve; });
    const pending = makeRequest(new ReadableStream({cancel(reason) { complete(reason); }}));
    const pendingProxy = new Request(pending);
    await pendingProxy.body.cancel(error);
    check(await cancelled === error, 'proxy cancellation reaches input');
  }
  return {errors};
}
