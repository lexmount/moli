(async () => {
  const controlled = navigator.serviceWorker.controller ? Promise.resolve() :
    new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once:true}));
  await navigator.serviceWorker.register('/native-referrer-worker.js', {scope:'/'});
  await navigator.serviceWorker.ready;
  await controlled;
  let id = 0;
  async function check(kind, override, documentPolicy, owner=window, token=String(++id)) {
    const url = '/native-referrer/resource?id=' + token;
    const element = owner.document.createElement(kind === 'style' ? 'link' : kind === 'iframe' ? 'iframe' : 'script');
    if (override !== null) element.referrerPolicy = override;
    if (kind === 'style') { element.rel='stylesheet'; element.href=url; }
    else { if (kind === 'module') element.type='module'; element.src=url; }
    await new Promise((resolve,reject) => {
      element.onload=resolve;
      element.onerror=() => reject(new Error('resource failed: ' +
        JSON.stringify({kind,override,documentPolicy,url,owner:owner.location.href})));
      owner.document.body.append(element);
    });
    const value = await (await fetch('/native-referrer/observations?id=' + token)).json();
    const policy = override || documentPolicy;
    const expected = policy === 'no-referrer' ? '' :
      policy === 'origin' ? owner.location.origin + '/' : owner.location.href.split('#')[0];
    const destination = kind === 'module' ? 'script' : kind;
    if (value.referrer !== expected || value.policy !== policy || value.destination !== destination)
      throw new Error([kind,override,documentPolicy].join('|') + ': ' + JSON.stringify(value) +
        ' != ' + JSON.stringify({referrer:expected,policy,destination}));
    element.remove();
  }
  await check('iframe', null, 'strict-origin-when-cross-origin');
  for (const policy of ['strict-origin-when-cross-origin','origin','no-referrer','unsafe-url']) {
    if (policy !== 'strict-origin-when-cross-origin') {
      const meta = document.createElement('meta');
      meta.name='referrer'; meta.content=policy; document.head.append(meta); meta.remove();
    }
    for (const kind of ['script','module','style']) {
      for (const override of [null,'origin','no-referrer','unsafe-url']) {
        await check(kind, override, policy);
      }
    }
  }
  for (const policy of ['origin','no-referrer']) {
    const meta = document.createElement('meta');
    meta.name='referrer'; meta.content=policy; document.head.append(meta); meta.remove();
    await check('style', null, policy, window, 'shared-css');
  }
  const parentPolicy = document.createElement('meta');
  parentPolicy.name='referrer';parentPolicy.content='unsafe-url';
  document.head.append(parentPolicy);parentPolicy.remove();
  const child = document.createElement('iframe');
  child.src='/native-referrer/resource?id=child-document';
  await new Promise(resolve => {child.onload=resolve;document.body.append(child);});
  const other = child.contentWindow;
  const meta = other.document.createElement('meta');
  meta.name='referrer';meta.content='no-referrer';other.document.head.append(meta);meta.remove();
  await check('style', null, 'no-referrer', other);
  await check('script', 'origin', 'no-referrer', other);
  await check('style', 'unsafe-url', 'no-referrer', other);
  child.remove();

  const parserChild = document.createElement('iframe');
  parserChild.src='/native-referrer/resource?id=parser-document&parser-css';
  await new Promise(resolve => {parserChild.onload=resolve;document.body.append(parserChild);});
  const parsed = await (await fetch('/native-referrer/observations?id=parser-css')).json();
  if (parsed.policy !== 'no-referrer' || parsed.referrer !== '' || parsed.destination !== 'style')
    throw new Error('parser stylesheet policy: ' + JSON.stringify(parsed));
  parserChild.remove();
  return true;
})();
