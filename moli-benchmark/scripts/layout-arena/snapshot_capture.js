(() => {
  const root=document.documentElement;
  const clone=root.cloneNode(true);
  const originals=[root,...root.querySelectorAll('*')];
  const copies=[clone,...clone.querySelectorAll('*')];
  const css={};const canvases=[];const scroll=[];let openShadows=0;
  function path(node) {
    const result=[];
    while(node&&node!==root) {
      const parent=node.parentElement;if(!parent)return null;
      result.push(Array.from(parent.children).indexOf(node));node=parent;
    }
    return result.reverse();
  }
  const active=path(document.activeElement);
  for(let i=0;i<originals.length;i++){
    const node=originals[i], copy=copies[i];
    if(node.shadowRoot)openShadows++;
    for(const attr of Array.from(copy.attributes))if(attr.name.toLowerCase().startsWith('on'))copy.removeAttribute(attr.name);
    if(node.localName==='script'&&!['application/json','application/ld+json'].includes(node.type))copy.setAttribute('type','application/x-moli-arena-snapshot');
    if(node.localName==='style'&&node.sheet){
      try {copy.textContent=node.sheet.disabled?'':Array.from(node.sheet.cssRules,r=>r.cssText).join('\n')}catch{}
    }
    if(node.localName==='link'&&node.sheet){
      try {
        if(node.sheet.disabled)copy.setAttribute('rel','moli-arena-disabled-stylesheet');
        else css[node.href]=Array.from(node.sheet.cssRules,r=>r.cssText).join('\n');
      }catch{}
    }
    if(node.localName==='input'){
      copy.setAttribute('value',node.type==='password'?'':node.value);
      if(node.checked)copy.setAttribute('checked','');else copy.removeAttribute('checked');
    }
    if(node.localName==='textarea')copy.textContent=node.value;
    if(node.localName==='option'){
      if(node.selected)copy.setAttribute('selected','');else copy.removeAttribute('selected');
    }
    if(node.localName==='canvas'){
      try{canvases.push(node.toDataURL())}catch{canvases.push(null)}
    }
    if(node.scrollTop||node.scrollLeft)scroll.push({path:path(node),x:node.scrollLeft,y:node.scrollTop});
    if(node.nonce)copy.setAttribute('nonce',node.nonce);
    // Preserve adjacent text-node boundaries without affecting selectors.
    if(!['script','style','textarea'].includes(node.localName)){
      const children=Array.from(copy.childNodes);
      for(let j=1;j<children.length;j++)if(children[j-1].nodeType===3&&children[j].nodeType===3)
        copy.insertBefore(document.createComment('moli-arena-text-boundary'),children[j]);
    }
  }
  for(const meta of clone.querySelectorAll('meta[http-equiv]'))if(meta.httpEquiv.toLowerCase()==='content-security-policy')meta.remove();
  const d=document.doctype;
  const doctype=d?'<!DOCTYPE '+d.name+(d.publicId?' PUBLIC "'+d.publicId+'"':'')+(d.systemId?(d.publicId?' "':' SYSTEM "')+d.systemId+'"':'')+'>\n':'';
  return {url:location.href,html:doctype+clone.outerHTML,css,canvases,active,scroll,openShadows,viewportScroll:[scrollX,scrollY]};
})()
