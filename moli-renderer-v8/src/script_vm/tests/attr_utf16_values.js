(() => {
  const failed = [], check = (name, fn) => {
    try { if (!fn()) failed.push(name); }
    catch (e) { failed.push(`${name}: ${e.name}: ${e.message}`); }
  };
  const frame = document.createElement('iframe');
  document.body.append(frame);
  const docs = [document, frame.contentDocument,
    document.implementation.createHTMLDocument(''),
    new DOMParser().parseFromString('<root/>', 'application/xml')];
  const values = ['a\ud800b\udc00c', '\ud800', '\udc00', '\u0000\udfff'];
  for (const [i, doc] of docs.entries()) for (const value of values) {
    const el = doc.createElementNS('urn:element', 'e:root');
    el.setAttributeNS('urn:a', 'p:value', value);
    const attr = el.getAttributeNodeNS('urn:a', 'value');
    const label = `${i}/${JSON.stringify(value)}`;
    check(`${label} attached aliases`, () =>
      attr.value === value && attr.nodeValue === value && attr.textContent === value);
    check(`${label} native lookup ignores public owner methods`, () => {
      const a = el.getAttribute, b = el.getAttributeNS;
      try {
        el.getAttribute = el.getAttributeNS = () => { throw new Error('author method'); };
        return attr.value === value;
      } finally { el.getAttribute = a; el.getAttributeNS = b; }
    });
    check(`${label} attached clone snapshot`, () => {
      const clone = attr.cloneNode(true);
      return clone.value === value && clone.ownerElement === null &&
        clone.namespaceURI === 'urn:a' && clone.localName === 'value';
    });
    check(`${label} element clone/import reads`, () => {
      const clone = el.cloneNode(true), imported = document.importNode(el, true);
      return clone.getAttributeNodeNS('urn:a', 'value').value === value &&
        imported.getAttributeNodeNS('urn:a', 'value').value === value;
    });
    check(`${label} native updates visible`, () => {
      const updated = value + '\ud812';
      el.setAttributeNS('urn:a', 'q:value', updated);
      return attr.name === 'p:value' && attr.prefix === 'p' && attr.value === updated && el.getAttributeNodeNS('urn:a','value') === attr &&
        el.getAttributeNode('p:value') === attr && el.getAttributeNode('q:value') === null;
    });
    check(`${label} setter preserves code units`, () => {
      attr.value = value;
      return el.getAttributeNS('urn:a', 'value') === value && attr.value === value &&
        attr.name === 'p:value' && attr.prefix === 'p';
    });
    check(`${label} failed conversion is atomic`, () => {
      const marker = {};
      try { attr.value = { toString() { throw marker; } }; return false; }
      catch (e) { if (e !== marker) return false; }
      try { attr.value = Symbol(); return false; } catch (e) { if (e.name !== 'TypeError') return false; }
      return attr.value === value && el.getAttributeNS('urn:a', 'value') === value;
    });
    check(`${label} removal retains snapshot`, () => {
      el.removeAttributeNS('urn:a', 'value');
      return attr.ownerElement === null && attr.value === value && attr.cloneNode().value === value;
    });
    check(`${label} qualified-name removal retains original prefix`, () => {
      el.setAttributeNS('urn:a', 'p:value', value);
      const a = el.getAttributeNodeNS('urn:a', 'value');
      el.setAttributeNS('urn:a', 'q:value', value);
      el.removeAttribute('p:value');
      return a.ownerElement === null && a.name === 'p:value' &&
        a.prefix === 'p' && a.value === value && a.cloneNode().value === value;
    });
    check(`${label} namespace removal snapshots externally updated value`, () => {
      el.setAttributeNS('urn:a', 'p:value', value);
      const a = el.getAttributeNodeNS('urn:a', 'value');
      const updated = value + '\ud812';
      el.setAttributeNS('urn:a', 'q:value', updated);
      el.removeAttributeNS('urn:a', 'value');
      return a.ownerElement === null && a.name === 'p:value' &&
        a.value === updated && a.cloneNode().value === updated;
    });
    check(`${label} standalone aliases and clone`, () => {
      const standalone = doc.createAttribute('data-value');
      standalone.value = value;
      return standalone.nodeValue === value && standalone.textContent === value &&
        standalone.cloneNode().value === value;
    });
    check(`${label} nullable Node aliases`, () => {
      const a = doc.createAttribute('data-value');
      a.nodeValue = null;
      if (a.value !== '') return false;
      a.textContent = undefined;
      if (a.value !== '') return false;
      a.value = null;
      return a.value === 'null';
    });
  }
  for (const [i, doc] of docs.entries()) {
    check(`${i} standalone scalar aliases`, () => {
      const a = doc.createAttribute('data-value');
      a.value = 'value';
      if (a.nodeValue !== 'value' || a.textContent !== 'value') return false;
      a.nodeValue = 'node';
      if (a.value !== 'node' || a.textContent !== 'node') return false;
      a.textContent = 'text';
      return a.value === 'text' && a.nodeValue === 'text';
    });
    check(`${i} standalone undefined DOMString`, () => {
      const a = doc.createAttribute('data-value');
      a.value = undefined;
      return a.value === 'undefined';
    });
  }
  check('same qualified name in separate namespaces', () => {
    const el = document.createElementNS('urn:element', 'root');
    el.setAttributeNS('urn:a', 'value', '\ud800');
    el.setAttributeNS('urn:b', 'value', '\udc00');
    const a = el.getAttributeNodeNS('urn:a', 'value');
    const b = el.getAttributeNodeNS('urn:b', 'value');
    a.value = '\ud812';
    return a !== b && a.value === '\ud812' && b.value === '\udc00' &&
      el.getAttributeNS('urn:b', 'value') === '\udc00';
  });
  return JSON.stringify(failed);
})()
