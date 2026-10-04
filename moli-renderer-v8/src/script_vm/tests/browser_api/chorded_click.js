globalThis.__installChordedClickProbe = function (context, pointer, pair, reverse, mode, poison, modifiers) {
  const windows = [window], names = new Map(), rows = [], checks = [];
  document.body.style.cssText = 'margin:0;user-select:none';
  function frame(win) {
    const node = win.document.createElement('iframe');
    node.style.cssText = 'position:fixed;left:0;top:0;width:760px;height:300px;border:0';
    win.document.body.appendChild(node);
    const child = node.contentWindow; child.document.body.style.cssText = 'margin:0;user-select:none';
    windows.push(child); return child;
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
    const shadow = host.attachShadow({mode:context === 'shadow-closed' ? 'closed' : 'open'});
    if (context === 'slotted') {shadow.appendChild(doc.createElement('slot'));holder=host;} else holder=shadow;
  }
  const common = element(holder, 'common', 20, 700);
  const source = element(common, 'source', 40, 140), peer = element(common, 'peer', 500, 140);
  const intrinsics = windows.map(win => ({win, Pointer:win.PointerEvent, Mouse:win.MouseEvent}));
  const kinds = intrinsics.find(entry => entry.win === owner), seen = new WeakSet();
  let phase='setup', id=null, reads=0, parentReads=0;
  source.addEventListener('pointerdown', event => {
    id=event.pointerId; if(mode === 'capture-peer') peer.setPointerCapture(id);
  });
  owner.addEventListener('contextmenu', event => event.preventDefault());
  const types=['pointerdown','pointermove','pointerup','mousedown','mouseup','gotpointercapture','lostpointercapture','click','auxclick','contextmenu'];
  for (const node of [common,source,peer]) for (const type of types) node.addEventListener(type,event => {
    if(seen.has(event)) return; seen.add(event);
    if(phase==='hover'||phase==='setup') return;
    const isPointer = !['mousedown','mouseup'].includes(type), Kind=isPointer?kinds.Pointer:kinds.Mouse;
    rows.push({phase,type,target:names.get(event.target)||event.target.nodeName,
      exactPrototype:Object.getPrototypeOf(event)===Kind.prototype,typed:event instanceof Kind,
      onlyOwner:intrinsics.every(entry=>entry.win===owner||!(event instanceof (isPointer?entry.Pointer:entry.Mouse))),
      view:event.view===owner,pathIdentity:event.composedPath()[0]===event.target,
      trusted:event.isTrusted,bubbles:event.bubbles,cancelable:event.cancelable,composed:event.composed,
      button:event.button,buttons:event.buttons,x:event.clientX,y:event.clientY,
      pointerType:event.pointerType,pressure:event.pressure,detail:event.detail,
      modifiers:[event.altKey,event.ctrlKey,event.metaKey,event.shiftKey]});
  }, true);
  if(poison==='getter') for(const win of windows) {
    for(const name of ['PointerEvent','MouseEvent']) Object.defineProperty(win,name,{configurable:true,get(){reads++;throw new Error('author constructor');}});
    Object.defineProperty(win.Node.prototype,'parentNode',{configurable:true,get(){parentReads++;throw new Error('author parent');}});
  }
  const masks=[1,4,2,8,16], [first,second]=pair, release=reverse?[second,first]:[first,second];
  const endX=mode==='same'?80:520, full=masks[first]|masks[second];
  const steps=[{phase:'hover',event:'mousemove',x:80,button:-1,buttons:0},
    {phase:'first-down',event:'mousedown',x:80,button:first,buttons:masks[first]},
    {phase:'held-move',event:'mousemove',x:endX,button:-1,buttons:masks[first]},
    {phase:'second-down',event:'mousedown',x:endX,button:second,buttons:full},
    {phase:'first-up',event:'mouseup',x:endX,button:release[0],buttons:masks[release[1]]},
    {phase:'last-up',event:'mouseup',x:endX,button:release[1],buttons:0}];
  function check(name,actual,expected) {checks.push({name,actual,expected,pass:JSON.stringify(actual)===JSON.stringify(expected)});}
  return {steps,prepare(value){phase=value;},finish(){
    const activation=rows.filter(row=>row.type==='click'||row.type==='auxclick');
    check('both releases activate their corresponding press',activation.map(row=>[row.phase,row.type,row.target,row.button,row.buttons]),
      release.map((button,index)=>[index===0?'first-up':'last-up',button===0?'click':'auxclick',
        mode==='same'?'source':mode==='capture-peer'||button===second?'peer':'common',button,index===0?masks[release[1]]:0]));
    for(const type of ['pointerdown','pointerup']) check(type+' only once',rows.filter(row=>row.type===type).length,1);
    for(const type of ['mousedown','mouseup']) check(type+' once per button',rows.filter(row=>row.type===type).length,2);
    check('intermediate release is pointermove',rows.filter(row=>row.phase==='first-up'&&row.type==='pointermove').map(row=>[row.button,row.buttons]),[[release[0],masks[release[1]]]]);
    check('last release is pointerup',rows.filter(row=>row.phase==='last-up'&&row.type==='pointerup').map(row=>[row.button,row.buttons]),[[release[1],0]]);
    for(const type of ['gotpointercapture','lostpointercapture']) check(type+' count',rows.filter(row=>row.type===type).length,mode==='capture-peer'?1:0);
    if(mode==='capture-peer') check('capture lasts through intermediate release',rows.filter(row=>row.type==='lostpointercapture').map(row=>row.phase),['last-up']);
    for(const [i,row] of rows.entries()) {
      for(const key of ['exactPrototype','typed','onlyOwner','view','pathIdentity','trusted','bubbles','composed']) check(i+'/'+key,row[key],true);
      check(i+'/cancelable',row.cancelable,!row.type.endsWith('pointercapture'));
      check(i+'/modifiers',row.modifiers,[Boolean(modifiers&1),Boolean(modifiers&2),Boolean(modifiers&4),Boolean(modifiers&8)]);
      if(!['mousedown','mouseup'].includes(row.type)) check(i+'/pointerType',row.pointerType,pointer);
    }
    check('capture released',source.hasPointerCapture(id)||peer.hasPointerCapture(id),false);
    check('author constructors not read',reads,0);check('author ancestry not read',parentReads,0);
    return {context,pointer,pair,reverse,mode,poison,modifiers,rows,checks,reads,parentReads,
      passed:checks.filter(c=>c.pass).length,total:checks.length,complete:checks.every(c=>c.pass)};
  }};
};
