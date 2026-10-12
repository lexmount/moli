(owner, callee, kind, label) => {
    const checks = [];
    const check = (name, run) => {
        try { checks.push({name: label + ':' + name, passed: run() === true}); }
        catch (error) { checks.push({name: label + ':' + name, passed: false, error: String(error)}); }
    };
    const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(6, 5);
    canvas.width = 6; canvas.height = 5;
    const context = canvas.getContext('2d');
    const proto = callee[kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D'].prototype;
    const get = (...args) => Reflect.apply(proto.getImageData, context, args);
    const put = (...args) => Reflect.apply(proto.putImageData, context, args);
    const source = new owner.ImageData(4, 3);
    for (let i = 0; i < 12; i++) source.data.set([(i*17+1)%256, (i*31+9)%256, (i*53+23)%256, 255], i*4);
    const same = (actual, expected) => actual.length === expected.length && actual.every((v, i) => v === expected[i]);
    let seed = 719;
    const next = () => { seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0; return seed; };
    const dirtyCases = [];
    for (const destination of [-3, 0, 2]) for (const start of [-2, 0, 1, 4]) for (const span of [-5, -2, 0, 1, 3, 5]) {
        dirtyCases.push([destination, 0, start, 0, span, 3]);
        dirtyCases.push([0, destination, 0, start, 4, span]);
    }
    for (let i = 0; i < 160; i++) dirtyCases.push(Array.from({length: 6}, () => next()%13 - 6));
    for (const bound of [-2147483648, 2147483647]) {
        dirtyCases.push([0, 0, bound, 0, bound, 3], [0, 0, 1, 0, bound, 3],
                        [0, 0, 0, bound, 4, bound], [bound, 0, 0, 0, 4, 3],
                        [0, bound, 0, 0, 4, 3], [0, 0, bound, bound, 2, 2]);
    }
    dirtyCases.forEach(([dx, dy, x, y, w, h], index) => check('dirty rectangle ' + index, () => {
        canvas.width = 6;
        put(source, dx, dy, x, y, w, h);
        const expected = [];
        // Independent per-destination-pixel oracle: do not duplicate clipping/copy code.
        for (let py = 0; py < 5; py++) for (let px = 0; px < 6; px++) {
            const sx = px-dx, sy = py-dy;
            const selected = sx >= 0 && sx < 4 && sy >= 0 && sy < 3 &&
                sx >= Math.min(x,x+w) && sx < Math.max(x,x+w) &&
                sy >= Math.min(y,y+h) && sy < Math.max(y,y+h);
            expected.push(...(selected ? Array.from(source.data.slice((sy*4+sx)*4,(sy*4+sx+1)*4)) : [0,0,0,0]));
        }
        return same(Array.from(get(0,0,6,5).data), expected);
    }));
    const readCases=[];
    for(const x of [-2,0,2,5]) for(const y of [-2,0,2,4])
        for(const [w,h] of [[3,2],[-3,2],[3,-2],[-3,-2]]) readCases.push([x,y,w,h]);
    for(const bound of [-2147483648,2147483647])
        readCases.push([bound,0,-2,2],[0,bound,2,-2],[bound,bound,-2,-2]);
    readCases.forEach(([x,y,w,h],index)=>check('read rectangle '+index,()=>{
        canvas.width=6;put(source,1,1);
        const data=get(x,y,w,h), expected=[];
        for(let py=0;py<Math.abs(h);py++) for(let px=0;px<Math.abs(w);px++) {
            const sx=Math.min(x,x+w)+px-1,sy=Math.min(y,y+h)+py-1;
            expected.push(...(sx>=0&&sx<4&&sy>=0&&sy<3 ? Array.from(source.data.slice((sy*4+sx)*4,(sy*4+sx+1)*4)) : [0,0,0,0]));
        }
        return data.width===Math.abs(w)&&data.height===Math.abs(h)&&same(Array.from(data.data),expected);
    }));
    check('dirty write ignores drawing state',()=>{
        canvas.width=6;
        context.translate(100,100);context.globalAlpha=0;context.globalCompositeOperation='destination-out';
        context.beginPath();context.rect(0,0,0,0);context.clip();
        put(source,0,0,1,1,2,1);
        return same(Array.from(get(1,1,2,1).data),Array.from(source.data.slice(20,28)));
    });
    check('numeric conversion can resize canvas',()=>{
        canvas.width=6;
        put(source,{valueOf(){canvas.width=7;return 1;}},0,1,0,1,1);
        return same(Array.from(get(2,0,1,1).data),Array.from(source.data.slice(4,8)));
    });
    for(const name of ['getImageData','putImageData']) {
        const revoked=owner.Proxy.revocable(context,{});revoked.revoke();
        let hooks=0;
        const author=new owner.Proxy(context,{get(){hooks++;throw Error('trap');},getPrototypeOf(){hooks++;throw Error('trap');}});
        const receivers=[null,undefined,{},Object.create(proto),Object.create(context),author,revoked.proxy];
        if(owner.document) receivers.push(kind==='element'?new owner.OffscreenCanvas(1,1).getContext('2d'):owner.document.createElement('canvas').getContext('2d'));
        receivers.forEach((receiver,index)=>check(name+' receiver '+index,()=>{
            const arg={valueOf(){hooks++;return 1;}};
            try { Reflect.apply(proto[name],receiver,name==='getImageData'?[arg,0,1,1]:[source,arg,0]); }
            catch(error) { return error instanceof callee.TypeError&&hooks===0; }
            return false;
        }));
    }
    for(let count=0;count<7;count++) if(count!==3) check('put arity '+count,()=>{
        let conversions=0;
        const args=[source,...Array.from({length:6},()=>({valueOf(){conversions++;return 0;}}))].slice(0,count);
        try{put(...args);}catch(error){return error instanceof callee.TypeError&&conversions===0;}
        return false;
    });
    const invalidSources=[{},Object.create(owner.ImageData.prototype),Object.create(source),new owner.Proxy(source,{})];
    const revokedSource=owner.Proxy.revocable(source,{});revokedSource.revoke();invalidSources.push(revokedSource.proxy);
    invalidSources.forEach((image,index)=>check('source brand '+index,()=>{
        let conversions=0;
        try{put(image,{valueOf(){conversions++;return 0;}},0);}catch(error){return error instanceof callee.TypeError&&conversions===0;}
        return false;
    }));
    for(const dirty of [false,true]) check('detached buffer '+dirty,()=>{
        const image=new owner.ImageData(1,1);owner.structuredClone(image.data.buffer,{transfer:[image.data.buffer]});
        try{put(image,0,0,...(dirty?[0,0,0,0]:[]));}
        catch(error){return error.name==='InvalidStateError'&&error instanceof callee.DOMException;}
        return false;
    });
    check('conversion detaches source before dirty clipping',()=>{
        const image=new owner.ImageData(1,1);
        try{put(image,{valueOf(){owner.structuredClone(image.data.buffer,{transfer:[image.data.buffer]});return 0;}},0,2147483647,0,-2147483648,0);}
        catch(error){return error.name==='InvalidStateError'&&error instanceof callee.DOMException;}
        return false;
    });
    const create = (...args) => Reflect.apply(proto.createImageData, context, args);
    check('create length is shortest overload',()=>proto.createImageData.length===1);
    check('create negative dimensions allocate blank data',()=>{
        const data=create(-2,-3);return data.width===2&&data.height===3&&Array.from(data.data).every(v=>v===0);
    });
    check('create single source clones dimensions without pixels',()=>{
        const data=create(source);return data.width===4&&data.height===3&&Array.from(data.data).every(v=>v===0)&&data.data!==source.data;
    });
    check('create overload selected before source inspection',()=>{
        const image=new owner.ImageData(4,3);let conversions=0;
        image.valueOf=()=>{conversions++;return -2;};
        const data=create(image,3);return conversions===1&&data.width===2&&data.height===3;
    });
    check('create does not convert non-interface single argument',()=>{
        let conversions=0;
        try{create({valueOf(){conversions++;return 2;}});}
        catch(error){return error instanceof callee.TypeError&&conversions===0;}
        return false;
    });
    check('create source dimensions survive detached pixels',()=>{
        const image=new owner.ImageData(2,1);owner.structuredClone(image.data.buffer,{transfer:[image.data.buffer]});
        const data=create(image);return data.width===2&&data.height===1&&data.data.length===8;
    });
    const revokedContext=owner.Proxy.revocable(context,{});revokedContext.revoke();
    const createReceivers=[null,undefined,{},Object.create(proto),Object.create(context),new owner.Proxy(context,{}),revokedContext.proxy];
    if(owner.document)createReceivers.push(kind==='element'?new owner.OffscreenCanvas(1,1).getContext('2d'):owner.document.createElement('canvas').getContext('2d'));
    createReceivers.forEach((receiver,index)=>check('create receiver '+index,()=>{
        let conversions=0;
        try{Reflect.apply(proto.createImageData,receiver,[{valueOf(){conversions++;return 1;}},1]);}
        catch(error){return error instanceof callee.TypeError&&conversions===0;}
        return false;
    }));
    return checks;
}
