function addBlobHistoryPolicy(kind) {
  const meta = document.createElement('meta');
  meta.httpEquiv = 'Content-Security-Policy';
  meta.content = kind === 'img' ? "img-src 'none'" : "require-trusted-types-for 'script'";
  document.head.appendChild(meta);
}
async function checkBlobHistoryPolicy(kind, imageURL) {
  if (kind === 'tt') {
    try { document.createElement('div').innerHTML = ''; return 'allowed'; }
    catch (error) { return error.name + ':' + (error instanceof TypeError); }
  }
  return new Promise(resolve => {
    const image = new Image();
    image.onload = () => resolve('allowed');
    image.onerror = () => resolve('blocked');
    image.src = imageURL;
  });
}
async function blobHistoryPolicy(base, kind, mode, cross) {
  const foreign = new URL(base);
  foreign.hostname = foreign.hostname === 'localhost' ? '127.0.0.1' : 'localhost';
  const away = (cross ? foreign.origin : base) + '/csp-history-away';
  const imageURL = base + '/csp-history-image.png';
  let popup;
  const wait = predicate => new Promise(resolve => {
    function receive(event) {
      if ((!popup || event.source === popup) && predicate(event.data)) {
        removeEventListener('message', receive);
        resolve(event.data);
      }
    }
    addEventListener('message', receive);
  });
  const target = '<!doctype html><body><script>' +
    addBlobHistoryPolicy.toString() + '\n' + checkBlobHistoryPolicy.toString() + '\n' +
    'const kind=' + JSON.stringify(kind) + ', imageURL=' + JSON.stringify(imageURL) + ';' +
    `addEventListener('load', async () => opener.postMessage({result: await checkBlobHistoryPolicy(kind,imageURL)},'*'));
     addEventListener('message', async event => {
       if(event.data?.targetPolicy) { addBlobHistoryPolicy(kind); opener.postMessage({targetCheck:await checkBlobHistoryPolicy(kind,imageURL)},'*'); }
       if(event.data?.away) location.href=event.data.away;
     });
     addEventListener('beforeunload',()=>{});` + '</script>';
  if (mode === 'early') addBlobHistoryPolicy(kind);
  const url = URL.createObjectURL(new Blob([target], {type:'text/html'}));
  let ready = wait(data => data?.result);
  popup = open(url);
  const first = (await ready).result;
  const originalDocument = popup.document;
  let targetCheck = null;
  if (mode === 'target-only') {
    ready = wait(data => data?.targetCheck);
    popup.postMessage({targetPolicy:true}, '*');
    targetCheck = (await ready).targetCheck;
  }
  ready = wait(data => data?.awayReady);
  popup.postMessage({away}, '*');
  await ready;
  if (mode === 'late') addBlobHistoryPolicy(kind);
  const creatorCheck = await checkBlobHistoryPolicy(kind, imageURL);
  ready = wait(data => data?.result);
  popup.postMessage('back', '*');
  const second = (await ready).result;
  const recreated = popup.document !== originalDocument;
  popup.close();
  URL.revokeObjectURL(url);
  return {kind, mode, cross, first, second, targetCheck, creatorCheck, recreated};
}
