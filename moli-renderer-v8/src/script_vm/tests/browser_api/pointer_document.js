globalThis.__installPointerDocumentProbe = (first, second, realm, mode) => {
  const firstDocument = first.ownerDocument, secondDocument = second.ownerDocument;
  const set = realm.Element.prototype.setPointerCapture;
  const release = realm.Element.prototype.releasePointerCapture;
  const has = realm.Element.prototype.hasPointerCapture;
  const rows = [], errors = [], events = [], seen = new Set();
  const state = {id: null, moves: 0, ended: false, lost: 0, documentReads: 0};
  const record = (label, checks, observed = null) => rows.push({label, checks, observed});
  const protect = (label, work) => {try {work();} catch(error) {errors.push({label, error: String(error.stack || error)});}};
  const outcome = work => {try {return {value:work()};} catch(error) {return {error};}};
  const captureState = () => [has.call(first, state.id), has.call(second, state.id)];
  const wrongDocument = (label, own, other) => {
    const before = captureState();
    const otherBefore = has.call(other, state.id);
    const result = outcome(() => set.call(other, state.id));
    const after = captureState();
    record(label, {noForeignAcquisition: otherBefore || !has.call(other, state.id),
      pendingUnchanged: before[0] === after[0] && before[1] === after[1]},
      {before, after, error:result.error?.name || null});
  };
  for (const element of [first, second]) Object.defineProperty(element, 'ownerDocument', {
    configurable:true, get() {state.documentReads++; throw new Error('author ownerDocument getter');}
  });
  for (const [label, document, own, other] of [
    ['first', firstDocument, first, second], ['second', secondDocument, second, first]
  ]) {
    for (const type of ['pointerdown', 'pointerover', 'pointerenter', 'pointerout',
      'pointerleave', 'pointerrawupdate', 'pointermove', 'gotpointercapture',
      'pointerup', 'lostpointercapture']) {
      document.addEventListener(type, event => protect(label + '/' + type, () => {
        events.push({document:label,type,phase:globalThis.__inputPhase,
          idMatches:state.id === null || event.pointerId === state.id,buttons:event.buttons});
        if (state.id === null && type !== 'pointerdown') return;
        if (type === 'pointerdown') {
          state.id = event.pointerId;
          event.preventDefault();
          const result = outcome(() => set.call(own, state.id));
          record('down/' + label + '/valid', {returns:!result.error && result.value === undefined,
            sameDocumentCapture:has.call(own, state.id),otherDocumentFree:!has.call(other, state.id)});
          wrongDocument('down/' + label + '/wrong', own, other);
          const foreignDocument = document === firstDocument ? secondDocument : firstDocument;
          foreignDocument.addEventListener('pointer-document-synthetic', () =>
            wrongDocument('down/' + label + '/synthetic-keeps-native-document', own, other), {once:true});
          foreignDocument.dispatchEvent(new realm.PointerEvent('pointer-document-synthetic', {
            pointerId:state.id,buttons:1
          }));
          const otherRelease = outcome(() => release.call(other, state.id));
          record('down/' + label + '/foreign-release', {pendingPreserved:has.call(own, state.id) &&
            !has.call(other, state.id)}, {error:otherRelease.error?.name || null});
          if (mode === 'move') {
            release.call(own, state.id);
            record('down/' + label + '/release', {pendingCleared:!has.call(own, state.id)});
          }
          return;
        }
        const key = globalThis.__inputPhase + '/' + label + '/' + type;
        if (!seen.has(key)) {
          seen.add(key);
          wrongDocument(key + '/wrong', own, other);
        }
        if (type === 'pointermove') {
          state.moves++;
          const result = outcome(() => set.call(own, state.id));
          record(key + '/valid', {returns:!result.error && result.value === undefined,
            documentMatchesInput:label === (mode === 'capture' ? 'first' : 'second'),
            captureAllowed:event.buttons !== 0 ? has.call(own, state.id) : !has.call(own, state.id),
            otherDocumentFree:!has.call(other, state.id)});
        }
        if (type === 'pointerup') {
          state.ended = true;
          const before = captureState();
          set.call(own, state.id);
          record(key + '/inactive-buttons', {noMutation:JSON.stringify(before) === JSON.stringify(captureState())});
        }
        if (type === 'lostpointercapture') state.lost++;
      }), true);
    }
  }
  return {rows,errors,events,state,finish() {
    record('finish', {pointerObserved:state.id !== null,moveObserved:state.moves > 0,
      upObserved:state.ended,noOwnerDocumentGetter:state.documentReads === 0,
      captureReleased:state.id !== null && !has.call(first,state.id) && !has.call(second,state.id)});
    const checks = rows.flatMap(row => Object.values(row.checks));
    return {rows,errors,events,state,complete:errors.length === 0 && checks.every(value=>value === true),
      passed:checks.filter(Boolean).length,total:checks.length};
  }};
};
