(async () => {
  const controlled = navigator.serviceWorker.controller ? Promise.resolve() :
    new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once:true}));
  await navigator.serviceWorker.register('/css-import-referrer-worker.js', {scope:'/'});
  await navigator.serviceWorker.ready;
  await controlled;
  const setPolicy = policy => {
    const meta=document.createElement('meta');
    meta.name='referrer';meta.content=policy;document.head.append(meta);meta.remove();
  };
  const failures=[];
  globalThis.cssImportFailures=failures;
  for (const name of ['default','no-referrer','origin','unsafe','multiple','invalid',
      'redirect','inline','inline-direct','reset','nested','diamond','data','inline-data',
      'external-data','insert-root','insert-child','synthetic-inherit','request-override']) {
    setPolicy('no-referrer');
    const resource=file => new URL('/css-referrer/' + name + '/' + file, location.href).href;
    const root=resource('root.css'), mid=resource('mid.css');
    const row=(file,referrer,policy) => ({file,referrer,policy});
    const expected=[];
    if (name === 'redirect') expected.push(row('start.css','','no-referrer'));
    if (!['data','inline-data','inline-direct'].includes(name))
      expected.push(name === 'request-override' ? row('root.css',location.origin+'/','origin') : row('root.css','','no-referrer'));
    if (name === 'reset') expected.push(row('mid.css','','no-referrer'),row('leaf.css','','no-referrer'));
    else if (name === 'nested') expected.push(row('mid.css',root,'unsafe-url'),row('leaf.css','','no-referrer'));
    else if (name === 'synthetic-inherit') expected.push(row('mid.css',root,'unsafe-url'),row('leaf.css','','no-referrer'));
    else if (name === 'diamond') expected.push(row('left.css',root,'unsafe-url'),row('right.css',root,'unsafe-url'),
      row('leaf.css','','no-referrer'));
    else if (name === 'insert-child') expected.push(row('mid.css',root,'unsafe-url'),row('leaf.css',location.origin+'/','origin'));
    else if (['no-referrer','insert-root','inline-direct'].includes(name)) expected.push(row('leaf.css','','no-referrer'));
    else if (name === 'origin') expected.push(row('leaf.css',location.origin+'/','origin'));
    else if (['unsafe','multiple','redirect'].includes(name)) expected.push(row('leaf.css',root,'unsafe-url'));
    else if (['data','inline-data','external-data'].includes(name)) expected.push(row('leaf.css','','no-referrer'));
    else expected.push(row('leaf.css','','no-referrer'));

    const inline=name.startsWith('inline');
    const element=document.createElement(inline ? 'style' : 'link');
    let url=name === 'redirect' ? resource('start.css') : root;
    if (name === 'data' || name === 'inline-data')
      url='data:text/css,'+encodeURIComponent('@import url("'+resource('leaf.css')+'");');
    if (name === 'inline-direct') url=resource('leaf.css');
    if (inline) element.textContent='@import url("'+url+'");';
    else {element.rel='stylesheet';element.href=url;}
    if (name === 'request-override') element.referrerPolicy='origin';
    if (inline) document.head.append(element);
    else await new Promise((resolve,reject) => {
      element.onload=resolve;
      element.onerror=() => reject(new Error(name+': stylesheet failed'));
      document.head.append(element);
    });
    if (name === 'insert-root' || name === 'insert-child') {
      setPolicy('unsafe-url');
      const sheet=name === 'insert-root' ? element.sheet : element.sheet.cssRules[0].styleSheet;
      sheet.insertRule('@import url("'+resource('leaf.css')+'");',0);
    }
    let actual=[];
    for (let attempt=0;attempt<200;attempt++) {
      actual=await (await fetch('/css-import-observations?case='+name)).json();
      if (actual.length >= expected.length) break;
      await new Promise(resolve => setTimeout(resolve,10));
    }
    const normalize=rows => rows.map(value => JSON.stringify(value)).sort();
    if (JSON.stringify(normalize(actual)) !== JSON.stringify(normalize(expected)))
      failures.push(name+': '+JSON.stringify(actual)+' != '+JSON.stringify(expected));
    element.remove();
  }
  if (failures.length) throw new Error(failures.join('\n'));
  return true;
})();
