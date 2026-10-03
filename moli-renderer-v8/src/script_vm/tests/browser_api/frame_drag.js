globalThis.__installFrameDragProbe = (origin, mode) => {
  const frame = document.getElementById('child');
  const childWindow = frame.contentWindow;
  const childDocument = childWindow.document;
  childDocument.documentElement.id = 'childRoot';
  const first = document.getElementById('outside');
  const inner = childDocument.getElementById('inside');
  const own = origin === 'child' ? inner : first;
  const documents = new Map([[document, 'parent'], [childDocument, 'child']]);
  const events = [], rows = [], errors = [];
  const state = {id:null, captureReleased:false, removed:false, navigated:false, getterReads:0};
  const record = (label, checks, observed=null) => rows.push({label, checks, observed});
  const guard = work => {try {work();} catch (e) {errors.push(String(e.stack || e));}};
  const capture = ['capture', 'release'].includes(mode);
  function observe(doc, label) {
    doc.addEventListener('contextmenu',event=>event.preventDefault());
    for (const type of ['pointerdown','pointermove','pointerup','gotpointercapture',
      'lostpointercapture','mousedown','mousemove','mouseup']) {
      doc.addEventListener(type, event => guard(() => {
        events.push({phase:globalThis.__framePhase,type,document:label,
          target:event.target === doc ? label + 'Document' : event.target.id,
          buttons:event.buttons,button:event.button,id:event.pointerId,
          clientX:event.clientX,clientY:event.clientY,trusted:event.isTrusted});
        if (type === 'pointerdown') {
          state.id = event.pointerId;
          if (capture) {own.setPointerCapture(state.id);
            record('capture-acquired',{pending:own.hasPointerCapture(state.id)});}
          if (mode === 'cancelled') event.preventDefault();
        }
        if (type === 'pointermove' && mode === 'release' && !state.captureReleased
          && own.hasPointerCapture(state.id)) {
          own.releasePointerCapture(state.id);state.captureReleased=true;
          record('capture-released',{pendingCleared:!own.hasPointerCapture(state.id)});
        }
      }),true);
    }
  }
  observe(document,'parent');observe(childDocument,'child');
  // Native frame ownership must not consult author getters on frame/element wrappers.
  Object.defineProperty(own,'ownerDocument',{configurable:true,get() {
    state.getterReads++;throw new Error('author ownerDocument getter');
  }});
  Object.defineProperty(frame,'contentDocument',{configurable:true,get() {
    state.getterReads++;throw new Error('author contentDocument getter');
  }});
  function expected(phase) {
    if (phase === 'down' || phase === 'chord-down') return [origin,origin==='child'?'inside':'outside'];
    if (phase === 'post-up-hover') return ['parent','outside'];
    if (capture && (mode === 'capture' || phase === 'within-move'))
      return [origin,origin==='child'?'inside':'outside'];
    if (phase === 'within-move' || phase === 'covered-within-move') {
      if (origin==='parent' && ['plain','chorded'].includes(mode)) return ['parent','child'];
      return ['child','second'];
    }
    if (state.removed) return ['parent','outside'];
    if (state.navigated) return ['replacement','replacementRoot'];
    return origin === 'child' ? ['child','childRoot'] : ['parent','outside'];
  }
  return {events,rows,errors,state,async mutate() {
    if (mode === 'remove') {frame.remove();state.removed=true;}
    if (mode === 'navigate') {
      await new Promise(resolve => {
        frame.addEventListener('load',resolve,{once:true});
        frame.srcdoc='<html id="replacementRoot"><body style="margin:0"><div id="replacement">replacement</div></body></html>';
      });
      state.navigated=true;observe(frame.contentWindow.document,'replacement');
      record('navigation-owner-changed',{documentReplaced:frame.contentWindow.document !== childDocument});
    }
    if (mode === 'cover') {
      const cover=document.createElement('div');cover.id='cover';
      cover.style.cssText='position:absolute;left:220px;top:40px;width:160px;height:160px;z-index:1000';
      document.body.appendChild(cover);
    }
    if (mode === 'reposition') frame.style.left='420px';
  },finish() {
    const phases = ['down','within-move','outside-move','up','post-up-hover'];
    if (mode === 'chorded') phases.splice(3,0,'chord-release','held-after-release');
    if (mode === 'cover') phases.splice(2,0,'covered-within-move');
    for (const phase of phases) {
      const type=phase==='down'?'pointerdown':phase==='up'?'pointerup':'pointermove';
      const found=events.filter(e=>e.phase===phase && e.type===type);
      const [doc,target]=expected(phase);
      record(phase,{oneEvent:found.length===1,document:found[0]?.document===doc,
        target:found[0]?.target===target,trusted:found[0]?.trusted===true},found);
      if (found.length===1 && ['child','replacement'].includes(doc)) {
        const left=mode==='reposition' && !['down','within-move'].includes(phase)?420:220;
        const x=phase==='down'?240:['within-move','covered-within-move'].includes(phase)?310:phase==='held-after-release'?100:80;
        record(phase+'/coordinates',{x:found[0].clientX===x-left,y:found[0].clientY===40},found[0]);
      }
    }
    if (mode==='chorded') record('chord-retains-frame',{
      heldButtons:events.some(e=>e.phase==='chord-release'&&e.type==='pointermove'&&e.buttons===2),
      noPrematureUp:!events.some(e=>e.phase==='chord-release'&&e.type==='pointerup')});
    if (capture) record('capture-cleanup',{
      acquired:events.some(e=>e.type==='gotpointercapture'),
      lost:events.some(e=>e.type==='lostpointercapture'),pendingClear:!own.hasPointerCapture(state.id)});
    record('author-getters',{notRead:state.getterReads===0});
    const checks=rows.flatMap(r=>Object.values(r.checks));
    return {events,rows,errors,state,complete:errors.length===0&&checks.every(v=>v===true),
      passed:checks.filter(v=>v===true).length,total:checks.length};
  }};
};
