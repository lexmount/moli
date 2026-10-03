(() => {
 const rows=[],errors=[],realms=[window,document.getElementById('child').contentWindow],ns='urn:reflection-test';
 const cases=[];
 const add=(kind,tag,property,attribute=property.toLowerCase(),values)=>cases.push({kind,tag,property,attribute,values});
 for(const [tag,property,attribute] of [['a','ping'],['area','ping'],['img','srcset'],['link','imageSrcset','imagesrcset'],['source','srcset']]) add('usv',tag,property,attribute,['\ud800x','ok',null]);
 for(const [tag,property] of [['frame','longDesc'],['iframe','longDesc'],['del','cite'],['ins','cite'],['object','codeBase'],['object','data'],['blockquote','cite'],['q','cite'],['a','href'],['area','href'],['link','href'],['form','action'],['input','src'],['source','src']]) add('url',tag,property,undefined,['#\ud800x','relative',null]);
 for(const [tag,property,attribute] of [['div','autofocus'],['div','inert'],['div','hidden'],['input','disabled'],['input','readOnly','readonly'],['input','required'],['input','multiple'],['input','defaultChecked','checked'],['form','noValidate','novalidate'],['button','disabled'],['button','formNoValidate','formnovalidate'],['select','disabled'],['select','multiple'],['select','required'],['option','disabled'],['option','defaultSelected','selected'],['optgroup','disabled'],['textarea','disabled'],['textarea','readOnly','readonly'],['textarea','required'],['script','defer'],['script','noModule','nomodule'],['img','isMap','ismap'],['ol','reversed'],['details','open'],['dialog','open'],['track','default'],['audio','autoplay'],['audio','controls'],['audio','loop'],['audio','defaultMuted','muted'],['video','autoplay'],['video','controls'],['video','loop'],['video','defaultMuted','muted'],['iframe','allowFullscreen','allowfullscreen'],['template','shadowRootClonable','shadowrootclonable'],['template','shadowRootSerializable','shadowrootserializable'],['template','shadowRootDelegatesFocus','shadowrootdelegatesfocus']]) add('boolean',tag,property,attribute,[true,false,0]);
 for(const tag of ['img','object','marquee']) for(const property of ['hspace','vspace']) add('unsigned',tag,property,undefined,[11,'12',0]);
 for(const [tag,property] of [['ol','start'],['input','width'],['input','height'],['input','size'],['textarea','cols'],['textarea','rows'],['select','size'],['td','colSpan'],['td','rowSpan'],['canvas','width'],['canvas','height']]) add('numeric',tag,property,undefined,[11,'12',1]);
 for(const [tag,property,attribute,values] of [['form','method','method',['post','invalid','get']],['form','autocomplete','autocomplete',['off','invalid','on']],['button','type','type',['reset','invalid','submit']],['input','type','type',['checkbox','invalid','text']],['img','crossOrigin','crossorigin',[null,'anonymous','use-credentials']],['link','crossOrigin','crossorigin',[null,'anonymous','use-credentials']],['script','crossOrigin','crossorigin',[null,'anonymous','use-credentials']],['video','crossOrigin','crossorigin',[null,'anonymous','use-credentials']],['audio','crossOrigin','crossorigin',[null,'anonymous','use-credentials']]]) add('enum',tag,property,attribute,values);
 const descriptor=(object,key)=>{while(object){const d=Object.getOwnPropertyDescriptor(object,key);if(d)return d;object=Object.getPrototypeOf(object);}};
 const records=observer=>observer.takeRecords().map(r=>[r.type,r.attributeName,r.attributeNamespace,r.oldValue]);
 for(let realm=0;realm<realms.length;realm++) {
  const w=realms[realm],docs=[['live',w.document],['windowless',w.document.implementation.createHTMLDocument('')],['parsed',new w.DOMParser().parseFromString('<body></body>','text/html')]];
  for(const [kind,doc] of docs) for(const c of cases) for(const order of ['foreign-only','foreign-first','null-first','prefixed']) for(let index=0;index<c.values.length;index++) {
   const label=[realm,kind,c.kind,c.tag,c.property,order,index].join('/');
   let observer,cleanObserver;
   try {
    const e=doc.createElement(c.tag),clean=doc.createElement(c.tag),d=descriptor(e,c.property);
    if(typeof d?.get!=='function'||typeof d?.set!=='function') throw new Error('missing native accessor');
    const foreignValue=c.kind==='boolean'?'':c.kind==='numeric'||c.kind==='unsigned'?'37':'foreign';
    const seed=c.kind==='boolean'?'':c.kind==='numeric'||c.kind==='unsigned'?'5':c.kind==='enum'?'anonymous':'native';
    const foreign=()=>e.setAttributeNS(ns,order==='prefixed'?'f:'+c.attribute:c.attribute,foreignValue);
    const native=()=>{e.setAttributeNS(null,c.attribute,seed);clean.setAttributeNS(null,c.attribute,seed);};
    if(order==='null-first') {native();foreign();} else {foreign();if(order==='foreign-first')native();}
    const attr=e.getAttributeNodeNS(ns,c.attribute),read=e[c.property]===clean[c.property];
    const qualified=e.hasAttribute(order==='prefixed'?'f:'+c.attribute:c.attribute)&&e.getAttribute(order==='prefixed'?'f:'+c.attribute:c.attribute)===(order==='null-first'?seed:foreignValue);
    observer=new w.MutationObserver(()=>{});cleanObserver=new w.MutationObserver(()=>{});
    observer.observe(e,{attributes:true,attributeOldValue:true});cleanObserver.observe(clean,{attributes:true,attributeOldValue:true});
    const value=c.values[index];e[c.property]=value;clean[c.property]=value;
    const got=records(observer),expected=records(cleanObserver);
    const checks={nativeRead:read,qualifiedDOM:qualified,reflectedValue:e[c.property]===clean[c.property],nullAttribute:e.getAttributeNS(null,c.attribute)===clean.getAttributeNS(null,c.attribute),foreignValue:attr.value===foreignValue,foreignIdentity:e.getAttributeNodeNS(ns,c.attribute)===attr,notification:JSON.stringify(got)===JSON.stringify(expected)};
    if(c.kind==='usv') checks.scalarConversion=e.getAttributeNS(null,c.attribute)===(index===0?'\ufffdx':index===1?'ok':'null');
    rows.push({label,checks,actual:{value:e[c.property],attribute:e.getAttributeNS(null,c.attribute),records:got},expected:{value:clean[c.property],attribute:clean.getAttributeNS(null,c.attribute),records:expected}});
   } catch(e) {errors.push({label,error:String(e)});} finally {observer?.disconnect();cleanObserver?.disconnect();}
  }
 }
 for(let realm=0;realm<realms.length;realm++) {
  const w=realms[realm];
  class Reflected extends w.HTMLElement {static observedAttributes=['hidden'];attributeChangedCallback(...args){this.changes.push(args);}constructor(){super();this.changes=[];}}
  class Face extends w.HTMLElement {static formAssociated=true;constructor(){super();this.changes=[];this.internals=this.attachInternals();}formDisabledCallback(value){this.changes.push(value);}}
  w.customElements.define('ns-reflect-'+realm,Reflected);w.customElements.define('ns-face-'+realm,Face);
  const docs=[['live',w.document],['windowless',w.document.implementation.createHTMLDocument('')],['parsed',new w.DOMParser().parseFromString('<body></body>','text/html')]];
  for(const [kind,doc] of docs) {
   const run=(name,fn)=>{try{rows.push({label:realm+'/'+kind+'/'+name,checks:fn()});}catch(e){errors.push({label:realm+'/'+kind+'/'+name,error:String(e)});}};
   run('qualified-dom-methods',()=>{
    const e=doc.createElement('div');e.setAttributeNS(ns,'hidden','foreign');
    const present=e.hasAttribute('hidden')&&e.getAttribute('hidden')==='foreign';
    const result=e.toggleAttribute('hidden',false);
    return {qualifiedRead:present,toggleResult:result===false,qualifiedRemove:!e.hasAttributeNS(ns,'hidden')};
   });
   run('custom-element-reactions',()=>{
    const e=doc.adoptNode(new Reflected());e.setAttributeNS(ns,'hidden','foreign');e.changes=[];
    const attr=e.getAttributeNodeNS(ns,'hidden');e.hidden=true;e.hidden=false;
    return {reactions:JSON.stringify(e.changes)===JSON.stringify([['hidden',null,'',null],['hidden','',null,null]]),foreignIdentity:e.getAttributeNodeNS(ns,'hidden')===attr,foreignValue:attr.value==='foreign'};
   });
   run('custom-form-disabled-callback',()=>{
    const form=doc.body.appendChild(doc.createElement('form')),fieldset=form.appendChild(doc.createElement('fieldset'));
    try {
     const e=fieldset.appendChild(doc.adoptNode(new Face()));fieldset.setAttributeNS(ns,'disabled','');e.changes=[];
     fieldset.disabled=true;fieldset.disabled=false;fieldset.setAttributeNS(null,'disabled','');fieldset.removeAttributeNS(null,'disabled');
     return {callbacks:JSON.stringify(e.changes)==='[true,false,true,false]',foreignRetained:fieldset.hasAttributeNS(ns,'disabled')};
    } finally {form.remove();}
   });
   run('native-select-state',()=>{
    const e=doc.createElement('select'),clean=doc.createElement('select');e.setAttributeNS(ns,'multiple','');e.setAttributeNS(ns,'size','2');
    for(const control of [e,clean]) for(const value of ['one','two']) {const option=control.appendChild(doc.createElement('option'));option.value=value;}
    const snapshot=control=>JSON.stringify([control.value,control.selectedIndex,Array.from(control.options,o=>o.selected)]);
    const initial=snapshot(e)===snapshot(clean);e.options[1].selected=true;clean.options[1].selected=true;e.options[0].selected=true;clean.options[0].selected=true;
    return {defaultSelection:initial,singleSelection:snapshot(e)===snapshot(clean),reflectedMultiple:e.multiple===false,reflectedSize:e.size===0};
   });
  }
 }
 globalThis.__uiEventResults={rows,errors,cases:cases.length};
 return errors.length===0&&rows.every(r=>Object.values(r.checks).every(v=>v===true));
})()
