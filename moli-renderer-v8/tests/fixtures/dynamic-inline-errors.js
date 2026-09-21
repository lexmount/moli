function runDynamicInlineErrors(borrowChildInsertion) {
  const events = [];
  const errors = [];
  const elementEvents = [];
  const marker = {};
  let insert = (parent, script) => parent.appendChild(script);
  if (borrowChildInsertion) {
    const frame = document.createElement('iframe');
    document.body.appendChild(frame);
    frame.contentWindow.onerror = () => { events.push('wrong Window'); return true; };
    insert = frame.contentWindow.Function('parent', 'script',
      'return Node.prototype.appendChild.call(parent, script)');
  }
  const current = () => document.currentScript && document.currentScript.id;
  const makeScript = (id, source) => {
    const script = document.createElement('script');
    script.id = id;
    script.textContent = source;
    script.onload = () => elementEvents.push(id + ':load');
    script.onerror = () => elementEvents.push(id + ':error');
    return script;
  };
  window.onerror = (message, source, line, column, error) => {
    const id = current();
    events.push('error:' + id);
    errors.push([
      error === marker ? 'marker' : error.name,
      id,
      error === marker || error instanceof Error,
      line > 0 && column > 0,
    ]);
    if (id === 'runtime') {
      insert(document.body, makeScript('recovery',
        "__dynamicInlineProbe.events.push('recovery:' + document.currentScript.id)"));
      events.push('handler-restored:' + current());
    }
    Promise.resolve().then(() => events.push('microtask:' + current()));
    return true;
  };
  globalThis.__dynamicInlineProbe = {events, errors, elementEvents, marker, runOuter() {
    events.push('outer:' + current());
    for (const [id, source] of [
      ['syntax', '{'],
      ['runtime', "throw new TypeError('inline runtime')"],
      ['value', 'throw __dynamicInlineProbe.marker'],
      ['success', "__dynamicInlineProbe.events.push('success:' + document.currentScript.id)"],
    ]) {
      const script = makeScript(id, source);
      try {
        insert(document.body, script);
        events.push('returned:' + id + ':' + current());
        script.remove();
        insert(document.body, script);
      } catch (error) {
        events.push('escaped:' + error.name);
      }
    }
    const host = document.createElement('div');
    document.body.appendChild(host);
    insert(host.attachShadow({mode: 'open'}), makeScript('shadow', '{'));
    events.push('shadow-returned:' + current());
  }};
  insert(document.body, makeScript('outer', '__dynamicInlineProbe.runOuter()'));
  events.push('returned:' + current());
  return {events, errors, elementEvents};
}
