globalThis.__installDragInitProbe = function(mode) {
  document.body.innerHTML = '<style>body{margin:0;user-select:none}#outer{position:absolute;left:40px;top:40px;width:140px;height:140px}#inner{display:block;width:100px;height:100px}#destination{position:absolute;left:300px;top:40px;width:140px;height:140px}</style><div id="outer"><div id="inner"></div></div><div id="destination"></div>';
  const outer=document.getElementById('outer'), destination=document.getElementById('destination');
  let inner=document.getElementById('inner'), reads=0, expected='outer';
  outer.draggable=true;
  if (['plain','false','auto','invalid','anchor-no-href','svg','secondary','cancel-mousedown','cancel-pointerdown','remove-source','toggle-off','no-motion','jitter-3','chorded'].includes(mode)) expected=null;
  if (['plain','auto','invalid','toggle-on'].includes(mode)) outer.removeAttribute('draggable');
  if(mode==='false') outer.draggable=false;
  if(mode==='invalid') outer.setAttribute('draggable','invalid');
  if(mode==='case-true') outer.setAttribute('draggable','TrUe');
  if(mode==='inner-draggable') {inner.draggable=true;expected='inner';}
  if(mode==='inner-false') inner.draggable=false;
  if(mode==='anchor' || mode==='anchor-no-href') {
    outer.removeAttribute('draggable');
    const link=document.createElement('a');link.id='inner';link.style.cssText='display:block;width:100px;height:100px';
    if(mode==='anchor') {link.href='#destination';expected='inner';}
    inner.replaceWith(link);inner=link;
  }
  if(mode==='image') {
    outer.removeAttribute('draggable');
    const image=document.createElement('img');image.id='inner';image.width=100;image.height=100;
    image.src='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==';
    inner.replaceWith(image);inner=image;expected='inner';
  }
  if(mode==='svg') {
    outer.removeAttribute('draggable');outer.innerHTML='<svg id="inner" width="100" height="100" draggable="true"><rect width="100" height="100"/></svg>';inner=outer.firstChild;
  }
  if(mode==='shadow') {
    outer.removeAttribute('draggable');outer.attachShadow({mode:'open'}).innerHTML='<div id="shadowSource" draggable="true" style="width:140px;height:140px"><slot></slot></div>';expected=null;
  }
  if(mode==='shadow-descendant' || mode==='shadow-closed-descendant') {
    outer.removeAttribute('draggable');outer.attachShadow({mode:mode==='shadow-descendant'?'open':'closed'}).innerHTML='<div id="shadowSource" draggable="true" style="width:140px;height:140px"><div id="shadowInner" style="width:100px;height:100px"></div></div>';expected=mode==='shadow-descendant'?'shadowSource':'outer';
  }
  if(mode==='native-getters') {
    for(const [object,key] of [[outer,'draggable'],[inner,'parentNode'],[inner,'ownerDocument']]) Object.defineProperty(object,key,{get(){reads++;throw new Error('author getter must not run');}});
  }
  outer.addEventListener('mousedown',event=>{
    if(mode==='cancel-mousedown') event.preventDefault();
    if(mode==='toggle-on') outer.draggable=true;
    if(mode==='toggle-off') outer.draggable=false;
    if(mode==='reparent-inner') destination.append(inner);
    if(mode==='reparent-source') {outer.style.left='300px';expected=null;}
    if(mode==='remove-source') outer.remove();
    if(mode==='replace-target') {
      const replacement=document.createElement('div');replacement.id='replacement';replacement.draggable=true;replacement.style.cssText='width:100px;height:100px';inner.replaceWith(replacement);expected='replacement';
    }
  });
  outer.addEventListener('pointerdown',event=>{if(mode==='cancel-pointerdown') event.preventDefault();});
  const starts=[],trace=[];
  for(const type of ['pointerdown','mousedown','pointermove','mousemove','pointerup','mouseup','pointercancel','dragstart','dragover','drop','click']) {
    document.addEventListener(type,event=>{
      trace.push({phase:globalThis.__dragPhase,type,target:event.composedPath()[0].id||event.composedPath()[0].nodeName,button:event.button,buttons:event.buttons,clientX:event.clientX,clientY:event.clientY,defaultPrevented:event.defaultPrevented});
      if(type==='dragstart') {
        starts.push({phase:globalThis.__dragPhase,target:event.composedPath()[0].id||event.composedPath()[0].nodeName,clientX:event.clientX,clientY:event.clientY,bubbles:event.bubbles,cancelable:event.cancelable,composed:event.composed,isTrusted:event.isTrusted,transfer:event.dataTransfer instanceof DataTransfer});
        event.preventDefault();
      }
    });
  }
  return {finish(){
    const checks=[];
    const check=(name,actual,wanted)=>checks.push({name,actual,expected:wanted,pass:JSON.stringify(actual)===JSON.stringify(wanted)});
    check('dragstart count',starts.length,expected===null?0:1);
    check('dragstart source',starts.map(row=>row.target),expected===null?[]:[expected]);
    check('author getter reads',reads,0);
    check('canceled drag cannot dragover or drop',trace.filter(row=>row.type==='dragover'||row.type==='drop').length,0);
    if(expected!==null) {
      check('dragstart origin coordinates',starts.map(row=>[row.clientX,row.clientY]),[[80,80]]);
      check('dragstart flags',starts.map(row=>[row.bubbles,row.cancelable,row.composed,row.isTrusted,row.transfer]),[[true,true,true,true,true]]);
      check('threshold phase',starts.map(row=>row.phase),[mode==='subthreshold-then-cross'?'cross':'move1']);
    }
    return {mode,expected,passed:checks.filter(row=>row.pass).length,total:checks.length,complete:checks.every(row=>row.pass),checks,starts,trace};
  }};
};
