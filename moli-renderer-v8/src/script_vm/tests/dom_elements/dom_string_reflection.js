(() => {
  const globals=[window,document.getElementById('child').contentWindow];
  const rows=[],errors=[];
  const cases=[
    ['div','id','id'],['div','className','class'],['div','title','title'],
    ['div','lang','lang'],['div','accessKey','accesskey'],['div','slot','slot'],
    ['div','role','role','nullable'],['div','ariaLabel','aria-label','nullable'],
    ...['a','button','details','embed','fieldset','form','frame','iframe','img','input',
        'map','meta','object','output','param','select','slot','textarea'].map(tag=>[tag,'name','name']),
    ['input','accept','accept'],['input','placeholder','placeholder'],
    ['textarea','placeholder','placeholder'],['textarea','dirName','dirname'],
    ['form','acceptCharset','accept-charset'],['form','target','target'],['a','download','download'],
    ['a','target','target'],['area','target','target'],['base','target','target'],
    ['area','alt','alt'],['img','alt','alt'],['meta','content','content'],
    ['meta','httpEquiv','http-equiv'],['label','htmlFor','for'],
    ['track','srclang','srclang'],['data','value','value'],['param','value','value'],
    ['table','summary','summary'],['table','cellPadding','cellpadding','null-empty'],
    ['table','cellSpacing','cellspacing','null-empty'],['td','abbr','abbr'],
    ['link','media','media'],['link','hreflang','hreflang'],['link','type','type'],
    ['a','ping','ping','usv'],['area','ping','ping','usv'],
    ['img','srcset','srcset','usv'],['source','srcset','srcset','usv'],
    ['link','imageSrcset','imagesrcset','usv']
  ];
  const values=['prefix\uD800suffix','prefix\uDC01suffix','pair\uD800\uDC01',
    'replacement\uFFFD','nul\0\r\n'];
  const scalar=value=>value.toWellFormed();
  const descriptor=(g,node,key)=>{
    for(let object=node;object;object=g.Object.getPrototypeOf(object)){
      const desc=g.Object.getOwnPropertyDescriptor(object,key);
      if(desc)return desc;
    }
  };
  for(let realm=0;realm<globals.length;realm++) {
    const g=globals[realm],other=globals[1-realm];
    for(const documentKind of ['live','windowless','parsed']) {
      const d=documentKind==='live'?g.document:documentKind==='windowless'?
        g.document.implementation.createHTMLDocument(''):
        new g.DOMParser().parseFromString('<!doctype html><body>','text/html');
      for(const [tag,property,attribute,kind] of cases) for(let valueIndex=0;valueIndex<values.length;valueIndex++) {
        const node=d.createElement(tag),value=values[valueIndex];
        const label=[realm,documentKind,tag,property,valueIndex].join('/');
        const expected=kind==='usv'?scalar(value):value;
        const empty=kind==='nullable'?null:'';
        try {
          const desc=descriptor(g,node,property),borrowed=descriptor(other,other.document.createElement(tag),property);
          let conversions=0;
          const checks={accessor:typeof desc?.get==='function' && typeof desc?.set==='function',missing:node[property]===empty};
          borrowed.set.call(node,{toString(){conversions++;return value;}});
          checks.convertOnce=conversions===1;
          checks.setter=node[property]===expected;
          checks.attribute=node.getAttribute(attribute)===expected;
          checks.attrValue=node.getAttributeNode(attribute).value===expected;
          const clone=node.cloneNode(false);
          checks.clone=clone[property]===expected && clone.getAttribute(attribute)===expected;
          node.setAttribute(attribute,value);
          checks.externalAttribute=node[property]===expected && node.getAttribute(attribute)===value;
          node.getAttributeNode(attribute).value=value;
          checks.externalAttr=node[property]===expected;
          const sentinel={};let thrown;
          try {borrowed.set.call(node,{toString(){throw sentinel;}});}catch(error){thrown=error;}
          checks.exception=thrown===sentinel && node.getAttribute(attribute)===value;
          let symbolError;
          try {borrowed.set.call(node,g.Symbol('value'));}catch(error){symbolError=error;}
          checks.symbol=symbolError instanceof other.TypeError && node.getAttribute(attribute)===value;
          const revoked=g.Proxy.revocable(node,{});revoked.revoke();
          let receiverConversions=0,traps=0;
          const authorProxy=new g.Proxy(node,{get(){traps++;},set(){traps++;}});
          checks.receiver=true;
          for(const invalid of [{},g.Object.create(node),authorProxy,revoked.proxy,d.createTextNode('')]) {
            let error;
            try {borrowed.set.call(invalid,{toString(){receiverConversions++;return value;}});}catch(e){error=e;}
            checks.receiver &&= error instanceof other.TypeError;
          }
          checks.receiverOrder=receiverConversions===0 && traps===0;
          if(kind==='usv' && ['img','source','link'].includes(tag)) {
            const wrong=d.createElement('div');let getterError,setterError;
            try {borrowed.get.call(wrong);}catch(e){getterError=e;}
            try {borrowed.set.call(wrong,{toString(){receiverConversions++;return value;}});}catch(e){setterError=e;}
            checks.interfaceReceiver=getterError instanceof other.TypeError &&
              setterError instanceof other.TypeError && receiverConversions===0 && wrong.attributes.length===0;
          }
          node[property]=null;
          const nullExpected=kind==='nullable'?null:kind==='null-empty'?'':'null';
          checks.nullValue=node[property]===nullExpected && node.getAttribute(attribute)===nullExpected;
          node[property]=undefined;
          const undefinedExpected=kind==='nullable'?null:'undefined';
          checks.undefinedValue=node[property]===undefinedExpected && node.getAttribute(attribute)===undefinedExpected;
          node[property]=value;
          node[property]='plain';
          checks.replacement=node[property]==='plain' && node.getAttribute(attribute)==='plain';
          node.removeAttribute(attribute);
          checks.removal=node[property]===empty && node.getAttribute(attribute)===null;
          rows.push({label,kind:kind||'dom',checks});
        }catch(error){errors.push({label,message:String(error)});}
      }
    }
  }
  globalThis.__uiEventResults={rows,errors,cases:cases.length,values:values.length,
    passed:errors.length===0 && rows.every(row=>Object.values(row.checks).every(value=>value===true))};
  return __uiEventResults.passed;
})()
