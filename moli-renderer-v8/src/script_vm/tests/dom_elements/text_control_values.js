(() => {
  const globals=[window,document.getElementById('child').contentWindow];
  const rows=[],errors=[];
  const cases=['text','search','tel','url','password','email','checkbox','radio','textarea'];
  const values=['prefix\uD800suffix','prefix\uDC01suffix','pair\uD83D\uDE00',
    'replacement\uFFFD','nul\0\r\nx\ry\n',' \t\uD800@example.test\r\n ',
    ' \uD800@example.test , \uDC01@example.test '];
  const normalize=value=>value.replace(/\r\n?/g,'\n');
  const trim=value=>value.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g,'');
  const sanitize=(type,value,multiple=false)=>{
    if(type==='textarea')return normalize(value);
    if(type==='checkbox' || type==='radio')return value;
    const stripped=value.replace(/[\r\n]/g,'');
    if(type==='url' || type==='email')return multiple?
      stripped.split(',').map(trim).join(','):trim(stripped);
    return stripped;
  };
  const descriptor=(g,node,key)=>{
    for(let object=node;object;object=g.Object.getPrototypeOf(object)){
      const desc=g.Object.getOwnPropertyDescriptor(object,key);
      if(desc)return desc;
    }
  };
  const add=(label,checks)=>rows.push({label,checks});
  for(let realm=0;realm<globals.length;realm++) {
    const g=globals[realm],other=globals[1-realm];
    for(const documentKind of ['live','windowless','parsed']) {
      const d=documentKind==='live'?g.document:documentKind==='windowless'?
        g.document.implementation.createHTMLDocument(''):
        new g.DOMParser().parseFromString('<!doctype html><body>','text/html');
      for(const type of cases)for(let valueIndex=0;valueIndex<values.length;valueIndex++) {
        const tag=type==='textarea'?'textarea':'input';
        const form=d.body.appendChild(d.createElement('form'));
        const node=form.appendChild(d.createElement(tag)),value=values[valueIndex];
        const label=[realm,documentKind,type,valueIndex].join('/');
        try {
          if(tag==='input')node.type=type;
          const checkable=type==='checkbox' || type==='radio';
          const selectable=type==='textarea' || ['text','search','tel','url','password'].includes(type);
          const desc=descriptor(other,other.document.createElement(tag),'value');
          const expected=sanitize(type,value);
          const checks={accessor:typeof desc?.get==='function' && typeof desc?.set==='function'};
          node.defaultValue=value;
          checks.defaultValue=node.defaultValue===value;
          checks.cleanValue=node.value===expected;
          let conversions=0;
          desc.set.call(node,{toString(){conversions++;return value;}});
          checks.convertOnce=conversions===1;
          checks.value=node.value===expected;
          const clone=node.cloneNode(true);
          checks.clone=clone.value===expected && clone.defaultValue===value;
          const imported=other.document.importNode(node,true);
          checks.import=imported.value===expected && imported.defaultValue===value;
          const adopted=other.document.adoptNode(clone);
          checks.adopt=adopted===clone && adopted.value===expected && adopted.defaultValue===value;
          if(selectable){
            node.setSelectionRange(0,node.value.length);
            checks.selection=node.selectionStart===0 && node.selectionEnd===expected.length;
            node.setRangeText(value,0,node.value.length,'end');
            checks.rangeText=node.value===expected;
            if(type==='textarea')checks.textLength=node.textLength===expected.length;
          }
          const sentinel={};let thrown;
          try {desc.set.call(node,{toString(){throw sentinel;}});}catch(error){thrown=error;}
          checks.exception=thrown===sentinel && node.value===expected;
          let symbolError;
          try {desc.set.call(node,g.Symbol('value'));}catch(error){symbolError=error;}
          checks.symbol=symbolError instanceof other.TypeError && node.value===expected;
          const revoked=g.Proxy.revocable(node,{});revoked.revoke();
          let receiverConversions=0,traps=0;
          const authorProxy=new g.Proxy(node,{get(){traps++;},set(){traps++;}});
          checks.receiver=true;
          for(const invalid of [{},g.Object.create(node),authorProxy,revoked.proxy,d.createTextNode('')]) {
            let error;
            try {desc.set.call(invalid,{toString(){receiverConversions++;return value;}});}catch(e){error=e;}
            checks.receiver &&= error instanceof other.TypeError;
          }
          checks.receiverOrder=receiverConversions===0 && traps===0;
          node.defaultValue='next-\uDC00';
          checks.dirtyDefault=node.value===(checkable?'next-\uDC00':expected);
          checks.dirtyClone=node.cloneNode(true).value===(checkable?'next-\uDC00':expected);
          checks.dirtyImport=other.document.importNode(node,true).value===(checkable?'next-\uDC00':expected);
          if(documentKind==='live') {
            form.reset();
            checks.reset=node.value==='next-\uDC00';
            node.defaultValue='reset-\uD800';
            checks.resetRestoresClean=node.value==='reset-\uD800';
          }
          node.value=null;
          checks.nullValue=node.value==='';
          node.value=undefined;
          checks.undefinedValue=node.value==='undefined';
          node.value=value;
          node.value='plain';
          checks.replacement=node.value==='plain';
          add(label,checks);
        }catch(error){errors.push({label,message:String(error)});}
        form.remove();
      }
      for(const type of ['text','textarea']) {
        const node=d.body.appendChild(d.createElement(type==='text'?'input':'textarea'));
        const label=[realm,documentKind,type,'unit-edit'].join('/');
        try {
          node.value='\uD83D\uDE00';
          node.setRangeText('x',1,2,'end');
          const checks={split:node.value==='\uD83Dx' && node.selectionStart===2};
          node.setRangeText('\uDE00',1,2,'end');
          checks.join=node.value==='\uD83D\uDE00';
          node.value='\uD800';node.setSelectionRange(0,0);node.value='\uDC00';
          checks.identity=node.value==='\uDC00' && node.selectionStart===1 && node.selectionEnd===1;
          if(documentKind==='live') {
            node.focus();node.select();
            checks.documentSelection=String(g.getSelection())==='\uDC00';
          }
          if(type==='textarea') {
            node.value='base';node.defaultValue='base';node.defaultValue='new-\uD800';
            checks.dirtyEquality=node.value==='base' && node.defaultValue==='new-\uD800';
            const clean=d.createElement('textarea');
            clean.setAttribute('value','ignored-\uD800');
            checks.contentAttributeIgnored=clean.value==='' && clean.defaultValue==='';
            clean.appendChild(d.createTextNode('\uD83D'));
            clean.appendChild(d.createTextNode('\uDE00\r\n\uD800'));
            checks.childText=clean.defaultValue==='\uD83D\uDE00\r\n\uD800' &&
              clean.value==='\uD83D\uDE00\n\uD800' && clean.textLength===4;
            const nested=clean.appendChild(d.createElement('span'));nested.textContent='ignored';
            checks.directChildText=clean.defaultValue==='\uD83D\uDE00\r\n\uD800';
            clean.firstChild.data='\uDC00';
            checks.cleanMutation=clean.value==='\uDC00\uDE00\n\uD800';
          }
          add(label,checks);
        }catch(error){errors.push({label,message:String(error)});}
        node.remove();
      }
      const email=d.createElement('input');email.type='email';email.multiple=true;
      email.value=' \uD800@example.test , \uDC01@example.test ';
      add([realm,documentKind,'multiple-email'].join('/'),{
        value:email.value==='\uD800@example.test,\uDC01@example.test',
        clone:email.cloneNode().value===email.value,
      });
      for(const type of ['number','date','month','time','week','datetime-local']) {
        const node=d.createElement('input');node.type=type;node.value='\uD800';
        add([realm,documentKind,type,'invalid'].join('/'),{value:node.value===''});
      }
    }
  }
  globalThis.__uiEventResults={rows,errors,cases:cases.length,values:values.length,
    passed:errors.length===0 && rows.every(row=>Object.values(row.checks).every(value=>value===true))};
  return __uiEventResults.passed;
})()
