(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const other = fixture.appendChild(document.createElement('iframe')).contentWindow;
  const otherRoot = other.document.documentElement || other.document.appendChild(other.document.createElement('html'));
  const otherBody = other.document.body || otherRoot.appendChild(other.document.createElement('body'));
  const failures = [];
  const check = (actual, expected, label) => {
    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      failures.push(label + ': ' + JSON.stringify(actual) + ' expected ' + JSON.stringify(expected));
    }
  };
  const selected = selection => [selection.rangeCount, selection.type, selection.direction, String(selection)];
  const offsets = control => [control.selectionStart, control.selectionEnd, control.selectionDirection];
  for (const owner of [window, other]) {
    const doc = owner.document, selection = owner.getSelection();
    const holder = (owner === window ? fixture : otherBody).appendChild(doc.createElement('div'));
    const button = holder.appendChild(doc.createElement('button'));
    const outside = holder.appendChild(doc.createTextNode('outside'));
    for (const kind of ['text', 'search', 'tel', 'url', 'password', 'email', 'number', 'textarea']) {
      for (const mode of ['light', 'open', 'closed']) {
        const box = holder.appendChild(doc.createElement('div'));
        const parent = mode === 'light' ? box : box.appendChild(doc.createElement('div')).attachShadow({mode});
        const control = parent.appendChild(doc.createElement(kind === 'textarea' ? 'textarea' : 'input'));
        if (kind !== 'textarea') control.type = kind;
        if (!['email', 'number'].includes(kind)) {
          check(offsets(control), [0, 0, 'forward'], kind + '/' + mode + '/initial cache direction');
        }
        control.value = kind === 'number' ? '12345' : 'A😀BC';
        const label = [owner === window ? 'parent' : 'child', kind, mode].join('/');
        // Call through the other realm's function to exercise owner-document routing.
        const caller = owner === window ? other : window;
        const proto = kind === 'textarea' ? caller.HTMLTextAreaElement.prototype : caller.HTMLInputElement.prototype;
        proto.select.call(control);
        check(selected(selection), [1, 'Range', 'forward', kind === 'password' ? '••••' : control.value], label + '/select');
        check(selection.isCollapsed, true, label + '/public projection stays collapsed');
        const projected = selection.getRangeAt(0);
        check([projected.startContainer === box, projected.startOffset, projected.collapsed], [true, 0, true], label + '/range projection');
        const composed = selection.getComposedRanges({shadowRoots: mode === 'light' ? [] : [parent]})[0];
        check([composed.startContainer === parent, composed.startOffset, composed.endContainer === parent, composed.endOffset], [true, 0, true, 1], label + '/composed range');
        if (!['email', 'number'].includes(kind)) {
          const setRange = (...args) => proto.setSelectionRange.call(control, ...args);
          setRange(1, 3, 'backward');
          check(selected(selection), [1, 'Range', 'backward', kind === 'password' ? '•' : '😀'], label + '/backward');
          button.focus();
          setRange(0, 1, 'forward');
          check(selected(selection), [1, 'Range', 'backward', kind === 'password' ? '•' : '😀'], label + '/background cache is independent');
          control.focus();
          check(selected(selection), [1, 'Range', 'forward', kind === 'password' ? '•' : 'A'], label + '/restore background cache');
          setRange(1, 3, 'backward');
          selection.removeAllRanges();
          check(offsets(control), [0, 0, 'forward'], label + '/focused empty selection');
          control.blur();
          check(offsets(control), [1, 3, 'backward'], label + '/blur exposes preserved cache');
          control.focus();
          check(selected(selection), [1, 'Range', 'backward', kind === 'password' ? '•' : '😀'], label + '/refocus after clear');
          selection.setBaseAndExtent(outside, 0, outside, 4);
          check(offsets(control), [0, 0, 'forward'], label + '/selection outside editor');
          control.blur();
          check(offsets(control), [1, 3, 'backward'], label + '/outside selection preserves cache');
          control.focus();
          check(selected(selection), [1, 'Range', 'backward', kind === 'password' ? '•' : '😀'], label + '/refocus after outside selection');
          control.value = 'X😀Y';
          check(offsets(control), [4, 4, 'forward'], label + '/value setter uses UTF-16 end');
          check(selected(selection), [1, 'Caret', 'none', ''], label + '/value setter collapses selection');
          setRange(1, 3);
          button.focus();
          control.value = 'A😀BC';
          control.value = 'X😀Y';
          check(selected(selection), [1, 'Caret', 'none', ''], label + '/background value roundtrip invalidates visible range');
        }
        box.remove();
        check(String(selection), '', label + '/removed editor has no selected text');
      }
    }
    holder.remove();
  }

  const selection = window.getSelection();
  const holder = fixture.appendChild(document.createElement('div'));
  const control = holder.appendChild(document.createElement('input'));
  const button = holder.appendChild(document.createElement('button'));
  const peer = holder.appendChild(document.createElement('input'));
  peer.value = 'XYZ';
  for (const action of ['none', 'range', 'value', 'clear', 'redirect', 'remove']) {
    holder.prepend(control);
    control.value = 'abcdef';
    peer.select();
    control.addEventListener('focus', () => {
      check(selected(selection), [1, 'Range', 'forward', 'XYZ'], action + '/select focus event preserves previous selection');
      if (action === 'range') control.setSelectionRange(2, 3, 'backward');
      if (action === 'value') control.value = 'X😀Y';
      if (action === 'clear') selection.removeAllRanges();
      if (action === 'redirect') peer.focus();
      if (action === 'remove') control.remove();
    }, {once: true});
    control.select();
    if (action === 'remove') check(document.activeElement === control, false, 'select respects removal during focus');
    else if (action === 'redirect') check(selected(selection), [1, 'Range', 'forward', 'XYZ'], 'select respects focus redirection');
    else if (action === 'value') check(selected(selection), [1, 'Caret', 'none', ''], 'select respects value changes during focus');
    else if (action === 'range') check(selected(selection), [1, 'Range', 'backward', 'c'], 'select respects cache changes during focus');
    else check(selected(selection), [1, 'Range', 'forward', 'abcdef'], action + '/select restores cached selection after focus event');
  }
  holder.prepend(control);
  control.value = 'abcdef';
  button.focus();
  control.addEventListener('focus', () => selection.removeAllRanges(), {once: true});
  control.focus();
  check(selection.rangeCount, 0, 'normal focus does not overwrite listener selection');

  for (const [value, start, end, expected] of [
    ['A😀BC', 1, 2, '😀'], ['A😀BC', 2, 3, ''], ['A😀BC', 2, 4, 'B'],
    ['e\u0301X', 0, 1, 'e\u0301'], ['e\u0301X', 1, 2, ''],
    ['👩‍👩‍👧‍👦X', 0, 1, '👩‍👩‍👧‍👦'], ['👩‍👩‍👧‍👦X', 1, 11, ''],
    ['👍🏽X', 0, 2, '👍🏽'], ['אבג', 1, 3, 'בג']
  ]) {
    control.type = 'text';
    control.value = value;
    control.focus();
    control.setSelectionRange(start, end);
    check(String(selection), expected, 'visible grapheme ' + [value, start, end].join('/'));
  }
  control.type = 'password';
  control.value = 'A👩‍👩‍👧‍👦e\u0301X';
  control.select();
  check(String(selection), '••••', 'password masks each selected grapheme');

  control.type = 'text';
  control.value = 'A😀BC';
  control.setSelectionRange(1, 3, 'backward');
  const segmenter = Object.getOwnPropertyDescriptor(Intl, 'Segmenter');
  let authorCalls = 0;
  const forbidden = () => { authorCalls++; throw new Error('author hook'); };
  try {
    Object.defineProperty(Intl, 'Segmenter', {configurable: true, get: forbidden});
    for (const name of ['value', 'type', 'selectionStart', 'selectionEnd', 'selectionDirection', 'ownerDocument']) {
      Object.defineProperty(control, name, {configurable: true, get: forbidden});
    }
    check(selected(selection), [1, 'Range', 'backward', '😀'], 'Selection uses native state');
    check(authorCalls, 0, 'Selection ignores author getters and Intl');
  } finally {
    if (segmenter) Object.defineProperty(Intl, 'Segmenter', segmenter);
    else delete Intl.Segmenter;
    for (const name of ['value', 'type', 'selectionStart', 'selectionEnd', 'selectionDirection', 'ownerDocument']) delete control[name];
  }
  fixture.remove();
  return failures.join('\n');
})()
