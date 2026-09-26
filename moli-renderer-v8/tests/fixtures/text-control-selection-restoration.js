(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const fixture = body.appendChild(document.createElement('div'));
  const frame = fixture.appendChild(document.createElement('iframe'));
  const other = frame.contentWindow;
  const otherRoot = other.document.documentElement || other.document.appendChild(other.document.createElement('html'));
  const otherBody = other.document.body || otherRoot.appendChild(other.document.createElement('body'));
  const failures = [];
  const check = (condition, label) => { if (!condition) failures.push(label); };
  const at = (range, node, start, end = start) => range.startContainer === node &&
    range.startOffset === start && range.endContainer === node && range.endOffset === end;
  const roots = [window, other].map(owner => {
    const holder = (owner === window ? fixture : otherBody).appendChild(owner.document.createElement('div'));
    const button = holder.appendChild(owner.document.createElement('button'));
    const text = holder.appendChild(owner.document.createTextNode('document text'));
    return {owner, holder, button, text, selection: owner.getSelection()};
  });
  const methods = ['setSelectionRange', 'selectionStart', 'selectionEnd', 'selectionDirection', 'select'];
  for (const state of roots) for (const caller of [window, other]) {
    const {owner, holder, button, text, selection} = state;
    const unrelated = roots.find(entry => entry !== state);
    for (const tag of ['text', 'textarea']) {
      for (const mode of ['light', 'open', 'closed']) {
        const box = holder.appendChild(owner.document.createElement('div'));
        const parent = mode === 'light' ? box : box.appendChild(owner.document.createElement('div')).attachShadow({mode});
        const control = parent.appendChild(owner.document.createElement(tag === 'textarea' ? 'textarea' : 'input'));
        if (tag !== 'textarea') control.type = tag;
        control.value = 'A😀BC';
        const proto = tag === 'textarea' ? caller.HTMLTextAreaElement.prototype : caller.HTMLInputElement.prototype;
        const invoke = (name, value) => {
          if (name === 'setSelectionRange') proto[name].call(control, 1, 3, 'backward');
          else if (name === 'select') proto[name].call(control);
          else Object.getOwnPropertyDescriptor(proto, name).set.call(control, value);
        };
        for (const method of methods) for (const clear of ['removeAllRanges', 'empty', 'removeRange', 'collapse', 'superseded']) {
          const label = [owner === window ? 'parent' : 'child', caller === window ? 'parent' : 'child', tag, mode, method, clear].join(':');
          button.focus();
          control.focus();
          control.setSelectionRange(1, 3, 'backward');
          const original = selection.getRangeAt(0);
          unrelated.selection.setBaseAndExtent(unrelated.text, 0, unrelated.text, 13);
          const untouched = unrelated.selection.getRangeAt(0);
          if (clear === 'superseded') selection.setBaseAndExtent(text, 0, text, 13);
          else if (clear === 'removeRange') caller.Selection.prototype.removeRange.call(selection, original);
          else if (clear === 'collapse') caller.Selection.prototype.collapse.call(selection, null);
          else caller.Selection.prototype[clear].call(selection);
          if (clear !== 'superseded') {
            check(selection.rangeCount === 0, label + ': selection cleared');
            check(control.selectionStart === 0 && control.selectionEnd === 0, label + ': focused offsets cleared');
          }
          invoke(method, method === 'selectionDirection' ? 'backward' : method === 'selectionStart' ? 0 : 3);
          check(selection.rangeCount === 1, label + ': projection restored');
          if (selection.rangeCount) {
            check(at(selection.getRangeAt(0), box, 0), label + ': projected position');
            check(at(selection.getComposedRanges({shadowRoots: mode === 'light' ? [] : [parent]})[0], parent, 0, 1), label + ': composed position');
          }
          check(at(original, box, 0), label + ': retained range unchanged');
          check(unrelated.selection.getRangeAt(0) === untouched && at(untouched, unrelated.text, 0, 13), label + ': unrelated document unchanged');
        }
        for (const method of methods.filter(name => name !== 'select')) {
          control.focus();
          control.setSelectionRange(1, 3, 'backward');
          button.focus();
          selection.removeAllRanges();
          check(control.selectionStart === 1 && control.selectionEnd === 3, tag + ': blurred cache retained');
          invoke(method, method === 'selectionDirection' ? 'forward' : 2);
          check(selection.rangeCount === 0 && owner.document.activeElement === button, tag + ':' + method + ': background setter does not restore or focus');
        }
        // Conversion can move focus. The post-conversion native state decides
        // whether this control may replace the document selection.
        control.focus();
        selection.removeAllRanges();
        invoke('selectionEnd', {valueOf() { button.focus(); return 4; }});
        check(selection.rangeCount === 0 && owner.document.activeElement === button, tag + ': conversion blur respected');
        control.focus();
        selection.removeAllRanges();
        const sentinel = new Error('conversion');
        let caught;
        try { invoke('selectionStart', {valueOf() { throw sentinel; }}); } catch (error) { caught = error; }
        check(caught === sentinel && selection.rangeCount === 0, tag + ': failed conversion keeps selection empty');
        box.remove();
      }
    }
  }
  const {holder, selection, button, text} = roots[0];
  const control = holder.appendChild(document.createElement('input'));
  control.value = 'abcdef';
  control.focus();
  control.setSelectionRange(1, 4, 'backward');
  selection.removeAllRanges();
  let calls = 0;
  const forbidden = () => { calls++; throw new Error('author hook'); };
  const originalCreateRange = document.createRange, originalGetSelection = window.getSelection;
  const originalCollapse = Selection.prototype.collapse;
  try {
    document.createRange = forbidden;
    window.getSelection = forbidden;
    Selection.prototype.collapse = forbidden;
    for (const key of ['ownerDocument', 'parentNode', 'selectionStart', 'selectionEnd']) {
      Object.defineProperty(control, key, {configurable: true, get: forbidden});
    }
    control.setSelectionRange(1, 4);
    check(calls === 0 && selection.rangeCount === 1, 'restoration ignores author relationship and selection getters');
  } finally {
    document.createRange = originalCreateRange;
    window.getSelection = originalGetSelection;
    Selection.prototype.collapse = originalCollapse;
    for (const key of ['ownerDocument', 'parentNode', 'selectionStart', 'selectionEnd']) delete control[key];
  }
  button.focus();
  selection.setBaseAndExtent(text, 0, text, 13);
  const untouched = selection.getRangeAt(0);
  for (const location of ['fragment', 'windowless']) {
    const doc = location === 'windowless' ? document.implementation.createHTMLDocument('') : document;
    const input = doc.createElement('input');
    input.value = 'abcdef';
    (location === 'windowless' ? doc.body : doc.createDocumentFragment()).appendChild(input);
    input.setSelectionRange(1, 4);
    input.selectionEnd = 3;
    check(selection.getRangeAt(0) === untouched && at(untouched, text, 0, 13), location + ': setter retains live document selection');
  }
  fixture.remove();
  return failures.join('\n');
})()
