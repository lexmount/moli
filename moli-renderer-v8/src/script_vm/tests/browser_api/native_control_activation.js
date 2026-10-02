(() => {
  const globals=[window,document.getElementById('child').contentWindow];
  const rows=[],errors=[];
  for(let realm=0;realm<globals.length;realm++) {
    const g=globals[realm];
    for(const documentKind of ['live','windowless','parsed']) {
      const d=documentKind==='live'?g.document:documentKind==='windowless'?g.document.implementation.createHTMLDocument(''):new g.DOMParser().parseFromString('<!doctype html><body>', 'text/html');
      for(const attachment of ['document','fragment','detached','shadow-connected','shadow-detached']) {
        for(const type of ['checkbox','radio']) for(const activation of ['click','mouse','pointer']) for(const initial of [false,true]) {
          const input=d.createElement('input');input.type=type;input.checked=initial;input.indeterminate=true;
          let owner;
          if(attachment==='document')d.body.appendChild(input);
          if(attachment==='fragment'){owner=d.createDocumentFragment();owner.appendChild(input);}
          if(attachment.startsWith('shadow')){owner=d.createElement('div');owner.attachShadow({mode:'open'}).appendChild(input);if(attachment==='shadow-connected')d.body.appendChild(owner);}
          const events=[],host=[];
          const label=[realm,documentKind,attachment,type,activation,initial].join('/');
          const listen=event=>events.push({type:event.type,bubbles:event.bubbles,cancelable:event.cancelable,composed:event.composed,trusted:event.isTrusted,checked:input.checked,connected:input.isConnected,realm:event instanceof g.Event,tag:g.Object.prototype.toString.call(event),target:event.target===input});
          for(const e of ['click','input','change'])input.addEventListener(e,listen);
          if(owner&&attachment.startsWith('shadow'))for(const e of ['input','change'])owner.addEventListener(e,event=>host.push({type:event.type,retarget:event.target===owner}));
          try {
            if(activation==='click')globals[1-realm].HTMLElement.prototype.click.call(input);
            else input.dispatchEvent(new globals[1-realm][activation==='mouse'?'MouseEvent':'PointerEvent']('click',{bubbles:true,cancelable:true,composed:true}));
            const connected=attachment==='document'||attachment==='shadow-connected';
            const emits=connected && !(type==='radio' && initial);
            const checks={
              connected:input.isConnected===connected,
              checked:input.checked===(type==='checkbox'?!initial:true),
              indeterminate:input.indeterminate===(type==='radio'),
              sequence:events.map(e=>e.type).join(',')===(emits?'click,input,change':'click'),
              nativeRealm:events.filter(e=>e.type!=='click').every(e=>e.realm),
              nativeType:events.filter(e=>e.type!=='click').every(e=>e.tag==='[object Event]' && e.target),
              nativeFlags:events.filter(e=>e.type!=='click').every(e=>e.bubbles && !e.cancelable && e.composed===(e.type==='input')),
              trusted:events.every(e=>e.trusted===(e.type!=='click')),
              hostSequence:host.map(e=>e.type).join(',')===(emits&&attachment.startsWith('shadow')?'input':''),
              retarget:host.every(e=>e.retarget)};
            rows.push({label,checks,events,host});
          } catch(error){errors.push({label,message:String(error)});}
          input.remove();if(owner&&owner.remove)owner.remove();
        }
      }
    }
  }
  globalThis.__uiEventResults={rows,errors,passed:errors.length===0 && rows.every(row=>Object.values(row.checks).every(value=>value===true))};return __uiEventResults.passed;
})()
