(async () => {
  const checks=[];
  const progress=globalThis.__exifProgress={stage:'setup',checks};
  const colors=[[255,0,0,255],[0,255,0,255],[0,0,255,255],
    [255,0,255,255],[0,255,255,255],[255,255,0,255]];
  const orders=[[0,1,2,3,4,5],[2,1,0,5,4,3],[5,4,3,2,1,0],[3,4,5,0,1,2],
    [0,3,1,4,2,5],[3,0,4,1,5,2],[5,2,4,1,3,0],[2,5,1,4,0,3]];
  const assert=(ok,message)=>{if(!ok)throw Error(message);};
  const run=async(name,body)=>{
    progress.stage=name;
    try{await body();checks.push({name,passed:true});}
    catch(error){checks.push({name,passed:false,detail:String(error)});}
  };
  const concat=(...pieces)=>{
    const bytes=new Uint8Array(pieces.reduce((n,p)=>n+p.length,0));let offset=0;
    for(const piece of pieces){bytes.set(piece,offset);offset+=piece.length;}return bytes;
  };
  function pngExif(exif) {
    const body=concat([101,88,73,102],exif);let crc=0xffffffff;
    for(const byte of body){crc^=byte;for(let i=0;i<8;i++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}
    crc=(crc^0xffffffff)>>>0;
    return concat([0,0,0,exif.length],body,[crc>>>24,(crc>>>16)&255,(crc>>>8)&255,crc&255]);
  }
  function orientedBytes(format,orientation) {
    const raw=Uint8Array.from(atob(__exifSources[format].base64),c=>c.charCodeAt(0));
    const exif=new Uint8Array([73,73,42,0,8,0,0,0,1,0,18,1,3,0,1,0,0,0,orientation,0,0,0,0,0,0,0]);
    if(format==='png')return concat(raw.subarray(0,33),pngExif(exif),raw.subarray(33));
    if(format==='jpg')return concat(raw.subarray(0,2),[255,225,0,exif.length+8,69,120,105,102,0,0],exif,raw.subarray(2));
    // The lossless source has a simple RIFF/VP8L container. VP8X declares the
    // EXIF feature and the same 48x32 canvas; EXIF itself is a raw TIFF chunk.
    const vp8x=new Uint8Array([86,80,56,88,10,0,0,0,8,0,0,0,47,0,0,31,0,0]);
    const payload=concat(vp8x,[69,88,73,70,exif.length,0,0,0],exif,raw.subarray(12));
    const size=payload.length+4;
    return concat([82,73,70,70,size&255,(size>>>8)&255,(size>>>16)&255,size>>>24,87,69,66,80],payload);
  }
  function context(realm,width,height,offscreen=true) {
    const canvas=offscreen?new realm.OffscreenCanvas(width,height):realm.document.createElement('canvas');
    canvas.width=width;canvas.height=height;return canvas.getContext('2d');
  }
  function pixel(ctx,x,y,expected) {
    const actual=[...ctx.getImageData(x,y,1,1).data];
    assert(actual.every((value,i)=>Math.abs(value-expected[i])<=2),`pixel (${x},${y}) ${actual} expected ${expected}`);
  }
  function verify(realm,source,orientation,flip=false,offscreen=true) {
    const width=orientation>=5?32:48,height=orientation>=5?48:32;
    const columns=width/16,rows=height/16;
    const ctx=context(realm,width,height,offscreen);ctx.drawImage(source,0,0);
    for(let row=0;row<rows;row++)for(let col=0;col<columns;col++){
      const input=(flip?rows-1-row:row)*columns+col;
      pixel(ctx,col*16+8,row*16+8,colors[orders[orientation-1][input]]);
    }
  }
  function load(owner,image,url,svg) {
    return new Promise((resolve,reject)=>{
      image.onload=()=>{image.onload=image.onerror=null;resolve(image);};
      image.onerror=()=>reject(Error('image load error'));
      owner.document.body.appendChild(image);
      if(svg)image.setAttribute('href',url);else image.src=url;
    });
  }
  try {
    const realms=[['main',globalThis],['iframe',document.querySelector('iframe').contentWindow]];
    for(const [ownerName,owner] of realms)for(const [calleeName,callee] of realms)
      for(const format of ['png','jpg','webp'])for(let orientation=1;orientation<=8;orientation++){
        if(globalThis.__exifScenario && globalThis.__exifScenario !== `${ownerName}/${calleeName}/${format}`)continue;
        const prefix=`${ownerName}/${calleeName}/${format}/${orientation}`;
        progress.stage=prefix+'/load';
        const bytes=orientedBytes(format,orientation),mime=__exifSources[format].mime;
        const blob=new owner.Blob([bytes],{type:mime}),url=owner.URL.createObjectURL(blob);
        const image=new owner.Image(),svg=owner.document.createElementNS('http://www.w3.org/2000/svg','image');
        try{
          await load(owner,image,url,false);await load(owner,svg,url,true);
          // Canvas and ImageBitmap use media data, independently of CSS.
          image.style.imageOrientation='none';svg.style.imageOrientation='none';
          for(const [name,source] of [['img',image],['svg',svg]])for(const offscreen of [false,true])
            await run(prefix+`/${name}/draw/${offscreen?'offscreen':'element'}`,()=>verify(callee,source,orientation,false,offscreen));
          const width=orientation>=5?32:48,height=orientation>=5?48:32;
          for(const [name,source] of [['img',image],['svg',svg],['blob',blob]]){
            let bitmap;
            await run(prefix+`/${name}/default`,async()=>{
              bitmap=await callee.createImageBitmap(source);
              assert(bitmap instanceof callee.ImageBitmap && bitmap.width===width && bitmap.height===height,'bitmap realm/dimensions');
              verify(callee,bitmap,orientation);
            });
            await run(prefix+`/${name}/clone`,()=>{
              const copy=callee.structuredClone(bitmap);
              try{assert(copy.width===width && copy.height===height,'clone dimensions');verify(callee,copy,orientation);}
              finally{copy.close();}
            });
            await run(prefix+`/${name}/transfer`,()=>{
              const original=callee.structuredClone(bitmap),copy=callee.structuredClone(original,{transfer:[original]});
              try{assert(original.width===0 && original.height===0,'transfer must detach source');verify(callee,copy,orientation);}
              finally{copy.close();}
            });
            if(bitmap)bitmap.close();
            await run(prefix+`/${name}/flipY`,async()=>{
              const copy=await callee.createImageBitmap(source,{imageOrientation:'flipY'});
              try{assert(copy.width===width && copy.height===height,'flip dimensions');verify(callee,copy,orientation,true);}
              finally{copy.close();}
            });
            await run(prefix+`/${name}/crop-resize`,async()=>{
              const copy=await callee.createImageBitmap(source,16,0,16,16,{resizeWidth:8,resizeHeight:8,resizeQuality:'pixelated'});
              try{assert(copy.width===8 && copy.height===8,'crop dimensions');const ctx=context(callee,8,8);ctx.drawImage(copy,0,0);pixel(ctx,4,4,colors[orders[orientation-1][1]]);}
              finally{copy.close();}
            });
            await run(prefix+`/${name}/resize-aspect`,async()=>{
              const copy=await callee.createImageBitmap(source,{resizeWidth:width/2});
              try{assert(copy.width===width/2 && copy.height===height/2,'resize uses oriented aspect ratio');}
              finally{copy.close();}
            });
            await run(prefix+`/${name}/unconverted`,async()=>{
              const copy=await callee.createImageBitmap(source,{colorSpaceConversion:'none'});
              try{assert(copy.width===width && copy.height===height,'unconverted dimensions');verify(callee,copy,orientation);}
              finally{copy.close();}
            });
          }
        }finally{image.remove();svg.remove();owner.URL.revokeObjectURL(url);}
      }
  }catch(error){checks.push({name:'EXIF fixture setup',passed:false,detail:String(error)});}
  globalThis.__uiEventResults={checks,total:checks.length};
})()
