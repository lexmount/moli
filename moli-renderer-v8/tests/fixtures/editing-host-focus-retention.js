(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  fixture.innerHTML = '<button id="reset">reset</button><p id="outside">outside</p>' +
    '<div id="editor" contenteditable><span id="text">abcdef</span><input></div>' +
    '<div id="other" contenteditable>other</div>';
  const reset = fixture.querySelector('#reset');
  const outside = fixture.querySelector('#outside').firstChild;
  const editor = fixture.querySelector('#editor');
  const text = fixture.querySelector('#text').firstChild;
  const control = editor.querySelector('input');
  const other = fixture.querySelector('#other');
  const selection = getSelection();
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  function caret(node, offset) {
    return selection.rangeCount === 1 && selection.anchorNode === node &&
      selection.anchorOffset === offset && selection.focusNode === node && selection.focusOffset === offset;
  }
  for (const [anchor, a, focus, f] of [[text, 4, text, 2], [text, 2, outside, 1], [editor, 0, editor, 0]]) {
    selection.setBaseAndExtent(anchor, a, focus, f);
    const range = selection.getRangeAt(0);
    const direction = selection.direction;
    reset.focus();
    editor.focus();
    check(selection.getRangeAt(0) === range && selection.anchorNode === anchor && selection.anchorOffset === a &&
      selection.focusNode === focus && selection.focusOffset === f && selection.direction === direction,
      'refocus preserves selection anchored in host');
  }
  selection.setBaseAndExtent(outside, 1, text, 2);
  reset.focus();
  editor.focus();
  check(caret(text, 0), 'outside anchor resets selection');
  control.focus();
  editor.focus();
  check(caret(text, 0), 'nested text control does not retain control projection');
  selection.removeAllRanges();
  editor.focus();
  check(selection.rangeCount === 0, 'already focused editor does not recreate selection');
  reset.focus();
  editor.focus();
  check(caret(text, 0), 'blur and refocus recreates cleared selection');

  // A true or invalid state inside an editable parent does not create a host.
  for (const value of ['true', 'plaintext-only', '', ' true ', 'false ', 'invalid']) {
    text.parentNode.setAttribute('contenteditable', value);
    reset.focus();
    selection.collapse(outside, 1);
    const range = selection.getRangeAt(0);
    text.parentNode.focus();
    check(document.activeElement === reset && selection.getRangeAt(0) === range,
      value + ': nested editable element is not independently focusable');
    text.parentNode.tabIndex = 0;
    text.parentNode.focus();
    check(document.activeElement === text.parentNode && selection.getRangeAt(0) === range,
      value + ': focusable descendant does not initialize host selection');
    text.parentNode.removeAttribute('tabindex');
  }
  text.parentNode.contentEditable = 'false';
  selection.collapse(text, 2);
  reset.focus();
  editor.focus();
  check(caret(editor, 0), 'selection in noneditable island does not belong to host');
  text.parentNode.removeAttribute('contenteditable');

  const island = editor.appendChild(document.createElement('div'));
  island.contentEditable = 'false';
  const nested = island.appendChild(document.createElement('div'));
  nested.contentEditable = 'TrUe';
  nested.textContent = 'nested editor';
  reset.focus();
  nested.focus();
  check(document.activeElement === nested && caret(nested.firstChild, 0), 'true state below false creates host');
  const invalid = fixture.appendChild(document.createElement('div'));
  invalid.setAttribute('contenteditable', ' true ');
  reset.focus();
  invalid.focus();
  check(document.activeElement === reset, 'enumerated contenteditable does not trim whitespace');

  editor.focus();
  editor.addEventListener('blur', () => nested.focus(), {once: true});
  other.focus();
  check(document.activeElement === nested && caret(nested.firstChild, 0), 'blur redirection retains redirected selection');
  editor.addEventListener('focus', () => selection.collapse(text, 3), {once: true});
  editor.focus();
  check(caret(text, 3), 'focus listener selection is not overwritten');
  other.addEventListener('focus', () => nested.focus(), {once: true});
  other.focus();
  check(document.activeElement === nested && caret(nested.firstChild, 0), 'focus redirection retains redirected selection');

  const OriginalRange = Range;
  const getSelectionOriginal = window.getSelection;
  const computedStyleOriginal = window.getComputedStyle;
  const createRangeOriginal = document.createRange;
  const collapseOriginal = Selection.prototype.collapse;
  let calls = 0;
  const forbidden = () => { calls++; throw new Error('author hook entered'); };
  try {
    Object.defineProperty(window, 'Range', {configurable: true, get: forbidden});
    window.getSelection = window.getComputedStyle = document.createRange = Selection.prototype.collapse = forbidden;
    for (const name of ['parentNode', 'ownerDocument', 'contentEditable', 'firstChild', 'style']) {
      Object.defineProperty(other, name, {configurable: true, get: forbidden});
    }
    other.focus();
    check(selection.getRangeAt(0) instanceof OriginalRange &&
      caret(other.childNodes[0], 0) && calls === 0, 'initial selection uses native DOM and intrinsic Range');
  } finally {
    Object.defineProperty(window, 'Range', {configurable: true, writable: true, value: OriginalRange});
    window.getSelection = getSelectionOriginal;
    window.getComputedStyle = computedStyleOriginal;
    document.createRange = createRangeOriginal;
    Selection.prototype.collapse = collapseOriginal;
    for (const name of ['parentNode', 'ownerDocument', 'contentEditable', 'firstChild', 'style']) delete other[name];
  }
  // The visible caret follows its associated live Range while its initial
  // composed boundary remains at the start of the editing host.
  for (const action of ['remove-text', 'delete-prefix', 'split-prefix']) {
    const box = fixture.appendChild(document.createElement('div'));
    box.contentEditable = 'true';
    box.innerHTML = '<span> abc</span><b>def</b>';
    reset.focus();
    selection.removeAllRanges();
    box.focus();
    const range = selection.getRangeAt(0);
    const text = box.firstChild.firstChild;
    if (action === 'remove-text') box.firstChild.remove();
    if (action === 'delete-prefix') text.deleteData(0, 1);
    if (action === 'split-prefix') text.splitText(0);
    check(caret(range.startContainer, range.startOffset) && selection.getRangeAt(0) === range,
      action + ': live caret endpoints');
    const composed = selection.getComposedRanges()[0];
    check(composed.startContainer === box && composed.startOffset === 0 &&
      composed.endContainer === box && composed.endOffset === 0, action + ': composed host boundary');
    reset.focus();
    box.focus();
    check(selection.getRangeAt(0) === range, action + ': mutated range retained on refocus');
    box.remove();
  }
  fixture.remove();
  return failures.join('\n');
})()
