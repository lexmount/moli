(() => {
  const checks=[],ns='http://www.w3.org/2000/svg';
  const check=(name,fn)=>{try {checks.push({name,passed:fn()===true});}catch(error){checks.push({name,passed:false,error:String(error)});}};
  const near=(a,b)=>Math.abs(a-b)<1e-5;
  const realms=[window,document.querySelector('iframe').contentWindow];
  const lengths=['96','96px','1in','2.54cm','25.4mm','101.6Q','72pt','6pc'];
  for (const [ownerIndex,owner] of realms.entries()) {
    const docs=[owner.document,owner.document.implementation.createHTMLDocument(''),owner.document.implementation.createDocument(ns,'svg'),new owner.DOMParser().parseFromString('<svg xmlns="'+ns+'"/>','image/svg+xml')];
    for (const [docIndex,doc] of docs.entries()) {
      for (const [calleeIndex,callee] of realms.entries()) {
        const prefix=`units/${ownerIndex}/${docIndex}/${calleeIndex}`;
        const point=callee.SVGGeometryElement.prototype.getPointAtLength,total=callee.SVGGeometryElement.prototype.getTotalLength;
        const root=doc.createElementNS(ns,'svg');root.setAttribute('width','240');root.setAttribute('height','120');(doc.body||doc.documentElement).appendChild(root);
        for (const [index,raw] of lengths.entries()) {
          const line=doc.createElementNS(ns,'line');line.setAttribute('x1','0.5in');line.setAttribute('y1','1pc');line.setAttribute('x2',raw);line.setAttribute('y2','1pc');root.appendChild(line);
          check(prefix+'/absolute/'+index+'/total',()=>total.call(line)===48);
          check(prefix+'/absolute/'+index+'/point',()=>{const p=point.call(line,24);return near(p.x,72)&&near(p.y,16);});
          check(prefix+'/absolute/'+index+'/hidden',()=>{line.style.display='none';return total.call(line)===48;});
          check(prefix+'/absolute/'+index+'/detached',()=>{line.remove();const p=point.call(line,24);return total.call(line)===48&&near(p.x,72)&&near(p.y,16);});
        }
        const cases=[
          ['viewport',{},null,300,120],
          ['viewbox',{viewBox:'0 0 600 300'},null,750,300],
          ['partial-height',{height:'50',width:null},null,200,50],
          ['partial-width',{width:'150',height:null},null,400,150],
          ['nested-percent',{}, {width:'50%',height:'50%'},150,60],
          ['nested-viewbox',{}, {width:'50%',height:'50%',viewBox:'0 0 100 200'},200,50],
          ['nested-absolute',{}, {width:'2.54cm',height:'25.4mm'},144,48],
        ];
        for(const [name,attributes,nestedAttributes,expected,first] of cases) {
          root.setAttribute('width','240');root.setAttribute('height','120');root.removeAttribute('viewBox');for(const [key,value] of Object.entries(attributes))if(value===null)root.removeAttribute(key);else root.setAttribute(key,value);
          let viewport=root,nested;
          if(nestedAttributes){nested=doc.createElementNS(ns,'svg');for(const [key,value] of Object.entries(nestedAttributes))nested.setAttribute(key,value);root.appendChild(nested);viewport=nested;}
          const rect=doc.createElementNS(ns,'rect');rect.setAttribute('width',name==='partial-height'?'50':name==='partial-width'?'100%':'50%');rect.setAttribute('height',name==='partial-height'?'100%':name==='partial-width'?'50':'25%');viewport.appendChild(rect);
          check(prefix+'/'+name+'/total',()=>total.call(rect)===expected);
          check(prefix+'/'+name+'/point',()=>{const p=point.call(rect,first);return near(p.x,first)&&near(p.y,0);});
          check(prefix+'/'+name+'/mutation',()=>{if(name==='partial-height'){rect.setAttribute('height','50%');return total.call(rect)===150;}if(name==='partial-width'){rect.setAttribute('width','50%');return total.call(rect)===250;}rect.setAttribute('width','25%');return total.call(rect)===expected-first;});
          rect.remove();if(nested)nested.remove();
        }
        root.setAttribute('width','240');root.setAttribute('height','120');root.removeAttribute('viewBox');
        const line=doc.createElementNS(ns,'line');line.setAttribute('x2','100%');line.setAttribute('y2','100%');root.appendChild(line);
        check(prefix+'/percent-axes',()=>total.call(line)===Math.fround(Math.hypot(240,120)));
        check(prefix+'/viewport-mutation',()=>{root.setAttribute('width','120');root.setAttribute('height','160');return total.call(line)===200;});
        root.setAttribute('viewBox','0 0 30 40');check(prefix+'/viewbox-mutation',()=>total.call(line)===50);
        root.remove();
      }
    }
  }
  globalThis.__svgUnitsResults={complete:true,total:checks.length,passed:checks.filter(row=>row.passed).length,checks};globalThis.__uiEventResults=globalThis.__svgUnitsResults;return true;
})()
