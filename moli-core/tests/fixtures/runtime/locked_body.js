async function runLockedBodyProbe(url) {
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

    for (const source of sources) {
      for (const method of methods) {
        const body = await makeBody(source, method);
        const reader = body.body.getReader();
        Object.defineProperty(body.body, 'locked', {get() {
          throw new Error('public locked getter consulted');
        }});
        check(!body.bodyUsed, source + ': locking alone does not disturb');
        throwsTypeError(() => body.clone(), source + ': locked clone');
        await rejectsTypeError(() => body[method](), source + ': locked ' + method);
        check(!body.bodyUsed, source + ': failed ' + method + ' must not disturb');
        reader.releaseLock();
        try { await body.text(); }
        catch (error) { errors.push(source + ': released unused body cannot be consumed ' + error); }
      }
    }
  return {errors};
}
