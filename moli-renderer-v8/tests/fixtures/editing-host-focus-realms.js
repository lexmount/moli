(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const reset = fixture.appendChild(document.createElement('button'));
  const selection = getSelection();
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  function at(range, node, start, end = start) {
    return range && range.startContainer === node && range.startOffset === start &&
      range.endContainer === node && range.endOffset === end;
  }
  for (const mode of ['open', 'closed']) {
    const host = fixture.appendChild(document.createElement('div'));
    const outer = host.attachShadow({mode});
    const nested = outer.appendChild(document.createElement('div'));
    const inner = nested.attachShadow({mode});
    const editor = inner.appendChild(document.createElement('div'));
    editor.contentEditable = 'true';
    editor.innerHTML = '<p> abc</p>';
    selection.removeAllRanges();
    editor.focus();
    check(selection.rangeCount === 1, mode + ': selection exists');
    if (selection.rangeCount) {
      const range = selection.getRangeAt(0);
      const index = Array.prototype.indexOf.call(fixture.childNodes, host);
      check(at(range, fixture, index), mode + ': range before outer host');
      check(at(selection.getComposedRanges()[0], fixture, index, index + 1), mode + ': composed outer host');
      check(at(selection.getComposedRanges({shadowRoots: [outer]})[0], outer, 0, 1), mode + ': composed inner host');
      check(at(selection.getComposedRanges({shadowRoots: [inner]})[0], editor, 0), mode + ': raw editor boundary');
      reset.focus();
      editor.focus();
      check(selection.getRangeAt(0) === range, mode + ': refocus retains shadow range');
    }
    host.remove();
  }
  const editor = fixture.appendChild(document.createElement('div'));
  editor.contentEditable = 'true';
  editor.textContent = 'parent editor';
  editor.focus();
  const topRange = selection.getRangeAt(0);
  const frame = fixture.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  const doc = other.document;
  const childBody = doc.body || doc.documentElement.appendChild(doc.createElement('body'));
  const childEditor = childBody.appendChild(doc.createElement('div'));
  childEditor.contentEditable = 'plaintext-only';
  childEditor.textContent = 'child editor';
  const childSelection = other.getSelection();
  const OtherRange = other.Range;
  other.Range = function () { throw new Error('replaced child Range'); };
  HTMLElement.prototype.focus.call(childEditor);
  check(childSelection.rangeCount === 1, 'borrowed focus updates owner selection');
  if (childSelection.rangeCount) {
    const range = childSelection.getRangeAt(0);
    check(range instanceof OtherRange && !(range instanceof Range) && at(range, childEditor.firstChild, 0),
      'initial range belongs to owner realm');
    other.HTMLElement.prototype.focus.call(editor);
    check(childSelection.getRangeAt(0) === range, 'parent focus retains child range');
  }
  check(selection.getRangeAt(0) === topRange, 'child focus preserves parent selection');
  fixture.remove();
  return failures.join('\n');
})()
