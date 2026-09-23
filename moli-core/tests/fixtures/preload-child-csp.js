(async () => {
  const observations = [], failures = [];
  const counter = globalThis.preloadCspCounterPath || '/preload/resources/preload-count.py';
  const count = async () => Number((await (await fetch(counter+'?action=result')).text()).match(/\d+/)[0]);
  const escape = s => s.replaceAll('&','&amp;').replaceAll('"','&quot;');
  if (globalThis.preloadCspBlockParent) {
    const meta = document.createElement('meta');
    meta.httpEquiv = 'Content-Security-Policy';
    meta.content = "default-src 'none'; frame-src 'self'; connect-src 'self'; script-src 'unsafe-inline'";
    document.head.append(meta);
  }
  const cases = ['fetch','script','style','image','font','track'].flatMap(as =>
    ['blocked','allowed','self'].map(mode => ({as,mode})));
  for (const as of ['script','style'])
    for (const mode of ['nonce-allowed','nonce-blocked']) cases.push({as,mode});
  cases.push({as:'fetch',mode:'ignored-media'}, {as:'style',mode:'ignored-type'},
    {as:'video',mode:'ignored-as'});
  for (const {as,mode} of cases) {
    await count();
    const ignored = mode.startsWith('ignored-');
    const source = mode === 'blocked' || ignored ? "'none'" : mode === 'allowed' ? '*' : "'self'";
    const nonceCase = mode.startsWith('nonce-');
    const allowed = !ignored && mode !== 'blocked' && mode !== 'nonce-blocked';
    const policy = nonceCase
      ? "default-src 'none'; script-src 'nonce-reporter' 'nonce-resource'; style-src 'nonce-resource'"
      : `default-src ${source}; script-src ${source} 'nonce-reporter'`;
    const url = new URL(counter+'?action=load&case='+as+'-'+mode,location.href).href;
    const frame=document.createElement('iframe');
    const done=new Promise(resolve=>{ globalThis.__preloadCspDone=resolve; });
    const loaded=new Promise(resolve=>{ frame.onload=resolve; });
    globalThis.__preloadCspViolations=[];
    const markup='<!doctype html><meta http-equiv="Content-Security-Policy" content="'+escape(policy)+'">'+
      '<script nonce="reporter">'+
      'document.addEventListener("securitypolicyviolation",e=>parent.__preloadCspViolations.push(e.effectiveDirective));'+
      'document.addEventListener("load",e=>{if(e.target.localName==="link")parent.__preloadCspDone({type:e.type,ownRealm:e instanceof Event})},true);'+
      'document.addEventListener("error",e=>{if(e.target.localName==="link")parent.__preloadCspDone({type:e.type,ownRealm:e instanceof Event})},true);'+
      '<'+'/script><link rel="preload" as="'+as+'" href="'+escape(url)+'"'+
      (nonceCase ? ' nonce="'+(allowed ? 'resource' : 'wrong')+'"' : '')+
      (mode==='ignored-media' ? ' media="not all"' : '')+
      (mode==='ignored-type' ? ' type="application/x-unknown"' : '')+'>';
    if (globalThis.preloadCspDocumentPath) {
      const childUrl=new URL(preloadCspDocumentPath,location.href);
      childUrl.searchParams.set(globalThis.preloadCspMarkupParam || 'markup',markup);
      frame.src=childUrl.href;
    } else frame.srcdoc=markup;
    document.body.append(frame);
    const terminal=await Promise.race([
      done, new Promise(resolve=>setTimeout(()=>resolve('timeout'),2500)),
      ...(ignored ? [loaded.then(()=>new Promise(resolve=>setTimeout(()=>resolve('none'),100)))] : [])
    ]);
    const requests=await count();
    const violations=globalThis.__preloadCspViolations;
    const row={as,mode,requests,terminal,violations};observations.push(row);
    if(requests !== Number(allowed) || (ignored
       ? terminal!=='none' || violations.length!==0
       : terminal==='timeout' || !terminal.ownRealm || (!allowed && terminal.type!=='error')))
      failures.push(row);
    frame.remove();
  }
  delete globalThis.__preloadCspDone;
  delete globalThis.__preloadCspViolations;
  return {observations,failures};
})()
