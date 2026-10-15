(owner, callee, kind, label, videoFrameExposed = true) => {
    const rows = [], colors = [[255,0,0,255],[0,255,0,255],[0,0,255,255],[255,255,255,255]];
    const assert = (ok, message='assertion failed') => {if (!ok) throw Error(message);};
    const check = (name, run) => {
        try {run(); rows.push({name:label+': '+name,passed:true});}
        catch (error) {rows.push({name:label+': '+name,passed:false,error:String(error)});}
    };
    const proto = callee[kind==='element'?'CanvasRenderingContext2D':'OffscreenCanvasRenderingContext2D'].prototype;
    const call = (ctx,name,...args) => Reflect.apply(proto[name],ctx,args);
    const make = () => {
        const canvas = kind==='element'?owner.document.createElement('canvas'):new owner.OffscreenCanvas(8,8);
        canvas.width=8;canvas.height=8;
        const ctx=canvas.getContext('2d');ctx.imageSmoothingEnabled=false;
        return [canvas,ctx];
    };
    const source = () => {
        const canvas=new owner.OffscreenCanvas(2,2);
        canvas.getContext('2d').putImageData(new owner.ImageData(new owner.Uint8ClampedArray(colors.flat()),2,2),0,0);
        return canvas;
    };
    const pixel = (ctx,x,y) => Array.from(call(ctx,'getImageData',x,y,1,1).data);
    const near = (actual,expected,tolerance=2) => assert(actual.every((v,i)=>Math.abs(v-expected[i])<=tolerance),actual+' != '+expected);
    const red = ctx => {ctx.fillStyle='red';call(ctx,'fillRect',0,0,8,8);};
    const matrices = [
        ['identity',[1,0,0,1,0,0]], ['translation',[1,0,0,1,3,2]],
        ['scale',[2,0,0,2,1,1]], ['reflection',[-1,0,0,1,5,1]],
        ['rotation',[0,1,-1,0,5,1]], ['x shear',[1,0,1,1,1,1]],
        ['y shear',[1,1,0,1,1,1]], ['anisotropic scale',[3,0,0,2,1,1]],
    ];
    const palette = (matrix,args) => {
        const [,ctx]=make();call(ctx,'setTransform',...matrix);call(ctx,'drawImage',source(),...args);
        const [a,b,c,d,e,f]=matrix,det=a*d-b*c;
        const local = (x,y) => [(d*(x-e)-c*(y-f))/det,(-b*(x-e)+a*(y-f))/det];
        let inside=0,checked=0;
        for(let y=0;y<8;y++)for(let x=0;x<8;x++) {
            const corners=[[x,y],[x+1,y],[x+1,y+1],[x,y+1]].map(p=>local(...p));
            if(corners.every(([x,y])=>x>=0&&x<=2&&y>=0&&y<=2)) {
                const [sx,sy]=local(x+0.5,y+0.5);
                const indices = value => (Number.isInteger(value)?[value-1,value]:[Math.floor(value)]).map(v=>Math.max(0,Math.min(1,v)));
                const actual=pixel(ctx,x,y),candidates=indices(sy).flatMap(y=>indices(sx).map(x=>colors[y*2+x]));
                // Nearest-neighbor ties on an exact texel boundary can select
                // either neighbor. Geometry and all untied samples stay exact.
                assert(candidates.some(expected=>actual.every((v,i)=>Math.abs(v-expected[i])<=2)),actual+' is not a nearest sample');inside++;checked++;
            } else if([0,1].some(axis=>corners.every(p=>p[axis]<=0)||corners.every(p=>p[axis]>=2))) {
                near(pixel(ctx,x,y),[0,0,0,0]);checked++;
            }
        }
        assert(inside>0&&checked>inside,'matrix did not check inside and outside pixels');
    };
    for(const [name,matrix] of matrices)for(const [overload,args] of [[3,[0,0]],[5,[0,0,2,2]],[9,[0,0,2,2,0,0,2,2]]])
        check(name+' '+overload+' arguments',()=>palette(matrix,args));
    for(const [name,args] of [
        ['negative destination width',[2,0,-2,2]],['negative destination height',[0,2,2,-2]],
        ['negative source width',[2,0,-2,2,0,0,2,2]],['negative source and destination',[2,2,-2,-2,2,2,-2,-2]],
    ])check(name,()=>palette([1,0,0,1,0,0],args));
    check('source clipping proportionally clips transformed destination',()=>{
        const [,ctx]=make();call(ctx,'translate',1,1);call(ctx,'drawImage',source(),-1,0,3,2,0,0,6,4);
        near(pixel(ctx,1,1),[0,0,0,0]);near(pixel(ctx,3,1),colors[0]);near(pixel(ctx,5,1),colors[1]);
        near(pixel(ctx,3,3),colors[2]);near(pixel(ctx,5,3),colors[3]);
    });
    check('tiny nonzero source width is retained when scaled up',()=>{
        const [,ctx]=make();call(ctx,'drawImage',source(),0.5,0,1e-200,2,0,0,4,4);
        near(pixel(ctx,0,0),colors[0]);near(pixel(ctx,3,0),colors[0]);near(pixel(ctx,0,3),colors[2]);
    });
    for(const scale of [1e200,1e-200])check('finite extreme scale '+scale,()=>{
        const [,ctx]=make();call(ctx,'setTransform',scale,0,0,scale,0,0);
        call(ctx,'drawImage',source(),0,0,4/scale,4/scale);
        near(pixel(ctx,0,0),colors[0]);near(pixel(ctx,3,0),colors[1]);near(pixel(ctx,0,3),colors[2]);near(pixel(ctx,3,3),colors[3]);
    });
    check('large translation cancellation retains pixel-sized destination',()=>{
        const [,ctx]=make();call(ctx,'setTransform',1e200,0,0,1e200,-1e200,-1e200);
        call(ctx,'drawImage',source(),1,1,4e-200,4e-200);
        near(pixel(ctx,0,0),colors[0]);near(pixel(ctx,3,0),colors[1]);near(pixel(ctx,0,3),colors[2]);near(pixel(ctx,3,3),colors[3]);
    });
    check('copy composes transparent source outside transformed destination',()=>{
        const [,ctx]=make();red(ctx);ctx.globalCompositeOperation='copy';call(ctx,'translate',20,0);call(ctx,'drawImage',source(),0,0);
        near(pixel(ctx,0,0),[0,0,0,0]);near(pixel(ctx,7,7),[0,0,0,0]);
    });
    check('singular CTM does not compose',()=>{
        const [,ctx]=make();red(ctx);ctx.globalCompositeOperation='copy';call(ctx,'scale',0,1);call(ctx,'drawImage',source(),0,0);
        near(pixel(ctx,0,0),colors[0]);near(pixel(ctx,7,7),colors[0]);
    });
    check('completely clipped source rectangle does not compose',()=>{
        const [,ctx]=make();red(ctx);ctx.globalCompositeOperation='copy';call(ctx,'drawImage',source(),10,10,2,2,0,0,2,2);
        near(pixel(ctx,0,0),colors[0]);near(pixel(ctx,7,7),colors[0]);
    });
    for(const args of [[0,0,0,2],[0,0,2,0],[0,0,0,2,0,0,2,2]])check('zero rectangle '+args.join(','),()=>{
        const [,ctx]=make();red(ctx);ctx.globalCompositeOperation='copy';call(ctx,'drawImage',source(),...args);
        near(pixel(ctx,0,0),colors[0]);near(pixel(ctx,7,7),colors[0]);
    });
    check('shadow follows transformed image with device-space offset',()=>{
        const [,ctx]=make();ctx.shadowColor='blue';ctx.shadowOffsetX=-3;call(ctx,'setTransform',2,0,0,2,4,4);call(ctx,'drawImage',source(),0,0);
        near(pixel(ctx,1,4),[0,0,255,255]);near(pixel(ctx,4,4),colors[0]);
    });
    check('source clip does not remove a transformed image shadow',()=>{
        const [,ctx]=make();call(ctx,'rect',0,0,2,8);call(ctx,'clip');ctx.shadowColor='blue';ctx.shadowOffsetX=-3;
        call(ctx,'setTransform',2,0,0,2,4,4);call(ctx,'drawImage',source(),0,0);
        near(pixel(ctx,1,4),[0,0,255,255]);near(pixel(ctx,4,4),[0,0,0,0]);
    });
    check('self draw snapshots before transformed copy',()=>{
        const [canvas,ctx]=make();ctx.fillStyle='red';call(ctx,'fillRect',0,0,2,2);ctx.globalCompositeOperation='copy';
        call(ctx,'setTransform',-1,0,0,1,4,0);call(ctx,'drawImage',canvas,0,0);
        near(pixel(ctx,0,0),[0,0,0,0]);near(pixel(ctx,3,0),colors[0]);
    });
    check('numeric conversions precede CTM snapshot',()=>{
        const [,ctx]=make();call(ctx,'drawImage',source(),{valueOf(){call(ctx,'translate',4,3);return 0;}},0);
        near(pixel(ctx,4,3),colors[0]);near(pixel(ctx,0,0),[0,0,0,0]);
    });
    check('conversion exceptions do not paint',()=>{
        const [,ctx]=make();red(ctx);const sentinel={};let caught;
        try{call(ctx,'drawImage',source(),{valueOf(){call(ctx,'translate',4,3);return 0;}},{valueOf(){throw sentinel;}});}catch(error){caught=error;}
        assert(caught===sentinel);near(pixel(ctx,0,0),colors[0]);near(pixel(ctx,7,7),colors[0]);
    });
    check('drawing does not change the current path',()=>{
        const [,ctx]=make();call(ctx,'rect',6,6,2,2);call(ctx,'drawImage',source(),0,0);ctx.fillStyle='blue';call(ctx,'fill');
        near(pixel(ctx,6,6),[0,0,255,255]);near(pixel(ctx,0,0),colors[0]);
    });
    check('source dimensions use native slots',()=>{
        const [,ctx]=make(),image=source();
        for(const key of ['width','height'])Object.defineProperty(image,key,{get(){throw Error('author '+key);}});
        call(ctx,'translate',3,2);call(ctx,'drawImage',image,0,0);near(pixel(ctx,3,2),colors[0]);
    });
    check('HTMLCanvas source uses transformed native pixels',()=>{
        const [,ctx]=make(),image=owner.document?owner.document.createElement('canvas'):new owner.OffscreenCanvas(2,2);
        image.width=2;image.height=2;image.getContext('2d').putImageData(new owner.ImageData(new owner.Uint8ClampedArray(colors.flat()),2,2),0,0);
        call(ctx,'translate',3,2);call(ctx,'drawImage',image,0,0);near(pixel(ctx,3,2),colors[0]);near(pixel(ctx,4,3),colors[3]);
    });
    check('VideoFrame exposure and transformed native pixels',()=>{
        if(!videoFrameExposed) {assert(owner.VideoFrame===undefined,'VideoFrame is not exposed in this worker kind');return;}
        const [,ctx]=make(),frame=new owner.VideoFrame(new owner.Uint8Array(colors.flat()),{format:'RGBA',codedWidth:2,codedHeight:2,timestamp:1});
        try{call(ctx,'translate',3,2);call(ctx,'drawImage',frame,0,0);near(pixel(ctx,3,2),colors[0]);near(pixel(ctx,4,3),colors[3]);}finally{frame.close();}
    });
    check('bilinear sampling uses texel centers and full-image crop neighbors',()=>{
        const [,ctx]=make();ctx.imageSmoothingEnabled=true;call(ctx,'drawImage',source(),0,0,1,1,0,0,4,1);
        near(pixel(ctx,0,0),colors[0]);near(pixel(ctx,2,0),[223,32,0,255],3);near(pixel(ctx,3,0),[159,96,0,255],3);
    });
    check('transformed source obeys globalAlpha copy and final clip',()=>{
        const [,ctx]=make();red(ctx);call(ctx,'rect',2,2,4,4);call(ctx,'clip');ctx.globalCompositeOperation='copy';ctx.globalAlpha=0.5;
        call(ctx,'translate',4,2);call(ctx,'drawImage',source(),0,0);
        near(pixel(ctx,5,2),[0,255,0,128]);near(pixel(ctx,3,3),[0,0,0,0]);near(pixel(ctx,0,0),colors[0]);
    });
    return rows;
}
