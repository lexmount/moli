(() => {
  const rows = [];
  const check = (name, body) => {
    try { rows.push({name, passed: body() === true}); }
    catch (error) { rows.push({name, passed: false, error: String(error)}); }
  };
  const realms = [['main', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const makeDocument = (owner, flavor) => flavor === 'live' ? owner.document :
    flavor === 'html' ? owner.document.implementation.createHTMLDocument('fragment') :
    new owner.DOMParser().parseFromString('<!doctype html><title>fragment</title><body>', 'text/html');
  const makeContext = (doc, kind) => {
    const table = doc.createElement('table');
    if (kind === 'table') return {node: table, context: table};
    if (kind === 'text' || kind === 'comment') {
      const node = table.appendChild(kind === 'text' ? doc.createTextNode('x') : doc.createComment('x'));
      return {node, context: table};
    }
    const body = doc.createElement('body');
    if (kind === 'document') return {node: doc, context: body};
    if (kind === 'fragment') return {node: doc.createDocumentFragment(), context: body};
    if (kind === 'orphan-text') return {node: doc.createTextNode('x'), context: body};
    if (kind === 'html') return {node: doc.documentElement, context: body};
    const node = doc.createElement('div');
    return {node, context: node};
  };
  for (const [ownerName, owner] of realms) for (const flavor of ['live', 'html', 'parsed']) {
    const doc = makeDocument(owner, flavor);
    for (const [calleeName, callee] of realms) {
      const parse = callee.Range.prototype.createContextualFragment;
      for (const kind of ['table', 'text', 'comment', 'div', 'document', 'fragment', 'orphan-text', 'html']) {
        for (const property of [null, 'nodeType', 'parentNode', 'ownerDocument']) {
          check(`${calleeName}/${ownerName}/${flavor}/${kind}/${property || 'plain'}`, () => {
            const {node, context} = makeContext(doc, kind);
            const reference = doc.createRange(); reference.selectNodeContents(context);
            const expected = reference.createContextualFragment('<tr><td>x</td></tr>');
            const range = doc.createRange(); range.setStart(node, 0);
            let reads = 0;
            const previous = property && Object.getOwnPropertyDescriptor(node, property);
            if (property) Object.defineProperty(node, property, {configurable: true, get() {reads++; throw Error('public ' + property);}});
            let fragment;
            try { fragment = parse.call(range, '<tr><td>x</td></tr>'); }
            finally {
              if (property) {
                if (previous) Object.defineProperty(node, property, previous);
                else delete node[property];
              }
            }
            return reads === 0 && fragment.ownerDocument === doc && fragment.isEqualNode(expected) &&
              Object.getPrototypeOf(fragment) === owner.DocumentFragment.prototype &&
              range.startContainer === node && range.startOffset === 0;
          });
        }
      }
      check(`${calleeName}/${ownerName}/${flavor}/conversion changes start`, () => {
        const range = doc.createRange(), table = doc.createElement('table');
        range.selectNodeContents(doc.createElement('div'));
        let conversions = 0;
        const fragment = parse.call(range, {toString() {
          conversions++; range.selectNodeContents(table); return '<tr><td>x</td></tr>';
        }});
        return conversions === 1 && fragment.firstChild.localName === 'tbody' &&
          fragment.ownerDocument === doc && range.startContainer === table;
      });
      check(`${calleeName}/${ownerName}/${flavor}/conversion exception identity`, () => {
        const range = doc.createRange(), sentinel = {};
        try { parse.call(range, {toString() {throw sentinel;}}); return false; }
        catch (error) { return error === sentinel; }
      });
    }
    for (const [rangeName, rangeOwner] of realms) check(`${rangeName}/${ownerName}/${flavor}/moved range owner realm`, () => {
      const range = new rangeOwner.Range(), context = doc.createElement('table');
      range.selectNodeContents(context);
      const fragment = rangeOwner.Range.prototype.createContextualFragment.call(range, '<tr><td>x</td></tr>');
      return fragment.ownerDocument === doc && Object.getPrototypeOf(fragment) === rangeOwner.DocumentFragment.prototype &&
        Object.getPrototypeOf(fragment.firstChild) === rangeOwner.HTMLTableSectionElement.prototype &&
        fragment.firstChild.localName === 'tbody';
    });
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length,
    passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
