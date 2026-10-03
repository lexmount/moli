globalThis.__installDragDataModeProbe = function(mode, capture, modifiers) {
  document.body.innerHTML = '<style>body{margin:0;user-select:none}#source{position:absolute;left:40px;top:40px;width:140px;height:140px}#a{position:absolute;left:300px;top:40px;width:140px;height:140px}#b{position:absolute;left:500px;top:40px;width:140px;height:140px}</style><div id="source" draggable="true">source</div><div id="a">A</div><div id="b">B</div>';
  const source = document.getElementById('source');
  const file = new File(['private-file-contents'], 'private.txt', {type:'text/plain'});
  const rows = [], transfers = [], callbacks = [], microtasks = [], trace = [];
  const expectedTypes = ['text/plain', 'text/html', 'application/test', 'Files'];
  const checks = [];
  const check = (name, actual, expected) => checks.push({name, actual, expected, pass:JSON.stringify(actual)===JSON.stringify(expected)});
  const exceptionName = action => { try { action(); return 'none'; } catch (error) { return error.name; } };
  let starts = 0, drops = 0;
  function retain(dt, event) {
    const row = {dt, event, items:dt.items, files:dt.files, types:dt.types, string:dt.items[0], file:dt.items[3]};
    transfers.push(row);
    return row;
  }
  function disabled(row, label) {
    const prefix = label + ':' + row.event + ':' + transfers.indexOf(row);
    const dt = row.dt;
    check(prefix+':empty getData', dt.getData('text/plain'), '');
    check(prefix+':empty types', Array.from(dt.types), []);
    check(prefix+':new frozen types snapshot', dt.types !== row.types && Object.isFrozen(dt.types), true);
    check(prefix+':saved types unchanged', Array.from(row.types), expectedTypes);
    check(prefix+':items same object', dt.items === row.items, true);
    check(prefix+':items empty', row.items.length, 0);
    check(prefix+':indexed item absent', row.items[0] === undefined, true);
    check(prefix+':files same object', dt.files === row.files, true);
    check(prefix+':saved files empty', row.files.length, 0);
    if (row.string) {
      check(prefix+':saved string metadata disabled', [row.string.kind,row.string.type], ['','']);
      row.string.getAsString(value => callbacks.push({phase:'disabled',value}));
    }
    if (row.file) {
      check(prefix+':saved file metadata disabled', [row.file.kind,row.file.type], ['','']);
      check(prefix+':saved file unavailable', row.file.getAsFile(), null);
      check(prefix+':saved entry unavailable', row.file.webkitGetAsEntry(), null);
    }
    check(prefix+':add blocked', row.items.add('no','application/blocked'), null);
    check(prefix+':remove disabled throws', exceptionName(()=>row.items.remove(999)), 'InvalidStateError');
    const allowed = dt.effectAllowed;
    dt.effectAllowed = 'none';
    check(prefix+':effectAllowed immutable', dt.effectAllowed, allowed);
    dt.setData('text/plain','escaped');
    dt.clearData();
    row.items.clear();
    check(prefix+':store cannot be repopulated', [dt.getData('text/plain'),row.items.length,dt.types.length], ['',0,0]);
  }
  for (const type of ['dragstart','drag','dragenter','dragover','dragleave','drop','dragend']) {
    document.addEventListener(type, event => {
      const dt = event.dataTransfer;
      const prefix = type+':'+rows.length;
      check(prefix+':fresh event DataTransfer', transfers.every(row=>row.dt!==dt), true);
      for (const row of transfers) disabled(row, prefix+':previous');
      if (type==='dragstart') {
        starts++;
        dt.setData('text/plain','payload');
        dt.setData('text/html','<b>payload</b>');
        dt.items.add('extra','application/test');
        const added = dt.items.add(file);
        check(prefix+':added file is indexed view', added===dt.items[3], true);
        dt.effectAllowed='all';
      }
      const row = retain(dt,type);
      check(prefix+':same items object', dt.items===dt.items, true);
      check(prefix+':same files object', dt.files===dt.files, true);
      check(prefix+':stable frozen types', dt.types===row.types && Object.isFrozen(row.types), true);
      check(prefix+':metadata enumerable', Array.from(dt.types), expectedTypes);
      check(prefix+':indexed metadata', Array.from(dt.items, item=>[item.kind,item.type]), [['string','text/plain'],['string','text/html'],['string','application/test'],['file','text/plain']]);
      const readable = type==='dragstart' || type==='drop';
      check(prefix+':payload read mode', dt.getData('text/plain'), readable?'payload':'');
      check(prefix+':file list read mode', dt.files.length, readable?1:0);
      if (row.file) {
        check(prefix+':file item read mode', readable?row.file.getAsFile()===file:row.file.getAsFile()===null, true);
        check(prefix+':entry read mode', !!row.file.webkitGetAsEntry(), readable);
      }
      if (row.string) {
        row.string.getAsString(value=>callbacks.push({phase:type,value}));
        check(prefix+':callback conversion before mode', exceptionName(()=>row.string.getAsString({})), 'TypeError');
      }
      let conversions=0;
      check(prefix+':format conversion still runs', dt.getData({toString(){conversions++;return 'text/plain'}}), readable?'payload':'');
      check(prefix+':format conversion count', conversions, 1);
      const marker={};
      let caught;
      try { dt.getData({toString(){throw marker;}}); } catch (error) {caught=error;}
      check(prefix+':format conversion exception propagates', caught===marker, true);
      Promise.resolve().then(()=>microtasks.push({phase:type,data:dt.getData('text/plain'),items:dt.items.length}));
      if (type!=='dragstart') {
        const before=Array.from(dt.types), allowed=dt.effectAllowed;
        dt.setData('text/plain','tampered');
        dt.clearData('text/html');
        check(prefix+':string add blocked', dt.items.add('no','application/blocked'), null);
        check(prefix+':file add blocked', dt.items.add(file), null);
        check(prefix+':remove readonly/protected throws', exceptionName(()=>dt.items.remove(999)), 'InvalidStateError');
        dt.items.clear();
        dt.effectAllowed='none';
        check(prefix+':readonly/protected mutations ignored', Array.from(dt.types), before);
        check(prefix+':effectAllowed cannot change', dt.effectAllowed, allowed);
        check(prefix+':data remains unmodified', dt.getData('text/plain'), readable?'payload':'');
      }
      if (type==='dragenter' || type==='dragover') {
        dt.dropEffect='copy';
        check(prefix+':dropEffect writable', dt.dropEffect, 'copy');
        if (mode==='drop') event.preventDefault();
      }
      if (type==='drop') {drops++;event.preventDefault();}
      if (type==='dragstart' && mode==='cancel') event.preventDefault();
      rows.push({entryAvailable:!!row.file?.webkitGetAsEntry(),type,data:dt.getData('text/plain'),items:dt.items.length,files:dt.files.length,types:Array.from(dt.types),dropEffect:dt.dropEffect,effectAllowed:dt.effectAllowed});
      trace.push(type);
    });
  }
  if (capture) source.addEventListener('pointerdown', event=>source.setPointerCapture(event.pointerId));
  return {
    afterDispatch(label) {for(const row of transfers) disabled(row,label);},
    finish() {
      for(const row of transfers) disabled(row,'finish');
      check('one native dragstart', starts, 1);
      check('drop iff accepted', drops, mode==='drop'?1:0);
      check('only readable events schedule string callbacks', callbacks.filter(row=>!['dragstart','drop'].includes(row.phase)), []);
      check('captured callback payload survives disassociation', callbacks.filter(row=>['dragstart','drop'].includes(row.phase)).map(row=>row.value), mode==='drop'?['payload','payload']:['payload']);
      return {mode,capture,modifiers,checks,rows,callbacks,microtasks,trace,passed:checks.filter(c=>c.pass).length,total:checks.length,complete:checks.every(c=>c.pass)};
    }
  };
};
