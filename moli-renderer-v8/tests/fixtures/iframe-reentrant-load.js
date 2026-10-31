(async () => {
  const rows=[];
  for (const phase of ['before-load','window-load','window-microtask','owner-load','pageshow','grandparent']) {
    const events=[];let resolve;const done=new Promise(r=>{resolve=r;});
    const probe={record(name){events.push(name);if(events.includes('owner-element-load')&&events.includes('descendant-load')&&(phase!=='grandparent'||events.includes('grandparent-owner-load')))resolve();}};
    globalThis.__nestedLoadProbe=probe;
    const childSource=`
      const probe=top.__nestedLoadProbe;
      globalThis.addProbeLeaf=()=>{
        probe.record('descendant-insert');
        const leaf=document.createElement('iframe');
        leaf.onload=()=>probe.record('descendant-load');
        leaf.srcdoc='<!doctype html><body>leaf</body>';document.body.append(leaf);
      };
      addEventListener('load',()=>{
        probe.record('window-load');
        if (${JSON.stringify(phase)}==='window-load'||${JSON.stringify(phase)}==='grandparent')addProbeLeaf();
        if (${JSON.stringify(phase)}==='window-microtask')queueMicrotask(()=>{probe.record('window-microtask');addProbeLeaf();});
      });
      addEventListener('pageshow',()=>{probe.record('pageshow');if(${JSON.stringify(phase)}==='pageshow')addProbeLeaf();});
      if (${JSON.stringify(phase)}==='before-load')addProbeLeaf();
    `;
    const childHtml='<!doctype html><body><script>'+childSource+'</script>';
    const outer=document.createElement('iframe');
    if (phase==='grandparent') {
      const literal=JSON.stringify(childHtml).replaceAll('</script','<\\/script');
      const source=`const probe=top.__nestedLoadProbe;addEventListener('load',()=>probe.record('grandparent-window-load'));addEventListener('pageshow',()=>probe.record('grandparent-pageshow'));const child=document.createElement('iframe');child.onload=()=>probe.record('owner-element-load');child.srcdoc=${literal};document.body.append(child);`;
      outer.srcdoc='<!doctype html><body><script>'+source+'</script>';
      outer.onload=()=>probe.record('grandparent-owner-load');
    } else {
      outer.srcdoc=childHtml;
      outer.onload=()=>{probe.record('owner-element-load');if(phase==='owner-load')outer.contentWindow.addProbeLeaf();};
    }
    document.body.append(outer);
    let timer;
    const complete=await Promise.race([done.then(()=>true),new Promise(r=>{timer=setTimeout(()=>r(false),5000);})]);clearTimeout(timer);
    await new Promise(r=>setTimeout(r,0));rows.push({phase,complete,events});outer.remove();
  }
  return rows;
})()
