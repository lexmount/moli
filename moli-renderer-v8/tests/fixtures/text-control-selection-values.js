(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const button = fixture.appendChild(document.createElement('button'));
  const selection = getSelection(), failures = [];
  const check = (actual, expected, label) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      failures.push(label + ': ' + JSON.stringify(actual) + ' expected ' + JSON.stringify(expected));
    }
  };
  const offsets = control => [control.selectionStart, control.selectionEnd, control.selectionDirection];
  const selected = () => [selection.rangeCount, selection.type, selection.direction, String(selection)];
  const original = 'A😀B\nC';
  // Expected offsets include the intermediate removal step of replacements.
  // DOM nodes do not stand in for the text control's internal selection nodes.
  const cases = [
    ['append', c => c.append('XY'), original + 'XY', 4, '😀B'],
    ['prepend', c => c.prepend('XY'), 'XY' + original, 4, 'YA😀'],
    ['fragment', c => { const f = document.createDocumentFragment(); f.append('XY', 'Z'); c.append(f); }, original + 'XYZ', 4, '😀B'],
    ['same textContent', c => c.textContent = c.textContent, original, 0, ''],
    ['same defaultValue', c => c.defaultValue = c.defaultValue, original, 0, ''],
    ['empty', c => c.textContent = '', '', 0, ''],
    ['replace', c => c.replaceChild(document.createTextNode('uvwxyz'), c.firstChild), 'uvwxyzB\nC', 3, 'vw'],
    ['same replacement', c => c.replaceChild(document.createTextNode(c.firstChild.data), c.firstChild), original, 3, '😀'],
    ['replace self', c => c.replaceChild(c.firstChild, c.firstChild), original, 3, '😀'],
    ['insert before self', c => c.insertBefore(c.firstChild, c.firstChild), original, 3, '😀'],
    ['remove first', c => c.firstChild.remove(), 'B\nC', 3, '\nC'],
    ['remove last', c => c.lastChild.remove(), 'A😀', 3, '😀'],
    ['move first', c => c.append(c.firstChild), 'B\nCA😀', 3, '\nC'],
    ['data', c => c.firstChild.data = 'Q', 'QB\nC', 4, 'B\nC'],
    ['appendData', c => c.firstChild.appendData('XY'), 'A😀XYB\nC', 4, '😀X'],
    ['split', c => c.firstChild.splitText(1), original, 4, '😀B'],
    ['normalize', c => c.normalize(), original, 4, '😀B'],
    ['replaceChildren', c => c.replaceChildren('A😀', 'B\nC'), original, 0, ''],
    ['same data', c => c.firstChild.data = c.firstChild.data, original, 4, '😀B', false],
    ['empty text node', c => c.append(''), original, 4, '😀B', false],
    ['comment', c => c.append(document.createComment('ignored')), original, 4, '😀B', false],
    ['descendant text', c => { const e = document.createElement('b'); e.textContent = 'ignored'; c.append(e); }, original, 4, '😀B', false],
  ];
  for (const focused of [true, false]) for (const dirty of [false, true]) for (const cleared of [false, true]) {
    for (const [name, mutate, value, end, text, changes = true] of cases) {
      const control = fixture.appendChild(document.createElement('textarea'));
      control.append('A😀', 'B\nC');
      if (dirty) control.value = control.value;
      control.focus();
      control.setSelectionRange(1, 4, 'backward');
      if (cleared) selection.removeAllRanges();
      if (!focused) button.focus();
      mutate(control);
      const label = [name, focused, dirty, cleared].join('/');
      const changed = changes && !dirty;
      const cache = changed
        ? [cleared && focused ? 0 : Math.min(1, end), cleared && focused ? 0 : end, 'forward']
        : [1, 4, 'backward'];
      const restored = [1, cache[0] === cache[1] ? 'Caret' : 'Range', cache[0] === cache[1] ? 'none' : cache[2], cache[0] === cache[1] ? '' : changed ? text : '😀B'];
      const visible = changed && focused ? restored
        : cleared ? [0, 'None', 'none', '']
        : changed ? [1, 'Caret', 'none', '']
        : [1, 'Range', 'backward', '😀B'];
      check(control.value, dirty ? original : value, label + '/value');
      check(offsets(control), focused && cleared && !changed ? [0, 0, 'forward'] : cache, label + '/offsets');
      check(selected(), visible, label + '/visible selection');
      control.blur();
      check(offsets(control), cache, label + '/blurred cache');
      check(selected(), visible, label + '/blurred selection');
      control.focus();
      check(offsets(control), cache, label + '/restored offsets');
      check(selected(), restored, label + '/restored selection');
      control.remove();
    }
  }
  for (const tag of ['input', 'textarea']) for (const focused of [true, false]) {
    for (const mutation of ['reset', 'same reset', 'default', 'value', 'same value', 'roundtrip']) {
      const form = fixture.appendChild(document.createElement('form'));
      const control = form.appendChild(document.createElement(tag));
      control.defaultValue = 'A😀BC';
      if (mutation === 'reset') control.value = 'dirty';
      control.focus();
      control.setSelectionRange(1, 3, 'backward');
      if (!focused) button.focus();
      if (mutation.includes('reset')) form.reset();
      else if (mutation === 'default') control.defaultValue = 'xyz';
      else if (mutation === 'same value') control.value = 'A😀BC';
      else if (mutation === 'value') control.value = 'X😀Y';
      else { control.value = 'xyz'; control.value = 'A😀BC'; }
      const label = [tag, focused, mutation].join('/');
      const unchanged = mutation.startsWith('same');
      const cache = unchanged || mutation === 'default' && tag === 'input' ? [1, 3, 'backward']
        : mutation === 'default' ? [0, 0, 'forward']
        : mutation === 'value' ? [4, 4, 'forward'] : [5, 5, 'forward'];
      check(offsets(control), focused && mutation === 'default' && tag === 'input' ? [0, 0, 'forward'] : cache, label + '/offsets');
      check(selected(), unchanged ? [1, 'Range', 'backward', '😀'] : [1, 'Caret', 'none', ''], label + '/selection');
      control.blur();
      check(offsets(control), cache, label + '/blur');
      control.focus();
      check(selected(), unchanged ? [1, 'Range', 'backward', '😀']
        : mutation === 'default' && tag === 'input' ? [1, 'Range', 'backward', 'yz']
        : [1, 'Caret', 'none', ''], label + '/refocus');
      form.remove();
    }
  }
  // Observer bookkeeping must not control value/selection synchronization.
  for (const observe of [false, true]) {
    const control = fixture.appendChild(document.createElement('textarea'));
    control.append('A\r');
    const observer = new MutationObserver(() => {});
    if (observe) observer.observe(control, {subtree: true, childList: true, characterData: true});
    control.focus();
    control.setSelectionRange(1, 2, 'backward');
    control.append('\n');
    check(offsets(control), [1, 2, 'backward'], 'CRLF normalization preserves unchanged API selection/' + observe);
    check(selected(), [1, 'Range', 'backward', '\n'], 'normalized value/' + observe);
    control.firstChild.data = 'A😀';
    check(offsets(control), [1, 2, 'forward'], 'character data changes selection without observers/' + observe);
    check(selected(), [1, 'Range', 'forward', '😀'], 'character data updates selected text/' + observe);
    observer.disconnect();
    control.remove();
  }
  for (const focused of [true, false]) for (const action of ['number', 'same', 'nan', 'up', 'down', 'zero']) {
    const control = fixture.appendChild(document.createElement('input'));
    control.type = 'number';
    control.value = '123';
    control.select();
    if (!focused) button.focus();
    if (action === 'number') control.valueAsNumber = 4567;
    if (action === 'same') control.valueAsNumber = 123;
    if (action === 'nan') control.valueAsNumber = NaN;
    if (action === 'up') control.stepUp();
    if (action === 'down') control.stepDown();
    if (action === 'zero') control.stepUp(0);
    const expected = ['same', 'zero'].includes(action) ? [1, 'Range', 'forward', '123'] : [1, 'Caret', 'none', ''];
    check(selected(), expected, 'numeric setter/' + [focused, action].join('/'));
    control.blur();
    control.focus();
    check(selected(), expected, 'numeric setter refocus/' + [focused, action].join('/'));
    control.remove();
  }
  fixture.remove();
  return failures.join('\n');
})()
