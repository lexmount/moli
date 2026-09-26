(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const selection = getSelection();
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  let calls = 0;
  const forbidden = () => { calls++; throw new Error('author relationship getter entered'); };
  for (const mode of ['light', 'open', 'closed']) {
    const host = fixture.appendChild(document.createElement('div'));
    const parent = mode === 'light' ? host : host.attachShadow({mode});
    const text = parent.appendChild(document.createTextNode('selected'));
    const range = document.createRange();
    range.setStart(text, 1);
    range.setEnd(text, 4);
    selection.removeAllRanges();
    const nodes = [...new Set([host, parent, text])];
    try {
      for (const node of nodes) for (const key of ['parentNode', 'nodeType', 'host', 'ownerDocument']) {
        Object.defineProperty(node, key, {configurable: true, get: forbidden});
      }
      selection.addRange(range);
      check(selection.rangeCount === 1, mode + ': connected range accepted');
      if (selection.rangeCount) check(selection.getRangeAt(0) === range, mode + ': associated range returned');
    } catch (error) {
      failures.push(mode + ': ' + error.name);
    } finally {
      for (const node of nodes) for (const key of ['parentNode', 'nodeType', 'host', 'ownerDocument']) delete node[key];
    }
    check(calls === 0, mode + ': no author getters');
    host.remove();
  }
  const frame = fixture.appendChild(document.createElement('iframe'));
  const doc = frame.contentDocument;
  const foreign = doc.body.appendChild(doc.createTextNode('foreign'));
  const detached = document.createTextNode('detached');
  for (const [label, node] of [['foreign', foreign], ['detached', detached]]) {
    const range = document.createRange();
    range.setStart(node, 0);
    range.setEnd(node, 1);
    selection.removeAllRanges();
    try {
      Object.defineProperty(node, 'parentNode', {configurable: true, value: document});
      Object.defineProperty(node, 'ownerDocument', {configurable: true, value: document});
      selection.addRange(range);
      check(selection.rangeCount === 0, label + ': forged document relationship ignored');
    } finally {
      delete node.parentNode;
      delete node.ownerDocument;
    }
  }
  fixture.remove();
  return failures.join('\n');
})()
