(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const selection = getSelection();
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  function at(range, node, start, end = start) {
    return range.startContainer === node && range.startOffset === start &&
      range.endContainer === node && range.endOffset === end;
  }
  for (const mode of ['open', 'closed']) {
    const host = fixture.appendChild(document.createElement('div'));
    const outer = host.attachShadow({mode});
    const nested = outer.appendChild(document.createElement('div'));
    const inner = nested.attachShadow({mode});
    inner.appendChild(document.createTextNode('before'));
    const input = inner.appendChild(document.createElement('input'));
    selection.removeAllRanges();
    input.focus();
    check(selection.rangeCount === 1, mode + ': selection exists');
    if (selection.rangeCount) {
      const index = Array.prototype.indexOf.call(fixture.childNodes, host);
      check(at(selection.getRangeAt(0), fixture, index), mode + ': range before outer host');
      check(at(selection.getComposedRanges()[0], fixture, index, index + 1), mode + ': composed outer host');
      check(at(selection.getComposedRanges({shadowRoots: [outer]})[0], outer, 0, 1), mode + ': composed inner host');
      check(at(selection.getComposedRanges({shadowRoots: [inner]})[0], inner, 1, 2), mode + ': composed control');
    }
    host.remove();
  }

  const input = fixture.appendChild(document.createElement('input'));
  input.focus();
  const topRange = selection.getRangeAt(0);
  const frame = fixture.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  const doc = other.document;
  const childBody = doc.body || doc.documentElement.appendChild(doc.createElement('body'));
  const childInput = childBody.appendChild(doc.createElement('input'));
  const childSelection = other.getSelection();
  const OtherRange = other.Range;
  other.Range = function () { throw new Error('replaced child Range'); };
  HTMLElement.prototype.focus.call(childInput);
  check(childSelection.rangeCount === 1, 'borrowed focus updates owner selection');
  if (childSelection.rangeCount) {
    const range = childSelection.getRangeAt(0);
    const index = Array.prototype.indexOf.call(childBody.childNodes, childInput);
    check(range instanceof OtherRange && !(range instanceof Range) && at(range, childBody, index),
      'range belongs to owner realm');
    other.HTMLElement.prototype.focus.call(input);
    check(childSelection.getRangeAt(0) === range, 'top focus retains child range');
  }
  check(selection.getRangeAt(0) === topRange, 'child focus preserves top selection');
  fixture.remove();
  return failures.join('\n');
})()
