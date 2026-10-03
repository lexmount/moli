globalThis.__installPointerActivityProbe = (first, second, kind, chorded = false, realm = globalThis) => {
  const rows = [], events = [], errors = [], states = new Map();
  const set = realm.Element.prototype.setPointerCapture;
  const release = realm.Element.prototype.releasePointerCapture;
  const has = realm.Element.prototype.hasPointerCapture;
  const detached = first.ownerDocument.createElement('div');
  const hover = kind !== 'touch';
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const outcome = work => {try {return {value: work()};} catch(error) {return {error};}};
  const describe = result => result.error ? {name: result.error.name, code: result.error.code} : String(result.value);
  const notFound = result => result.error instanceof realm.DOMException && result.error.name === 'NotFoundError' && result.error.code === 8;
  const invalidState = result => result.error instanceof realm.DOMException && result.error.name === 'InvalidStateError' && result.error.code === 11;
  const protect = (label, work) => {try {work();} catch(error) {errors.push({label, error: String(error.stack || error)});}};
  const noButtons = (label, id, expectedCapture) => {
    const result = outcome(() => set.call(second, id));
    record(label + '/set-without-buttons', {returns: !result.error && result.value === undefined,
      unchanged: has.call(first, id) === expectedCapture, noNewCapture: !has.call(second, id)}, describe(result));
    const disconnected = outcome(() => set.call(detached, id));
    record(label + '/disconnected', {invalidState: invalidState(disconnected)}, describe(disconnected));
    const unknown = outcome(() => set.call(first, 2147483647));
    record(label + '/unknown', {notFound: notFound(unknown)}, describe(unknown));
  };
  for (const [label, element] of [['first', first], ['second', second]]) {
    for (const type of ['pointerover', 'pointerenter', 'pointerdown', 'pointermove', 'pointerup', 'pointercancel', 'gotpointercapture', 'lostpointercapture']) {
      element.addEventListener(type, event => protect(type + '@' + label, () => {
        if (event.pointerType !== kind) return;
        events.push({type, target: label, id: event.pointerId, buttons: event.buttons, button: event.button, phase: globalThis.__inputPhase});
        const id = event.pointerId;
        let state = states.get(id);
        if (!state) {state = {id, down: 0, end: 0, got: 0, lost: 0, seenHover: false}; states.set(id, state);}
        record(type + '/' + id + '/metadata', {pointerType: event.pointerType === kind, target: label === 'first'});
        if ((type === 'pointerover' || type === 'pointerenter' || type === 'pointermove') && hover && event.buttons === 0) {
          noButtons(type + '/' + id, id, false);
          const result = outcome(() => release.call(first, id));
          record(type + '/' + id + '/release-hover', {returns: !result.error && result.value === undefined}, describe(result));
          state.seenHover = true;
        }
        if (type === 'pointerdown') {
          state.down++;
          const result = outcome(() => set.call(first, id));
          record('down/' + id, {returns: !result.error && result.value === undefined,
            pending: has.call(first, id), other: !has.call(second, id), buttons: event.buttons !== 0}, describe(result));
        }
        if (type === 'pointermove' && event.buttons !== 0) {
          record('move/' + id, {pending: has.call(first, id), other: !has.call(second, id), notEnded: state.end === 0});
        }
        if (type === 'gotpointercapture') {
          state.got++;
          record('got/' + id, {pending: has.call(first, id), buttons: event.buttons !== 0});
        }
        if (type === 'pointerup' || type === 'pointercancel') {
          state.end++;
          record(type + '/' + id, {buttons: event.buttons === 0, pending: has.call(first, id)});
          noButtons(type + '/' + id, id, true);
        }
        if (type === 'lostpointercapture') {
          state.lost++;
          record('lost/' + id, {released: !has.call(first, id), other: !has.call(second, id), afterEnd: state.end === 1});
          for (const [name, method] of [['set', set], ['release', release]]) {
            const result = outcome(() => method.call(first, id));
            record('lost/' + id + '/' + name, {lifetime: !result.error && result.value === undefined, noCapture: !has.call(first, id)}, describe(result));
          }
        }
      }));
    }
  }
  const finish = () => {
    for (const state of states.values()) {
      protect('finish/' + state.id, () => {
        const id = state.id;
        for (const [name, method] of [['set', set], ['release', release]]) {
          const result = outcome(() => method.call(first, id));
          record('finish/' + id + '/' + name, {lifetime: hover ? !result.error && result.value === undefined : notFound(result), noCapture: !has.call(first, id)}, describe(result));
        }
        record('finish/' + id + '/events', {oneDown: state.down === 1, oneEnd: state.end === 1, oneGot: state.got === 1, oneLost: state.lost === 1, hover: !hover || state.seenHover}, state);
      });
    }
    record('contacts', {count: states.size === (globalThis.__expectedContacts || 1)}, states.size);
    if (chorded) {
      const press = events.filter(e => e.phase === 'second-press' && ['pointerdown', 'pointermove', 'pointerup'].includes(e.type));
      const release = events.filter(e => e.phase === 'first-release' && ['pointerdown', 'pointermove', 'pointerup'].includes(e.type));
      record('chorded/press', {singleMove: press.length === 1 && press[0].type === 'pointermove', buttons: press[0]?.buttons === 3, button: press[0]?.button === 2}, press);
      record('chorded/release', {singleMove: release.length === 1 && release[0].type === 'pointermove', buttons: release[0]?.buttons === 2, button: release[0]?.button === 0}, release);
    }
    const checks = rows.flatMap(row => Object.values(row.checks));
    return {rows, events, errors, passed: checks.filter(v => v === true).length, total: checks.length,
      complete: errors.length === 0 && checks.every(v => v === true)};
  };
  return {rows, events, errors, finish};
};
