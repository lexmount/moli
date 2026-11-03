(() => {
  if (!document.documentElement) document.appendChild(document.createElement('html'));
  if (!document.body) document.documentElement.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe');
  document.body.appendChild(frame);
  const checks = [];
  const check = (passed, label) => checks.push({passed, label});
  const operations = {
    beforeMixed: (text, parent) => text.before('', parent),
    beforeReversed: (text, parent) => text.before(parent, ''),
    afterMixed: (text, parent) => text.after('', parent),
    replaceMixed: (text, parent) => text.replaceWith('', parent),
    beforeNode: (text, parent) => text.before(parent)
  };
  let sequence = 0;
  for (const [realmName, win] of [['main', globalThis], ['child', frame.contentWindow]]) {
    const doc = win.document;
    win.addEventListener('error', event => event.preventDefault());
    for (const [operation, mutate] of Object.entries(operations)) {
      for (const callbackMode of ['absent', 'returns', 'throws', 'reentrant']) {
        const label = realmName + ':' + operation + ':' + callbackMode;
        const events = [];
        const marker = new win.Error('callback-marker');
        const parent = doc.createElement('div');
        const text = doc.createTextNode('');
        parent.appendChild(text);
        const nestedName = 'nested-reaction-' + (++sequence);
        win.customElements.define(nestedName, class extends win.HTMLElement {
          connectedCallback() { events.push('nested-connected'); }
        });
        let recording = true;
        const Constructor = class extends win.HTMLElement {
          disconnectedCallback() {
            if (!recording) return;
            events.push('disconnected');
            if (callbackMode === 'throws') throw marker;
            if (callbackMode === 'reentrant') {
              const nested = doc.createElement(nestedName);
              doc.body.appendChild(nested);
              nested.remove();
              const cyclic = doc.createElement('i');
              let error;
              try { cyclic.append(cyclic); } catch (caught) { error = caught; }
              check(error instanceof win.DOMException && error.name === 'HierarchyRequestError', label + ':nested exception');
            }
          }
        };
        if (callbackMode === 'absent') delete Constructor.prototype.disconnectedCallback;
        const name = 'outer-reaction-' + (++sequence);
        win.customElements.define(name, Constructor);
        const custom = doc.createElement(name);
        parent.appendChild(custom);
        doc.body.appendChild(parent);
        let threw = false;
        let error;
        try { mutate(text, parent); } catch (caught) { threw = true; error = caught; }
        const reacts = operation !== 'beforeNode' && callbackMode !== 'absent';
        const expectedEvents = !reacts ? [] : callbackMode === 'reentrant' ? ['disconnected', 'nested-connected'] : ['disconnected'];
        check(threw, label + ':original exception propagated');
        check(error instanceof win.DOMException, label + ':original exception realm');
        check(error?.name === 'HierarchyRequestError', label + ':original exception name');
        check(error?.code === 3, label + ':original exception code');
        check(error !== marker, label + ':callback exception does not replace original');
        check(JSON.stringify(events) === JSON.stringify(expectedEvents), label + ':synchronous reaction order');
        check(parent.isConnected === (operation === 'beforeNode'), label + ':conversion removal');
        check(text.parentNode === parent && custom.parentNode === parent, label + ':subtree retained');
        check(parent.ownerDocument === doc && custom.ownerDocument === doc, label + ':owner retained');
        recording = false;
        parent.remove();
      }
    }
  }
  frame.remove();
  return checks;
})()
