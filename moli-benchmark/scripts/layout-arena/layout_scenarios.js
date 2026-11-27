(() => {
  if(window.__arenaLayout)throw Error('Scenario already installed');
  const originalActive=document.activeElement;
  const scrolls=Array.from(document.querySelectorAll('*')).filter(n=>n.scrollLeft||n.scrollTop).map(n=>[n,n.scrollLeft,n.scrollTop]);
  const viewportScroll=[scrollX,scrollY];
  const box=n=>{const r=n.getBoundingClientRect();return [r.x,r.y,r.width,r.height]};
  const area=n=>{const r=n.getBoundingClientRect();return r.width*r.height};
  let candidates=Array.from(document.querySelectorAll('main,article,#content,#main,#app,#root,#hnmain,.main-content'));
  candidates=candidates.filter(n=>n!==document.body&&n.parentElement&&area(n)>1000);
  if(!candidates.length)candidates=Array.from(document.body.children).filter(n=>!['SCRIPT','STYLE','LINK','IFRAME'].includes(n.tagName)&&area(n)>1000);
  candidates.sort((a,b)=>area(b)-area(a));
  const target=candidates[0];if(!target)throw Error('No visible content target');
  const targetParent=target.parentNode,targetNext=target.nextSibling;
  const targetStyle=target.getAttribute('style'),bodyStyle=document.body.getAttribute('style');
  const restoreStyle=(n,s)=>s===null?n.removeAttribute('style'):n.setAttribute('style',s);
  const texts=[];
  const walker=document.createTreeWalker(target,NodeFilter.SHOW_TEXT);
  let t;while(texts.length<20&&(t=walker.nextNode())){
    if(!t.data.trim()||t.data.length>1000)continue;
    if(t.parentElement.closest('script,style,noscript,template,textarea,select,option,button,svg,math'))continue;
    if(!t.parentElement.getClientRects().length)continue;
    texts.push([t,t.data]);
  }
  if(!texts.length)throw Error('No visible text nodes');
  const restore=()=>{
    if(target.parentNode!==targetParent||target.nextSibling!==targetNext)targetParent.insertBefore(target,targetNext);
    restoreStyle(target,targetStyle);restoreStyle(document.body,bodyStyle);
    for(const [n,s]of texts)n.data=s;
    if(originalActive?.isConnected&&originalActive.focus)originalActive.focus({preventScroll:true});
    for(const [n,x,y]of scrolls){n.scrollLeft=x;n.scrollTop=y}scrollTo(...viewportScroll);
  };
  const observe=()=>{
    const boxes=[];
    for(const n of document.querySelectorAll('body *')){
      if(boxes.length===1000)break;
      if(!n.getClientRects().length)continue;
      boxes.push([n.localName,n.id,...box(n)]);
    }
    return {nodes:document.querySelectorAll('body *').length,text:document.body.textContent.length,
      viewport:[innerWidth,innerHeight],body:box(document.body),target:target.isConnected?box(target):null,boxes};
  };
  const step=(scenario,index)=>{
    const start=performance.now();
    if(scenario==='container_width'){
      if(index%4===3)restoreStyle(document.body,bodyStyle);
      else{document.body.style.setProperty('width',[960,640,1180][index%4]+'px','important');document.body.style.setProperty('min-width','0','important');document.body.style.setProperty('max-width','none','important')}
    }else if(scenario==='text'){
      for(const [n,s]of texts)n.data=index%4===3?s:s.repeat([2,3,2][index%4]);
    }else if(scenario==='display'){
      if(index%2===0)target.style.setProperty('display','none','important');else restoreStyle(target,targetStyle);
    }else if(scenario==='detach'){
      if(index%2===0)target.remove();else targetParent.insertBefore(target,targetNext);
    }else if(scenario!=='viewport')throw Error('Unknown scenario');
    const geometry=[box(document.body),target.isConnected?box(target):null];
    return {ms:performance.now()-start,geometry};
  };
  window.__arenaLayout={restore,observe,step};
  return {target:target.localName,targetId:target.id,textNodes:texts.length,targetElements:target.querySelectorAll('*').length,initial:observe()};
})()
