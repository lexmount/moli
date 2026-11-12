function adoptedSelectionAssociations(childFirst) {
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const frame = body.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  // Keep both Selection objects and their native records for the whole matrix.
  // Trying both adoption directions in this VM exposes either HashMap order.
  if (childFirst) { other.getSelection(); getSelection(); }
  else { getSelection(); other.getSelection(); }
  const parentSelection = getSelection();
  const childSelection = other.getSelection();
  const mutations = [
    ['setStart', (r, t, w) => w.Range.prototype.setStart.call(r, t, 2)],
    ['setEnd', (r, t, w) => w.Range.prototype.setEnd.call(r, t, 3)],
    ['setStartBefore', (r, t, w) => w.Range.prototype.setStartBefore.call(r, t)],
    ['setStartAfter', (r, t, w) => w.Range.prototype.setStartAfter.call(r, t)],
    ['setEndBefore', (r, t, w) => w.Range.prototype.setEndBefore.call(r, t)],
    ['setEndAfter', (r, t, w) => w.Range.prototype.setEndAfter.call(r, t)],
    ['selectNode', (r, t, w) => w.Range.prototype.selectNode.call(r, t)],
    ['selectNodeContents', (r, t, w) => w.Range.prototype.selectNodeContents.call(r, t)],
    ['collapse-start', (r, t, w) => w.Range.prototype.collapse.call(r, true)],
    ['collapse-end', (r, t, w) => w.Range.prototype.collapse.call(r, false)]
  ];
  let cases = 0;
  for (const source of [window, other]) for (const caller of [window, other]) {
    const target = source === window ? other : window;
    const selected = source === window ? parentSelection : childSelection;
    const adopted = source === window ? childSelection : parentSelection;
    for (const [method, mutate] of mutations) {
      cases++;
      const label = [childFirst ? 'child-first' : 'parent-first',
        source === window ? 'parent-to-child' : 'child-to-parent',
        caller === window ? 'parent-caller' : 'child-caller', method].join(':');
      parentSelection.removeAllRanges(); childSelection.removeAllRanges();
      const host = source.document.body.appendChild(source.document.createElement('div'));
      const shadow = host.attachShadow({mode: 'open'});
      const text = shadow.appendChild(source.document.createTextNode('abcdef'));
      const range = source.document.createRange();
      range.selectNodeContents(text);
      if (method === 'selectNodeContents') { range.setStart(text, 1); range.setEnd(text, 5); }
      selected.addRange(range);
      check(selected.getRangeAt(0) === range, label + ': source retains initial Range');
      // Adoption changes native node ownership without any Range setter.
      target.document.body.appendChild(target.document.adoptNode(host));
      check(text.ownerDocument === target.document, label + ': adopted shadow owner');
      // Moli rejects getRangeAt() once the Range belongs to another Document,
      // but rangeCount still exposes the retained native association.
      check(selected.rangeCount === 1, label + ': source record survives adoption');
      adopted.addRange(range);
      check(adopted.rangeCount === 1 && adopted.getRangeAt(0) === range,
        label + ': target retains the same Range');
      mutate(range, text, caller);
      check(adopted.rangeCount === 1 && adopted.getRangeAt(0) === range,
        label + ': target Range identity after mutation');
      check(adopted.anchorNode === range.startContainer && adopted.anchorOffset === range.startOffset,
        label + ': anchor offset ' + adopted.anchorOffset + ' vs ' + range.startOffset);
      check(adopted.focusNode === range.endContainer && adopted.focusOffset === range.endOffset,
        label + ': focus offset ' + adopted.focusOffset + ' vs ' + range.endOffset);
      check(adopted.isCollapsed === range.collapsed, label + ': collapsed state');
      check(selected.rangeCount === 0 && selected.anchorNode === null && selected.focusNode === null,
        label + ': clear the association outside its owner Document');
      parentSelection.removeAllRanges(); childSelection.removeAllRanges();
      host.remove();
    }
  }
  frame.remove();
  return {cases, failures};
}
