async function navigationReferrerProbe(config) {
  const {id, scope, action, policy, destination, snapshot} = config;
  const frame = document.createElement('iframe');
  const navigate = action === 'post'
    ? `const form=document.createElement('form');form.method='post';form.action=${JSON.stringify(destination)};document.body.append(form);form.submit();`
    : action === 'replace'
    ? `location.replace(${JSON.stringify(destination)});`
    : `location.href=${JSON.stringify(destination)};`;
  const mutate = snapshot ? `onbeforeunload=()=>{
    const meta=document.createElement('meta');meta.name='referrer';meta.content='no-referrer';document.head.append(meta);
    if(location.protocol==='http:') history.replaceState(null,'','/changed-source?${id}');
    else top.history.replaceState(null,'','/changed-root?${id}');
  };` : '';
  const html = '<!doctype html><head>' +
    (policy ? '<meta name="referrer" content="'+policy+'">' : '') +
    '<base href="http://base-must-not-be-referrer.invalid/">' +
    '</head><body><script>'+mutate+'onload=()=>{'+navigate+'};</script>';
  if (scope === 'http') {
    frame.src='/navigation-referrer-source?html='+encodeURIComponent(html);
  } else if (scope === 'nested-srcdoc') {
    frame.srcdoc='<!doctype html><body><iframe sandbox="allow-scripts allow-same-origin allow-forms" srcdoc="'+
      html.replaceAll('&','&amp;').replaceAll('"','&quot;')+'"></iframe>';
  } else {
    frame.sandbox = scope === 'opaque-srcdoc' ? 'allow-scripts allow-forms' :
      'allow-scripts allow-same-origin allow-forms';
    frame.srcdoc=html;
  }
  const sourceUrl = scope === 'http' ? new URL(frame.getAttribute('src'), location.href).href : location.href;
  return await new Promise(resolve=>{
    const listener=event=>{
      if(event.data?.navigationReferrerId!==id)return;
      removeEventListener('message',listener);
      frame.remove();
      resolve({documentReferrer:event.data.referrer,senderOrigin:event.origin,sourceUrl});
    };
    addEventListener('message',listener);
    document.body.append(frame);
  });
}
