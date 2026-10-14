(owner, callee, kind, label) => {
    const make = () => {
        const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(8,8);
        canvas.width = canvas.height = 8;
        return canvas;
    };
    const canvas = make(), ctx = canvas.getContext('2d');
    const iface = kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D';
    const clipMethod = callee[iface].prototype.clip;
    const clip = (...args) => Reflect.apply(clipMethod, ctx, args);
    const rows = [];
    const check = (name, run) => {
        try { rows.push({name: label + ':' + name, passed: run() === true}); }
        catch (error) { rows.push({name: label + ':' + name, passed: false, error: String(error)}); }
    };
    const is = (x,y,color) => Array.from(ctx.getImageData(x,y,1,1).data).join() === color;
    const red = '255,0,0,255', green = '0,255,0,255', blue = '0,0,255,255';
    const paint = color => { ctx.fillStyle=color; ctx.fillRect(0,0,8,8); };
    const fresh = () => { ctx.reset(); paint('red'); };
    const rect = (x,y,w,h) => { ctx.beginPath(); ctx.rect(x,y,w,h); };
    const throws = run => { try { run(); } catch(error) { return error instanceof callee.TypeError; } return false; };
    check('clip method length', () => clipMethod.length === 0);
    check('default clip restricts fillRect', () => { fresh(); rect(0,0,4,8); clip(); paint('lime'); return is(1,1,green) && is(6,1,red); });
    check('implicit close does not modify the current subpath', () => {
        fresh(); ctx.beginPath(); ctx.moveTo(0,0); ctx.lineTo(8,0); ctx.lineTo(8,8); clip();
        ctx.lineTo(0,8); const current=ctx.isPointInPath(1,7); paint('lime');
        return current && is(6,1,green) && is(1,6,red);
    });
    check('explicit clip snapshots Path2D without changing the default path', () => {
        fresh(); ctx.beginPath(); const p=new owner.Path2D(); p.rect(0,0,4,8); clip(p); p.rect(4,0,4,8); paint('lime');
        return is(1,1,green) && is(6,1,red) && !ctx.isPointInPath(1,1);
    });
    check('successive clips intersect', () => { fresh(); rect(0,0,6,8); clip(); rect(2,0,6,8); clip(); paint('lime'); return is(0,1,red)&&is(3,1,green)&&is(7,1,red); });
    check('evenodd double paths cancel', () => { fresh(); rect(0,0,8,8); ctx.rect(0,0,8,8); clip('evenodd'); paint('lime'); return is(3,3,red); });
    check('nonzero double paths retain area', () => { fresh(); rect(0,0,8,8); ctx.rect(0,0,8,8); clip('nonzero'); paint('lime'); return is(3,3,green); });
    check('evenodd holes survive later nonzero clips', () => {
        fresh(); rect(0,0,8,8); ctx.rect(2,2,4,4); clip('evenodd'); rect(0,0,8,8); clip(); paint('lime'); return is(0,0,green)&&is(3,3,red);
    });
    check('default clip uses the transform frozen when recorded', () => { fresh(); ctx.translate(2,0); rect(0,0,2,8); ctx.resetTransform(); clip(); paint('lime'); return is(2,1,green)&&is(0,1,red)&&is(5,1,red); });
    check('explicit clip freezes the CTM at invocation', () => {
        fresh(); const p=new owner.Path2D(); p.rect(0,0,2,8); ctx.translate(4,0); clip(p); ctx.resetTransform(); paint('lime'); return is(4,1,green)&&is(0,1,red);
    });
    check('save and restore retain immutable intersected regions', () => {
        fresh(); ctx.save(); rect(0,0,4,8); clip(); ctx.save(); rect(0,0,8,4); clip(); paint('lime');
        const nested=is(1,1,green)&&is(1,6,red)&&is(6,1,red);
        ctx.restore(); paint('blue'); const restored=is(1,6,blue)&&is(6,1,red);
        ctx.restore(); paint('lime'); return nested&&restored&&is(6,1,green);
    });
    check('restore does not restore the current path', () => {
        fresh(); rect(0,0,4,8); ctx.save(); rect(4,0,4,8); ctx.restore(); return !ctx.isPointInPath(1,1)&&ctx.isPointInPath(6,1);
    });
    check('empty clip blocks fill stroke image text and clearing', () => {
        fresh(); ctx.beginPath(); clip(); paint('lime'); ctx.strokeStyle='blue'; ctx.strokeRect(0,0,8,8);
        const source=make(); const sc=source.getContext('2d'); sc.fillStyle='blue'; sc.fillRect(0,0,8,8);
        ctx.drawImage(source,0,0); ctx.fillText('MMMM',0,7); ctx.clearRect(0,0,8,8);
        return Array.from(ctx.getImageData(0,0,8,8).data).every((value,index)=>value===[255,0,0,255][index%4]);
    });
    check('putImageData ignores the drawing clip', () => {
        fresh(); ctx.beginPath(); clip(); const data=new owner.ImageData(new owner.Uint8ClampedArray([0,0,255,255]),1,1); ctx.putImageData(data,3,3); paint('lime'); return is(3,3,blue)&&is(1,1,red);
    });
    check('reset restores an unbounded clip', () => { fresh(); ctx.beginPath(); clip(); ctx.reset(); paint('lime'); return is(6,6,green); });
    check('dimension reset restores an unbounded clip', () => { fresh(); ctx.beginPath(); clip(); canvas.width=8; paint('lime'); return is(6,6,green); });
    check('clearRect respects the clipping region', () => { fresh(); rect(0,0,4,8); clip(); ctx.clearRect(0,0,8,8); return is(1,1,'0,0,0,0')&&is(6,1,red); });
    check('clearRect applies CTM before clipping', () => { fresh(); rect(2,0,4,8); clip(); ctx.translate(2,0); ctx.clearRect(0,0,2,8); return is(3,1,'0,0,0,0')&&is(1,1,red)&&is(5,1,red); });
    check('clearRect ignores color alpha shadows and compositing', () => {
        fresh(); rect(0,0,4,8); clip(); ctx.globalAlpha=0; ctx.shadowColor='blue'; ctx.shadowOffsetX=4; ctx.globalCompositeOperation='destination-out'; ctx.clearRect(0,0,8,8); return is(1,1,'0,0,0,0')&&is(6,1,red);
    });
    check('opaque contexts clear clipped pixels to black', () => {
        const canvas=make(), c=canvas.getContext('2d',{alpha:false}); c.fillStyle='red'; c.fillRect(0,0,8,8); c.beginPath(); c.rect(0,0,4,8);
        Reflect.apply(clipMethod,c,[]); c.clearRect(0,0,8,8);
        return Array.from(c.getImageData(1,1,1,1).data).join()==='0,0,0,255'&&Array.from(c.getImageData(6,1,1,1).data).join()===red;
    });
    check('drawImage uses the same final clip', () => {
        fresh(); rect(0,0,4,8); clip(); const source=make(),c=source.getContext('2d'); c.fillStyle='blue';c.fillRect(0,0,8,8);ctx.drawImage(source,0,0);return is(1,1,blue)&&is(6,1,red);
    });
    check('stroke uses the same final clip', () => {
        fresh(); rect(0,0,4,8); clip(); ctx.beginPath();ctx.moveTo(0,4);ctx.lineTo(8,4);ctx.strokeStyle='blue';ctx.lineWidth=2;ctx.stroke();return is(1,4,blue)&&is(6,4,red);
    });
    check('text uses the same final clip', () => {
        fresh();rect(0,0,4,8);clip();ctx.fillStyle='blue';ctx.fillText('MMMM',0,7);
        let inside=false,outside=false;
        for(let y=0;y<8;y++)for(let x=0;x<8;x++)if(!is(x,y,red)){if(x<4)inside=true;else outside=true;}
        return inside&&!outside;
    });
    check('enum conversion happens before default geometry snapshot', () => {
        fresh();ctx.beginPath();clip({toString(){ctx.rect(0,0,4,8);return 'nonzero';}});paint('lime');return is(1,1,green)&&is(6,1,red);
    });
    check('enum conversion happens before explicit geometry and CTM snapshot', () => {
        fresh();const p=new owner.Path2D();clip(p,{toString(){p.rect(0,0,4,8);ctx.translate(2,0);return 'nonzero';}});ctx.resetTransform();paint('lime');return is(3,1,green)&&is(0,1,red);
    });
    check('invalid enum leaves the current clip unchanged', () => { fresh();rect(0,0,4,8);clip();const rejected=throws(()=>clip('bad'));paint('lime');return rejected&&is(1,1,green)&&is(6,1,red); });
    check('conversion exception keeps its identity and the current clip', () => {
        fresh();rect(0,0,4,8);clip();const marker={};let same=false;try{clip({toString(){throw marker;}});}catch(error){same=error===marker;}paint('lime');return same&&is(1,1,green)&&is(6,1,red);
    });
    check('getImageData ignores the drawing clip', () => { fresh();ctx.beginPath();clip();return is(7,7,red); });
    const revoked=owner.Proxy.revocable(ctx,{});revoked.revoke();
    const receivers=[null,undefined,{},Object.create(callee[iface].prototype),Object.create(ctx),new owner.Proxy(ctx,{}),revoked.proxy];
    for(const [index,receiver] of receivers.entries()) {
        check('clip receiver before conversion '+index,()=>{let count=0;return throws(()=>Reflect.apply(clipMethod,receiver,[{toString(){count++;return 'nonzero';}}]))&&count===0;});
        check('clearRect receiver before conversion '+index,()=>{let count=0;return throws(()=>Reflect.apply(callee[iface].prototype.clearRect,receiver,[{valueOf(){count++;return 0;}},0,8,8]))&&count===0;});
    }
    const path=new owner.Path2D(),revokedPath=owner.Proxy.revocable(path,{});revokedPath.revoke();
    for(const [index,p] of [null,undefined,{},Object.create(owner.Path2D.prototype),Object.create(path),new owner.Proxy(path,{}),revokedPath.proxy].entries()) {
        check('invalid explicit path before enum conversion '+index,()=>{let count=0;return throws(()=>clip(p,{toString(){count++;return 'nonzero';}}))&&count===0;});
    }
    return rows;
}
