(async () => {
  const checks = [];
  const progress = globalThis.__svgHrefProgress = {stage:'setup',checks};
  const svgNS='http://www.w3.org/2000/svg', xlinkNS='http://www.w3.org/1999/xlink';
  const foreignNS='urn:foreign-svg-attribute';
  const colors={red:[255,0,0,255],lime:[0,255,0,255],blue:[0,0,255,255]};
  const url=color=>'data:image/svg+xml,'+encodeURIComponent(
    `<svg xmlns="${svgNS}" width="4" height="2"><rect width="4" height="2" fill="${color}"/></svg>`);
  const urls={red:url('red'),lime:url('lime'),blue:url('blue')};
  const assert=(ok,detail)=>{if(!ok)throw Error(detail);};
  const same=(a,b)=>JSON.stringify(a)===JSON.stringify(b);
  const run=async(name,body)=>{
    progress.stage=name;
    try{await body();checks.push({name,passed:true});}
    catch(error){checks.push({name,passed:false,detail:String(error)});}
  };
  function loaded(owner,image,mutate) {
    return new Promise((resolve,reject)=>{
      const timeout=owner.setTimeout(()=>finish(Error('SVG load timed out')),1000);
      function finish(error){
        owner.clearTimeout(timeout);image.onload=image.onerror=null;
        if(error)reject(error);else resolve(image);
      }
      image.onload=()=>finish();image.onerror=()=>finish(Error('SVG load error'));
      try{mutate();}catch(error){finish(error);}
    });
  }
  function canvas(callee,offscreen) {
    const c=offscreen?new callee.OffscreenCanvas(4,2):callee.document.createElement('canvas');
    c.width=4;c.height=2;return c.getContext('2d');
  }
  function expectPixels(callee,image,color) {
    for(const offscreen of [false,true]){
      const ctx=canvas(callee,offscreen);ctx.drawImage(image,0,0);
      const actual=[...ctx.getImageData(0,0,1,1).data];
      assert(same(actual,colors[color]),'draw pixels '+actual+' expected '+colors[color]);
    }
  }
  async function expectBitmap(callee,image,color) {
    const bitmap=await callee.createImageBitmap(image,{resizeWidth:12,resizeHeight:6});
    try{
      assert(bitmap instanceof callee.ImageBitmap && bitmap.width===12 && bitmap.height===6,'bitmap realm/size');
      const c=new callee.OffscreenCanvas(12,6),ctx=c.getContext('2d');ctx.drawImage(bitmap,0,0);
      const actual=[...ctx.getImageData(6,3,1,1).data];
      assert(same(actual,colors[color]),'bitmap pixels '+actual+' expected '+colors[color]);
    }finally{bitmap.close();}
  }
  async function fences(callee) {
    const c=new callee.OffscreenCanvas(1,1);c.getContext('2d');
    for(let i=0;i<2;i++){const b=await callee.createImageBitmap(c);b.close();}
  }
  async function rejected(callee,image) {
    let caught;try{const b=await callee.createImageBitmap(image);b.close();}catch(error){caught=error;}
    assert(caught instanceof callee.DOMException && caught.name==='InvalidStateError','unusable SVG '+caught);
  }
  async function initial(owner,configure) {
    const image=owner.document.createElementNS(svgNS,'image');
    await loaded(owner,image,()=>{configure(image);owner.document.body.appendChild(image);});
    return image;
  }
  const xlinkGreen=image=>image.setAttributeNS(xlinkNS,'old:href',urls.lime);
  const plainGreen=image=>image.setAttribute('href',urls.lime);
  const cases=[
    ['setAttributeNS XLink after append',null,image=>image.setAttributeNS(xlinkNS,'xlink:href',urls.red),'red'],
    ['setAttributeNS alternate XLink prefix after append',null,image=>image.setAttributeNS(xlinkNS,'alternate:href',urls.red),'red'],
    ['setAttributeNS unprefixed XLink after append',null,image=>image.setAttributeNS(xlinkNS,'href',urls.red),'red'],
    ['setAttributeNS null namespace after append',null,image=>image.setAttributeNS(null,'href',urls.red),'red'],
    ['setAttribute updates an existing namespaced attribute',xlinkGreen,image=>image.setAttribute('old:href',urls.red),'red'],
    ['Attr.value updates XLink',xlinkGreen,image=>{image.getAttributeNodeNS(xlinkNS,'href').value=urls.red;},'red'],
    ['Attr.nodeValue updates XLink',xlinkGreen,image=>{image.getAttributeNodeNS(xlinkNS,'href').nodeValue=urls.red;},'red'],
    ['Attr.textContent updates XLink',xlinkGreen,image=>{image.getAttributeNodeNS(xlinkNS,'href').textContent=urls.red;},'red'],
    ['setAttributeNodeNS updates XLink',plainGreen,(image,owner)=>{
      image.removeAttribute('href');const attr=owner.document.createAttributeNS(xlinkNS,'node:href');
      attr.value=urls.red;image.setAttributeNodeNS(attr);
    },'red'],
    ['NamedNodeMap.setNamedItemNS updates XLink',plainGreen,(image,owner)=>{
      image.removeAttribute('href');const attr=owner.document.createAttributeNS(xlinkNS,'map:href');
      attr.value=urls.red;image.attributes.setNamedItemNS(attr);
    },'red'],
    ['SVGAnimatedString.baseVal updates selected XLink',xlinkGreen,image=>{image.href.baseVal=urls.red;},'red'],
    ['removeAttributeNS plain href reveals XLink',image=>{xlinkGreen(image);image.setAttribute('href',urls.red);},
      image=>image.removeAttributeNS(null,'href'),'lime'],
    ['removeAttribute plain href reveals XLink',image=>{xlinkGreen(image);image.setAttribute('href',urls.red);},
      image=>image.removeAttribute('href'),'lime'],
    ['removeAttributeNode plain href reveals XLink',image=>{xlinkGreen(image);image.setAttribute('href',urls.red);},
      image=>image.removeAttributeNode(image.getAttributeNodeNS(null,'href')),'lime'],
    ['NamedNodeMap.removeNamedItemNS reveals XLink',image=>{xlinkGreen(image);image.setAttribute('href',urls.red);},
      image=>image.attributes.removeNamedItemNS(null,'href'),'lime'],
    ['same-turn namespace mutations select final source',plainGreen,image=>{
      image.setAttributeNS(xlinkNS,'first:href',urls.red);
      image.setAttributeNS(xlinkNS,'second:href',urls.lime);
      image.setAttribute('href',urls.blue);image.removeAttribute('href');
    },'lime'],
    ['shadowed XLink set keeps ordinary href',image=>{xlinkGreen(image);plainGreen(image);},
      image=>image.setAttributeNS(xlinkNS,'alternate:href',urls.red),'lime'],
    ['shadowed XLink removeAttributeNS keeps ordinary href',image=>{xlinkGreen(image);plainGreen(image);},
      image=>image.removeAttributeNS(xlinkNS,'href'),'lime'],
    ['shadowed XLink removeAttribute keeps ordinary href',image=>{xlinkGreen(image);plainGreen(image);},
      image=>image.removeAttribute('old:href'),'lime'],
    ['shadowed XLink toggleAttribute keeps ordinary href',image=>{xlinkGreen(image);plainGreen(image);},
      image=>image.toggleAttribute('old:href',false),'lime'],
  ];
  const ignored=[
    ['foreign namespace href',image=>image.setAttributeNS(foreignNS,'href',urls.red)],
    ['wrong-namespace xlink spelling',image=>image.setAttributeNS(foreignNS,'xlink:href',urls.red)],
    ['literal unnamespaced xlink spelling',image=>image.setAttribute('xlink:href',urls.red)],
    ['case-sensitive SVG HREF',image=>image.setAttribute('HREF',urls.red)],
    ['foreign namespace crossorigin',image=>image.setAttributeNS(foreignNS,'crossorigin','anonymous')],
  ];
  try{
    const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms){
      const prefix=ownerName+' image / '+calleeName;
      for(const [name,setup,mutate,color] of cases)await run(prefix+' '+name,async()=>{
        let image;
        try{
          if(setup)image=await initial(owner,setup);
          else{image=owner.document.createElementNS(svgNS,'image');owner.document.body.appendChild(image);}
          await loaded(owner,image,()=>mutate(image,owner));
          expectPixels(callee,image,color);await expectBitmap(callee,image,color);
        }finally{if(image)image.remove();}
      });
      for(const [name,mutate] of ignored)await run(prefix+' ignores '+name,async()=>{
        const image=await initial(owner,image=>{xlinkGreen(image);plainGreen(image);});
        try{
          let events=0;image.onload=image.onerror=()=>events++;
          mutate(image);await fences(callee);
          expectPixels(callee,image,'lime');await expectBitmap(callee,image,'lime');
          assert(events===0,'ignored mutation dispatched '+events+' events');
        }finally{image.remove();}
      });
      for(const [name,mutate] of [
        ['removeAttributeNS',image=>image.removeAttributeNS(xlinkNS,'href')],
        ['removeAttribute',image=>image.removeAttribute('old:href')],
        ['removeAttributeNode',image=>image.removeAttributeNode(image.getAttributeNodeNS(xlinkNS,'href'))],
        ['NamedNodeMap.removeNamedItemNS',image=>image.attributes.removeNamedItemNS(xlinkNS,'href')],
      ])await run(prefix+' removing sole XLink with '+name+' retires source',async()=>{
        const image=await initial(owner,xlinkGreen);
        try{mutate(image);await fences(callee);await rejected(callee,image);}
        finally{image.remove();}
      });
      await run(prefix+' explicit empty href masks XLink',async()=>{
        const image=await initial(owner,xlinkGreen);
        try{image.setAttribute('href','');await fences(callee);await rejected(callee,image);}
        finally{image.remove();}
      });
      await run(prefix+' native resource selection ignores public metadata',async()=>{
        const image=await initial(owner,xlinkGreen);
        try{
          for(const name of ['href','getAttribute','getAttributeNS','hasAttributeNS'])
            Object.defineProperty(image,name,{get(){throw Error('public '+name);}});
          await loaded(owner,image,()=>callee.Element.prototype.setAttributeNS.call(image,xlinkNS,'poison:href',urls.red));
          expectPixels(callee,image,'red');await expectBitmap(callee,image,'red');
        }finally{image.remove();}
      });
      await run(prefix+' foreign crossorigin does not enable network CORS',async()=>{
        const image=owner.document.createElementNS(svgNS,'image');
        try{
          image.setAttributeNS(foreignNS,'crossorigin','anonymous');
          await loaded(owner,image,()=>{
            image.setAttributeNS(xlinkNS,'network:href',__svgHrefRemoteURL+'/foreign-cors-'+ownerName+'-'+calleeName+'.svg');
            owner.document.body.appendChild(image);
          });
          const ctx=canvas(callee,true);ctx.drawImage(image,0,0);
          let caught;try{ctx.getImageData(0,0,1,1);}catch(error){caught=error;}
          assert(caught instanceof callee.DOMException && caught.name==='SecurityError','expected source taint '+caught);
        }finally{image.remove();}
      });
    }
  }catch(error){checks.push({name:'SVG href fixture setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
