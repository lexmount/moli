(async () => {
  const checks = [];
  const check = (name, run) => {
    try { checks.push({ name, passed: run() === true }); }
    catch (error) { checks.push({ name, passed: false, error: String(error) }); }
  };
  const equal = (actual, expected) => actual.length === expected.length &&
    actual.every((value, index) => value === expected[index]);
  const values = ['', 'plain', '\0', '\ufffd', '\ud800', '\ud801', '\udc00', '\udc01',
    'A\ud800B', 'A\udc00B', '😀', '\ud800😀\udc00'];
  const frame = document.createElement('iframe');
  (document.body || document.documentElement).appendChild(frame);
  const popup = globalThis.__attributeMutationIncludePopup === false ? null : open('');
  const observers = [];
  const observe = (w, element, callback = () => {}, options = { attributes: true, attributeOldValue: true }) => {
    const observer = new w.MutationObserver(callback);
    observers.push(observer); observer.observe(element, options); return observer;
  };
  const wait = () => new Promise(resolve => setTimeout(resolve, 0));
  try {
    await wait();
    for (const [realm, w] of [['main', window], ['iframe', frame.contentWindow], ...(popup ? [['popup', popup]] : [])]) {
      const writers = [
        ['setAttribute', null, 'data-value', (e, value) => e.setAttribute('data-value', value)],
        ['setAttributeNS-null', null, 'data-value', (e, value) => e.setAttributeNS(null, 'data-value', value)],
        ['setAttributeNS', 'urn:values', 'v:data-value', (e, value) => e.setAttributeNS('urn:values', 'v:data-value', value)],
        ['Attr.value', null, 'data-value', (e, value) => { e.getAttributeNode('data-value').value = value; }],
        ['Attr.nodeValue', null, 'data-value', (e, value) => { e.getAttributeNode('data-value').nodeValue = value; }],
        ['Attr.textContent', null, 'data-value', (e, value) => { e.getAttributeNode('data-value').textContent = value; }],
        ['setAttributeNode', null, 'data-value', (e, value) => {
          const attr = e.ownerDocument.createAttribute('data-value'); attr.value = value; e.setAttributeNode(attr);
        }],
        ['NamedNodeMap.setNamedItem', null, 'data-value', (e, value) => {
          const attr = e.ownerDocument.createAttribute('data-value'); attr.value = value; e.attributes.setNamedItem(attr);
        }],
      ];
      const documents = [
        ['live', w.document],
        ['createHTMLDocument', w.document.implementation.createHTMLDocument('values')],
        ['DOMParser-HTML', new w.DOMParser().parseFromString('<p>values</p>', 'text/html')],
        ['DOMParser-XML', new w.DOMParser().parseFromString('<root/>', 'application/xml')],
        ['createDocument', w.document.implementation.createDocument(null, 'root')],
      ];
      for (const [documentKind, d] of documents) {
        for (const [operation, namespace, qualifiedName, write] of writers) {
          const label = `${realm}/${documentKind}/${operation}`;
          const e = d.createElement('div');
          e.setAttributeNS(namespace, qualifiedName, 'before');
          const observer = observe(w, e);
          for (const [index, value] of values.entries()) {
            write(e, value);
            check(`${label}/${index}/native value`, () => e.getAttributeNS(namespace, 'data-value') === value);
          }
          const records = observer.takeRecords();
          check(`${label}/old values`, () => equal(records.map(r => r.oldValue), ['before', ...values.slice(0, -1)]));
          check(`${label}/record metadata`, () => records.length === values.length && records.every(r =>
            r.type === 'attributes' && r.target === e && r.attributeName === 'data-value' &&
            r.attributeNamespace === namespace && r.addedNodes.length === 0 && r.removedNodes.length === 0));
          e.removeAttributeNS(namespace, 'data-value');
          const removed = observer.takeRecords();
          check(`${label}/removal snapshot`, () => removed.length === 1 && removed[0].oldValue === values.at(-1));
          e.setAttributeNS(namespace, qualifiedName, 'later');
          const added = observer.takeRecords();
          check(`${label}/addition snapshot`, () => added.length === 1 && added[0].oldValue === null);
          check(`${label}/previous records remain frozen`, () => equal(records.map(r => r.oldValue), ['before', ...values.slice(0, -1)]));
          observer.disconnect();
        }
        const e = d.createElement('div'), observer = observe(w, e, () => {}, { attributes: true });
        e.setAttribute('data-value', '\ud800'); e.setAttribute('data-value', '\ud801');
        check(`${realm}/${documentKind}/attributeOldValue disabled`, () => equal(observer.takeRecords().map(r => r.oldValue), [null, null]));
        observer.disconnect(); e.setAttribute('data-value', '\udc00');
        check(`${realm}/${documentKind}/disconnect clears recording`, () => observer.takeRecords().length === 0);
        const text = d.createTextNode('before');
        const textObserver = observe(w, text, () => {}, { characterData: true, characterDataOldValue: true });
        for (const value of values) text.data = value;
        check(`${realm}/${documentKind}/characterData control`, () => equal(textObserver.takeRecords().map(r => r.oldValue), ['before', ...values.slice(0, -1)]));
        textObserver.disconnect();
        const attrElement = d.createElement('div'), first = d.createAttribute('data-value'), next = d.createAttribute('data-value');
        first.value = '\ud800'; next.value = '\udc00';
        attrElement.setAttributeNode(first);
        const replaced = attrElement.attributes.setNamedItem(next);
        check(`${realm}/${documentKind}/replaced Attr retains snapshot`, () => replaced === first &&
          first.ownerElement === null && first.value === '\ud800' && next.value === '\udc00');
        const detached = attrElement.removeAttributeNode(next);
        check(`${realm}/${documentKind}/removed Attr retains snapshot`, () => detached === next &&
          next.ownerElement === null && next.value === '\udc00');
      }
      const name = `attribute-values-${realm}`;
      const callbacks = [];
      const C = w.Function('callbacks', `return class extends HTMLElement {
        static observedAttributes = ['data-value', 'id'];
        attributeChangedCallback(...args) { callbacks.push({ receiver: this, args }); }
      }`)(callbacks);
      w.customElements.define(name, C);
      for (const [operation, namespace, qualifiedName, write] of writers.concat([
        ['dataset', null, 'data-value', (e, value) => { e.dataset.value = value; }],
        ['id reflection', null, 'id', (e, value) => { e.id = value; }],
      ])) {
        const label = `${realm}/custom element/${operation}`;
        const e = w.document.createElement(name), localName = qualifiedName.split(':').at(-1);
        e.setAttributeNS(namespace, qualifiedName, 'before'); callbacks.length = 0;
        const observer = observe(w, e);
        for (const [index, value] of values.entries()) {
          write(e, value);
          const event = callbacks.at(-1);
          check(`${label}/${index}/callback snapshot`, () => event?.receiver === e &&
            equal(event.args, [localName, index ? values[index - 1] : 'before', value, namespace]));
          if (operation === 'dataset') {
            check(`${label}/${index}/dataset value`, () => e.dataset.value === value);
            check(`${label}/${index}/dataset descriptor`, () => Object.getOwnPropertyDescriptor(e.dataset, 'value')?.value === value);
          }
        }
        check(`${label}/callback count`, () => callbacks.length === values.length);
        check(`${label}/observer snapshots`, () => equal(observer.takeRecords().map(r => r.oldValue), ['before', ...values.slice(0, -1)]));
        e.removeAttributeNS(namespace, localName);
        check(`${label}/remove callback`, () => equal(callbacks.at(-1)?.args || [], [localName, values.at(-1), null, namespace]));
        observer.disconnect();
      }
      for (const [index, value] of values.entries()) {
        const lateName = `${name}-late-${index}`, lateCallbacks = [];
        const e = w.document.createElement(lateName);
        e.setAttribute('data-value', value); e.setAttributeNS('urn:values', 'v:data-value', value);
        const Late = w.Function('callbacks', `return class extends HTMLElement {
          static observedAttributes = ['data-value'];
          attributeChangedCallback(...args) { callbacks.push(args); }
        }`)(lateCallbacks);
        w.customElements.define(lateName, Late); w.customElements.upgrade(e);
        check(`${realm}/upgrade/${index}/initial snapshots`, () => lateCallbacks.length === 2 &&
          equal(lateCallbacks[0], ['data-value', null, value, null]) &&
          equal(lateCallbacks[1], ['data-value', null, value, 'urn:values']));
      }
      const reentrantName = `${name}-reentrant`, reentrant = [];
      const Reentrant = w.Function('callbacks', `return class extends HTMLElement {
        static observedAttributes = ['data-value'];
        attributeChangedCallback(...args) {
          callbacks.push(args);
          if (args[2] === '\\ud800') this.setAttribute('data-value', '\\ud801');
          else if (args[2] === '\\ud801') this.removeAttribute('data-value');
        }
      }`)(reentrant);
      w.customElements.define(reentrantName, Reentrant);
      const reentrantElement = w.document.createElement(reentrantName);
      reentrantElement.setAttribute('data-value', '\ud800');
      check(`${realm}/reentrant callback snapshots`, () => reentrant.length === 3 &&
        equal(reentrant[0], ['data-value', null, '\ud800', null]) &&
        equal(reentrant[1], ['data-value', '\ud800', '\ud801', null]) &&
        equal(reentrant[2], ['data-value', '\ud801', null, null]));
      const e = w.document.createElement(name); callbacks.length = 0;
      Object.defineProperty(e, 'getAttribute', { value() { throw new Error('author getter'); } });
      const observer = observe(w, e);
      w.Element.prototype.setAttribute.call(e, 'data-value', '\ud800');
      w.Element.prototype.setAttribute.call(e, 'data-value', '\udc00');
      check(`${realm}/native snapshots bypass author getters`, () => callbacks.length === 2 &&
        equal(callbacks[1].args, ['data-value', '\ud800', '\udc00', null]) &&
        equal(observer.takeRecords().map(r => r.oldValue), [null, '\ud800']));
      const conversionError = {}, beforeCount = callbacks.length;
      let caught;
      try { e.setAttribute('data-value', { [Symbol.toPrimitive]() { throw conversionError; } }); }
      catch (error) { caught = error; }
      check(`${realm}/conversion error leaves queues unchanged`, () => caught === conversionError &&
        callbacks.length === beforeCount && observer.takeRecords().length === 0 &&
        w.Element.prototype.getAttribute.call(e, 'data-value') === '\udc00');
      observer.disconnect();
      const delivered = [], target = w.document.createElement('div');
      const asyncObserver = observe(w, target, records => delivered.push(...records));
      for (const value of values) target.setAttribute('data-value', value);
      target.removeAttribute('data-value');
      await wait();
      check(`${realm}/asynchronous observer snapshots`, () => equal(delivered.map(r => r.oldValue), [null, ...values]));
      check(`${realm}/asynchronous delivery drains queue`, () => asyncObserver.takeRecords().length === 0);
      asyncObserver.disconnect();
      const input = w.document.createElement('input');
      input.setAttribute('type', 'text'); input.setAttribute('value', '\ud800'); input.value = '\udc00';
      const inputObserver = observe(w, input); input.type = 'button';
      const inputRecords = inputObserver.takeRecords();
      check(`${realm}/input type side effect preserves value`, () => input.getAttribute('value') === '\udc00');
      check(`${realm}/input type side effect preserves oldValue`, () => inputRecords.some(r => r.attributeName === 'value' && r.oldValue === '\ud800'));
      inputObserver.disconnect();
    }
  } catch (error) {
    checks.push({ name: 'fixture completed', passed: false, error: String(error), stack: error.stack });
  } finally {
    for (const observer of observers) observer.disconnect();
    frame.remove(); if (popup) popup.close();
  }
  globalThis.__uiEventResults = {
    complete: true, includePopup: popup !== null, total: checks.length, passed: checks.filter(row => row.passed).length, checks,
  };
  return checks.every(row => row.passed);
})()
