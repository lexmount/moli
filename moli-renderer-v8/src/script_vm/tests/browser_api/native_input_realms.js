globalThis.__installMouseWheelRealmProbe = function (context, poison, pointer, modifiers) {
  const root = window, nodes = [], names = new Map(), rows = [], checks = [];
  const windows = [root];
  document.documentElement.style.cssText='margin:0;padding:0';
  document.body.style.cssText='margin:0;padding:0';
  function frame(doc, left) {
    const frame=doc.createElement('iframe');
    frame.style.cssText=`position:fixed;left:${left}px;top:0;width:240px;height:220px;border:0`;
    doc.body.appendChild(frame);
    const win=frame.contentWindow;windows.push(win);
    win.document.documentElement.style.cssText='margin:0;padding:0';
    win.document.body.style.cssText='margin:0;padding:0';
    return win;
  }
  let owner=root;
  if (context!=='root') owner=frame(document,0);
  if (context==='nested') owner=frame(owner.document,0);
  const cross=frame(document,260);
  let holder=owner.document.body;
  if (context.startsWith('shadow-')) {
    const host=owner.document.createElement('div');
    host.style.cssText='position:fixed;left:0;top:0;width:240px;height:220px';holder.appendChild(host);
    names.set(host,'shadow-host');holder=host.attachShadow({mode:context==='shadow-closed'?'closed':'open'});
  }
  function element(win,parent,name,left,width) {
    const node=win.document.createElement('div');node.id=name;
    node.style.cssText=`position:fixed;left:${left}px;top:50px;width:${width}px;height:100px;background:#acf`;
    parent.appendChild(node);nodes.push({node,win});names.set(node,name);return node;
  }
  const source=element(owner,holder,'source',50,100);
  const peer=element(owner,holder,'peer',170,50);
  const other=element(cross,cross.document.body,'cross',50,100);
  const outside=element(root,document.body,'outside',520,80);
  const intrinsics=new Map(windows.map(win=>[win,{Mouse:win.MouseEvent,Wheel:win.WheelEvent,Ui:win.UIEvent}]));
  let phase='reset', constructorReads=0;
  for (const {node,win} of nodes) {
    for (const type of ['mouseover','mouseout','mouseenter','mouseleave','mousemove','wheel']) {
      node.addEventListener(type,e=>{
        if (phase==='reset'||node===outside) return;
        const constructors=intrinsics.get(win), Kind=type==='wheel'?constructors.Wheel:constructors.Mouse;
        rows.push({phase,type,target:names.get(e.target),listener:names.get(node),related:names.get(e.relatedTarget)||null,
          exactPrototype:Object.getPrototypeOf(e)===Kind.prototype,typed:e instanceof Kind,ui:e instanceof constructors.Ui,
          onlyOwner:windows.every(otherWin=>otherWin===win||!(e instanceof (type==='wheel'?intrinsics.get(otherWin).Wheel:intrinsics.get(otherWin).Mouse))),
          view:e.view===win,targetIdentity:e.target===node,currentIdentity:e.currentTarget===node,
          pathIdentity:e.composedPath()[0]===node,trusted:e.isTrusted,bubbles:e.bubbles,cancelable:e.cancelable,composed:e.composed,
          button:e.button,buttons:e.buttons,coordinates:[e.clientX,e.clientY],
          modifiers:[e.altKey,e.ctrlKey,e.metaKey,e.shiftKey],
          delta:type==='wheel'?[e.deltaX,e.deltaY,e.deltaZ,e.deltaMode]:null});
        if(type==='wheel') e.preventDefault();
      },{passive:false});
    }
  }
  for (const win of windows) for (const name of ['MouseEvent','WheelEvent']) {
    if(poison==='deleted') delete win[name];
    if(poison==='replaced') win[name]=function(){constructorReads++;throw new Error('author '+name);};
    if(poison==='getter') Object.defineProperty(win,name,{configurable:true,get(){constructorReads++;throw new Error('author '+name);}});
  }
  function check(name,actual,expected) {checks.push({name,actual,expected,pass:JSON.stringify(actual)===JSON.stringify(expected)});}
  return {prepare(value){phase=value;},finish(){
    const expected=[['enter','source',['mouseover','mouseenter','mousemove']],
      ['same','source',['mouseout','mouseleave']],['same','peer',['mouseover','mouseenter','mousemove']],
      ['peer-wheel','peer',['wheel']],['cross','peer',['mouseout','mouseleave']],
      ['cross','cross',['mouseover','mouseenter','mousemove']],['return','cross',['mouseout','mouseleave']],
      ['return','source',['mouseover','mouseenter','mousemove']],['source-wheel','source',['wheel']]];
    for (const [at,target,types] of expected) check(at+'/'+target+'/types',rows.filter(r=>r.phase===at&&r.listener===target).map(r=>r.type),types);
    for (const [i,r] of rows.entries()) {
      for (const key of ['exactPrototype','typed','ui','onlyOwner','view','targetIdentity','currentIdentity','pathIdentity','trusted']) check(i+'/'+key,r[key],true);
      const boundary=r.type==='mouseenter'||r.type==='mouseleave';
      for(const key of ['bubbles','cancelable','composed']) check(i+'/'+key,r[key],!boundary);
      check(i+'/button',r.button,0);check(i+'/buttons',r.buttons,0);
      check(i+'/modifiers',r.modifiers,[Boolean(modifiers&1),Boolean(modifiers&2),Boolean(modifiers&4),Boolean(modifiers&8)]);
      if(r.type==='wheel') check(i+'/delta',r.delta,[4,7,0,0]);
      if(r.phase==='same'&&r.type!=='mousemove') check(i+'/same-document-related',r.related,r.listener==='source'?'peer':'source');
    }
    check('author constructor not read',constructorReads,0);
    return {context,poison,pointer,modifiers,rows,checks,constructorReads,
      passed:checks.filter(c=>c.pass).length,total:checks.length,complete:checks.every(c=>c.pass)};
  }};
};
