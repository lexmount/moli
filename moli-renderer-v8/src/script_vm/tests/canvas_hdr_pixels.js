async (owner, callee, kind, label) => {
    const checks = [];
    const check = async (name, run) => {
        try { checks.push({name:label+':'+name, passed:await run() === true}); }
        catch (error) { checks.push({name:label+':'+name, passed:false, error:String(error)}); }
    };
    const make = (width, height, settings = {}) => {
        const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(width, height);
        canvas.width = width; canvas.height = height;
        return [canvas, canvas.getContext('2d', settings)];
    };
    const proto = callee[kind === 'element' ? 'CanvasRenderingContext2D' : 'OffscreenCanvasRenderingContext2D'].prototype;
    const get = (context, x=0, y=0, width=1, height=1, colorSpace='srgb') =>
        Reflect.apply(proto.getImageData, context, [x, y, width, height, {colorSpace, pixelFormat:'rgba-float16'}]);
    const put = (context, image, ...coordinates) => Reflect.apply(proto.putImageData, context, [image, ...coordinates]);
    const floats = (channels, colorSpace='srgb') => new owner.ImageData(new owner.Float16Array(channels), channels.length/4, 1, {colorSpace,pixelFormat:'rgba-float16'});
    const same = (actual, expected) => actual.length === expected.length && actual.every((value,index)=>Object.is(value,expected[index]) || value === expected[index]);
    const near = (actual, expected, epsilon=0.003) => actual.length === expected.length && actual.every((value,index)=>Math.abs(value-expected[index])<=epsilon);
    const read = (context, ...args) => Array.from(get(context, ...args).data);
    for (const colorSpace of ['srgb','display-p3']) for (const colorType of ['unorm8','float16']) {
        await check('native context settings '+colorSpace+' '+colorType, () => {
            const [canvas,context] = make(2,1,{colorSpace,colorType,willReadFrequently:true});
            const attributes = Reflect.apply(proto.getContextAttributes,context,[]);
            const other = Reflect.apply(proto.getContextAttributes,context,[]);
            other.colorSpace='author';
            const fresh = Reflect.apply(proto.createImageData,context,[1,1]);
            const image = Reflect.apply(proto.getImageData,context,[0,0,1,1]);
            return attributes !== other && attributes.alpha === true && attributes.colorSpace === colorSpace && attributes.colorType === colorType &&
                attributes.willReadFrequently === true && typeof attributes.desynchronized === 'boolean' && fresh.colorSpace === colorSpace && image.colorSpace === colorSpace &&
                canvas.getContext('2d',{get colorType(){throw Error('reacquired settings');}}) === context;
        });
        await check('quantization occurs at destination '+colorSpace+' '+colorType, () => {
            const [,context] = make(1,1,{colorSpace,colorType});
            put(context,floats([2,-1,0.5,1],colorSpace),0,0);
            if(!near(read(context,0,0,1,1,colorSpace), colorType === 'float16' ? [2,-1,0.5,1] : [1,0,128/255,1]))return false;
            put(context,floats([2,-1,0.5,0.5],colorSpace),0,0);
            return near(read(context,0,0,1,1,colorSpace), colorType === 'float16' ? [2,-1,0.5,0.5] : [1,0,0.5,128/255]);
        });
    }
    await check('context dictionary order and exception identity', () => {
        const canvas = kind === 'element' ? owner.document.createElement('canvas') : new owner.OffscreenCanvas(1,1);
        const log=[],sentinel=new owner.Error('settings');
        const settings={};
        for(const name of ['alpha','colorSpace','colorType','desynchronized','willReadFrequently'])
            Object.defineProperty(settings,name,{get(){log.push(name);if(name==='colorType')throw sentinel;return name==='colorSpace'?'srgb':true;}});
        try { canvas.getContext('2d',settings); } catch(error) {
            return error===sentinel && same(log,['alpha','colorSpace','colorType']) && canvas.getContext('2d',{colorType:'float16'}).getContextAttributes().colorType==='float16';
        }
        return false;
    });
    await check('attribute native receiver rejects author proxies', () => {
        const [,context]=make(1,1,{colorType:'float16'});
        let traps=0;
        try { Reflect.apply(proto.getContextAttributes,new owner.Proxy(context,{get(){traps++;}}),[]); }
        catch(error){return error instanceof callee.TypeError && traps===0;}
        return false;
    });
    await check('opaque contexts ignore input alpha and clear to opaque black', () => {
        const [,context]=make(2,1,{alpha:false,colorType:'float16'});
        if(!same(read(context),[0,0,0,1]))return false;
        put(context,floats([2,-1,0.5,0]),0,0);
        if(!same(read(context),[2,-1,0.5,1]))return false;
        context.clearRect(0,0,1,1);
        return same(read(context),[0,0,0,1]);
    });
    await check('opaque unorm8 output initializes and resets with alpha fixed', () => {
        const [canvas,context]=make(1,1,{alpha:false});
        if(!same(read(context),[0,0,0,1]))return false;
        put(context,new owner.ImageData(new owner.Uint8ClampedArray([255,128,64,0]),1,1),0,0);
        const image=Reflect.apply(proto.getImageData,context,[0,0,1,1]);
        if(!same(Array.from(image.data),[255,128,64,255]))return false;
        canvas.width=2;
        return same(read(context,0,0,2,1),[0,0,0,1,0,0,0,1]);
    });
    await check('dirty copies preserve unaffected HDR pixels', () => {
        const [,context]=make(3,1,{colorType:'float16'});
        put(context,floats([2,-1,0.5,1,3,-2,0.25,1,4,-3,0.125,1]),0,0);
        put(context,floats([8,-7,0.75,1,6,-5,0.625,1]),1,0,0,0,1,1);
        return same(read(context,0,0,3,1),[2,-1,0.5,1,8,-7,0.75,1,4,-3,0.125,1]);
    });
    await check('transparent ImageData RGB is discarded only by the output bitmap', () => {
        const [,context]=make(1,1,{colorType:'float16'}),source=floats([2,-1,0.5,0]);
        put(context,source,0,0);
        return same(read(context),[0,0,0,0]) && same(Array.from(source.data),[2,-1,0.5,0]);
    });
    await check('float16 alpha is retained through premultiplied storage', () => {
        const [,context]=make(1,1,{colorType:'float16'});
        for(const alpha of [0.1234,2,-1,NaN,Infinity]) {
            const source=floats([2,-1,0.5,alpha]);
            put(context,source,0,0);
            const expected=Number.isNaN(alpha) || alpha===Infinity ? [NaN,NaN,NaN,alpha] : Array.from(source.data);
            if(!same(read(context),expected))return false;
        }
        return true;
    });
    await check('partial writes retain uncovered semitransparent half pixels', () => {
        const [,context]=make(3,1,{colorType:'float16'});
        put(context,floats([0.000984,-1.25,2.5,0.1,2,-1,0.5,0.5,4,-3,0.125,0.25]),0,0);
        const before=read(context);
        put(context,floats([8,-7,0.75,0.5]),1,0,0,0,1,1);
        context.fillStyle='red';context.fillRect(1,0,1,1);context.clearRect(2,0,1,1);
        return same(read(context,0,0,3,1),[...before.slice(0,4),1,0,0,1,0,0,0,0]);
    });
    await check('modern CSS alpha is retained by float16 drawing', () => {
        const [,context]=make(1,1,{colorType:'float16'});
        context.fillStyle='color(srgb 1 0 0 / 0.123456)';context.fillRect(0,0,1,1);
        const expected=[1,0,0,new owner.Float16Array([0.123456])[0]],actual=read(context);
        if(!same(actual,expected))throw Error(JSON.stringify({actual,expected,style:context.fillStyle}));
        context.save();context.fillStyle='red';context.restore();context.clearRect(0,0,1,1);context.fillRect(0,0,1,1);
        return same(read(context),expected);
    });
    await check('fill and clear retain uncovered HDR values', () => {
        const [,context]=make(3,1,{colorType:'float16'});
        put(context,floats([2,-1,0.5,1,3,-2,0.25,1,4,-3,0.125,1]),0,0);
        context.fillStyle='blue';context.fillRect(1,0,1,1);context.clearRect(2,0,1,1);
        return same(read(context,0,0,3,1),[2,-1,0.5,1,0,0,1,1,0,0,0,0]);
    });
    await check('wide gamut fill is retained in sRGB HDR storage', () => {
        const [,context]=make(1,1,{colorType:'float16'});
        context.fillStyle='color(display-p3 1 0 0)';context.fillRect(0,0,1,1);
        const srgb=read(context);
        return srgb[0]>1 && srgb[1]<0 && srgb[2]<0 && near(read(context,0,0,1,1,'display-p3'),[1,0,0,1]);
    });
    await check('wide gamut get/put round trip', () => {
        const [,context]=make(1,1,{colorType:'float16'});
        put(context,floats([1,0,0,1],'display-p3'),0,0);
        const snapshot=get(context);
        context.fillStyle='blue';context.fillRect(0,0,1,1);
        put(context,snapshot,0,0);
        return near(read(context,0,0,1,1,'display-p3'),[1,0,0,1]);
    });
    for(const colorSpace of ['srgb','display-p3'])for(const premultiplyAlpha of ['none','premultiply','default']) {
        await check('ImageBitmap preserves source precision '+colorSpace+' '+premultiplyAlpha, async () => {
            const source=floats([1,2,-1,1],colorSpace);
            const pending=Reflect.apply(callee.createImageBitmap,callee,[source,{premultiplyAlpha}]);
            source.data.fill(0);
            const bitmap=await pending;
            try {
                const [,context]=make(1,1,{colorType:'float16',colorSpace});
                Reflect.apply(proto.drawImage,context,[bitmap,0,0]);
                return same(read(context,0,0,1,1,colorSpace),[1,2,-1,1]);
            } finally { bitmap.close(); }
        });
    }
    await check('P3 ImageBitmap keeps out of gamut color through sRGB HDR', async () => {
        const bitmap=await owner.createImageBitmap(new owner.ImageData(new owner.Uint8ClampedArray([255,0,0,255]),1,1,{colorSpace:'display-p3'}));
        try {
            const [,context]=make(1,1,{colorType:'float16'});
            Reflect.apply(proto.drawImage,context,[bitmap,0,0]);
            const p3=Reflect.apply(proto.getImageData,context,[0,0,1,1,{colorSpace:'display-p3'}]);
            return near(Array.from(p3.data),[255,0,0,255],1);
        } finally { bitmap.close(); }
    });
    await check('ImageBitmap crop resize and flip retain HDR', async () => {
        const source=new owner.ImageData(new owner.Float16Array([2,-1,0.5,1,3,-2,0.25,1,4,-3,0.125,1,5,-4,0.0625,1]),2,2,{pixelFormat:'rgba-float16'});
        const bitmap=await owner.createImageBitmap(source,0,0,1,2,{resizeWidth:2,resizeHeight:2,resizeQuality:'pixelated',imageOrientation:'flipY'});
        try {
            const [,context]=make(2,2,{colorType:'float16'});
            context.imageSmoothingEnabled=false;Reflect.apply(proto.drawImage,context,[bitmap,0,0]);
            return same(read(context,0,0,2,2),[4,-3,0.125,1,4,-3,0.125,1,2,-1,0.5,1,2,-1,0.5,1]);
        } finally { bitmap.close(); }
    });
    await check('Canvas and ImageBitmap snapshots keep independent HDR storage', async () => {
        const [canvas,context]=make(1,1,{colorType:'float16'});
        put(context,floats([2,-1,0.5,1]),0,0);
        const bitmap=await owner.createImageBitmap(canvas);
        context.clearRect(0,0,1,1);
        const copy=await owner.createImageBitmap(bitmap);
        bitmap.close();
        try { Reflect.apply(proto.drawImage,context,[copy,0,0]);return same(read(context),[2,-1,0.5,1]); }
        finally { copy.close(); }
    });
    await check('semitransparent Canvas and ImageBitmap clones avoid double premultiplication', async () => {
        const [canvas,context]=make(1,1,{colorType:'float16'});
        put(context,floats([2,-1,0.5,0.25]),0,0);
        const bitmap=await owner.createImageBitmap(canvas);
        context.clearRect(0,0,1,1);
        const copy=await owner.createImageBitmap(bitmap);
        bitmap.close();
        try {Reflect.apply(proto.drawImage,context,[copy,0,0]);return same(read(context),[2,-1,0.5,0.25]);}
        finally {copy.close();}
    });
    await check('drawImage alpha composites without clipping HDR channels', async () => {
        const bitmap=await owner.createImageBitmap(floats([4,-3,0.25,0.5]));
        try {
            const [,context]=make(1,1,{colorType:'float16'});
            put(context,floats([2,-1,0.5,1]),0,0);
            context.globalAlpha=0.5;Reflect.apply(proto.drawImage,context,[bitmap,0,0]);
            return same(read(context),[2.5,-1.5,0.4375,1]);
        } finally { bitmap.close(); }
    });
    await check('opaque drawing honors source alpha while keeping output alpha fixed', async () => {
        const bitmap=await owner.createImageBitmap(floats([2,-1,0.5,0.5]));
        try {
            const [,context]=make(1,1,{colorType:'float16',alpha:false});
            Reflect.apply(proto.drawImage,context,[bitmap,0,0]);
            return same(read(context),[1,-0.5,0.25,1]);
        } finally {bitmap.close();}
    });
    await check('resizing retains context settings and resets the bitmap', () => {
        const [canvas,context]=make(2,1,{colorSpace:'display-p3',colorType:'float16',alpha:false});
        put(context,floats([2,-1,0.5,1],'display-p3'),0,0);context.fillStyle='red';
        canvas.width=3;
        const attributes=Reflect.apply(proto.getContextAttributes,context,[]);
        return attributes.colorSpace==='display-p3' && attributes.colorType==='float16' && attributes.alpha===false && context.fillStyle==='#000000' && same(read(context,0,0,3,1,'display-p3'),[0,0,0,1,0,0,0,1,0,0,0,1]);
    });
    if(kind==='offscreen')await check('transferToImageBitmap retains HDR and initializes a new bitmap', () => {
        const [canvas,context]=make(1,1,{colorSpace:'display-p3',colorType:'float16'});
        put(context,floats([2,-1,0.5,1],'display-p3'),0,0);context.fillStyle='red';
        const bitmap=canvas.transferToImageBitmap();
        try {
            if(!same(read(context,0,0,1,1,'display-p3'),[0,0,0,0]) || context.fillStyle!=='#ff0000')return false;
            Reflect.apply(proto.drawImage,context,[bitmap,0,0]);
            return same(read(context,0,0,1,1,'display-p3'),[2,-1,0.5,1]);
        } finally { bitmap.close(); }
    });
    if(kind==='offscreen')await check('transfer preserves semitransparent storage and replacement alpha mode', () => {
        const [canvas,context]=make(1,1,{colorType:'float16'});
        put(context,floats([2,-1,0.5,0.25]),0,0);
        const bitmap=canvas.transferToImageBitmap();
        try {
            if(!same(read(context),[0,0,0,0]))return false;
            Reflect.apply(proto.drawImage,context,[bitmap,0,0]);
            if(!same(read(context),[2,-1,0.5,0.25]))return false;
            put(context,floats([3,-2,0.5,0]),0,0);
            return same(read(context),[0,0,0,0]);
        } finally {bitmap.close();}
    });
    return checks;
}
