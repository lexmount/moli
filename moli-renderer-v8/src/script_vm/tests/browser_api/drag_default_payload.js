globalThis.__installDragDefaultPayloadProbe = function(mode, inChild) {
  const root = globalThis;
  root.document.body.innerHTML = '<style>body{margin:0}#frame{position:absolute;left:0;top:0;width:240px;height:240px;border:0}#destination{position:absolute;left:300px;top:40px;width:140px;height:140px}</style><div id="destination"></div>';
  let win = root;
  if (inChild) {
    const frame = root.document.createElement('iframe'); frame.id = 'frame'; root.document.body.append(frame);
    win = frame.contentWindow;
  }
  const doc = win.document, nativeURL = root.URL;
  doc.head.innerHTML = '';
  if (mode === 'link-base' || inChild) {
    const base = doc.createElement('base'); base.href = new nativeURL('/owner-base/', root.document.baseURI).href; doc.head.append(base);
  }
  const style = doc.createElement('style');
  style.textContent = 'body{margin:0;user-select:none}#source{display:block;position:absolute;left:40px;top:40px;width:140px;height:140px}'; doc.head.append(style);
  if (inChild) doc.body.innerHTML = '';
  let source;
  const image = mode.startsWith('image');
  let rawURL = image ? '/asset.png?source=original' : 'destination?q=hello#frag';
  if (image) { source = doc.createElement('img'); source.width = 140; source.height = 140; }
  else if (mode === 'plain' || mode === 'container') { source = doc.createElement('div'); source.draggable = true; source.innerHTML = mode === 'container' ? '<a href="nested">Nested link</a>' : 'Plain'; }
  else { source = doc.createElement('a'); source.innerHTML = '<b>Drag &amp; me</b>'; }
  const attribute = image ? 'src' : 'href';
  if (mode === 'link-absolute') rawURL = 'https://url-payload.test/a%20b?x=1#fragment';
  if (mode === 'link-empty') rawURL = '';
  if (mode === 'link-fragment') rawURL = '#fragment';
  if (mode === 'link-unicode') rawURL = '雪?q=☃#😀';
  if (mode === 'link-invalid') rawURL = 'http://[';
  if (!['plain','container','link-nohref'].includes(mode)) source.setAttribute(attribute, rawURL);
  if (mode === 'link-nohref') source.draggable = true;
  if (mode === 'image-srcset') source.srcset = '/asset.png?source=selected 1x';
  source.id = 'source'; doc.body.append(source);
  if (mode === 'image-linked') {
    const link = doc.createElement('a'); link.href = 'outer-link'; source.replaceWith(link); link.append(source);
  }
  let expected = '';
  if (!['plain','container','link-nohref'].includes(mode)) {
    try { expected = new nativeURL(rawURL, doc.baseURI).href; } catch {}
  }
  if (mode === 'link-down-mutation') {
    const changed = 'changed-on-down?q=new#updated'; expected = new nativeURL(changed, doc.baseURI).href;
    source.addEventListener('mousedown', () => source.setAttribute('href', changed));
  }
  const initial = expected;
  let reads = 0;
  if (mode.endsWith('-poison')) {
    for (const key of [attribute, 'outerHTML', 'ownerDocument', 'baseURI', 'currentSrc']) {
      Object.defineProperty(source, key, {configurable:true, get(){reads++; throw new Error('author getter '+key);}});
    }
    Object.defineProperty(win, 'URL', {configurable:true, get(){reads++; throw new Error('public URL');}});
  }
  const rows = [], checks = [], transfers = [], callbacks = [];
  const check = (name, actual, wanted) => checks.push({name, actual, expected:wanted, pass:JSON.stringify(actual)===JSON.stringify(wanted)});
  const override = 'https://override.test/kept?value=1#drop';
  const expectedDrop = mode.endsWith('-override') ? override : mode.endsWith('-clear') ? '' : initial;
  const expectedPlain = image ? '' : expectedDrop;
  let started = 0, dropped = 0;
  const observe = event => {
    const dt = event.dataTransfer;
    const row = {phase:root.__dragPhase, type:event.type, target:event.target.id, uri:dt.getData('text/uri-list'), url:dt.getData('url'), plain:dt.getData('text/plain'), html:dt.getData('text/html'), types:Array.from(dt.types), items:Array.from(dt.items,item=>[item.kind,item.type]), files:Array.from(dt.files,file=>[file.name,file.type,file.size]), targetRealm:event instanceof event.currentTarget.defaultView.DragEvent};
    rows.push(row); transfers.push(dt);
    if (event.type === 'dragstart') {
      started++;
      check('default URL before dragstart handler', row.uri, initial);
      check('legacy URL alias before dragstart handler', row.url, initial);
      if (!image) check('link plain text default', row.plain, initial);
      check('URI type metadata', row.types.includes('text/uri-list'), initial !== '');
      if (initial) {
        check('URI item kind', row.items.find(item=>item[1]==='text/uri-list'), ['string','text/uri-list']);
        const item = Array.from(dt.items).find(item=>item.type==='text/uri-list'); item.getAsString(value=>callbacks.push(value));
      }
      if (mode.endsWith('-start-mutation')) source.setAttribute(attribute, 'changed-in-handler');
      if (mode.endsWith('-override')) {
        dt.setData('text/uri-list', override); if (!image) dt.setData('text/plain', override);
      }
      if (mode.endsWith('-clear')) dt.clearData();
      dt.effectAllowed = 'copy';
    } else if (event.type === 'drop') {
      dropped++;
      check('drop receives the stored URL', row.uri, expectedDrop);
      check('legacy alias receives the stored URL', row.url, expectedDrop);
      if (!image) check('drop receives stored plain text', row.plain, expectedPlain);
      dt.setData('text/uri-list','tampered'); dt.clearData();
      check('drop store remains readonly', dt.getData('text/uri-list'), expectedDrop);
      event.preventDefault();
    } else {
      check('protected URL payload '+event.type, row.uri, '');
      check('protected legacy alias '+event.type, row.url, '');
    }
  };
  for (const document of new Set([root.document, doc])) {
    for (const type of ['dragstart','drag','dragenter','dragover','drop','dragend']) document.addEventListener(type, observe);
    document.addEventListener('click',event=>event.preventDefault());
    for (const type of ['dragenter','dragover']) document.addEventListener(type,event=>{event.dataTransfer.dropEffect='copy';event.preventDefault();});
  }
  return {finish(){
    check('one native dragstart', started, 1); check('one native drop', dropped, 1); check('author getter reads',reads,0);
    check('all target realms', rows.every(row=>row.targetRealm),true);
    check('callback snapshot',callbacks,initial?[initial]:[]);
    return {mode,inChild,initial,expectedDrop,passed:checks.filter(row=>row.pass).length,total:checks.length,complete:checks.every(row=>row.pass),checks,rows,callbacks,retired:transfers.map(dt=>[dt.types.length,dt.getData('text/uri-list')])};
  }};
};
