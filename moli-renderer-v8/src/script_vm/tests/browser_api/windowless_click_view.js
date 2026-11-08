() => {
  const failures=[];
  const realms=[window,document.getElementById('child').contentWindow];
  for (const [realmIndex,w] of realms.entries()) {
    const docs=[w.document,w.document.implementation.createHTMLDocument(''),new w.DOMParser().parseFromString('<!doctype html><body>','text/html')];
    for (const [docIndex,doc] of docs.entries()) for (const attached of [false,true]) {
      const button=doc.createElement('button');if(attached)doc.body.appendChild(button);
      let expected=docIndex===0?w:null,hits=0,getterCalls=0;
      const original=Object.getOwnPropertyDescriptor(doc,'defaultView');
      Object.defineProperty(doc,'defaultView',{configurable:true,get(){getterCalls++;throw Error('author defaultView')}});
      button.addEventListener('click',function(event){
        hits++;
        if(event.view!==expected || !(event instanceof w.MouseEvent) || event.target!==button || this!==button || event.isTrusted || event.detail!==0 || !event.bubbles || !event.composed)
          failures.push([realmIndex,docIndex,attached,'event']);
        event.preventDefault();
      });
      try {
        w.HTMLElement.prototype.click.call(button);
        if(hits!==1 || getterCalls!==0)failures.push([realmIndex,docIndex,attached,'dispatch']);
        if(docIndex!==0){
          w.document.adoptNode(button);w.document.body.appendChild(button);expected=w;
          w.HTMLElement.prototype.click.call(button);
          if(hits!==2 || getterCalls!==0)failures.push([realmIndex,docIndex,attached,'adoption']);
        }
      } finally {
        if(original)Object.defineProperty(doc,'defaultView',original);else delete doc.defaultView;
        button.remove();
      }
    }
  }
  return JSON.stringify(failures);
}
