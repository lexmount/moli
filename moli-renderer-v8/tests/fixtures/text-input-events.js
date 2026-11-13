globalThis.installTextInputProbe = (kind, options = {}) => {
  globalThis.disposeTextInputProbe?.();
  document.body.innerHTML = '';
  let owner = document;
  let root = document.body;
  let frame = null;
  if (kind === 'frame') {
    frame = document.createElement('iframe');
    document.body.appendChild(frame);
    owner = frame.contentDocument;
    root = owner.body;
  } else if (kind === 'shadow') {
    const host = document.createElement('div');
    host.id = 'host';
    document.body.appendChild(host);
    root = host.attachShadow({mode: 'open'});
  }
  root.innerHTML = kind === 'textarea' ? '<textarea id="field"></textarea>' :
    kind === 'editor' ? '<div id="field" contenteditable="true"></div>' :
    kind === 'plain' ? '<div id="field" tabindex="0"></div>' : '<input id="field">';
  const field = root.querySelector('#field');
  const other = owner.createElement('input');
  other.id = 'other';
  owner.body.appendChild(other);
  const view = owner.defaultView;
  const NativeTextEvent = view.TextEvent;
  const NativeUIEvent = view.UIEvent;
  const NativeInputEvent = view.InputEvent;
  if (kind === 'readonly') field.readOnly = true;
  if (options.maxLength !== undefined) field.maxLength = options.maxLength;
  const events = [], textEvents = [], editEvents = [], seen = new WeakSet();
  const types = ['keydown', 'keypress', 'beforeinput', 'textInput', 'input', 'keyup'];
  const listener = event => {
    if (seen.has(event)) return;
    seen.add(event);
    const original = event.composedPath()[0];
    events.push(`${event.type}:${event.target.id}`);
    if (event.type === 'beforeinput' || event.type === 'input')
      editEvents.push({type: event.type, inputType: event.inputType, data: event.data});
    if (event.type === 'textInput') textEvents.push({
      type: event.type, target: event.target.id, originalTarget: original.id,
      data: event.data, native: event instanceof NativeTextEvent,
      ui: event instanceof NativeUIEvent, input: event instanceof NativeInputEvent,
      trusted: event.isTrusted, bubbles: event.bubbles, cancelable: event.cancelable,
      composed: event.composed, view: event.view === view, detail: event.detail,
      value: original.value ?? original.textContent,
    });
    if (event.type === options.cancel) event.preventDefault();
    if (event.type === options.phase) {
      if (options.effect === 'focus') other.focus();
      if (options.effect === 'remove') field.remove();
      if (options.effect === 'readonly') field.readOnly = true;
      if (options.effect === 'disable') field.contentEditable = 'false';
      if (options.effect === 'adopt') document.implementation.createHTMLDocument('').adoptNode(field);
      if (options.effect === 'remove-frame') frame.remove();
      if (options.effect === 'selection') {field.value = 'abc'; field.setSelectionRange(1, 2);}
    }
  };
  for (const type of types) {
    owner.addEventListener(type, listener, true);
    field.addEventListener(type, listener);
    other.addEventListener(type, listener);
  }
  globalThis.disposeTextInputProbe = () => {
    for (const type of types) {
      owner.removeEventListener(type, listener, true);
      field.removeEventListener(type, listener);
      other.removeEventListener(type, listener);
    }
  };
  globalThis.textInputProbe = {field, other, events, textEvents, editEvents};
  field.focus();
  if (options.poison) {
    view.TextEvent = function () {throw new Error('author TextEvent constructor');};
    NativeTextEvent.prototype.initTextEvent = () => {throw new Error('author initializer');};
    owner.createEvent = () => {throw new Error('author createEvent');};
  }
  return 'ready';
};

globalThis.readTextInputProbe = () => {
  const {field, other, events, textEvents, editEvents} = textInputProbe;
  return {value: field.value ?? field.textContent, other: other.value, events, textEvents, editEvents,
    selection: field.setSelectionRange ? [field.selectionStart, field.selectionEnd] : null};
};
