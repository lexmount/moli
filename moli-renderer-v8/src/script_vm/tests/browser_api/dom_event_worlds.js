function installDomEventWorldProbe(label) {
  const target = document.getElementById('target');
  const peer = document.getElementById('peer');
  const cases = [
    ['Event', {}],
    ['CustomEvent', {detail: 7}],
    ['UIEvent', {view: window, detail: 3}],
    ['KeyboardEvent', {view: window, key: 'k', code: 'KeyK'}],
    ['MouseEvent', {view: window, clientX: 12}],
    ['FocusEvent', {view: window, relatedTarget: peer}],
    ['InputEvent', {view: window, data: 'text', inputType: 'insertText'}],
    ['CompositionEvent', {view: window, data: 'text'}]
  ];
  const state = globalThis.__domEventWorld = {rows: [], originals: {}, saved: {}, dispatches: [], errors: []};
  target.worldLabel = label;
  document.worldLabel = label;
  window.worldLabel = label;
  window.addEventListener('error', event => {
    state.errors.push(String(event.message));
    event.preventDefault();
  });
  for (const [name] of cases) {
    const type = 'world-' + name;
    for (const current of [target, document, window]) {
      current.addEventListener(type, function(event) {
        const path = event.composedPath();
        const previous = state.saved[name];
        if (!previous) {
          state.saved[name] = event;
          event.worldLabel = label;
        }
        const fields = name === 'CustomEvent' ? event.detail === 7 :
          name === 'KeyboardEvent' ? event.key === 'k' && event.code === 'KeyK' :
          name === 'MouseEvent' ? event.clientX === 12 :
          name === 'InputEvent' ? event.data === 'text' && event.inputType === 'insertText' :
          name === 'CompositionEvent' ? event.data === 'text' : true;
        const checks = {
          interface: event instanceof globalThis[name] && event instanceof Event,
          receiver: this === current && this.worldLabel === label,
          target: event.target === target && event.srcElement === target,
          currentTarget: event.currentTarget === current,
          path: path[0] === target && path.includes(document) && path.at(-1) === window,
          localPath: path instanceof Array && path.every(value => value === window || value instanceof Node),
          windowEvent: window.event === event,
          reused: !previous || previous === event,
          identity: !state.originals[name] || event === state.originals[name],
          expandos: event.worldLabel === label && event.createdIn === (state.originals[name] ? label : undefined),
          view: !(event instanceof UIEvent) || event.view === window,
          relatedTarget: name !== 'FocusEvent' || event.relatedTarget === peer,
          fields,
          synthetic: event.isTrusted === false,
          phase: event.eventPhase === (current === target ? Event.AT_TARGET : Event.BUBBLING_PHASE)
        };
        state.rows.push({name, current: current === target ? 'node' : current === document ? 'document' : 'window', checks});
        if (label === 'isolated') event.preventDefault();
      });
    }
  }
  globalThis.resetDomEventWorldProbe = () => {
    state.rows = []; state.originals = {}; state.saved = {}; state.dispatches = []; state.errors = [];
  };
  globalThis.dispatchDomEventWorldProbe = () => {
    for (const [name, init] of cases) {
      const event = new globalThis[name]('world-' + name, {...init, bubbles: true, cancelable: true});
      state.originals[name] = event;
      event.createdIn = label;
      state.dispatches.push({name, checks: {
        canceled: target.dispatchEvent(event) === false && event.defaultPrevented,
        currentTargetCleared: event.currentTarget === null,
        phaseCleared: event.eventPhase === Event.NONE,
        pathCleared: event.composedPath().length === 0,
        originalExpando: event.createdIn === label && event.worldLabel === label
      }});
    }
  };
  globalThis.readDomEventWorldProbe = () => ({
    rows: state.rows, dispatches: state.dispatches, errors: state.errors,
    checks: {
      count: state.rows.length === cases.length * 3,
      cleared: Object.values(state.saved).every(event => event.currentTarget === null && event.eventPhase === Event.NONE && event.composedPath().length === 0),
      windowEventCleared: window.event === undefined
    }
  });
}
