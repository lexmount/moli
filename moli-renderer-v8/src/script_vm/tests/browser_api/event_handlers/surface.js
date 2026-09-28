(() => {
  function assert(value, message) {
    if (!value) throw new Error(message);
  }
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const popup = open();
  assert(popup !== null, 'popup fixture');
  try {
    const child = frame.contentWindow;
    for (const w of [window, child, popup]) {
      for (const name of ['onselectstart', 'ongamepadconnected', 'ongamepaddisconnected']) {
        const descriptor = Object.getOwnPropertyDescriptor(w, name);
        assert(descriptor && descriptor.enumerable && descriptor.configurable &&
          typeof descriptor.get === 'function' && typeof descriptor.set === 'function', name + ': descriptor');
        assert(w[name] === null, name + ': initial null');
        const type = name.slice(2);
        const trace = [];
        const before = () => trace.push('before');
        const after = () => trace.push('after');
        w.addEventListener(type, before);
        w[name] = () => trace.push('stale');
        w.addEventListener(type, after);
        w[name] = function(event) {
          assert(this === w && event.currentTarget === w && event.type === type, name + ': receiver');
          trace.push('handler');
          return false;
        };
        assert(!w.dispatchEvent(new Event(type, {cancelable:true})), name + ': canceled');
        assert(trace.join(',') === 'before,handler,after', name + ': listener order');
        w[name] = 1;
        assert(w[name] === null, name + ': primitive clears');
        trace.length = 0;
        assert(w.dispatchEvent(new Event(type, {cancelable:true})), name + ': cleared dispatch');
        assert(trace.join(',') === 'before,after', name + ': cleared order');
        w.removeEventListener(type, before);
        w.removeEventListener(type, after);
      }
      for (const name of ['ongamepadconnected', 'ongamepaddisconnected']) {
        for (const tag of ['body', 'frameset']) {
          const element = w.document.createElement(tag);
          element[name] = () => false;
          assert(typeof w[name] === 'function' && element[name] === w[name], name + ': body reflection');
          assert(!w.dispatchEvent(new Event(name.slice(2), {cancelable:true})), name + ': reflected dispatch');
          element[name] = null;
          assert(w[name] === null && element[name] === null, name + ': reflected removal');
        }
        assert(!(name in w.document) && !(name in w.document.createElement('div')), name + ': WindowEventHandlers only');
      }
      for (const target of [w.document, w.document.createElement('div'),
        w.document.createElementNS('http://www.w3.org/2000/svg', 'svg'),
        w.document.createElementNS('http://www.w3.org/1998/Math/MathML', 'math')]) {
        assert(target.onselectstart === null, 'selectstart: DOM initial null');
        let calls = 0;
        target.onselectstart = function(event) {
          assert(this === target && event.currentTarget === target, 'selectstart: DOM receiver');
          ++calls;
          return false;
        };
        assert(!target.dispatchEvent(new Event('selectstart', {cancelable:true})) && calls === 1,
          'selectstart: DOM dispatch');
        target.onselectstart = null;
      }
    }
    return true;
  } finally {
    popup.close();
    frame.remove();
  }
})()
