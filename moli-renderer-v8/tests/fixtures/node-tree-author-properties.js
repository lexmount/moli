(() => {
  const rows = [], other = document.querySelector('#child').contentWindow;
  for (const [oi, owner] of [globalThis, other].entries()) for (const flavor of ['live', 'html', 'xml', 'parsed', 'new']) {
    const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
      owner.document.implementation.createHTMLDocument('mutation') : flavor === 'xml' ?
      owner.document.implementation.createDocument(null, 'r') : flavor === 'new' ?
      new owner.Document() : new owner.DOMParser().parseFromString('<r/>', 'application/xml');
    const parentGetter = Object.getOwnPropertyDescriptor(owner.Node.prototype, 'parentNode').get;
    for (const method of ['appendChild', 'insertBefore', 'replaceChild', 'removeChild']) {
      const source = owner.document.createElement('source'), child = owner.document.createElement('child');
      const descendant = owner.document.createElement('descendant'); child.appendChild(descendant);source.appendChild(child);
      const destination = doc.createElement('destination'), old = doc.createComment('old');destination.appendChild(old);
      if (method === 'removeChild') destination.appendChild(child);
      let reads = 0;
      const getter = () => {reads++; throw Error('author parentNode getter');};
      Object.defineProperty(child, 'parentNode', {get: getter, configurable: true});
      Object.defineProperty(source, 'removeChild', {value() {throw Error('author removeChild');}, configurable: true});
      try {
        const args = method === 'insertBefore' ? [child, old] : method === 'replaceChild' ? [child, old] : [child];
        const returned = owner.Node.prototype[method].apply(destination, args);
        const nativeParent = parentGetter.call(child);
        const descendantsPreserved = child.firstChild === descendant && descendant.parentNode === child &&
          child.ownerDocument === doc && descendant.ownerDocument === doc;
        const descriptor = Object.getOwnPropertyDescriptor(child, 'parentNode');
        const passed = reads === 0 && descriptor && descriptor.get === getter &&
          nativeParent === (method === 'removeChild' ? null : destination) && descendantsPreserved &&
          child.ownerDocument === doc && descendant.ownerDocument === doc &&
          returned === (method === 'replaceChild' ? old : child) && !source.contains(child);
        rows.push({name: `${oi}/${flavor}/${method}/preserve-own-getter`, passed: !!passed,
          error: passed ? undefined : JSON.stringify({reads, descriptor: !!descriptor, sameGetter: !!descriptor && descriptor.get === getter})});
      } catch (error) {rows.push({name: `${oi}/${flavor}/${method}/preserve-own-getter`, passed: false, error: String(error)});}
    }
  }
  try {
    const doc = document.implementation.createDocument(null, 'container', null);
    const nodes = [document.createElement('div')];
    for (let i = 0; i < 514; i++) {
      const child = document.createElement('div');
      nodes[nodes.length - 1].appendChild(child);
      nodes.push(child);
    }
    const returned = doc.documentElement.appendChild(nodes[0]);
    const passed = returned === nodes[0] && doc.documentElement.childNodes.length === 1 &&
      nodes.every((node, i) => node.ownerDocument === doc &&
        node.parentNode === (i ? nodes[i - 1] : doc.documentElement) &&
        node.firstChild === (nodes[i + 1] || null));
    rows.push({name: 'deep-native-subtree-preserves-identity-and-owner', passed});
  } catch (error) {rows.push({name: 'deep-native-subtree-preserves-identity-and-owner', passed: false, error: String(error)});}
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
