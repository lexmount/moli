(async () => {
  const checks = [], beforeNavigation = navigation.onnavigate;
  let serial = 0;
  try {
    for (const depth of [2,3]) for (const target of ['_parent','_PaReNt']) {
      for (const borrowed of [false,true]) for (const navigate of [false,true]) {
        let first;
        const name = JSON.stringify({depth,target,borrowed,navigate});
        try {
          let receiver = window;
          for (let i=0;i<depth;i++) {
            const frame = receiver.document.createElement('iframe');
            if (!first) first=frame;
            const parent=receiver;
            await new Promise(resolve => {
              frame.onload=resolve;
              frame.srcdoc='<body>initial document</body>';
              parent.document.body.appendChild(frame);
            });
            receiver=frame.contentWindow;
          }
          const expected=receiver.parent, before=expected.document;
          const parentHref=expected.location.href, receiverHref=receiver.location.href, rootHref=location.href;
          const seen=[];
          navigation.onnavigate=event => {seen.push('top');event.preventDefault()};
          const url=navigate ? location.origin+'/audit.html?parent-target='+serial++ : '';
          const open=borrowed ? window.open : receiver.open;
          const opened=open.call(receiver,url,target);
          const immediate=[expected!==window, opened===expected, opened===window,
            expected.location.href===parentHref, receiver.location.href===receiverHref,
            location.href===rootHref, seen.length===0];
          if (immediate.join()!=='true,true,false,true,true,true,true') throw Error('immediate '+JSON.stringify(immediate));
          if (navigate) {
            for (let attempt=0;attempt<200 && expected.location.href!==url;attempt++) {
              await new Promise(resolve => setTimeout(resolve,10));
            }
            if (expected.location.href!==url || expected.document===before) throw Error('parent navigation did not commit');
          } else {
            await new Promise(resolve => setTimeout(resolve,10));
            if (expected.document!==before || expected.location.href!==parentHref) throw Error('empty URL changed parent');
          }
          if (location.href!==rootHref || seen.length) throw Error('top navigation');
          checks.push({name,passed:true});
        } catch (error) {checks.push({name,passed:false,error:String(error)});}
        finally {first?.remove();}
      }
    }
  } finally {navigation.onnavigate=beforeNavigation;}
  globalThis.__uiEventResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};
  return checks.every(row=>row.passed);
})()
