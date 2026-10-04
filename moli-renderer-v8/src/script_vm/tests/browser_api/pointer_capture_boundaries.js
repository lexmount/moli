globalThis.__installCaptureBoundaryProbe = function (context, pointer, mode, poison, modifiers) {
  const windows = [window], names = new Map(), rows = [], checks = [];
  document.body.style.cssText = 'margin:0;user-select:none';
  function frame(win) {
    const frame = win.document.createElement('iframe');
    frame.style.cssText = 'position:fixed;left:0;top:0;width:760px;height:300px;border:0';
    win.document.body.appendChild(frame);
    const child = frame.contentWindow;
    child.document.body.style.cssText = 'margin:0;user-select:none';
    windows.push(child);
    return child;
  }
  let owner = window;
  if (context === 'child' || context === 'nested') owner = frame(owner);
  if (context === 'nested') owner = frame(owner);
  const doc = owner.document;
  function element(parent, name, left, width) {
    const node = doc.createElement('div'); node.id = name;
    node.style.cssText = `position:fixed;left:${left}px;top:40px;width:${width}px;height:140px;background:#acf`;
    parent.appendChild(node); names.set(node, name); return node;
  }
  let holder = doc.body;
  if (context.startsWith('shadow-') || context === 'slotted') {
    const host = element(holder, 'host', 0, 760);
    const shadow = host.attachShadow({mode: context === 'shadow-closed' ? 'closed' : 'open'});
    if (context === 'slotted') {shadow.appendChild(doc.createElement('slot'));holder=host;}
    else holder = shadow;
  }
  const outer = element(holder, 'outer', 20, 180);
  const source = element(outer, 'source', 40, 140);
  const peer = element(holder, 'peer', 500, 140);
  // Only these three nodes log events, avoiding shadow retargeting and duplicate
  // bubbling observations. Ancestor boundaries remain visible on outer.
  const intrinsics = windows.map(win => ({win, Pointer:win.PointerEvent, Mouse:win.MouseEvent}));
  const kinds = intrinsics.find(entry => entry.win === owner);
  let phase='setup', id=null, reads=0, parentReads=0;
  source.addEventListener('pointerdown', event => {
    id=event.pointerId; (mode==='implicit' ? peer : source).setPointerCapture(id);
  });
  source.addEventListener('pointermove', () => {
    if (mode==='explicit' && phase==='cross') source.releasePointerCapture(id);
  });
  source.addEventListener('pointerrawupdate', () => {
    if (mode==='raw' && phase==='cross') peer.setPointerCapture(id);
  });
  const types=['pointerdown','pointermove','pointerup','gotpointercapture','lostpointercapture',
    'pointerout','pointerleave','pointerover','pointerenter','mouseout','mouseleave','mouseover','mouseenter'];
  for (const [node,name] of [[source,'source'],[outer,'outer'],[peer,'peer']]) for (const type of types) node.addEventListener(type,event => {
    event.stopPropagation(); if (phase==='hover' || phase==='setup') return;
    const isPointer=type.startsWith('pointer') || type.endsWith('pointercapture'), Kind=isPointer?kinds.Pointer:kinds.Mouse;
    rows.push({phase,type,target:name,related:names.get(event.relatedTarget)||null,
      exactPrototype:Object.getPrototypeOf(event)===Kind.prototype, typed:event instanceof Kind,
      onlyOwner:intrinsics.every(entry=>entry.win===owner||!(event instanceof (isPointer?entry.Pointer:entry.Mouse))),
      view:event.view===owner,targetIdentity:event.target===node,currentIdentity:event.currentTarget===node,
      pathIdentity:event.composedPath()[0]===node,trusted:event.isTrusted,
      bubbles:event.bubbles,cancelable:event.cancelable,composed:event.composed,
      button:event.button,buttons:event.buttons,x:event.clientX,y:event.clientY,
      pointerType:event.pointerType,pressure:event.pressure,
      modifiers:[event.altKey,event.ctrlKey,event.metaKey,event.shiftKey]});
  });
  if (poison==='getter') for (const win of windows) {
    for (const name of ['PointerEvent','MouseEvent']) Object.defineProperty(win,name,{configurable:true,get(){reads++;throw new Error('author constructor');}});
    Object.defineProperty(win.Node.prototype,'parentNode',{configurable:true,get(){parentReads++;throw new Error('author parent');}});
  }
  function check(name,actual,expected) {checks.push({name,actual,expected,pass:JSON.stringify(actual)===JSON.stringify(expected)});}
  const transition=(from,to)=>[['pointerout',from],['pointerleave',from],
    ...(from==='source'?[['pointerleave','outer']]:[]),['pointerover',to],
    ...(to==='source'?[['pointerenter','outer']]:[]),['pointerenter',to]];
  return {prepare(value){phase=value;},finish(){
    const pointerRows=rows.filter(row=>row.type.startsWith('pointer')||row.type.endsWith('pointercapture'));
    const event=(type,target)=>[[type,target]];
    const expected=[...event('pointerdown','source')];
    if(mode==='implicit') expected.push(...transition('source','peer'),...event('gotpointercapture','peer'),
      ...event('pointerup','peer'),...event('lostpointercapture','peer'),...transition('peer','source'),...event('pointermove','source'));
    else {
      expected.push(...event('gotpointercapture','source'),...event('pointermove','source'));
      if(mode==='raw') expected.push(...event('lostpointercapture','source'),...transition('source','peer'),...event('gotpointercapture','peer'),...event('pointermove','peer'));
      else expected.push(...event('pointermove','source'));
      if(mode==='explicit') expected.push(...event('lostpointercapture','source'),...transition('source','peer'),...event('pointermove','peer'),...event('pointerup','peer'));
      else expected.push(...event('pointerup',mode==='raw'?'peer':'source'),...event('lostpointercapture',mode==='raw'?'peer':'source'),...(mode==='raw'?[]:transition('source','peer')));
      expected.push(...event('pointermove','peer'));
    }
    check('pointer and capture event order',pointerRows.map(row=>[row.type,row.target]),expected);
    const lostAt=rows.findLastIndex(row=>row.phase==='up'&&row.type==='lostpointercapture');
    const releaseRows=rows.slice(lostAt+1).filter(row=>row.phase==='up'&&['pointerout','pointerleave','pointerover','pointerenter','mouseout','mouseleave','mouseover','mouseenter'].includes(row.type));
    if(mode==='implicit'||mode==='drag') {
      check('release reconciles without another move',releaseRows.length,10);
      check('release coordinates',releaseRows.every(row=>row.x===(mode==='implicit'?80:520)&&row.y===80),true);
    }
    check('no delayed release boundary',rows.filter(row=>row.phase==='after'&&row.type!=='pointermove').length,0);
    for(const [i,row] of rows.entries()) {
      for(const key of ['exactPrototype','typed','onlyOwner','view','targetIdentity','currentIdentity','pathIdentity','trusted']) check(i+'/'+key,row[key],true);
      const boundary=row.type.endsWith('enter')||row.type.endsWith('leave'), capture=row.type.endsWith('pointercapture');
      check(i+'/bubbles',row.bubbles,!boundary);check(i+'/cancelable',row.cancelable,!boundary&&!capture);check(i+'/composed',row.composed,!boundary);
      check(i+'/modifiers',row.modifiers,[Boolean(modifiers&1),Boolean(modifiers&2),Boolean(modifiers&4),Boolean(modifiers&8)]);
      if(row.type.startsWith('pointer')||capture) check(i+'/pointerType',row.pointerType,pointer);
    }
    check('capture released',source.hasPointerCapture(id)||peer.hasPointerCapture(id),false);
    check('author constructors not read',reads,0);check('author ancestry not read',parentReads,0);
    return {context,pointer,mode,poison,modifiers,rows,checks,reads,parentReads,passed:checks.filter(c=>c.pass).length,total:checks.length,complete:checks.every(c=>c.pass)};
  }};
};
