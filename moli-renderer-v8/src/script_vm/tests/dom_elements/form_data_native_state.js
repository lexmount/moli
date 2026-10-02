(() => {
  const rows=[],errors=[];
  const realms=[window,document.getElementById('child').contentWindow];
  const inspect=fd=>Array.from(fd,([name,value])=>[name,typeof value==='string'?value:['file',value.name,value.size,value.type]]);
  for(let realm=0;realm<realms.length;realm++) {
    const w=realms[realm],FD=w.FormData,NativeFile=w.File;
    class Face extends w.HTMLElement {static formAssociated=true;constructor(){super();this.internals=this.attachInternals();}}
    w.customElements.define('fd-state-'+realm,Face);
    const docs=[['live',w.document],['windowless',w.document.implementation.createHTMLDocument('')],['parsed',new w.DOMParser().parseFromString('<body></body>','text/html')]];
    for(const [kind,doc] of docs) {
      const label=realm+'/'+kind;
      const run=(suffix,prepare,poison)=>{
        const form=doc.body.appendChild(doc.createElement('form'));
        let restore=()=>{};
        try {
          const state=prepare(form),expected=state.expected;let reads=0,events=0;
          form.addEventListener('formdata',()=>events++);
          const fail=()=>{reads++;throw new Error('author getter');};
          if(poison) restore=poison(state,fail)??restore;
          let value,error=null;
          try {value=inspect(new FD(form,state.submitter));} catch(e) {error=String(e);}
          rows.push({label:label+'/'+suffix,checks:{nativeEntries:error===null&&JSON.stringify(value)===JSON.stringify(expected),noGetterReads:reads===0,formdataEvent:events===1},error,value,expected});
        } catch(e) {errors.push({label:label+'/'+suffix,error:String(e)});}
        finally {restore();form.remove();}
      };
      const input=(form,type='text',checked=false)=>{
        const control=form.appendChild(doc.createElement('input'));control.type=type;control.name='field';control.value='value';control.checked=checked;
        return {control,expected:type==='checkbox'&&!checked?[]:[['field','value']]};
      };
      const override=(object,key,fail)=>{
        const old=Object.getOwnPropertyDescriptor(object,key);
        Object.defineProperty(object,key,{configurable:true,get:fail});
        return ()=>old?Object.defineProperty(object,key,old):delete object[key];
      };
      for(const property of ['tagName','type','name','value','checked','disabled','dirName','dir','parentElement']) {
        const prepare=property==='checked'?form=>input(form,'checkbox',true):form=>{
          const state=input(form);state.control.dirName='direction';state.expected.push(['direction','ltr']);return state;
        };
        for(const location of ['own','prototype']) run('input/'+property+'/'+location,prepare,(state,fail)=>override(location==='own'?state.control:w.HTMLInputElement.prototype,property,fail));
      }
      const select=form=>{
        const control=form.appendChild(doc.createElement('select'));control.name='choice';control.multiple=true;
        const first=control.appendChild(doc.createElement('option'));first.value='first';first.selected=true;
        const group=control.appendChild(doc.createElement('optgroup'));const second=group.appendChild(doc.createElement('option'));second.value='second';second.selected=true;
        return {control,first,second,group,expected:[['choice','first'],['choice','second']]};
      };
      for(const property of ['tagName','type','multiple','options','selectedOptions','selectedIndex','disabled']) run('select/'+property,select,(state,fail)=>override(state.control,property,fail));
      for(const property of ['selected','disabled','value','parentElement','namespaceURI','localName']) run('option/'+property,select,(state,fail)=>override(state.second,property,fail));
      for(const property of ['disabled','tagName','parentElement']) run('optgroup/'+property,select,(state,fail)=>override(state.group,property,fail));
      for(const mode of ['index-none','value-none','listbox-none','selected-disabled','group-disabled','multiple-none']) run('selection/'+mode,form=>{
        const state=select(form);state.control.multiple=mode==='multiple-none';
        if(mode==='listbox-none') state.control.size=2;
        state.first.selected=false;state.second.selected=false;
        if(mode==='index-none') state.control.selectedIndex=-1;
        else if(mode==='value-none') state.control.value='missing';
        else if(mode==='listbox-none') state.control.size=2;
        else if(mode==='selected-disabled') {state.second.selected=true;state.second.disabled=true;}
        else if(mode==='group-disabled') {state.second.selected=true;state.group.disabled=true;}
        state.expected=[];return state;
      });
      for(const owner of ['input','fieldset','option','optgroup']) run('foreign-disabled/'+owner,form=>{
        if(owner==='input') {
          const state=input(form);state.control.setAttributeNS('urn:foreign','disabled','');return state;
        }
        if(owner==='fieldset') {
          const fieldset=form.appendChild(doc.createElement('fieldset'));
          fieldset.setAttributeNS('urn:foreign','disabled','');return input(fieldset);
        }
        const state=select(form);state[owner==='option'?'second':'group'].setAttributeNS('urn:foreign','disabled','');return state;
      });
      for(const mode of ['ancestor-rtl','ancestor-auto','control-auto','tel-rtl','foreign-dir']) run('direction/'+mode,form=>{
        const parent=form.appendChild(doc.createElement('div'));parent.dir=mode==='ancestor-auto'?'auto':'rtl';parent.appendChild(doc.createTextNode('\u05d0'));
        const state=input(parent,mode==='tel-rtl'?'tel':'text');state.control.value='\u05d0';state.control.dirName='direction';
        if(mode==='control-auto') state.control.dir='auto';
        if(mode==='foreign-dir') state.control.setAttributeNS('urn:foreign','dir','ltr');
        state.expected=[['field','\u05d0'],['direction',mode==='tel-rtl'?'ltr':'rtl']];return state;
      });
      for(const type of ['hidden','submit','checkbox']) run('dirname-applicability/'+type,form=>{
        const state=input(form,type,true);state.control.dirName='direction';
        if(type==='submit') state.submitter=state.control;
        state.expected=[['field','value']];if(type!=='checkbox') state.expected.push(['direction','ltr']);return state;
      });
      for(const property of ['value','dirName','dir','parentElement','tagName']) run('textarea/'+property,form=>{
        const control=form.appendChild(doc.createElement('textarea'));control.name='text';control.value='\ud800x\nz';control.dirName='direction';
        return {control,expected:[['text','\ufffdx\nz'],['direction','ltr']]};
      },(state,fail)=>override(state.control,property,fail));
      for(const mode of ['plain','files-getter','global-file','bag-lastModified','bag-endings']) run('empty-file/'+mode,form=>{
        const control=form.appendChild(doc.createElement('input'));control.type='file';control.name='upload';
        return {control,expected:[['upload',['file','',0,'application/octet-stream']]]};
      },(state,fail)=>mode==='files-getter'?override(state.control,'files',fail):mode==='global-file'?override(w,'File',fail):mode.startsWith('bag-')?override(w.Object.prototype,mode.slice(4),fail):undefined);
      for(const mode of ['string','formdata','no-name-formdata','file','null']) run('custom/'+mode,form=>{
        const control=doc.adoptNode(new Face());form.append(control);control.setAttribute('name','custom');
        let value='private',expected=[['custom','private']];
        if(mode.includes('formdata')) {value=new FD();value.append('one','first');value.append('one','second');expected=[['one','first'],['one','second']];}
        if(mode==='no-name-formdata') control.removeAttribute('name');
        if(mode==='file') {value=new NativeFile(['one'],'picked',{type:'text/plain',lastModified:7});expected=[['custom',['file','picked',3,'text/plain']]];}
        if(mode==='null') {value=null;expected=[];}
        control.internals.setFormValue(value);return {control,expected};
      },(state,fail)=>override(state.control,'tagName',fail));
      for(const [name,submitter,throws] of [['undefined',undefined,false],['null',null,false],['div',doc.createElement('div'),false],['button',doc.createElement('button'),false],['object',{},true],['number',1,true],['proxy',new Proxy(doc.createElement('button'),{}),true],['forged',Object.create(w.HTMLElement.prototype),true]]) {
        let result,error=null;try {result=new FD(undefined,submitter);}catch(e){error=e;}
        rows.push({label:label+'/omitted-form/'+name,checks:{conversion:throws?error instanceof w.TypeError:error===null&&Array.from(result).length===0,calleeRealm:!error||error instanceof w.TypeError}});
      }
    }
  }
  globalThis.__uiEventResults={rows,errors};
  return errors.length===0&&rows.every(row=>Object.values(row.checks).every(value=>value===true));
})()
