(() => {
  const popup = open('about:blank', 'registry-document-owners');
  const rows = [];
  try {
    for (const [targetName, target, other] of [
      ['main', window, popup],
      ['popup', popup, window],
    ]) {
      for (const [calleeName, callee] of [['own', target], ['borrowed', other]]) {
        const document = target.document;
        const methods = callee.Document.prototype;
        const operations = [
          ['createElement', registry => methods.createElement.call(document, 'div', {
            customElementRegistry: registry,
          })],
          ['createElementNS', registry => methods.createElementNS.call(document,
            'http://www.w3.org/1999/xhtml', 'div', {customElementRegistry: registry})],
          ['attachShadow', registry => callee.Element.prototype.attachShadow.call(
            document.createElement('div'), {mode: 'open', customElementRegistry: registry})],
          ['importNode', registry => methods.importNode.call(document,
            document.createElement('div'), {customElementRegistry: registry})],
          ['initialize', registry => {
            const node = document.createElement('div');
            callee.CustomElementRegistry.prototype.initialize.call(registry, node);
            return node;
          }],
        ];
        for (const [method, invoke] of operations) {
          let foreignRejected = false;
          let exceptionRealm = false;
          try {
            invoke(other.customElements);
          } catch (error) {
            foreignRejected = error.name === 'NotSupportedError';
            exceptionRealm = error instanceof callee.DOMException;
          }
          let ownAccepted = false;
          let ownIdentity = false;
          try {
            const result = invoke(target.customElements);
            ownAccepted = true;
            ownIdentity = result.customElementRegistry === target.customElements;
          } catch (_) {
            // Retain the failed control in this row and continue every case.
          }
          rows.push({case: `${targetName}-${calleeName}-${method}`,
            foreignRejected, exceptionRealm, ownAccepted, ownIdentity});
        }
      }
    }
    return JSON.stringify({rows});
  } finally {
    popup.close();
  }
})()
