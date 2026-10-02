(() => {
  const NativeObject=Object, Ui=UIEvent, Mouse=MouseEvent;
  const other=document.getElementById('child').contentWindow;
  const ChildUi=other.UIEvent;
  const parentClick=HTMLElement.prototype.click, childClick=other.HTMLElement.prototype.click;
  const parentFocus=HTMLElement.prototype.focus, childFocus=other.HTMLElement.prototype.focus;
  const top=document.body.appendChild(document.createElement('button'));
  const child=other.document.body.appendChild(other.document.createElement('button'));
  const detachedDocument=document.implementation.createHTMLDocument('');
  const detached=detachedDocument.body.appendChild(detachedDocument.createElement('button'));
  const rows=[], restores=[];
  let reads=0;
  const record=(target,type,expectedView,label)=>{
    target.addEventListener(type,event=>rows.push({label,passed:
      (event instanceof Ui||event instanceof ChildUi)&&event.view===expectedView&&
      event.target===target&&event.currentTarget===target,
      viewMatches:event.view===expectedView,
      viewNull:event.view===null,
      trusted:event.isTrusted,
      eventType:event.type}));
  };
  record(top,'click',window,'top click');
  record(child,'click',other,'child click from parent');
  record(detached,'click',null,'windowless click');
  record(top,'focus',window,'top focus from child');
  record(child,'focus',other,'child focus from parent');
  record(top,'probe',other,'explicit synthetic view');
  const synthetic=new Mouse('probe',{view:other});
  function poison(object,key){
    restores.push([object,key,NativeObject.getOwnPropertyDescriptor(object,key)]);
    NativeObject.defineProperty(object,key,{configurable:true,get(){reads++;throw Error('author native UI initialization');}});
  }
  for(const global of [window,other]){
    for(const name of ['MouseEvent','PointerEvent','FocusEvent'])poison(global,name);
    poison(global.Object.prototype,'view');
    poison(global.Object.prototype,'detail');
  }
  try{
    parentClick.call(top);
    parentClick.call(child);
    childClick.call(top);
    parentClick.call(detached);
    childFocus.call(top);
    parentFocus.call(child);
    top.dispatchEvent(synthetic);
  }finally{
    for(let i=restores.length-1;i>=0;i--){
      const entry=restores[i];
      if(entry[2])NativeObject.defineProperty(entry[0],entry[1],entry[2]);
      else delete entry[0][entry[1]];
    }
  }
  globalThis.__uiEventResults={rows,reads,passed:rows.length===7&&reads===0&&rows.every(row=>row.passed)};
  return __uiEventResults.passed;
})()
