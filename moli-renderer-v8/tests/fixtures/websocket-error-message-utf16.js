function websocketErrorMessageUtf16Probe(w) {
  const assert = (ok, message) => { if (!ok) throw Error(message); };
  const units = value => Array.from({length:value.length}, (_, i) => value.charCodeAt(i));
  const values = ['', '\0', '\ud800', '\udfff', '\ud800X\udfff', '\ud83d\udca1', 'A\0\ud800\ud834\udd1e\udfffZ'];
  for (const message of values) {
    const error = new w.WebSocketError(message, {closeCode:3000, reason:message});
    assert(JSON.stringify(units(error.message)) === JSON.stringify(units(message)), 'message must preserve DOMString code units');
    assert(error.reason === message.toWellFormed(), 'reason must remain USVString');
    assert(error.name === 'WebSocketError' && error.code === 0 && error.closeCode === 3000, 'native error metadata');
    class Derived extends w.WebSocketError {}
    const derived = new Derived(message);
    assert(Object.getPrototypeOf(derived) === Derived.prototype && derived.message === message, 'subclass message');
  }
  for (const message of [undefined, null, false, 42, 12n]) {
    const error = new w.WebSocketError(message);
    assert(error.message === (message === undefined ? '' : String(message)), 'message conversion');
  }
  const sentinel = {};
  const log = [];
  const message = {toString(){log.push('message');throw sentinel;}};
  const init = {get closeCode(){log.push('closeCode');return 3000;},get reason(){log.push('reason');return '';}};
  let caught;
  try { new w.WebSocketError(message, init); } catch (e) { caught=e; }
  assert(caught === sentinel && log.join() === 'message', 'original exception before dictionary');
  log.length = 0; caught = null;
  try { new w.WebSocketError(Symbol(), init); } catch (e) { caught=e; }
  assert(caught instanceof w.TypeError && log.length === 0, 'callee TypeError before dictionary');
  const converted = new w.WebSocketError({toString(){log.push('message');return '\ud800';}}, init);
  assert(converted.message === '\ud800' && log.join() === 'message,closeCode,reason', 'conversion order');
  return 'ok';
}
