function snapshot(value) {
  const object = value !== null && (typeof value === 'object' || typeof value === 'function');
  return {type: typeof value, tag: Object.prototype.toString.call(value),
    name: object && typeof value.name === 'string' ? value.name : null,
    message: object && typeof value.message === 'string' ? value.message : null,
    isError: value instanceof Error, isTypeError: value instanceof TypeError,
    isDOMException: value instanceof DOMException,
    marker: object && typeof value.marker === 'string' ? value.marker : null,
    nested: object && value.nested ? value.nested.value : null,
    self: object && value.self === value,
    primitive: object || value === undefined ? null : value};
}
function makeReason(kind) {
  switch (kind) {
    case 'default': return undefined;
    case 'null': return null;
    case 'string': return 'caller-reason';
    case 'object': { const value = {marker: 'before', nested: {value: 21}}; value.self = value; return value; }
    case 'error': return new Error('original-message');
    case 'typeerror': return new TypeError('original-message');
    case 'custom': { const value = new Error('original-message'); value.name = 'CustomError'; return value; }
    case 'domexception': return new DOMException('caller-message', 'TimeoutError');
    case 'function': return function uncloneable() {};
    case 'getter': return {get marker() { globalThis.serializationGets++; return 'before'; }};
    case 'throwing': return {get marker() { globalThis.serializationGets++; throw new Error('serialization-getter'); }};
  }
}
async function runAbort(kind, id) {
  globalThis.serializationGets = 0;
  const controller = new AbortController();
  const reason = makeReason(kind);
  const result = fetch('/controlled/data?case=' + id, {signal: controller.signal}).then(
    () => ({unexpectedResolve: true}),
    error => ({same: kind === 'default' ? error === controller.signal.reason : error === reason,
      type: typeof error, isError: error instanceof Error, isDOMException: error instanceof DOMException}));
  while (!(await (await fetch('/state?case=' + id)).json()).started) await new Promise(resolve => setTimeout(resolve, 0));
  let thrown = null;
  try { kind === 'default' ? controller.abort() : controller.abort(reason); }
  catch (error) { thrown = String(error); }
  const serializationGetsOnAbort = serializationGets;
  if (kind === 'object') { reason.marker = 'after'; reason.nested.value = 99; }
  if (['error', 'typeerror', 'custom'].includes(kind)) { reason.name = 'ChangedAfterAbort'; reason.message = 'changed-after-abort'; }
  const local = await result;
  let state;
  do { state = await (await fetch('/state?case=' + id)).json(); if (!state.report) await new Promise(resolve => setTimeout(resolve, 0)); } while (!state.report);
  return {id, kind, thrown, local, serializationGetsOnAbort, observed: state.report};
}
