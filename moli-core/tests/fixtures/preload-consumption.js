async function () {
 const basic = await (async () => {return (async () => {
  const rows=[];
  for (const as of ['script','image']) for (const cache of ['max-age=60','no-store'])
  for (const phase of ['pending','complete']) for (const cors of [null,'anonymous','use-credentials']) {
    const token=String(rows.length), url=new URL('/probe-asset',location.href);
    url.searchParams.set('token',token);url.searchParams.set('as',as);url.searchParams.set('cache',cache);
    const link=document.createElement('link');Object.assign(link,{rel:'preload',as,href:url.href});
    if(cors!==null)link.crossOrigin=cors;
    const linkDone=new Promise(resolve=>{link.onload=link.onerror=e=>resolve(e.type)});
    document.head.append(link);
    await fetch('/probe-started?token='+token);
    if(phase==='complete'){await fetch('/probe-release?token='+token);await linkDone;}
    const consumer=document.createElement(as==='script'?'script':'img');
    if(cors!==null)consumer.crossOrigin=cors;
    let timingAtConsumer;
    const consumerDone=new Promise(resolve=>{consumer.onload=consumer.onerror=e=>{timingAtConsumer=performance.getEntriesByName(url.href).length;resolve(e.type)}});
    consumer.src=url.href;document.body.append(consumer);
    if(phase==='pending')await fetch('/probe-release?token='+token);
    const events=await Promise.all([linkDone,consumerDone]);
    const actual=await (await fetch('/probe-stats?token='+token)).json();
    rows.push({as,cache,phase,cors,events,timingAtConsumer,...actual,timing:performance.getEntriesByName(url.href).map(e=>({initiator:e.initiatorType,transfer:e.transferSize,encoded:e.encodedBodySize}))});
    link.remove();consumer.remove();
  }
  return {rows,executions:globalThis.preloadExecuted||0,failures:rows.filter(r=>r.count!==1 || r.timing.length!==1 || r.timingAtConsumer!==1 || r.events.some(e=>e!=='load'))};
})();
})();
 const integrity = await (async () => {return (async () => {
 const rows=[];
 for (const cache of ['max-age=60','no-store']) for (const phase of ['pending','complete'])
 for (const [name,preloadIntegrity,consumerIntegrity,preloadEvent,consumerEvent,requests] of [["good-empty", "sha384-l+rtjjQWlmlLFL2PC8iIPwnXbiW45bZ6LU1ERjlAxYKUFswREZzY3pPyoT0RsvxJ", "", "load", "load", 1], ["bad-empty", "sha384-AAAA", "", "error", "error", 1], ["bad-same", "sha384-AAAA", "sha384-AAAA", "error", "error", 1], ["bad-good", "sha384-AAAA", "sha384-l+rtjjQWlmlLFL2PC8iIPwnXbiW45bZ6LU1ERjlAxYKUFswREZzY3pPyoT0RsvxJ", "error", "load", 2], ["good-bad", "sha384-l+rtjjQWlmlLFL2PC8iIPwnXbiW45bZ6LU1ERjlAxYKUFswREZzY3pPyoT0RsvxJ", "sha384-AAAA", "load", "error", 2], ["good-options", "sha384-l+rtjjQWlmlLFL2PC8iIPwnXbiW45bZ6LU1ERjlAxYKUFswREZzY3pPyoT0RsvxJ?ignored=option", "sha384-l+rtjjQWlmlLFL2PC8iIPwnXbiW45bZ6LU1ERjlAxYKUFswREZzY3pPyoT0RsvxJ", "load", "load", 1], ["good-different-algorithm", "sha384-l+rtjjQWlmlLFL2PC8iIPwnXbiW45bZ6LU1ERjlAxYKUFswREZzY3pPyoT0RsvxJ", "sha256-cRlncoZ/tNV1cbKTR2cXiKEwVFkjvOz4kToIRLKBiSE=", "load", "load", 2], ["empty-good", "", "sha384-l+rtjjQWlmlLFL2PC8iIPwnXbiW45bZ6LU1ERjlAxYKUFswREZzY3pPyoT0RsvxJ", "load", "load", 2], ["malformed-empty", "sha384-AAAA===", "", "error", "error", 1], ["good-malformed", "sha384-l+rtjjQWlmlLFL2PC8iIPwnXbiW45bZ6LU1ERjlAxYKUFswREZzY3pPyoT0RsvxJ", "sha384-A=AAA", "load", "error", 2]]) {
  const token='sri-'+rows.length,url=new URL('/probe-asset',location.href);
  url.searchParams.set('token',token);url.searchParams.set('as','script');url.searchParams.set('cache',cache);
  const link=document.createElement('link');Object.assign(link,{rel:'preload',as:'script',href:url.href,integrity:preloadIntegrity});
  const linkDone=new Promise(resolve=>{link.onload=link.onerror=e=>resolve(e.type)});document.head.append(link);
  await fetch('/probe-started?token='+token);
  if(phase==='complete'){await fetch('/probe-release?token='+token);await linkDone;}
  const consumer=document.createElement('script');consumer.integrity=consumerIntegrity;consumer.src=url.href;
  const consumerDone=new Promise(resolve=>{consumer.onload=consumer.onerror=e=>resolve(e.type)});document.body.append(consumer);
  if(phase==='pending')await fetch('/probe-release?token='+token);
  const events=await Promise.all([linkDone,consumerDone]);
  const actual=await(await fetch('/probe-stats?token='+token)).json();
  rows.push({name,cache,phase,events,expectedEvents:[preloadEvent,consumerEvent],expectedRequests:requests,...actual,timing:performance.getEntriesByName(url.href).map(e=>({initiator:e.initiatorType,transfer:e.transferSize}))});
  link.remove();consumer.remove();
 }
 return {rows,failures:rows.filter(r=>JSON.stringify(r.events)!==JSON.stringify(r.expectedEvents)||r.timing.length!==r.expectedRequests || (r.cache==='no-store' && r.count!==r.expectedRequests) || (r.expectedRequests===1 && r.count!==1))};
})();})();
 const oneShot = await (async () => {
    const url = new URL('/probe-asset?token=one-shot&as=script', location.href);
    await fetch('/probe-release?token=one-shot');
    const link = document.createElement('link');
    Object.assign(link, {rel:'preload', as:'fetch', crossOrigin:'anonymous', href:url.href});
    await new Promise(resolve => { link.onload=link.onerror=resolve; document.head.append(link); });
    const count = async () => (await (await fetch('/probe-stats?token=one-shot')).json()).count;
    const counts = [await count()];
    await (await fetch(url, {headers:{Accept:'text/javascript'}})).text(); counts.push(await count());
    await (await fetch(url)).text(); counts.push(await count());
    await (await fetch(url)).text(); counts.push(await count());
    link.remove();
    return counts;
 })();
 return {basic,integrity,oneShot};
}
