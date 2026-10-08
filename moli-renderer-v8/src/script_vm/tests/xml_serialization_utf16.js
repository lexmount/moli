(async () => {
  const checks=[];
  const observe=action=>{try{return action();}catch(error){return {exception:error.name,message:String(error)};}};
  const check=(name,action,expected)=>{const actual=observe(action);checks.push({name,actual,expected,passed:JSON.stringify(actual)===JSON.stringify(expected)});};
  const text=value=>value.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;');
  const attribute=value=>text(value).replaceAll('"','&quot;').replaceAll('\t','&#9;').replaceAll('\n','&#10;').replaceAll('\r','&#13;');
  const htmlNS='http://www.w3.org/1999/xhtml', xmlnsNS='http://www.w3.org/2000/xmlns/';
  const values=['','ascii','\0','\uD800','\uD801','\uDC00','\uD800\uD800','\uDC00\uD800','\uD83D\uDE00','\uFFFD','a\uD800&<>"\t\n\r\u00a0z','\uD800\uD83D\uDE00&\uDC00'];
  const frame=document.createElement('iframe'); document.body.appendChild(frame);
  let popup;
  try {
    const realms=[['main',window],['iframe',frame.contentWindow]];
    if(globalThis.__xmlSerializationIncludePopup!==false) {
      popup=window.open('','_blank'); if(!popup) throw new Error('popup unavailable');
      await new Promise(resolve=>setTimeout(resolve,0)); realms.push(['popup',popup]);
    }
    for(const [ownerName,owner] of realms) {
      const docs=[['live',owner.document,false],['html',owner.document.implementation.createHTMLDocument(''),false],['xml',owner.document.implementation.createDocument(null,'root'),true],['parsed',new owner.DOMParser().parseFromString('<root/>','application/xml'),true],['xhtml',new owner.DOMParser().parseFromString(`<html xmlns="${htmlNS}"/>`,'application/xhtml+xml'),true]];
      for(const [docName,doc,isXML] of docs) {
        const serializer=new owner.XMLSerializer();
        for(const [index,value] of values.entries()) {
          const prefix=`${ownerName}/${docName}/${index}`;
          const element=doc.createElementNS('urn:payload','p:root');
          element.setAttribute('v',value);
          element.setAttributeNS(xmlnsNS,'xmlns:a','urn:attribute');
          element.setAttributeNS('urn:attribute','a:v',value);
          element.appendChild(doc.createTextNode(value));
          element.appendChild(doc.createComment(value));
          const inner=`${text(value)}<!--${value}-->`;
          const outer=`<p:root xmlns:p="urn:payload" v="${attribute(value)}" xmlns:a="urn:attribute" a:v="${attribute(value)}">${inner}</p:root>`;
          check(prefix+'/element',()=>serializer.serializeToString(element),outer);
          check(prefix+'/clone',()=>serializer.serializeToString(element.cloneNode(true)),outer);
          const other=owner.document.implementation.createDocument(null,'other');
          check(prefix+'/import',()=>serializer.serializeToString(other.importNode(element,true)),outer);
          const adopted=element.cloneNode(true); other.adoptNode(adopted);
          check(prefix+'/adopt',()=>serializer.serializeToString(adopted),outer);
          const attr=element.getAttributeNode('v');
          check(prefix+'/attached-attr',()=>serializer.serializeToString(attr),attribute(value));
          const detachedAttr=doc.createAttribute('v'); detachedAttr.value=value;
          check(prefix+'/detached-attr',()=>serializer.serializeToString(detachedAttr),attribute(value));
          check(prefix+'/cloned-attr',()=>serializer.serializeToString(detachedAttr.cloneNode()),attribute(value));
          for(const [calleeName,callee] of realms) {
            const method=callee.XMLSerializer.prototype.serializeToString, receiver=new callee.XMLSerializer();
            check(prefix+`/borrowed-${calleeName}`,()=>method.call(receiver,element),outer);
            check(prefix+`/borrowed-${calleeName}-attr`,()=>method.call(receiver,attr),attribute(value));
            if(isXML) {
              const innerGetter=Object.getOwnPropertyDescriptor(callee.Element.prototype,'innerHTML').get;
              const outerGetter=Object.getOwnPropertyDescriptor(callee.Element.prototype,'outerHTML').get;
              check(prefix+`/borrowed-${calleeName}-inner`,()=>innerGetter.call(element),inner);
              check(prefix+`/borrowed-${calleeName}-outer`,()=>outerGetter.call(element),outer);
            }
          }
          if(isXML) {
            check(prefix+'/inner',()=>element.innerHTML,inner);
            check(prefix+'/outer',()=>element.outerHTML,outer);
          }
          element.removeAttributeNode(attr);
          check(prefix+'/removed-attr',()=>serializer.serializeToString(attr),attribute(value));
          const nodes=[['text',doc.createTextNode(value),text(value)],['comment',doc.createComment(value),`<!--${value}-->`],['pi',doc.createProcessingInstruction('target',value),`<?target ${value}?>`]];
          if(isXML) nodes.push(['cdata',doc.createCDATASection(value),`<![CDATA[${value}]]>`]);
          for(const [kind,node,expected] of nodes) {
            check(prefix+`/${kind}`,()=>serializer.serializeToString(node),expected);
            const fragment=doc.createDocumentFragment(); fragment.appendChild(node);
            check(prefix+`/${kind}-fragment`,()=>serializer.serializeToString(fragment),expected);
            if(isXML) {
              const parent=doc.createElementNS(null,'parent'); parent.appendChild(node.cloneNode(true));
              check(prefix+`/${kind}-inner`,()=>parent.innerHTML,expected);
              check(prefix+`/${kind}-outer`,()=>parent.outerHTML,`<parent>${expected}</parent>`);
            }
          }
        }
        const adjacent=doc.createElementNS(null,'pair'); adjacent.append(doc.createTextNode('\uD83D'),doc.createTextNode('\uDE00'));
        check(`${ownerName}/${docName}/split-pair`,()=>serializer.serializeToString(adjacent),'<pair>\uD83D\uDE00</pair>');
        if(isXML) check(`${ownerName}/${docName}/split-pair-inner`,()=>adjacent.innerHTML,'\uD83D\uDE00');
        const collision=doc.createElementNS(null,'root');
        collision.setAttributeNS(xmlnsNS,'xmlns:p','urn:\uD800');
        collision.setAttributeNS('urn:\uFFFD','value','x');
        const expected='<root xmlns:p="urn:\uD800" xmlns:ns1="urn:\uFFFD" ns1:value="x"/>';
        check(`${ownerName}/${docName}/namespace-collision`,()=>serializer.serializeToString(collision),expected);
        if(isXML) check(`${ownerName}/${docName}/namespace-collision-outer`,()=>collision.outerHTML,expected);
        let reads=0;
        const altered=doc.createElementNS(null,'root'); altered.setAttribute('v','\uD800'); altered.appendChild(doc.createTextNode('\uDC00'));
        const alteredAttr=altered.getAttributeNode('v');
        for(const name of ['nodeType','tagName','childNodes','getAttributeNames','getAttribute']) Object.defineProperty(altered,name,{get(){reads++;throw new Error(name);}});
        Object.defineProperty(alteredAttr,'value',{get(){reads++;throw new Error('value');}});
        check(`${ownerName}/${docName}/native-element-data`,()=>serializer.serializeToString(altered),'<root v="\uD800">\uDC00</root>');
        check(`${ownerName}/${docName}/native-attr-data`,()=>serializer.serializeToString(alteredAttr),'\uD800');
        check(`${ownerName}/${docName}/native-data-no-author-reads`,()=>reads,0);
        if(isXML) {
          const document=owner.document.implementation.createDocument(null,'root'); document.documentElement.appendChild(document.createTextNode('\uD800'));
          check(`${ownerName}/${docName}/document`,()=>serializer.serializeToString(document),'<root>\uD800</root>');
        }
      }
      const real=owner.document.implementation.createHTMLDocument('').createElement('select');
      check(`${ownerName}/registered-native-proxy`,()=>new owner.XMLSerializer().serializeToString(real),`<select xmlns="${htmlNS}"></select>`);
      for(const [calleeName,callee] of realms) {
        const method=callee.XMLSerializer.prototype.serializeToString, serializer=new callee.XMLSerializer();
        let traps=0;
        const fake={}; for(const name of ['nodeType','childNodes','toString','valueOf']) Object.defineProperty(fake,name,{get(){traps++;throw new Error(name);}});
        const proxy=new Proxy(real,{get(){traps++;throw new Error('get');},getPrototypeOf(){traps++;throw new Error('prototype');}});
        const revoked=Proxy.revocable(real,{}); revoked.revoke();
        const invalid=[undefined,null,{},fake,'root',0,true,Symbol(),Object.create(callee.Node.prototype),Object.create(real),proxy,revoked.proxy];
        const typeError=action=>{try{action();return null;}catch(error){return [error.name,error instanceof callee.TypeError];}};
        for(const [i,value] of invalid.entries()) check(`${ownerName}/${calleeName}/invalid-${i}`,()=>typeError(()=>method.call(serializer,value)),['TypeError',true]);
        check(`${ownerName}/${calleeName}/missing`,()=>typeError(()=>method.call(serializer)),['TypeError',true]);
        for(const [i,receiver] of [{},Object.create(callee.XMLSerializer.prototype),new Proxy(serializer,{})].entries()) check(`${ownerName}/${calleeName}/receiver-${i}`,()=>typeError(()=>method.call(receiver,fake)),['TypeError',true]);
        check(`${ownerName}/${calleeName}/brand-check-no-author-reads`,()=>traps,0);
      }
    }
    globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks,includePopup:globalThis.__xmlSerializationIncludePopup!==false};
    return globalThis.__uiEventResults.passed===checks.length;
  } finally {frame.remove();if(popup)popup.close();}
})()
