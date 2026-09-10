async function runOpaqueStreamProbe(url, expected) {
  const errors = [];
  const check = (condition, message) => { if (!condition) errors.push(message); };
  const controller = new AbortController();
  const deadline = setTimeout(() => {
    errors.push('fetch waited for the unfinished body');
    controller.abort();
  }, 2000);
  try {
    const response = await fetch(url, {mode: 'no-cors', signal: controller.signal});
    clearTimeout(deadline);
    check(expected === 'opaque', 'CORP response was accepted');
    check(response.type === 'opaque' && response.status === 0 && !response.ok, 'opaque status');
    check(response.url === '' && !response.redirected, 'opaque URL');
    check(response.headers.entries().next().done, 'opaque headers');
    check(response.body === null && !response.bodyUsed, 'opaque body');
    const clone = response.clone();
    check(await response.text() === '', 'opaque text');
    check((await clone.bytes()).length === 0, 'opaque clone bytes');
    check(!response.bodyUsed && !clone.bodyUsed, 'empty bodies became disturbed');
    controller.abort(new Error('stop transfer'));
    check(await response.text() === '', 'abort exposed the internal body error');
  } catch (error) {
    check(expected === 'blocked' && error instanceof TypeError, 'unexpected fetch error: ' + error);
  } finally {
    clearTimeout(deadline);
    controller.abort();
  }
  // The server answers only after it observes the streaming socket close.
  // This runs before the harness disposes the page or terminates its worker.
  const closedURL = new URL(url);
  closedURL.pathname = '/closed';
  const observed = await fetch(closedURL).then(response => response.json());
  check(observed.closed === true, 'underlying connection did not close');
  return {errors};
}
