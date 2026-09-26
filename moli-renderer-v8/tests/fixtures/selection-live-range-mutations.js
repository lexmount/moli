(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const selection = getSelection();
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  function endpoints(s, r, backward, label) {
    check(s.rangeCount === 1 && s.getRangeAt(0) === r, label + ': range identity');
    const a = backward && !r.collapsed ? 'end' : 'start';
    const f = backward && !r.collapsed ? 'start' : 'end';
    check(s.anchorNode === r[a + 'Container'] && s.anchorOffset === r[a + 'Offset'] &&
      s.focusNode === r[f + 'Container'] && s.focusOffset === r[f + 'Offset'], label + ': observable endpoints');
    check(s.isCollapsed === r.collapsed && s.direction === (r.collapsed ? 'none' : backward ? 'backward' : 'forward'),
      label + ': direction and collapsed state');
    const c = s.getComposedRanges()[0];
    check(c.startContainer === r.startContainer && c.startOffset === r.startOffset &&
      c.endContainer === r.endContainer && c.endOffset === r.endOffset, label + ': composed endpoints');
  }
  for (const backward of [false, true]) {
    for (const action of ['delete-data', 'insert-data', 'reset-data', 'split-text', 'remove-text', 'remove-before']) {
      fixture.innerHTML = '<div><span>abcdef</span><span>xyz</span><span>last</span></div>';
      const editor = fixture.firstChild;
      const text = editor.firstChild.firstChild;
      if (action === 'remove-before') selection.collapse(editor, 2);
      else selection.setBaseAndExtent(text, backward ? 5 : 1, text, backward ? 1 : 5);
      const range = selection.getRangeAt(0);
      if (action === 'delete-data') text.deleteData(0, 2);
      if (action === 'insert-data') text.insertData(0, 'XY');
      if (action === 'reset-data') text.data = 'new';
      if (action === 'split-text') text.splitText(2);
      if (action === 'remove-text') editor.firstChild.remove();
      if (action === 'remove-before') editor.children[1].remove();
      endpoints(selection, range, backward, action + ':' + backward);
    }
  }
  fixture.innerHTML = '<p>parent</p><iframe></iframe>';
  selection.setBaseAndExtent(fixture.firstChild.firstChild, 1, fixture.firstChild.firstChild, 4);
  const retained = selection.getRangeAt(0);
  const other = fixture.lastChild.contentWindow;
  const text = other.document.body.appendChild(other.document.createTextNode('abcdef'));
  const childSelection = other.getSelection();
  childSelection.setBaseAndExtent(text, 5, text, 1);
  const childRange = childSelection.getRangeAt(0);
  CharacterData.prototype.deleteData.call(text, 0, 2);
  endpoints(childSelection, childRange, true, 'borrowed CharacterData mutation in child realm');
  endpoints(selection, retained, false, 'unrelated parent selection');
  fixture.remove();
  return failures.join('\n');
})()
