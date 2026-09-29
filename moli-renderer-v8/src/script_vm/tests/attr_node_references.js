(() => {
  const other = document.getElementById('child').contentWindow;
  const documents = [['main', document, Attr], ['iframe', other.document, other.Attr],
    ['windowless', document.implementation.createHTMLDocument(''), Attr],
    ['xml', document.implementation.createDocument('urn:root', 'root'), Attr]];
  const rows = [];
  function check(name, expected, run) {
    let actual;
    try { actual = run(); } catch (error) { actual = {error: error.name}; }
    rows.push({name, expected, actual, pass: JSON.stringify(actual) === JSON.stringify(expected)});
  }
  const units = value => Array.from({length: value.length}, (_, i) => value.charCodeAt(i));
  function attribute(doc, namespace, name, value) {
    const node = doc.createAttributeNS(namespace, name); node.value = value; return node;
  }
  for (const [context, doc, AttrConstructor] of documents) {
    const a = attribute(doc, 'urn:attribute', 'p:key', 'value');
    const b = attribute(doc, 'urn:attribute', 'q:key', 'value');
    for (const [surface, api, ErrorConstructor] of [['own', a, context === 'iframe' ? other.TypeError : TypeError],
      ['top', Node.prototype, TypeError], ['iframe', other.Node.prototype, other.TypeError]]) {
      const prefix = context + '/' + surface;
      check(prefix + '/contains', [true, false, false], () =>
        [api.contains.call(a, a), api.contains.call(a, b), api.contains.call(a, null)]);
      check(prefix + '/identity', [true, false, false], () =>
        [api.isSameNode.call(a, a), api.isSameNode.call(a, b), api.isSameNode.call(a, null)]);
      check(prefix + '/equality', [true, true, false, false, false, false], () => [
        api.isEqualNode.call(a, a), api.isEqualNode.call(a, b),
        api.isEqualNode.call(a, attribute(doc, 'urn:other', 'p:key', 'value')),
        api.isEqualNode.call(a, attribute(doc, 'urn:attribute', 'p:other', 'value')),
        api.isEqualNode.call(a, attribute(doc, 'urn:attribute', 'p:key', 'other')),
        api.isEqualNode.call(a, null)]);
      check(prefix + '/self-position', 0, () => api.compareDocumentPosition.call(a, a));
      check(prefix + '/disconnected', true, () => {
        const ab = api.compareDocumentPosition.call(a, b), ba = api.compareDocumentPosition.call(b, a);
        return (ab & 33) === 33 && (ba & 33) === 33 && ((ab & 6) === ((ba & 6) ^ 6))
          && api.compareDocumentPosition.call(a, b) === ab;
      });
      check(prefix + '/clone', [true, true, true, true, null, 'value', 'p', 'urn:attribute', 'key'], () => {
        const clone = api.cloneNode.call(a, true);
        return [clone !== a, clone instanceof AttrConstructor, clone.ownerDocument === doc,
          api.isEqualNode.call(a, clone), clone.ownerElement, clone.value, clone.prefix, clone.namespaceURI, clone.localName];
      });
      check(prefix + '/forged-clone', true, () => {
        try { api.cloneNode.call({__moliAttrState: {name: 'fake', value: 'fake'}}); }
        catch (error) { return error instanceof ErrorConstructor; }
        return false;
      });
      check(prefix + '/leaf', [false, true, true, true], () => [api.hasChildNodes.call(a),
        api.getRootNode.call(a) === a, api.getRootNode.call(a, {composed: true}) === a,
        api.normalize.call(a) === undefined]);
      check(prefix + '/root-options-conversion', [true, 1], () => {
        let reads = 0; const marker = new Error('options'); let caught;
        try { api.getRootNode.call(a, {get composed() { reads++; throw marker; }}); } catch (error) { caught = error; }
        return [caught === marker, reads];
      });
      check(prefix + '/root-invalid-options', true, () => {
        try { api.getRootNode.call(a, 1); } catch (error) { return error instanceof ErrorConstructor; }
        return false;
      });
      check(prefix + '/remove-child', {error: 'NotFoundError'}, () => api.removeChild.call(a, b));
      check(prefix + '/remove-child-invalid-argument', true, () => {
        try { api.removeChild.call(a, {}); } catch (error) { return error instanceof ErrorConstructor; }
        return false;
      });
      for (const method of ['contains', 'isSameNode', 'isEqualNode', 'compareDocumentPosition', 'lookupNamespaceURI', 'lookupPrefix', 'isDefaultNamespace']) {
        check(prefix + '/required/' + method, true, () => {
          try { api[method].call(a); } catch (error) { return error instanceof ErrorConstructor; }
          return false;
        });
      }
      for (const receiver of [Object.create(a), new Proxy(a, {}), {}]) {
        check(prefix + '/namespace-receiver-before-conversion', [true, 0], () => {
          let conversions = 0;
          try { api.lookupNamespaceURI.call(receiver, {toString() { conversions++; return 'p'; }}); }
          catch (error) { return [error instanceof ErrorConstructor, conversions]; }
          return [false, conversions];
        });
      }
      for (const method of ['lookupNamespaceURI', 'lookupPrefix', 'isDefaultNamespace']) {
        check(prefix + '/namespace-ownerless-conversion/' + method, [true, 1], () => {
          let conversions = 0; const marker = new Error('convert'); let caught;
          try { api[method].call(a, {toString() { conversions++; throw marker; }}); } catch (error) { caught = error; }
          return [caught === marker, conversions];
        });
      }
    }
    check(context + '/inherited-method-descriptors', [false, false, true, true, 0, 1, true, true], () => [
      Object.hasOwn(a, 'cloneNode'), Object.hasOwn(a, 'lookupNamespaceURI'),
      a.cloneNode === (context === 'iframe' ? other.Node.prototype : Node.prototype).cloneNode,
      a.lookupNamespaceURI === (context === 'iframe' ? other.Node.prototype : Node.prototype).lookupNamespaceURI,
      a.cloneNode.length, a.lookupNamespaceURI.length,
      Object.getOwnPropertyDescriptor(Node.prototype, 'cloneNode').enumerable,
      Object.getOwnPropertyDescriptor(Node.prototype, 'lookupNamespaceURI').enumerable]);
    const parent = doc.createElement('div');
    const owner = parent.appendChild(doc.createElementNS('urn:element', 'p:owner'));
    const descendant = owner.appendChild(doc.createElement('span'));
    const first = attribute(doc, null, 'first', 'one'), second = attribute(doc, null, 'second', 'two');
    owner.setAttributeNode(first); owner.setAttributeNode(second);
    check(context + '/attached-position', [20, 10, 36, 34, 20, 10, 4, 2, false, false], () => [
      owner.compareDocumentPosition(first), first.compareDocumentPosition(owner), first.compareDocumentPosition(second),
      second.compareDocumentPosition(first), parent.compareDocumentPosition(first), first.compareDocumentPosition(parent),
      first.compareDocumentPosition(descendant), descendant.compareDocumentPosition(first), owner.contains(first), first.contains(owner)]);
    owner.removeAttributeNode(first); owner.setAttributeNode(first);
    check(context + '/reinsert-position', [34, 36], () => [first.compareDocumentPosition(second), second.compareDocumentPosition(first)]);
    owner.setAttributeNS('http://www.w3.org/2000/xmlns/', 'xmlns', 'urn:default');
    for (const data of [doc.createComment('comment'), doc.createTextNode('text'), doc.createProcessingInstruction('target', 'data')]) {
      owner.appendChild(data);
      check(context + '/character-data-namespace/' + data.nodeType, ['p', 'p', 'p'], () => [
        data.lookupPrefix('urn:element'), Node.prototype.lookupPrefix.call(data, 'urn:element'),
        other.Node.prototype.lookupPrefix.call(data, 'urn:element')]);
    }
    let publicCalls = 0;
    for (const method of ['getAttribute', 'getAttributeNS', 'lookupNamespaceURI', 'lookupPrefix', 'isDefaultNamespace']) {
      Object.defineProperty(owner, method, {configurable: true, value() { publicCalls++; throw new Error('public method'); }});
    }
    Object.defineProperty(owner, 'ownerDocument', {configurable: true, get() { publicCalls++; return {}; }});
    check(context + '/private-owner', [true, 'one', 'urn:element', 'p', true, 'urn:default', 0], () => [
      first.ownerDocument === doc, first.value, first.lookupNamespaceURI('p'), first.lookupPrefix('urn:element'),
      first.isDefaultNamespace('urn:default'), first.lookupNamespaceURI(null), publicCalls]);
    check(context + '/private-owner-clone', [true, true, 'one', 0], () => {
      const clone = Node.prototype.cloneNode.call(first);
      return [clone.ownerDocument === doc, clone instanceof AttrConstructor, clone.value, publicCalls];
    });
    const u = doc.createElement('div');
    u.setAttribute('data-value', '\ud800');
    const unicode = u.getAttributeNode('data-value');
    check(context + '/native-utf16-read-clone', [[55296], [55296], [55296], [55296], true], () => [
      units(unicode.value), units(unicode.nodeValue), units(unicode.textContent),
      units(Node.prototype.cloneNode.call(unicode).value), unicode.isEqualNode(unicode.cloneNode())]);
    const v = doc.createElement('div'); v.setAttribute('data-value', '\ud801');
    check(context + '/native-utf16-distinct', false, () => unicode.isEqualNode(v.getAttributeNode('data-value')));
    let reads = 0;
    const left = attribute(doc, 'urn:attribute', 'x:key', 'same'), right = attribute(doc, 'urn:attribute', 'y:key', 'same');
    for (const node of [left, right]) for (const key of ['nodeType', 'namespaceURI', 'localName', 'value', 'ownerElement'])
      Object.defineProperty(node, key, {configurable: true, get() { reads++; throw new Error('public property'); }});
    check(context + '/private-equality', [true, 0], () => [Node.prototype.isEqualNode.call(left, right), reads]);
    check(context + '/native-proxy-node', [true, true], () => {
      const select = doc.createElement('select');
      return [Node.prototype.contains.call(select, select), Node.prototype.isSameNode.call(select, select)];
    });
  }
  const pool = Array.from({length: 32}, (_, i) => Object.preventExtensions(document.createAttribute('a' + i)));
  check('disconnected/stable-total-order', true, () => {
    const sorted = pool.slice().sort((a, b) => a.compareDocumentPosition(b) & 4 ? -1 : 1);
    return sorted.every((a, i) => sorted.every((b, j) =>
      a.compareDocumentPosition(b) === (i === j ? 0 : i < j ? 37 : 35)));
  });
  const childAttribute = attribute(other.document, null, 'owned', 'value');
  const childOwner = other.document.createElement('div'); childOwner.setAttributeNode(childAttribute);
  const nextDocument = document.implementation.createHTMLDocument(''); nextDocument.adoptNode(childOwner);
  check('adoption/owner-document', [true, true], () => [childAttribute.ownerDocument === nextDocument,
    Node.prototype.cloneNode.call(childAttribute).ownerDocument === nextDocument]);
  globalThis.__attrNodeReferenceFailures = rows.filter(row => !row.pass);
  globalThis.__nodeReplacementResults = {total: rows.length, passed: rows.filter(row => row.pass).length, failures: __attrNodeReferenceFailures, rows};
  return rows.every(row => row.pass);
})()
