(() => {
  const rows = [];
  const realms = [['main', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const same = (a, b) => a.length === b.length && a.every((value, index) => value === b[index]);
  for (const [ownerName, owner] of realms) {
    const callbacks = [];
    owner.customElements.define('bulk-removal-child', class extends owner.HTMLElement {
      disconnectedCallback() {
        callbacks.push({node: this, siblingsGone: this.savedSiblings.every(sibling => sibling.parentNode === null),
          replacement: this.savedParent.firstChild});
      }
    });
    for (const [calleeName, callee] of realms) {
      for (const sink of ['innerHTML', 'textContent', 'replaceChildren', 'setHTMLUnsafe']) {
        for (const replace of [false, true]) {
          const name = `${calleeName}/${ownerName}/${sink}/${replace}`;
          const host = owner.document.body.appendChild(owner.document.createElement('div'));
          const nodes = ['alpha', 'middle', 'omega'].map(text => {
            const node = host.appendChild(owner.document.createElement('bulk-removal-child'));
            node.appendChild(owner.document.createTextNode(text));
            return node;
          });
          nodes.forEach(node => {node.savedSiblings = nodes; node.savedParent = host;});
          callbacks.length = 0;
          const firstText = nodes[0].firstChild;
          const inside = owner.document.createRange(); inside.setStart(firstText, 1); inside.setEnd(firstText, 3);
          const boundary = owner.document.createRange(); boundary.setStart(host, 1); boundary.setEnd(host, 3);
          const subtreeIterator = owner.document.createNodeIterator(nodes[0]);
          subtreeIterator.nextNode();subtreeIterator.nextNode();
          const walker = owner.document.createTreeWalker(host);walker.currentNode = firstText;
          const observer = new owner.MutationObserver(() => {});
          observer.observe(host, {childList: true, subtree: true, characterData: true, characterDataOldValue: true});
          try {
            if (sink === 'innerHTML') Object.getOwnPropertyDescriptor(callee.Element.prototype, 'innerHTML').set.call(host,
              replace ? '<u>replacement</u>' : '');
            else if (sink === 'textContent') Object.getOwnPropertyDescriptor(callee.Node.prototype, 'textContent').set.call(host,
              replace ? 'replacement' : '');
            else if (sink === 'replaceChildren') {
              if (replace) callee.Element.prototype.replaceChildren.call(host, owner.document.createElement('u'));
              else callee.Element.prototype.replaceChildren.call(host);
            } else callee.Element.prototype.setHTMLUnsafe.call(host, replace ? '<u>replacement</u>' : '');
            const added = [...host.childNodes];
            const records = observer.takeRecords();
            const failures = [];
            if (![inside, boundary].every(range => range.startContainer === host && range.endContainer === host &&
                range.startOffset === 0 && range.endOffset === 0 && range.collapsed)) failures.push('ranges');
            if (subtreeIterator.referenceNode !== firstText || subtreeIterator.nextNode() !== null) failures.push('subtree iterator');
            if (walker.currentNode !== firstText) failures.push('walker');
            if (!same(callbacks.map(row => row.node), nodes) || !callbacks.every(row => row.siblingsGone && row.replacement === host.firstChild))
              failures.push('reactions');
            if (records.length !== 1 || records[0].target !== host || records[0].previousSibling !== null || records[0].nextSibling !== null ||
                !same([...records[0].removedNodes], nodes) || !same([...records[0].addedNodes], added)) failures.push('replacement record');
            rows.push({name, passed: failures.length === 0, failures});
          } catch (error) {rows.push({name, passed: false, error: String(error)});}
          finally {observer.disconnect();host.remove();}
        }
      }
    }
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
