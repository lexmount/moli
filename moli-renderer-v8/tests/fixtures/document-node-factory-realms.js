(() => {
  const rows = [];
  const check = (name, body) => {
    try {
      if (!body()) throw Error('false');
      rows.push({name, passed: true});
    } catch (error) {
      rows.push({name, passed: false, error: String(error)});
    }
  };
  const other = document.querySelector('#child').contentWindow;
  for (const [ci, callee] of [globalThis, other].entries()) {
    for (const [oi, owner] of [globalThis, other].entries()) {
      for (const flavor of ['live', 'html', 'xml', 'parsed', 'new']) {
        const doc = flavor === 'live' ? owner.document : flavor === 'html' ?
          owner.document.implementation.createHTMLDocument('nodes') : flavor === 'xml' ?
          owner.document.implementation.createDocument(null, 'r') : flavor === 'new' ?
          new owner.Document() : new owner.DOMParser().parseFromString('<r/>', 'application/xml');
        const prefix = `${ci}/${oi}/${flavor}`;
        const kinds = ['Text', 'Comment', 'ProcessingInstruction', 'DocumentFragment'];
        if (!['live', 'html'].includes(flavor)) kinds.push('CDATASection');
        for (const kind of kinds) {
          const ctor = owner[kind], proto = ctor.prototype;
          const make = () => {
            const methods = callee.Document.prototype, data = '\ud800a\udfff';
            switch (kind) {
              case 'Text': return methods.createTextNode.call(doc, data);
              case 'Comment': return methods.createComment.call(doc, data);
              case 'ProcessingInstruction': return methods.createProcessingInstruction.call(doc, 'target', data);
              case 'CDATASection': return methods.createCDATASection.call(doc, data);
              case 'DocumentFragment': return methods.createDocumentFragment.call(doc);
            }
          };
          check(`${prefix}/${kind}/prototype`, () => Object.getPrototypeOf(make()) === proto);
          check(`${prefix}/${kind}/constructor`, () => make().constructor === ctor);
          check(`${prefix}/${kind}/tag`, () => {
            const node = make();
            return !Object.hasOwn(node, Symbol.toStringTag) &&
              Object.prototype.toString.call(node) === `[object ${kind}]`;
          });
          check(`${prefix}/${kind}/owner`, () => make().ownerDocument === doc);
          check(`${prefix}/${kind}/inherited-members`, () => {
            const node = make();
            return ['nodeType', 'ownerDocument', 'textContent', 'nodeValue', 'cloneNode', 'appendChild']
              .every(name => !Object.hasOwn(node, name));
          });
          check(`${prefix}/${kind}/clone`, () => {
            const node = make(), clone = callee.Node.prototype.cloneNode.call(node, true);
            return clone !== node && Object.getPrototypeOf(clone) === proto &&
              clone.ownerDocument === doc && callee.Node.prototype.isEqualNode.call(node, clone);
          });
          check(`${prefix}/${kind}/tree`, () => {
            const node = make(), holder = doc.createElement('holder');
            if (kind === 'DocumentFragment') {
              const child = doc.createElement('a');
              callee.Node.prototype.appendChild.call(node, child);
              callee.Node.prototype.appendChild.call(holder, node);
              return node.childNodes.length === 0 && holder.firstChild === child && child.ownerDocument === doc;
            }
            callee.Node.prototype.appendChild.call(holder, node);
            const inserted = node.parentNode === holder && holder.firstChild === node;
            callee.Node.prototype.removeChild.call(holder, node);
            return inserted && node.parentNode === null;
          });
          if (kind !== 'DocumentFragment') {
            check(`${prefix}/${kind}/utf16`, () => {
              const node = make(), get = Object.getOwnPropertyDescriptor(callee.CharacterData.prototype, 'data').get;
              return get.call(node) === '\ud800a\udfff' && node.length === 3;
            });
            check(`${prefix}/${kind}/mutate`, () => {
              const node = make();
              callee.CharacterData.prototype.appendData.call(node, '\ud800');
              callee.CharacterData.prototype.replaceData.call(node, 1, 1, 'x');
              return node.data === '\ud800x\udfff\ud800' && node.length === 4;
            });
          }
          check(`${prefix}/${kind}/intrinsic`, () => {
            try {
              owner[kind] = function AuthorConstructor() { throw Error('author constructor'); };
              const node = make();
              return Object.getPrototypeOf(node) === proto && node.constructor === ctor;
            } finally {
              owner[kind] = ctor;
            }
          });
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
