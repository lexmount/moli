(owner, callee, kind, label) => {
    const rows = [];
    const check = (name, fn) => { try { rows.push({name:label+'/'+name,passed:!!fn()}); } catch(error) { rows.push({name:label+'/'+name,passed:false,error:error.name+': '+error.message}); } };
    const make = () => {const c=kind==='element'?owner.document.createElement('canvas'):new owner.OffscreenCanvas(4,4);c.width=4;c.height=4;return c;};
    const canvas=make(),ctx=canvas.getContext('2d');
    const iface=kind==='element'?'CanvasRenderingContext2D':'OffscreenCanvasRenderingContext2D';
    const methods=callee[iface].prototype;
    const linear=(...args)=>Reflect.apply(methods.createLinearGradient,ctx,args);
    const radial=(...args)=>Reflect.apply(methods.createRadialGradient,ctx,args);
    const conic=(...args)=>Reflect.apply(methods.createConicGradient,ctx,args);
    const stop=(gradient,...args)=>Reflect.apply(callee.CanvasGradient.prototype.addColorStop,gradient,args);
    const pixel=(x=0,y=0)=>Array.from(ctx.getImageData(x,y,1,1).data);
    const near=(expected,x=0,y=0)=>pixel(x,y).every((v,i)=>Math.abs(v-expected[i])<=2);
    const fresh=()=>{ctx.reset();};
    const paint=(g)=>{ctx.fillStyle=g;ctx.fillRect(0,0,4,4);};
    const throws=(fn,name)=>{try{fn();return false;}catch(error){return error.name===name&&(name==='TypeError'?Object.getPrototypeOf(error)===callee.TypeError.prototype:error instanceof callee.DOMException);}};
    check('factory descriptors',()=>methods.createLinearGradient.length===4&&methods.createRadialGradient.length===6&&methods.createConicGradient.length===3&&callee.CanvasGradient.prototype.addColorStop.length===2);
    check('factory realm and style identity',()=>{fresh();const g=linear(0,0,4,0);ctx.fillStyle=g;ctx.strokeStyle=g;return Object.getPrototypeOf(g)===owner.CanvasGradient.prototype&&ctx.fillStyle===g&&ctx.strokeStyle===g;});
    check('linear colors use bitmap pixel centers',()=>{fresh();const g=linear(0,0,4,0);stop(g,0,'red');stop(g,1,'blue');paint(g);return near([223,0,32,255],0)&&near([32,0,223,255],3);});
    check('empty gradients paint transparent black',()=>{fresh();ctx.fillStyle='lime';ctx.fillRect(0,0,4,4);paint(linear(0,0,4,0));return near([0,255,0,255]);});
    check('one stop extends across the whole gradient',()=>{fresh();const g=linear(0,0,4,0);stop(g,0.5,'rgba(0,0,255,0.5)');paint(g);return near([0,0,255,128],0)&&near([0,0,255,128],3);});
    check('colors interpolate without premultiplication',()=>{fresh();const g=linear(0,0,1,0);stop(g,0,'red');stop(g,1,'rgba(0,0,255,0)');paint(g);return near([128,0,128,128]);});
    check('equal offsets preserve insertion order',()=>{fresh();const g=linear(0,0,1,0);stop(g,0.5,'red');stop(g,-0,'red');stop(g,0.5,'blue');paint(g);return near([0,0,255,255]);});
    check('degenerate linear gradients paint nothing',()=>{fresh();ctx.fillStyle='lime';ctx.fillRect(0,0,4,4);const g=linear(1,1,1,1);stop(g,0,'red');paint(g);return near([0,255,0,255]);});
    check('gradient is live after assignment',()=>{fresh();const g=linear(0,0,4,0);stop(g,0,'red');stop(g,1,'blue');ctx.fillStyle=g;stop(g,0,'lime');ctx.fillRect(0,0,4,4);return near([0,223,32,255]);});
    check('saved state keeps the same live gradient',()=>{fresh();const g=linear(0,0,4,0);stop(g,0,'red');ctx.fillStyle=g;ctx.save();ctx.fillStyle='blue';stop(g,0,'lime');ctx.restore();ctx.fillRect(0,0,4,4);return ctx.fillStyle===g&&near([0,255,0,255]);});
    check('gradients can be reused by another context',()=>{fresh();const g=linear(0,0,4,0);stop(g,0,'blue');const other=make().getContext('2d');other.fillStyle=g;other.fillRect(0,0,4,4);return other.fillStyle===g&&Array.from(other.getImageData(1,1,1,1).data).join()==='0,0,255,255';});
    check('gradient coordinates use the draw-time CTM',()=>{fresh();const g=linear(0,0,4,0);stop(g,0,'red');stop(g,1,'blue');ctx.scale(2,1);paint(g);return near([239,0,16,255],0)&&near([143,0,112,255],3);});
    check('gradient creation does not freeze the CTM',()=>{fresh();ctx.scale(2,1);const g=linear(0,0,4,0);stop(g,0,'red');stop(g,1,'blue');ctx.resetTransform();paint(g);return near([223,0,32,255]);});
    check('linear native range preserves huge endpoints',()=>{fresh();const g=linear(-Number.MAX_VALUE,0,Number.MAX_VALUE,0);stop(g,0,'red');stop(g,1,'blue');paint(g);return near([128,0,128,255]);});
    check('radial concentric rings',()=>{fresh();const g=radial(0.5,0.5,0,0.5,0.5,4);stop(g,0,'red');stop(g,1,'blue');paint(g);return near([128,0,128,255],2);});
    check('radial reverse radii select a positive circle',()=>{fresh();const g=radial(0.5,0.5,4,0.5,0.5,0);stop(g,0,'red');stop(g,1,'blue');paint(g);return near([128,0,128,255],2);});
    check('radial degenerate cone paints nothing',()=>{fresh();ctx.fillStyle='lime';ctx.fillRect(0,0,4,4);const g=radial(0,0,2,0,0,2);stop(g,0,'red');paint(g);return near([0,255,0,255]);});
    check('radial outside a constant-radius cone is transparent',()=>{fresh();const g=radial(0,0,0.25,4,0,0.25);stop(g,0,'red');paint(g);return near([0,0,0,0],1,1);});
    check('conic starts on the positive x axis',()=>{fresh();const g=conic(0,0.5,0.5);stop(g,0,'red');stop(g,1,'blue');paint(g);return near([255,0,0,255],1,0)&&near([191,0,64,255],0,1);});
    check('invalid color uses SyntaxError',()=>throws(()=>stop(linear(0,0,4,0),0,'definitely-not-a-color'),'SyntaxError'));
    check('invalid offset uses IndexSizeError',()=>throws(()=>stop(linear(0,0,4,0),2,'red'),'IndexSizeError'));
    check('nonfinite offset rejects before color conversion',()=>{let count=0;return throws(()=>stop(linear(0,0,4,0),Infinity,{toString(){count++;return 'red';}}),'TypeError')&&count===0;});
    check('color conversion precedes offset range validation',()=>{const marker={};try{stop(linear(0,0,4,0),2,{toString(){throw marker;}});}catch(error){return error===marker;}return false;});
    check('factory nonfinite rejection precedes later conversion',()=>{let count=0;return throws(()=>linear(Infinity,0,{valueOf(){count++;return 4;}},0),'TypeError')&&count===0;});
    check('negative radius uses IndexSizeError',()=>throws(()=>radial(0,0,-1,0,0,1),'IndexSizeError'));
    check('radius validation follows all argument conversion',()=>{const marker={};try{radial(0,0,-1,0,0,{valueOf(){throw marker;}});}catch(error){return error===marker;}return false;});
    check('paint union falls back to DOMString for ordinary objects',()=>{fresh();let count=0;ctx.fillStyle={toString(){count++;return 'blue';}};ctx.fillRect(0,0,4,4);return count===1&&near([0,0,255,255]);});
    check('gradient color parsing is canvas-neutral',()=>{fresh();if(canvas.style)canvas.style.color='red';const g=linear(0,0,4,0);stop(g,0,'currentColor');paint(g);return near([0,0,0,255]);});
    check('gradient colors convert into receiving float16 P3 context',()=>{const g=linear(0,0,4,0);stop(g,0,'color(display-p3 1 0.25 0.5 / 0.5)');const other=make().getContext('2d',{colorSpace:'display-p3',colorType:'float16'});other.fillStyle=g;other.fillRect(0,0,4,4);const p=other.getImageData(0,0,1,1,{colorSpace:'display-p3',pixelFormat:'rgba-float16'}).data;return p.every((v,i)=>Math.abs(v-[1,0.25,0.5,0.5][i])<0.003);});
    const revoked=owner.Proxy.revocable(ctx,{});revoked.revoke();
    for(const [index,receiver] of [{},Object.create(ctx),new owner.Proxy(ctx,{}),revoked.proxy].entries()) {
        for(const method of ['createLinearGradient','createRadialGradient','createConicGradient']) {
            check(method+' receiver before conversion '+index,()=>{let count=0;return throws(()=>Reflect.apply(methods[method],receiver,[{valueOf(){count++;return 0;}},0,0,0,0,1]),'TypeError')&&count===0;});
        }
    }
    const real=linear(0,0,4,0),revokedGradient=owner.Proxy.revocable(real,{});revokedGradient.revoke();
    for(const [index,receiver] of [{},Object.create(real),new owner.Proxy(real,{}),revokedGradient.proxy].entries()) {
        check('addColorStop receiver before conversion '+index,()=>{let count=0;return throws(()=>stop(receiver,{valueOf(){count++;return 0;}},'red'),'TypeError')&&count===0;});
    }
    return rows;
}
