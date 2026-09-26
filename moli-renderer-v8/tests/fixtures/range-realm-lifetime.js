(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const text = fixture.appendChild(document.createTextNode('abcd'));
  const range = document.createRange();
  range.setStart(text, 1);
  range.setEnd(text, 3);
  const selection = getSelection();
  selection.removeAllRanges();
  selection.addRange(range);
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };

  const frame = fixture.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  const doc = other.document;
  const childBody = doc.body || doc.documentElement.appendChild(doc.createElement('body'));
  const childText = childBody.appendChild(doc.createTextNode('wxyz'));
  const childRange = new other.Range();
  childRange.setStart(childText, 1);
  childRange.setEnd(childText, 3);
  const childSelection = other.getSelection();
  childSelection.addRange(childRange);

  const sibling = fixture.appendChild(document.createElement('iframe'));
  sibling.contentDocument.body.appendChild(sibling.contentDocument.createElement('span'));
  check(range.startContainer === text && range.startOffset === 1 &&
    range.endContainer === text && range.endOffset === 3, 'parent range survives new realms');
  check(childRange.startContainer === childText && childRange.startOffset === 1 &&
    childRange.endContainer === childText && childRange.endOffset === 3, 'child range survives sibling realm');
  check(selection.getRangeAt(0) === range && childSelection.getRangeAt(0) === childRange,
    'selections retain associated range identity');

  text.insertData(0, '!');
  childText.deleteData(0, 1);
  check(range.startOffset === 2 && range.endOffset === 4 && range.toString() === 'bc',
    'parent range still tracks mutations');
  check(childRange.startOffset === 0 && childRange.endOffset === 2 && childRange.toString() === 'xy',
    'child range still tracks mutations');
  const clone = childRange.cloneRange();
  check(clone instanceof other.Range && clone.toString() === 'xy', 'child range remains usable in its realm');
  fixture.remove();
  return failures.join('\n');
})()
