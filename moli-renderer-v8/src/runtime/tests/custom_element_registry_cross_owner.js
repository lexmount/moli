(async () => {
  const popup = open('about:blank');
  const rows = [];
  const registry = new CustomElementRegistry();
  const doc = popup.document;
  const host = doc.createElement('div');
  const root = host.attachShadow({mode: 'open', customElementRegistry: registry});
  const name = 'cross-popup-late-upgrade';
  const node = doc.createElement(name);
  root.append(node);
  doc.body.append(host);
  const calls = [];
  class Element extends HTMLElement {
    constructor() { super(); calls.push(this.ownerDocument === doc); }
  }
  try {
    rows.push({case: 'association', rootRegistry: root.customElementRegistry === registry,
      inheritedRegistry: node.customElementRegistry === registry,
      ownerDocument: node.ownerDocument === doc});
    registry.define(name, Element);
    rows.push({case: 'late-upgrade', instance: node instanceof Element,
      ownerDocument: node.ownerDocument === doc,
      constructorCalls: calls.length, correctDocuments: calls.every(Boolean),
      sourceLookup: registry.get(name) === Element});
    const created = doc.createElement(name, {customElementRegistry: registry});
    root.append(created);
    rows.push({case: 'create', instance: created instanceof Element,
      ownerDocument: created.ownerDocument === doc,
      registry: created.customElementRegistry === registry,
      totalConstructorCalls: calls.length, correctDocuments: calls.every(Boolean)});
    popup.close();
    await new Promise(resolve => setTimeout(resolve, 500));
    class AfterClose extends HTMLElement {}
    const afterName = 'cross-popup-source-still-active';
    let resolved;
    registry.whenDefined(afterName).then(value => { resolved = value; });
    registry.define(afterName, AfterClose);
    await Promise.resolve();
    rows.push({case: 'target-retirement', oldDefinition: registry.get(name) === Element,
      newDefinition: registry.get(afterName) === AfterClose,
      whenDefined: resolved === AfterClose,
      rootRegistry: root.customElementRegistry === registry});
    return JSON.stringify({rows});
  } finally { popup.close(); }
})()
