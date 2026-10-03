globalThis.__installDragLifecycleProbe = function(mode, capture, modifiers) {
  document.body.innerHTML = '<style>body{margin:0;user-select:none}#source{position:absolute;left:40px;top:40px;width:140px;height:140px}#a{position:absolute;left:300px;top:40px;width:140px;height:140px}#b{position:absolute;left:500px;top:40px;width:140px;height:140px}</style><div id="source" draggable="true">source</div><div id="a">A</div><div id="b">B</div>';
  const source=document.getElementById('source'), a=document.getElementById('a'), b=document.getElementById('b');
  const trace=[], starts=[], ends=[], retained=[], authored=[], errors=[];
  let pointerId=null, transfer=null;
  const keyBits=event=>[event.altKey,event.ctrlKey,event.metaKey,event.shiftKey];
  const expectedBits=value=>[!!(value&1),!!(value&2),!!(value&4),!!(value&8)];
  for(const type of ['pointerdown','pointerrawupdate','pointermove','pointerup','pointercancel','gotpointercapture','lostpointercapture','pointerout','pointerleave','pointerover','pointerenter','mousedown','mousemove','mouseup','click','keydown','keyup','dragstart','drag','dragenter','dragover','dragleave','drop','dragend']) {
    document.addEventListener(type,event=>{
      const target=event.composedPath()[0], related=event.relatedTarget;
      trace.push({phase:globalThis.__dragPhase,type,target:target.id||target.nodeName,related:related?(related.id||related.nodeName):null,button:event.button,buttons:event.buttons,clientX:event.clientX,clientY:event.clientY,keys:keyBits(event),bubbles:event.bubbles,cancelable:event.cancelable,composed:event.composed,trusted:event.isTrusted,defaultPrevented:event.defaultPrevented,capture:pointerId===null?false:source.hasPointerCapture(pointerId),pointerId:event.pointerId,pointerType:event.pointerType,pressure:event.pressure,dropEffect:event.dataTransfer?.dropEffect,data:event.dataTransfer?.getData('text/plain')});
      if(type==='pointerdown' && target===source) {
        pointerId=event.pointerId;
        if(capture) source.setPointerCapture(pointerId);
      }
      if(type==='dragstart') {
        starts.push(trace[trace.length-1]); transfer=event.dataTransfer;
        transfer.setData('text/plain','native-drag'); transfer.effectAllowed='all';
        if(mode==='cancel') event.preventDefault();
      }
      if(type==='dragenter' || type==='dragover') {
        if(mode==='drop' && (target===a || target===b)) {event.dataTransfer.dropEffect='copy';event.preventDefault();}
      }
      if(type==='drop') {retained.push(event.dataTransfer.getData('text/plain'));event.preventDefault();}
      if(type==='dragend') {
        ends.push(trace[trace.length-1]);
        event.preventDefault(); errors.push({name:'dragend preventDefault',actual:event.defaultPrevented,expected:false});
      }
      if(type==='pointercancel' || type==='dragleave') {
        event.preventDefault(); errors.push({name:type+' preventDefault',actual:event.defaultPrevented,expected:false});
      }
    },true);
  }
  for(const type of ['dragstart','dragenter','dragover','dragleave','drop','dragend']) {
    const event=new DragEvent(type,{button:2,buttons:3,cancelable:true,bubbles:false,composed:false,altKey:true,dataTransfer:new DataTransfer()});
    event.preventDefault();authored.push([type,event.button,event.buttons,event.cancelable,event.bubbles,event.composed,event.altKey,event.defaultPrevented]);
  }
  return {finish(){
    const checks=[];
    const check=(name,actual,expected)=>checks.push({name,actual,expected,pass:JSON.stringify(actual)===JSON.stringify(expected)});
    check('one native dragstart',starts.length,1);
    if(starts.length) {
      check('dragstart pressed buttons',starts.map(e=>[e.button,e.buttons]),[[0,1]]);
      check('dragstart modifiers',starts.map(e=>e.keys),[expectedBits(modifiers)]);
      check('dragstart flags',starts.map(e=>[e.bubbles,e.cancelable,e.composed,e.trusted]),[[true,true,true,true]]);
      check('dragstart origin',starts.map(e=>[e.clientX,e.clientY]),[[80,80]]);
    }
    const startIndex=trace.findIndex(e=>e.type==='dragstart');
    const after=trace.slice(startIndex+1), canceled=mode==='cancel';
    check('pointercancel count',after.filter(e=>e.type==='pointercancel').length,canceled?0:1);
    check('dragend count',ends.length,canceled?0:1);
    if(!canceled) {
      check('input suppressed during drag',after.filter(e=>['pointermove','pointerrawupdate','pointerup','mousedown','mousemove','mouseup','click','keydown','keyup'].includes(e.type) && e.phase!=='after').map(e=>e.type),[]);
      const cancel=after.findIndex(e=>e.type==='pointercancel'), lost=after.findIndex(e=>e.type==='lostpointercapture'), out=after.findIndex(e=>e.type==='pointerout'&&e.target==='source'), leave=after.findIndex(e=>e.type==='pointerleave'&&e.target==='source');
      const continuation=after.findIndex(e=>['drag','dragenter','dragover','dragleave','drop','dragend'].includes(e.type));
      check('cancel then boundary events before DND continuation',cancel>=0 && out>cancel && leave>out && continuation>leave,true);
      check('capture release before boundary',capture?lost>cancel && out>lost && continuation>out:lost===-1,true);
      check('pointercancel preserves last pointer geometry',after.filter(e=>e.type==='pointercancel').map(e=>[e.clientX,e.clientY,e.pressure]),[[100,80,0.5]]);
      check('pointercancel uncancelable',after.filter(e=>e.type==='pointercancel').map(e=>[e.cancelable,e.composed,e.trusted]),[[false,true,true]]);
      check('capture released',source.hasPointerCapture(pointerId),false);
      check('native dragend flags',ends.map(e=>[e.button,e.buttons,e.bubbles,e.cancelable,e.composed,e.trusted]),[[0,0,true,false,true,true]]);
      check('dragend source',ends.map(e=>e.target),['source']);
      check('drop count',after.filter(e=>e.type==='drop').length,mode==='drop'?1:0);
      check('drop data',retained,mode==='drop'?['native-drag']:[]);
      check('dragend effect',ends.map(e=>e.dropEffect),[mode==='drop'?'copy':'none']);
      if(mode==='drop' || mode==='reject') {
        const enterA=after.find(e=>e.type==='dragenter'&&e.target==='a'),enterB=after.find(e=>e.type==='dragenter'&&e.target==='b'),leaveA=after.find(e=>e.type==='dragleave'&&e.target==='a');
        check('entered both drop zones',!!enterA&&!!enterB,true);
        check('drag target relatedTarget',enterB&&leaveA?[enterB.related,leaveA.related]:null,['a','b']);
        check('dragover held buttons',after.filter(e=>e.type==='dragover'&&['a','b'].includes(e.target)).map(e=>e.buttons).every(v=>v===1),true);
        check('drop released buttons',after.filter(e=>e.type==='drop').map(e=>[e.button,e.buttons]),mode==='drop'?[[0,0]]:[]);
      }
    } else {
      check('cancel preserves pointer stream',after.some(e=>e.type==='pointerup'),true);
      check('canceled drag emits no DND continuation',after.filter(e=>['drag','dragenter','dragover','dragleave','drop','dragend'].includes(e.type)).map(e=>e.type),[]);
    }
    for(const row of errors) check(row.name,row.actual,row.expected);
    check('authored DragEvent options preserved',authored,['dragstart','dragenter','dragover','dragleave','drop','dragend'].map(type=>[type,2,3,true,false,false,true,true]));
    return {mode,capture,modifiers,checks,passed:checks.filter(e=>e.pass).length,total:checks.length,complete:checks.every(e=>e.pass),trace,authored,errors};
  }};
};
