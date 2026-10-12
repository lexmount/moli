(owner => {
  const checks = [];
  const check = (name, passed, observed = null) => checks.push({name, passed, observed});
  const frames = [owner.document.createElement('iframe'), owner.document.createElement('iframe')];
  frames.forEach(frame => owner.document.body.appendChild(frame));
  const allowed = frames[0].contentWindow, blocked = frames[1].contentWindow;
  const policy = blocked.document.createElement('meta');
  policy.httpEquiv = 'Content-Security-Policy';
  policy.content = "script-src-attr 'none' 'report-sample'";
  blocked.document.head.appendChild(policy);
  const source = "this.setAttribute('data-called', 'yes')";
  const factories = [
    ['Document', realm => new realm.Document()],
    ['createHTMLDocument', realm => realm.document.implementation.createHTMLDocument('')],
    ['createDocument', realm => realm.document.implementation.createDocument(null, '', null)],
    ['DOMParser-HTML', realm => new realm.DOMParser().parseFromString('<!doctype html><body>', 'text/html')],
    ['DOMParser-XHTML', realm => new realm.DOMParser().parseFromString('<html xmlns="http://www.w3.org/1999/xhtml"/>', 'application/xhtml+xml')],
    ['DOMParser-SVG', realm => new realm.DOMParser().parseFromString('<svg xmlns="http://www.w3.org/2000/svg"/>', 'image/svg+xml')],
    ['clone-Document', realm => realm.document.cloneNode(false)],
    ['clone-windowless', realm => realm.document.implementation.createHTMLDocument('').cloneNode(true)],
    ['parseHTMLUnsafe', realm => realm.Document.parseHTMLUnsafe('<!doctype html><body>')],
    ['template-owner', realm => realm.document.createElement('template').content.ownerDocument],
  ];
  const mutations = [
    ['setAttribute', (caller, doc, node) => caller.Element.prototype.setAttribute.call(node, 'onclick', source)],
    ['setAttributeNS', (caller, doc, node) => caller.Element.prototype.setAttributeNS.call(node, null, 'onclick', source)],
    ['Attr.value', (caller, doc, node) => {
      node.setAttribute('onclick', 'return 1');
      Object.getOwnPropertyDescriptor(caller.Attr.prototype, 'value').set.call(node.getAttributeNode('onclick'), source);
    }],
    ['setAttributeNode', (caller, doc, node) => {
      const attr = doc.createAttribute('onclick'); attr.value = source;
      caller.Element.prototype.setAttributeNode.call(node, attr);
    }],
    ['importNode', (caller, doc, node) => {
      node.setAttribute('onclick', source);
      return caller.Document.prototype.importNode.call(doc, node, true);
    }],
  ];
  for (const [label, creator, destination, permitted] of [
    ['allowed-to-blocked', allowed, blocked, true],
    ['blocked-to-allowed', blocked, allowed, false],
  ]) {
    for (const [factory, make] of factories) {
      for (const [mutation, mutate] of mutations) {
        const key = label + ':' + factory + ':' + mutation;
        try {
          const doc = make(creator);
          let node = doc.createElementNS('http://www.w3.org/1999/xhtml', 'button');
          node = mutate(destination, doc, node) || node;
          check(key + ':windowless', doc.defaultView === null);
          check(key + ':attribute-retained', node.getAttribute('onclick') === source);
          check(key + ':inert-getter', node.onclick === null);
          destination.document.body.appendChild(node);
          check(key + ':adopted', node.ownerDocument === destination.document);
          node.dispatchEvent(new destination.Event('click'));
          check(key + ':creation-global-policy', (node.getAttribute('data-called') === 'yes') === permitted);
          check(key + ':handler-value', permitted ? typeof node.onclick === 'function' : node.onclick === null);
          node.remove();
        } catch (error) {check(key + ':unexpected-exception', false, String(error));}
      }
    }
    const markup = '<button xmlns="http://www.w3.org/1999/xhtml" onclick="' + source + '"></button>';
    const parsed = [
      ['parsed-HTML', () => new creator.DOMParser().parseFromString(markup, 'text/html')],
      ['parsed-XHTML', () => new creator.DOMParser().parseFromString(markup, 'application/xhtml+xml')],
      ['parsed-XML', () => new creator.DOMParser().parseFromString(markup, 'application/xml')],
      ['parsed-SVG', () => new creator.DOMParser().parseFromString('<svg xmlns="http://www.w3.org/2000/svg">' + markup + '</svg>', 'image/svg+xml')],
      ['parsed-static', () => creator.Document.parseHTMLUnsafe(markup)],
      ['parsed-innerHTML', () => {const doc=creator.document.implementation.createHTMLDocument(''); doc.body.innerHTML=markup; return doc;}],
      ['parsed-template', () => {const template=creator.document.createElement('template'); template.innerHTML=markup; return template.content;}],
    ];
    for (const [name, make] of parsed) {
      const key = label + ':' + name;
      try {
        const root = make(), node = root.querySelector('[onclick]');
        check(key + ':windowless', node.ownerDocument.defaultView === null);
        check(key + ':attribute-retained', node.getAttribute('onclick') === source);
        check(key + ':inert-getter', node.onclick === null);
        destination.document.body.appendChild(node);
        check(key + ':adopted', node.ownerDocument === destination.document);
        node.dispatchEvent(new destination.Event('click'));
        check(key + ':creation-global-policy', (node.getAttribute('data-called') === 'yes') === permitted);
        check(key + ':handler-value', permitted ? typeof node.onclick === 'function' : node.onclick === null);
        node.remove();
      } catch (error) {check(key + ':unexpected-exception', false, String(error));}
    }
  }
  frames.forEach(frame => frame.remove());
  return {complete:true, total:checks.length, passed:checks.filter(row => row.passed).length, checks};
})
