(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  const frame = document.createElement('iframe'); body.append(frame);
  const failures = [];
  const check = (value, label) => { if (!value) failures.push(label); };
  const eventTypes = ['blur', 'focusout', 'focus', 'focusin', 'change'];
  for (const doc of [document, frame.contentDocument]) {
    for (const tag of ['button', 'input', 'textarea']) {
      for (const shadow of [false, true]) {
        for (const operation of ['remove', 'removeChild', 'removeAncestor', 'append', 'insertBefore',
          'selfAppend', 'replaceChild', 'replaceWith', 'replaceChildren', 'textContent', 'innerHTML', 'adoptNode']) {
          const label = `${doc === document ? 'main' : 'iframe'}/${tag}/${shadow}/${operation}`;
          const source = doc.createElement('div'), destination = doc.createElement('div');
          const target = doc.createElement(tag), sibling = doc.createElement('span');
          const parent = shadow ? source.attachShadow({mode: 'open'}) : source;
          parent.append(target); destination.append(sibling); doc.body.append(source, destination);
          target.focus();
          check(target.matches(':focus'), `${label}/initial focus`);
          if (tag !== 'button' && !shadow) doc.execCommand('insertText', false, 'edited');
          const events = [], observe = e => events.push(e.type);
          for (const type of eventTypes) {
            target.addEventListener(type, observe);
            doc.addEventListener(type, observe, true);
          }
          let error;
          try {
            switch (operation) {
              case 'remove': target.remove(); break;
              case 'removeChild': parent.removeChild(target); break;
              case 'removeAncestor': source.remove(); break;
              case 'append': destination.append(target); break;
              case 'insertBefore': destination.insertBefore(target, sibling); break;
              case 'selfAppend': parent.append(target); break;
              case 'replaceChild': destination.replaceChild(target, sibling); break;
              case 'replaceWith': target.replaceWith('replacement'); break;
              case 'replaceChildren': parent.replaceChildren(); break;
              case 'textContent': parent.textContent = ''; break;
              case 'innerHTML': parent.innerHTML = ''; break;
              case 'adoptNode': document.implementation.createHTMLDocument('').adoptNode(target); break;
            }
          } catch (e) { error = e.name; }
          check(!error, `${label}/exception:${error}`);
          check(events.length === 0, `${label}/events:${events.join(',')}`);
          check(doc.activeElement === doc.body, `${label}/viewport`);
          check(!target.matches(':focus') && !target.matches(':focus-within'), `${label}/target state`);
          check(!source.matches(':focus-within') && !destination.matches(':focus-within'), `${label}/ancestor state`);
          for (const type of eventTypes) {
            target.removeEventListener(type, observe);
            doc.removeEventListener(type, observe, true);
          }
          source.remove(); destination.remove();
        }
      }
    }
    // Author focus/blur still dispatch events, while removal discards an
    // uncommitted text edit instead of dispatching change or reviving it later.
    {
      const input = doc.createElement('input'); doc.body.append(input); input.focus();
      doc.execCommand('insertText', false, 'first');
      const events = [];
      for (const type of ['change', 'blur', 'focusout']) input.addEventListener(type, () => events.push(type));
      input.remove(); doc.body.append(input); input.focus(); input.blur();
      check(events.join(',') === 'blur,focusout', 'removed text edit is not committed on a later blur');
      events.length = 0; input.focus(); doc.execCommand('insertText', false, 'second'); input.blur();
      check(events.join(',') === 'change,blur,focusout', 'explicit blur commits a new edit and dispatches focus events');
      input.remove();
    }
    // Invalid insertion leaves focus intact. Atomic moves preserve it, and an
    // actual explicit focus change remains observable after either operation.
    {
      const source = doc.createElement('div'), destination = doc.createElement('div');
      const button = doc.createElement('button'); source.append(button); doc.body.append(source, destination);
      button.focus(); const events = [];
      for (const type of ['blur', 'focusout']) button.addEventListener(type, () => events.push(type));
      let error; try { button.append(source); } catch (e) { error = e.name; }
      check(error === 'HierarchyRequestError' && doc.activeElement === button && events.length === 0,
        'rejected insertion does not reset focus');
      destination.moveBefore(button, null);
      check(doc.activeElement === button && destination.matches(':focus-within') && events.length === 0,
        'atomic move preserves focus without events');
      button.blur(); check(events.join(',') === 'blur,focusout', 'explicit blur after atomic move');
      source.remove(); destination.remove();
    }
  }
  // Removing a retained focused area in an inactive child must not change
  // the currently focused parent element or revive the child's old focus.
  {
    const parentButton = document.createElement('button'); body.append(parentButton);
    const child = frame.contentDocument, input = child.createElement('input'); child.body.append(input);
    input.focus(); parentButton.focus();
    check(child.activeElement === input, 'inactive child retains its focused area');
    const events = []; input.addEventListener('blur', () => events.push('blur'));
    input.remove(); child.body.append(input);
    check(document.activeElement === parentButton && child.activeElement === child.body && !events.length,
      'inactive child removal clears only its focused area');
    input.remove(); parentButton.remove();
  }
  {
    const parentButton = body.appendChild(document.createElement('button'));
    const child = frame.contentDocument, host = child.createElement('div'); child.body.append(host);
    const shadow = host.attachShadow({mode:'open'}), input = child.createElement('input'); shadow.append(input);
    input.focus(); parentButton.focus();
    check(child.activeElement === host && shadow.activeElement === input,
      'inactive child document and shadow root retain the same focused area');
    check(!input.matches(':focus') && !host.matches(':focus') && document.activeElement === parentButton,
      'retaining a document focused area does not change the global focus target');
    input.remove(); shadow.append(input);
    check(child.activeElement === child.body && shadow.activeElement === null && document.activeElement === parentButton,
      'inactive shadow removal resets local focus without affecting the parent');
    host.remove(); parentButton.remove();
  }
  frame.remove();
  return failures.length ? JSON.stringify(failures) : '';
})()
