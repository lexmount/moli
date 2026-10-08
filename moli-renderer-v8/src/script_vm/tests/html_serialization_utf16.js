(async () => {
  const checks = [], observe = action => {try {return action();} catch(error) {return {exception: error.name, message: String(error)};}};
  const check = (name, action, expected) => {const actual=observe(action); checks.push({name,actual,expected,passed:JSON.stringify(actual)===JSON.stringify(expected)});};
  const attribute = value => String(value).replaceAll('&','&amp;').replaceAll('"','&quot;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('\u00a0','&nbsp;');
  const text = value => value.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('\u00a0','&nbsp;');
  const htmlNS='http://www.w3.org/1999/xhtml';
  const values=['','ascii','\0','\uD800','\uD801','\uDC00','\uD800\uD800','\uDC00\uD800','\uD83D\uDE00','\uFFFD','a\uD800&<>"\u00a0z','\uD800\uD83D\uDE00&\uDC00'];
  const frame=document.createElement('iframe'); document.body.appendChild(frame);
  let popup;
  try {
    const realms=[['main',window],['iframe',frame.contentWindow]];
    if(globalThis.__htmlSerializationIncludePopup !== false) {
      popup=window.open('', '_blank'); if(!popup) throw new Error('popup unavailable');
      await new Promise(resolve=>setTimeout(resolve,0)); realms.push(['popup',popup]);
    }
    for(const [ownerName,owner] of realms) {
      const docs=[['live',owner.document],['html',owner.document.implementation.createHTMLDocument('')],['parsed',new owner.DOMParser().parseFromString('<!doctype html><body>','text/html')]];
      for(const [docName,doc] of docs) {
        for(const [index,value] of values.entries()) {
          const prefix=`${ownerName}/${docName}/${index}`;
          const element=doc.createElement('div'); element.setAttribute('data-value',value); element.setAttributeNS('urn:data','p:value',value);
          element.appendChild(doc.createTextNode(value)); element.appendChild(doc.createComment(value));
          const inner=`${text(value)}<!--${value}-->`, outer=`<div data-value="${attribute(value)}" p:value="${attribute(value)}">${inner}</div>`;
          check(prefix+'/inner',()=>element.innerHTML,inner);
          check(prefix+'/outer',()=>element.outerHTML,outer);
          check(prefix+'/getHTML',()=>element.getHTML(),inner);
          check(prefix+'/clone',()=>element.cloneNode(true).outerHTML,outer);
          const other=owner.document.implementation.createHTMLDocument('');
          check(prefix+'/import',()=>other.importNode(element,true).outerHTML,outer);
          const adopted=element.cloneNode(true); other.adoptNode(adopted);
          check(prefix+'/adopt',()=>adopted.outerHTML,outer);
          for(const [calleeName,callee] of realms) {
            const innerGetter=Object.getOwnPropertyDescriptor(callee.Element.prototype,'innerHTML').get;
            const outerGetter=Object.getOwnPropertyDescriptor(callee.Element.prototype,'outerHTML').get;
            check(prefix+`/borrowed-${calleeName}-inner`,()=>innerGetter.call(element),inner);
            check(prefix+`/borrowed-${calleeName}-outer`,()=>outerGetter.call(element),outer);
            check(prefix+`/borrowed-${calleeName}-getHTML`,()=>callee.Element.prototype.getHTML.call(element),inner);
          }
          for(const [ns,method] of [[false,doc.createElement],[true,doc.createElementNS]]) {
            const name=`${prefix}/${ns?'ns':'html'}`;
            const options={is:{toString(){return value;}}};
            const button=ns?method.call(doc,htmlNS,'button',options):method.call(doc,'button',options);
            const expected=`<button is="${attribute(value)}"></button>`;
            check(name+'/is',()=>button.outerHTML,expected);
            check(name+'/attribute',()=>button.getAttribute('is'),null);
            check(name+'/clone',()=>button.cloneNode(true).outerHTML,expected);
            check(name+'/import',()=>other.importNode(button,true).outerHTML,expected);
            check(name+'/adopt',()=>other.adoptNode(button.cloneNode(true)).outerHTML,expected);
            button.setAttribute('is','content-'+value);
            check(name+'/content-is',()=>button.outerHTML,`<button is="${attribute('content-'+value)}"></button>`);
            button.removeAttribute('is');
            check(name+'/removed-content-is',()=>button.outerHTML,expected);
            check(name+'/removed-content-is-clone',()=>button.cloneNode(true).outerHTML,expected);
          }
          for(const tag of ['script','style','textarea','noscript']) {
            const raw=doc.createElement(tag); raw.appendChild(doc.createTextNode(value));
            const expected=(tag==='script'||tag==='style'||(tag==='noscript'&&docName==='live'))?value:text(value);
            check(prefix+`/raw-${tag}-inner`,()=>raw.innerHTML,expected);
            check(prefix+`/raw-${tag}-getHTML`,()=>raw.getHTML(),expected);
          }
          const template=doc.createElement('template'); template.content.appendChild(element.cloneNode(true));
          check(prefix+'/template-inner',()=>template.innerHTML,outer);
          check(prefix+'/template-getHTML',()=>template.getHTML(),outer);
          const nested=doc.createElement('noscript'); nested.appendChild(template.content.ownerDocument.createTextNode(value)); template.content.appendChild(nested);
          check(prefix+'/template-inert-noscript',()=>template.innerHTML,outer+`<noscript>${text(value)}</noscript>`);
          const host=doc.createElement('section'), shadow=host.attachShadow({mode:'closed',serializable:true}); shadow.appendChild(element.cloneNode(true)); host.appendChild(doc.createTextNode(value));
          const shadowHTML=`<template shadowrootmode="closed" shadowrootserializable="">${outer}</template>${text(value)}`;
          check(prefix+'/shadow-inner',()=>shadow.innerHTML,outer);
          check(prefix+'/shadow-getHTML',()=>shadow.getHTML(),outer);
          check(prefix+'/host-excluded',()=>host.getHTML(),text(value));
          check(prefix+'/host-serializable',()=>host.getHTML({serializableShadowRoots:true}),shadowHTML);
          check(prefix+'/host-explicit',()=>host.getHTML({shadowRoots:[shadow]}),shadowHTML);
        }
        const adjacent=doc.createElement('div'); adjacent.appendChild(doc.createTextNode('\uD83D')); adjacent.appendChild(doc.createTextNode('\uDE00'));
        check(`${ownerName}/${docName}/adjacent-pair`,()=>adjacent.innerHTML,'\uD83D\uDE00');
        check(`${ownerName}/${docName}/adjacent-pair-getHTML`,()=>adjacent.getHTML(),'\uD83D\uDE00');
      }
      for(const late of [false,true]) {
        const name=`x-html-unit-${ownerName}-${late}-\uFFFD`, invalid=name.replace('\uFFFD','\uD800'); let constructed=0;
        class Collision extends owner.HTMLButtonElement {constructor(){super();constructed++;}}
        const create=()=>owner.document.createElement('button',{is:invalid});
        const early=late?create():null;
        owner.customElements.define(name,Collision,{extends:'button'});
        const candidate=early||create();
        check(`${ownerName}/collision-${late}-initial`,()=>[constructed,candidate instanceof Collision],[0,false]);
        const copied=candidate.cloneNode(true), imported=owner.document.importNode(candidate,true);
        check(`${ownerName}/collision-${late}-copy`,()=>[constructed,copied instanceof Collision,imported instanceof Collision],[0,false,false]);
        candidate.setAttribute('is',name); owner.document.body.appendChild(candidate); owner.customElements.upgrade(candidate);
        check(`${ownerName}/collision-${late}-attribute`,()=>[constructed,candidate instanceof Collision],[0,false]); candidate.remove();
        const valid=owner.document.createElement('button',{is:name});
        check(`${ownerName}/collision-${late}-valid`,()=>[constructed,valid instanceof Collision],[1,true]);
      }
    }
    globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks,includePopup:globalThis.__htmlSerializationIncludePopup!==false};
    return globalThis.__uiEventResults.passed===checks.length;
  } finally {frame.remove(); if(popup) popup.close();}
})()
