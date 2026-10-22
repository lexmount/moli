(() => {
  const rows = [], realms = [['main', globalThis], ['child', document.querySelector('#child').contentWindow]];
  const same = (a, b) => a.length === b.length && a.every((value, index) => value === b[index]);
  for (const [ownerName, owner] of realms) for (const [calleeName, callee] of realms) {
    for (const sink of ['innerHTML', 'textContent', 'replaceChildren', 'setHTMLUnsafe']) for (const replacement of [false, true]) {
      const name = `${calleeName}/${ownerName}/${sink}/${replacement}`;
      const host = owner.document.body.appendChild(owner.document.createElement('div'));
      const nodes = ['i', 'b', 'u'].map(tag => host.appendChild(owner.document.createElement(tag)));
      const observer = new owner.MutationObserver(() => {});
      observer.observe(host, {childList: true});
      try {
        host.removeChild(nodes[0]);
        host.insertBefore(nodes[0], nodes[1]);
        if (sink === 'innerHTML') Object.getOwnPropertyDescriptor(callee.Element.prototype, 'innerHTML').set.call(host,
          replacement ? '<em>new</em>' : '');
        else if (sink === 'textContent') Object.getOwnPropertyDescriptor(callee.Node.prototype, 'textContent').set.call(host,
          replacement ? 'new' : '');
        else if (sink === 'replaceChildren') {
          if (replacement) callee.Element.prototype.replaceChildren.call(host, owner.document.createElement('em'));
          else callee.Element.prototype.replaceChildren.call(host);
        } else callee.Element.prototype.setHTMLUnsafe.call(host, replacement ? '<em>new</em>' : '');
        const records = observer.takeRecords();
        const added = [...host.childNodes];
        const passed = records.length === 3 && records.every(record => record.target === host) &&
          same([...records[0].removedNodes], [nodes[0]]) && records[0].addedNodes.length === 0 &&
          records[0].previousSibling === null && records[0].nextSibling === nodes[1] &&
          same([...records[1].addedNodes], [nodes[0]]) && records[1].removedNodes.length === 0 &&
          records[1].previousSibling === null && records[1].nextSibling === nodes[1] &&
          same([...records[2].removedNodes], nodes) && same([...records[2].addedNodes], added) &&
          records[2].previousSibling === null && records[2].nextSibling === null;
        rows.push({name, passed});
      } catch (error) {rows.push({name, passed: false, error: String(error)});}
      finally {observer.disconnect();host.remove();}
    }
  }
  globalThis.__uiEventResults = {complete: true, total: rows.length, passed: rows.filter(row => row.passed).length, rows};
  return rows.every(row => row.passed);
})()
