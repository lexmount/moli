(() => {
 const cases=[['a','ping','ping','usv'],['area','ping','ping','usv']];
 const frame=document.body.appendChild(document.createElement('iframe'));
 const realms=[window,frame.contentWindow],failures=[];let total=0;
 try {for(const [realm,w] of realms.entries()) {
  const docs=[w.document,w.document.implementation.createHTMLDocument(''),new w.DOMParser().parseFromString('<body></body>','text/html')];
  for(const [kind,doc] of docs.entries()) for(const [tag,property,attribute,type] of cases)
   for(const order of ['foreign-only','foreign-first','null-first','prefixed']) for(const value of type==='boolean'?[true,false,0]:type==='usv'?['\ud800x','ok',null]:['first','ok',null]) {
    const label=[realm,kind,tag,property,order,String(value)].join('/');let observer,controlObserver;
    try {
     const e=doc.createElement(tag),control=doc.createElement(tag),ns='urn:reflection-test';
     let proto=e,descriptor;while(proto&&!descriptor){descriptor=Object.getOwnPropertyDescriptor(proto,property);proto=Object.getPrototypeOf(proto);}
     if(typeof descriptor?.get!=='function'||typeof descriptor?.set!=='function')throw Error('missing native accessor');
     const foreignValue=type==='boolean'?'':'foreign',seed=type==='boolean'?'':'native';
     const foreign=()=>e.setAttributeNS(ns,order==='prefixed'?'f:'+attribute:attribute,foreignValue);
     const native=()=>{e.setAttributeNS(null,attribute,seed);control.setAttributeNS(null,attribute,seed);};
     if(order==='null-first'){native();foreign();}else{foreign();if(order==='foreign-first')native();}
     const attr=e.getAttributeNodeNS(ns,attribute);
     const check=(ok,name)=>{total++;if(!ok)failures.push(label+':'+name);};
     check(e[property]===control[property],'initial native getter');
     check(e.getAttribute(order==='prefixed'?'f:'+attribute:attribute)===(order==='null-first'?seed:foreignValue),'qualified DOM lookup');
     observer=new w.MutationObserver(()=>{});controlObserver=new w.MutationObserver(()=>{});
     for(const [o,node] of [[observer,e],[controlObserver,control]])o.observe(node,{attributes:true,attributeOldValue:true});
     e[property]=value;control[property]=value;
     check(e[property]===control[property],'reflected result');
     check(e.getAttributeNS(null,attribute)===control.getAttributeNS(null,attribute),'null namespace value');
     check(e.getAttributeNodeNS(ns,attribute)===attr&&attr.value===foreignValue,'foreign identity and value');
     const records=o=>o.takeRecords().map(r=>[r.type,r.attributeName,r.attributeNamespace,r.oldValue]);
     check(JSON.stringify(records(observer))===JSON.stringify(records(controlObserver)),'mutation records');
     if(type==='usv')check(e.getAttributeNS(null,attribute)===(value===null?'null':String(value).replace(/\ud800/g,'\ufffd')),'scalar conversion');
    }catch(error){failures.push(label+':'+String(error));}finally{observer?.disconnect();controlObserver?.disconnect();}
   }
 }}finally{frame.remove();}
 return JSON.stringify({total,failures});
})()
