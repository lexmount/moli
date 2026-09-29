(() => {
  const other = document.getElementById('child').contentWindow;
  const documents = [['main', document, globalThis], ['iframe', other.document, other],
    ['windowless', document.implementation.createHTMLDocument(''), globalThis],
    ['xml', document.implementation.createDocument('urn:root', 'root'), globalThis]];
  const attrKeys = ['namespaceURI', 'prefix', 'localName', 'name', 'value', 'ownerElement', 'specified'];
  const nodeKeys = ['nodeType', 'nodeName', 'nodeValue', 'textContent', 'ownerDocument', 'baseURI',
    'isConnected', 'parentNode', 'parentElement', 'firstChild', 'lastChild', 'previousSibling', 'nextSibling', 'childNodes'];
  const writable = ['value', 'nodeValue', 'textContent'];
  const rows = [];
  function check(name, expected, run) {
    let actual;
    try { actual = run(); } catch (error) { actual = {error: error.name, message: error.message}; }
    rows.push({name, expected, actual, pass: JSON.stringify(actual) === JSON.stringify(expected)});
  }
  const units = value => Array.from({length: value.length}, (_, i) => value.charCodeAt(i));
  function make(doc, attached) {
    const attr = doc.createAttributeNS('urn:attribute', 'p:key'); attr.value = 'old';
    const owner = doc.createElement('div'); if (attached) owner.setAttributeNodeNS(attr);
    return [attr, owner];
  }
  function descriptor(realm, key) {
    return Object.getOwnPropertyDescriptor(attrKeys.includes(key) ? realm.Attr.prototype : realm.Node.prototype, key);
  }
  for (const [label, doc, home] of documents) for (const attached of [false, true]) {
    const context = label + (attached ? '/attached' : '/ownerless');
    const [attr, owner] = make(doc, attached);
    const expected = {namespaceURI: 'urn:attribute', prefix: 'p', localName: 'key', name: 'p:key',
      value: 'old', specified: true, nodeType: 2, nodeName: 'p:key', nodeValue: 'old', textContent: 'old',
      isConnected: false, parentNode: null, parentElement: null, firstChild: null, lastChild: null, previousSibling: null, nextSibling: null};
    check(context + '/own-properties', [], () => Object.getOwnPropertyNames(attr));
    check(context + '/stringifier', [false, '[object Attr]', true], () => [
      Object.hasOwn(home.Attr.prototype, 'toString'), String(attr), attr.toString === home.Object.prototype.toString]);
    for (const [surface, realm] of [['home', home], ['main', globalThis], ['iframe', other]]) {
      for (const key of [...attrKeys, ...nodeKeys]) {
        const prefix = context + '/' + surface + '/' + key;
        check(prefix + '/descriptor', [true, true, 'function', 0, writable.includes(key) ? 'function' : 'undefined', writable.includes(key) ? 1 : null], () => {
          const d = descriptor(realm, key);
          return [d.enumerable, d.configurable, typeof d.get, d.get.length, typeof d.set, d.set ? d.set.length : null];
        });
        check(prefix + '/genuine', true, () => {
          const value = descriptor(realm, key).get.call(attr);
          if (key === 'ownerElement') return value === (attached ? owner : null);
          if (key === 'ownerDocument') return value === doc;
          if (key === 'baseURI') return value === Object.getOwnPropertyDescriptor(home.Node.prototype, 'baseURI').get.call(doc);
          if (key === 'childNodes') return value instanceof home.NodeList && value.length === 0 && value.item(0) === null
            && value === attr.childNodes && value[0] === undefined && Array.from(value).length === 0;
          return value === expected[key];
        });
        const revoked = Proxy.revocable(attr, {}); revoked.revoke();
        let traps = 0;
        const trapped = new Proxy(attr, {get() { traps++; throw new Error('trap'); }, getPrototypeOf() { traps++; throw new Error('trap'); }});
        const invalids = [null, undefined, {}, Object.create(home.Attr.prototype), Object.create(attr), new Proxy(attr, {}), revoked.proxy, trapped];
        check(prefix + '/reject-receivers', [true, 0], () => [invalids.every(receiver => {
          try { descriptor(realm, key).get.call(receiver); } catch (error) { return error instanceof realm.TypeError; }
          return false;
        }), traps]);
        if (writable.includes(key)) {
          check(prefix + '/reject-before-conversion', [true, 0, 0], () => {
            let conversions = 0;
            const value = {toString() { conversions++; return 'invalid'; }};
            const rejected = invalids.every(receiver => {
              try { descriptor(realm, key).set.call(receiver, value); } catch (error) { return error instanceof realm.TypeError; }
              return false;
            });
            return [rejected, conversions, traps];
          });
          check(prefix + '/conversion-error', [true, 1, 'old'], () => {
            const [a] = make(doc, attached); let conversions = 0, caught;
            const marker = new Error('conversion');
            try { descriptor(realm, key).set.call(a, {toString() { conversions++; throw marker; }}); } catch (error) { caught = error; }
            return [caught === marker, conversions, a.value];
          });
          check(prefix + '/symbol-error-realm', true, () => {
            const [a] = make(doc, attached);
            try { descriptor(realm, key).set.call(a, Symbol()); } catch (error) { return error instanceof realm.TypeError; }
            return false;
          });
          check(prefix + '/nullable', key === 'value' ? ['null', 'undefined'] : ['', ''], () => {
            const [a] = make(doc, attached), d = descriptor(realm, key);
            d.set.call(a, null); const first = a.value; d.set.call(a, undefined); return [first, a.value];
          });
          check(prefix + '/utf16-write', [[65, 55296, 0, 56320, 66], [65, 55296, 0, 56320, 66], 1], () => {
            const [a, e] = make(doc, attached); let conversions = 0;
            descriptor(realm, key).set.call(a, {toString() { conversions++; return 'A\ud800\0\udc00B'; }});
            return [units(a.value), units(attached ? e.getAttributeNS('urn:attribute', 'key') : a.cloneNode().value), conversions];
          });
        }
      }
    }
    for (const key of [...attrKeys, ...nodeKeys].filter(key => !writable.includes(key))) {
      check(context + '/readonly/' + key, 'TypeError', () => {
        try { (function() { 'use strict'; attr[key] = 42; })(); } catch (error) { return error.name; }
        return 'accepted';
      });
    }
    check(context + '/frozen-childNodes-same-object', true, () => {
      Object.preventExtensions(attr);
      return attr.childNodes === Object.getOwnPropertyDescriptor(other.Node.prototype, 'childNodes').get.call(attr);
    });
    check(context + '/direct-proxy-set', ['TypeError', 'old', 0], () => {
      const [a] = make(doc, attached); let error = '', conversions = 0;
      try { new Proxy(a, {}).value = {toString() { conversions++; return 'changed'; }}; } catch (e) { error = e.name; }
      return [error, a.value, conversions];
    });
    check(context + '/conversion-detaches-owner', ['updated', null, null], () => {
      const [a, e] = make(doc, true);
      a.value = {toString() { e.removeAttributeNode(a); return 'updated'; }};
      return [a.value, a.ownerElement, e.getAttributeNS('urn:attribute', 'key')];
    });
    check(context + '/conversion-adopts-owner', [true, true, 'updated', 'updated'], () => {
      const [a, e] = make(doc, true); const target = doc === other.document ? document : other.document;
      a.value = {toString() { target.adoptNode(e); return 'updated'; }};
      return [a.ownerDocument === target, e.ownerDocument === target, a.value, e.getAttributeNS('urn:attribute', 'key')];
    });
  }
  for (const [label, doc, home] of documents) {
    check(label + '/private-base-uri', [true, 0], () => {
      const [a, e] = make(doc, true); const expected = a.baseURI; let reads = 0;
      Object.defineProperty(e, 'ownerDocument', {configurable: true, get() { reads++; throw new Error('public owner'); }});
      Object.defineProperty(doc, 'baseURI', {configurable: true, get() { reads++; return 'forged'; }});
      const result = [Object.getOwnPropertyDescriptor(other.Node.prototype, 'baseURI').get.call(a) === expected, reads];
      delete doc.baseURI; return result;
    });
    check(label + '/registered-native-proxy', [1, label === 'xml' ? 'select' : 'SELECT', null, true, true], () => {
      const e = doc.createElement('select'); const p = Node.prototype;
      return [Object.getOwnPropertyDescriptor(p, 'nodeType').get.call(e),
        Object.getOwnPropertyDescriptor(p, 'nodeName').get.call(e),
        Object.getOwnPropertyDescriptor(p, 'nodeValue').get.call(e),
        Object.getOwnPropertyDescriptor(p, 'ownerDocument').get.call(e) === doc,
        Object.getOwnPropertyDescriptor(p, 'childNodes').get.call(e) instanceof home.NodeList];
    });
  }
  globalThis.__attrAccessorFailures = rows.filter(row => !row.pass);
  globalThis.__nodeReplacementResults = {total: rows.length, passed: rows.filter(row => row.pass).length, failures: __attrAccessorFailures, rows};
  return rows.every(row => row.pass);
})()
