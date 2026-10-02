(() => {
  const rows = [], errors = [];
  const types = ['hidden','text','search','tel','url','email','password','datetime-local','date','month','week','time','number','range','color','checkbox','radio','file','submit','image','reset','button'];
  const starts = ['text','url','hidden','checkbox','file','number','range','color'];
  const mode = type => type === 'file' ? 'filename' : ['checkbox','radio'].includes(type) ? 'default/on' : ['hidden','submit','image','reset','button'].includes(type) ? 'default' : 'value';
  const selectable = type => ['text','search','tel','url','password'].includes(type);
  const sanitize = (type, value) => {
    if (['text','search','tel','password'].includes(type)) return value.replace(/[\r\n]/g,'');
    if (['url','email'].includes(type)) return value.replace(/[\r\n]/g,'').replace(/^[\t\f ]+|[\t\f ]+$/g,'');
    if (type === 'range') return /^\d+$/.test(value) ? String(Math.min(100,Math.max(0,Number(value)))) : '50';
    if (type === 'color') return /^#[0-9a-f]{6}$/i.test(value) ? value.toLowerCase() : '#000000';
    if (type === 'number') return /^\d+$/.test(value) ? value : '';
    if (['date','month','week','time','datetime-local','file'].includes(type)) return '';
    return value;
  };
  const read = (type, attribute, internal) => mode(type) === 'value' ? internal : mode(type) === 'filename' ? '' : attribute === null ? (mode(type) === 'default/on' ? 'on' : '') : attribute;
  const unitArray = value => Array.from({length:value.length},(_,i)=>value.charCodeAt(i));
  const record = (label, fn) => {
    try { const row = fn(); rows.push({label,...row}); }
    catch (error) { errors.push({label,name:error.name,message:error.message}); }
  };
  const realms = [globalThis, document.getElementById('child').contentWindow];
  for (let realmIndex=0; realmIndex<realms.length; realmIndex++) {
    const realm=realms[realmIndex];
    for (const [kind,doc] of [
      ['live',realm.document],
      ['windowless',realm.document.implementation.createHTMLDocument('')],
      ['parsed',new realm.DOMParser().parseFromString('<body></body>','text/html')],
    ]) {
      record(realmIndex+'/'+kind+'/namespace',() => {
        const input=doc.createElement('input');
        input.setAttributeNS('urn:foreign','type','checkbox');
        input.setAttributeNS('urn:foreign','value','foreign');
        const checks={foreignType:input.type==='text',foreignDefault:input.defaultValue===''};
        input.type='hidden';input.value='native\ud800';
        checks.typeNamespace=input.getAttributeNS(null,'type')==='hidden' && input.getAttributeNS('urn:foreign','type')==='checkbox';
        checks.valueNamespace=input.value==='native\ud800' && input.defaultValue==='native\ud800' && input.getAttributeNS('urn:foreign','value')==='foreign';
        input.defaultValue='low\udc00';checks.defaultNamespace=input.value==='low\udc00' && input.getAttributeNS('urn:foreign','value')==='foreign';
        input.type='invalid\ud800';checks.typeDomString=input.type==='text' && input.getAttributeNS(null,'type')==='invalid\ud800';
        return {checks};
      });
      const directionProbe=doc.createElement('input'); directionProbe.setSelectionRange(0,0,'none'); const noneDirection=directionProbe.selectionDirection;
      for (const type of types) for (const payload of ['raw\ud800\r\n\u0000\udc00','41','#123456']) {
        record(realmIndex+'/'+kind+'/setter/'+type+'/'+unitArray(payload),() => {
          const input=doc.createElement('input');
          input.type=type;
          input.setAttributeNS(null,'value','seed');
          let error=null;
          try { input.value=payload; } catch (caught) { error=caught; }
          const expectedValue=mode(type)==='filename' ? '' : sanitize(type,payload);
          const expectedAttribute=['default','default/on'].includes(mode(type)) ? payload : 'seed';
          const checks={
            type:input.type===type,
            value:input.value===expectedValue,
            attribute:input.getAttributeNS(null,'value')===expectedAttribute,
            defaultValue:input.defaultValue===expectedAttribute,
            error:mode(type)==='filename' ? error?.name==='InvalidStateError' : error===null,
          };
          input.setAttributeNS(null,'value','follow');
          checks.attributeFollows=['default','default/on'].includes(mode(type)) ? input.value==='follow' : input.value===expectedValue;
          const clone=input.cloneNode(true);
          checks.cloneValue=clone.value===input.value;
          checks.cloneDefault=clone.defaultValue===input.defaultValue;
          return {checks};
        });
      }
      for (const from of starts) for (const to of types) {
        if (from===to) continue;
        for (const dirty of [false,true]) for (const api of ['property','attribute','namespace','attr-node']) {
          record(realmIndex+'/'+kind+'/transition/'+from+'/'+to+'/'+dirty+'/'+api,() => {
            const input=doc.createElement('input');
            input.type=from;
            let attribute='seed\ud800\r\n ';
            input.setAttributeNS(null,'value',attribute);
            let internal=sanitize(from,attribute);
            let dirtyFlag=false;
            if (dirty && mode(from)!=='filename') {
              const assigned=['number','range'].includes(from) ? '41' : from==='color' ? '#123456' : '\t dirty\ud801\r\n ';
              input.value=assigned;
              if (mode(from)==='value') { internal=sanitize(from,assigned); dirtyFlag=true; }
              else attribute=assigned;
            }
            const before=read(from,attribute,internal);
            if (selectable(from)) input.setSelectionRange(1,3,'backward');
            const selection=[input.selectionStart,input.selectionEnd,input.selectionDirection];
            const observer=new realm.MutationObserver(()=>{});
            observer.observe(input,{attributes:true,attributeOldValue:true});
            if (api==='property') input.type=to;
            else if (api==='attribute') input.setAttribute('type',to);
            else if (api==='namespace') input.setAttributeNS(null,'type',to);
            else input.getAttributeNode('type').value=to;
            const reflect=mode(from)==='value' && before!=='' && ['default','default/on'].includes(mode(to));
            if (reflect) attribute=before;
            if (mode(from)!=='value' && mode(to)==='value') { internal=attribute??'';dirtyFlag=false; }
            else if (mode(to)==='filename') internal='';
            if (mode(to)==='value') internal=sanitize(to,internal);
            const expected=read(to,attribute,internal);
            let expectedSelection=[null,null,null];
            if (selectable(to)) {
              if (!selectable(from)) expectedSelection=[0,0,noneDirection];
              else if (expected!==before) expectedSelection=[expected.length,expected.length,noneDirection];
              else expectedSelection=selection;
            }
            const records=observer.takeRecords();
            observer.disconnect();
            const checks={
              type:input.type===to,
              value:input.value===expected,
              attribute:input.getAttributeNS(null,'value')===attribute,
              defaultValue:input.defaultValue===attribute,
              selection:JSON.stringify([input.selectionStart,input.selectionEnd,input.selectionDirection])===JSON.stringify(expectedSelection),
              mutationOrder:JSON.stringify(records.map(r=>r.attributeName))===JSON.stringify(reflect?['type','value']:['type']),
            };
            input.setAttributeNS(null,'value','follow\udc00\r\n');
            const followed=['default','default/on'].includes(mode(to)) ? 'follow\udc00\r\n' : mode(to)==='filename' ? '' : dirtyFlag ? expected : sanitize(to,'follow\udc00\r\n');
            checks.dirtyState=input.value===followed;
            return {checks};
          });
        }
      }
    }
  }
  globalThis.__uiEventResults={rows,errors};
  return errors.length===0 && rows.every(row=>Object.values(row.checks).every(value=>value===true));
})()
