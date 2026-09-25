(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const reset = fixture.appendChild(document.createElement('button'));
  const seed = fixture.appendChild(document.createTextNode('original selection'));
  const selection = getSelection();
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  function at(range, node, start, end = start) {
    return range.startContainer === node && range.startOffset === start &&
      range.endContainer === node && range.endOffset === end;
  }
  function seedSelection() {
    reset.focus();
    selection.setBaseAndExtent(seed, 1, seed, 4);
    return selection.getRangeAt(0);
  }
  for (const type of ['text', 'search', 'tel', 'url', 'email', 'password', 'number',
                       'textarea', 'TEXT', 'invalid']) {
    const box = fixture.appendChild(document.createElement('div'));
    box.appendChild(document.createTextNode('before'));
    const control = box.appendChild(document.createElement(type === 'textarea' ? 'textarea' : 'input'));
    if (type !== 'textarea') control.type = type;
    control.value = type === 'number' ? '42' : 'abcdef';
    const supportsSelection = !['number', 'email'].includes(type);
    if (supportsSelection) control.setSelectionRange(1, 3, 'backward');
    const oldRange = seedSelection();
    let focusObserved = false;
    control.addEventListener('focus', () => {
      focusObserved = selection.rangeCount === 1 && at(selection.getRangeAt(0), box, 1);
    }, {once: true});
    control.focus();
    check(document.activeElement === control && focusObserved, type + ': updated before focus');
    check(selection.rangeCount === 1, type + ': range exists');
    if (!selection.rangeCount) { box.remove(); continue; }
    const range = selection.getRangeAt(0);
    check(at(range, box, 1) && range.collapsed, type + ': projected boundary');
    check(selection.anchorNode === box && selection.anchorOffset === 1 &&
      selection.focusNode === box && selection.focusOffset === 1, type + ': projected endpoints');
    check(range !== oldRange && at(oldRange, seed, 1, 4), type + ': retained old range unchanged');
    check(at(selection.getComposedRanges()[0], box, 1, 2), type + ': composed boundary encloses control');
    if (supportsSelection) {
      check(control.selectionStart === 1 && control.selectionEnd === 3 &&
        control.selectionDirection === 'backward', type + ': control selection restored');
    }
    control.focus();
    check(selection.getRangeAt(0) === range, type + ': repeated focus retains range');
    reset.focus();
    check(selection.getRangeAt(0) === range, type + ': button retains range');
    control.focus();
    check(selection.getRangeAt(0) === range, type + ': refocus retains range');
    selection.removeAllRanges();
    control.focus();
    check(selection.rangeCount === 0, type + ': focused control does not recreate cleared selection');
    reset.focus();
    control.focus();
    check(selection.rangeCount === 1 && at(selection.getRangeAt(0), box, 1),
      type + ': refocus recreates cleared selection');
    box.remove();
  }
  for (const type of ['button', 'submit', 'reset', 'checkbox', 'radio', 'range', 'color', 'file', 'image']) {
    const control = fixture.appendChild(document.createElement('input'));
    control.type = type;
    const range = seedSelection();
    control.focus();
    check(selection.getRangeAt(0) === range && at(range, seed, 1, 4), type + ': keeps DOM selection');
    control.remove();
  }
  for (const property of ['disabled', 'hidden', 'inert']) {
    const control = fixture.appendChild(document.createElement('input'));
    control[property] = true;
    const range = seedSelection();
    control.focus();
    check(document.activeElement === reset && selection.getRangeAt(0) === range,
      property + ': failed focus keeps selection');
    control.remove();
  }

  const first = fixture.appendChild(document.createElement('input'));
  const second = fixture.appendChild(document.createElement('input'));
  const third = fixture.appendChild(document.createElement('input'));
  first.focus();
  const retained = selection.getRangeAt(0);
  for (const type of ['button', 'range', 'color', 'file']) {
    const control = fixture.appendChild(document.createElement('input'));
    control.type = type;
    control.focus();
    first.focus();
    check(selection.getRangeAt(0) === retained, type + ': refocusing text field keeps its range');
    control.remove();
  }
  first.focus();
  first.addEventListener('blur', () => third.focus(), {once: true});
  second.focus();
  check(document.activeElement === third && at(selection.getRangeAt(0), fixture,
    Array.prototype.indexOf.call(fixture.childNodes, third)), 'blur redirection keeps redirected selection');
  second.addEventListener('focus', () => selection.collapse(seed, 2), {once: true});
  second.focus();
  check(at(selection.getRangeAt(0), seed, 2), 'focus listener selection is not overwritten');
  first.addEventListener('focus', () => third.focus(), {once: true});
  first.focus();
  check(document.activeElement === third && at(selection.getRangeAt(0), fixture,
    Array.prototype.indexOf.call(fixture.childNodes, third)), 'focus redirection keeps redirected selection');

  seedSelection();
  const OriginalRange = Range;
  const originalGetSelection = window.getSelection;
  const originalCreateRange = document.createRange;
  const originalCollapse = Selection.prototype.collapse;
  let calls = 0;
  const forbidden = () => { calls++; throw new Error('author hook entered'); };
  try {
    Object.defineProperty(window, 'Range', {configurable: true, get: forbidden});
    window.getSelection = forbidden;
    document.createRange = forbidden;
    Selection.prototype.collapse = forbidden;
    for (const name of ['parentNode', 'ownerDocument']) {
      Object.defineProperty(first, name, {configurable: true, get: forbidden});
    }
    first.focus();
    const range = selection.getRangeAt(0);
    check(calls === 0 && range instanceof OriginalRange && at(range, fixture,
      Array.prototype.indexOf.call(fixture.childNodes, first)), 'focus uses native selection and intrinsic Range');
  } finally {
    Object.defineProperty(window, 'Range', {configurable: true, writable: true, value: OriginalRange});
    window.getSelection = originalGetSelection;
    document.createRange = originalCreateRange;
    Selection.prototype.collapse = originalCollapse;
    delete first.parentNode;
    delete first.ownerDocument;
  }
  fixture.remove();
  return failures.join('\n');
})()
