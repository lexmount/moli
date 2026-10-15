(async (owner, callee, label) => {
    const checks = [];
    const assert = (condition, message = 'assertion failed') => { if (!condition) throw Error(message); };
    const check = async (name, body) => {
        try { await body(); checks.push({name: label + ': ' + name, passed: true}); }
        catch (error) { checks.push({name: label + ': ' + name, passed: false, error: String(error)}); }
    };
    const expectError = (name, body) => {
        let error;
        try { body(); } catch (caught) { error = caught; }
        assert(error instanceof callee.DOMException && error.name === name, String(error));
    };
    const make = (width = 3, height = 2) => new owner.OffscreenCanvas(width, height);
    const clone = (data, transfer = []) => callee.structuredClone(data, {transfer});
    const width = Object.getOwnPropertyDescriptor(callee.OffscreenCanvas.prototype, 'width');
    const height = Object.getOwnPropertyDescriptor(callee.OffscreenCanvas.prototype, 'height');
    const getContext = callee.OffscreenCanvas.prototype.getContext;
    const convertToBlob = callee.OffscreenCanvas.prototype.convertToBlob;
    const bitmap = callee.OffscreenCanvas.prototype.transferToImageBitmap;
    const dimensions = (canvas, w, h) => assert(width.get.call(canvas) === w && height.get.call(canvas) === h);
    const detached = canvas => expectError('InvalidStateError', () => getContext.call(canvas, '2d'));
    for (const [w, h] of [[3,2], [0,4], [4,0], [0,0], [0xffffffff,0], [0,0xffffffff]]) {
        await check('transfer dimensions ' + w + 'x' + h, () => {
            const source = make(w, h), sw = source.width, sh = source.height;
            const result = clone(source, [source]);
            dimensions(result, sw, sh); dimensions(source, 0, 0);
            assert(Object.getPrototypeOf(result) === callee.OffscreenCanvas.prototype);
            detached(source);
        });
    }
    await check('received canvas acquires a fresh drawing context', () => {
        const source = make(), result = clone(source, [source]);
        const context = getContext.call(result, '2d');
        assert(context.canvas === result);
        assert(context instanceof callee.OffscreenCanvasRenderingContext2D);
        assert([...context.getImageData(0,0,1,1).data].join() === '0,0,0,0');
        context.fillStyle = 'lime'; context.fillRect(0,0,1,1);
        assert([...context.getImageData(0,0,1,1).data].join() === '0,255,0,255');
        detached(source);
    });
    await check('cyclic graphs maps sets and aliases preserve identity', () => {
        const source = make(), graph = {source, alias: source, map: new Map([[source,source]]), set: new Set([source])};
        graph.self = graph;
        const result = clone(graph, [source]);
        assert(result.self === result && result.source === result.alias);
        assert(result.map.get(result.source) === result.source && result.set.has(result.source));
        dimensions(result.source, 3, 2);
    });
    await check('unreferenced transfers still detach', () => {
        const source = make(); assert(clone(17, [source]) === 17);
        dimensions(source,0,0); detached(source);
    });
    await check('several transfers retain their own list indices', () => {
        const a = make(2,3), b = make(4,5), c = make(6,7);
        const result = clone({a,c,b}, [b,a,c]);
        dimensions(result.a,2,3); dimensions(result.b,4,5); dimensions(result.c,6,7);
    });
    await check('transfer may be forwarded before context acquisition', () => {
        const source = make(), first = clone(source,[source]), second = clone(first,[first]);
        dimensions(second,3,2); detached(source); detached(first);
    });
    await check('source expandos are not serialized', () => {
        const source = make(); let reads = 0;
        Object.defineProperty(source,'extra',{get(){reads++;throw Error('expando');},enumerable:true});
        const result = clone(source,[source]); assert(reads === 0 && !('extra' in result));
    });
    await check('native dimensions ignore poisoned public getters', () => {
        const source = make(); let reads = 0;
        for (const key of ['width','height']) Object.defineProperty(source,key,{get(){reads++;throw Error(key);}});
        const result = clone(source,[source]); dimensions(result,3,2); dimensions(source,0,0); assert(reads === 0);
    });
    await check('receiving does not call a replaced global constructor', () => {
        const source = make(), ctor = callee.OffscreenCanvas, prototype = ctor.prototype;
        try {
            callee.OffscreenCanvas = function(){throw Error('author constructor');};
            const result = clone(source,[source]);
            assert(Object.getPrototypeOf(result) === prototype); dimensions(result,3,2);
        } finally { callee.OffscreenCanvas = ctor; }
    });
    await check('OffscreenCanvas is transferable but not cloneable', () => {
        const source = make(); expectError('DataCloneError', () => clone({source})); dimensions(source,3,2);
    });
    await check('duplicate transfers reject before message getters', () => {
        const source = make(); let reads = 0;
        expectError('DataCloneError', () => clone({get value(){reads++;}}, [source,source]));
        assert(reads === 0); dimensions(source,3,2);
    });
    for (const kind of ['2d','bitmaprenderer','webgl','webgl2']) {
        await check('context-mode ' + kind + ' rejects transfer after message getters', () => {
            const source = make(); let reads = 0;
            assert(getContext.call(source, kind) !== null);
            expectError('InvalidStateError', () => clone({get value(){reads++;return source;}},[source]));
            assert(reads === 1); dimensions(source,3,2); assert(getContext.call(source,kind) !== null);
        });
    }
    await check('message serialization failure does not detach', () => {
        const source = make(), buffer = new owner.ArrayBuffer(1);
        expectError('DataCloneError', () => clone({source,bad:()=>{}},[buffer,source]));
        assert(buffer.byteLength === 1); dimensions(source,3,2); assert(getContext.call(source,'2d') !== null);
    });
    await check('message getter exceptions keep identity and sender state', () => {
        const source = make(), sentinel = {}; let error;
        try { clone({get value(){throw sentinel;}},[source]); } catch (caught) { error = caught; }
        assert(error === sentinel); dimensions(source,3,2);
    });
    await check('message getters resize before transfer snapshot', () => {
        const source = make();
        const result = clone({source,get value(){width.set.call(source,7);height.set.call(source,8);return 1;}},[source]);
        dimensions(result.source,7,8); dimensions(source,0,0);
    });
    await check('message getters may acquire a context before transfer steps', () => {
        const source = make();
        expectError('InvalidStateError', () => clone({get value(){getContext.call(source,'2d');return 1;}},[source]));
        dimensions(source,3,2); assert(getContext.call(source,'2d') !== null);
    });
    await check('reentrant transfer during a message getter is detected', () => {
        const source = make(); let inner;
        expectError('DataCloneError', () => clone({source,get value(){inner = clone(source,[source]);return 1;}},[source]));
        dimensions(inner,3,2); dimensions(source,0,0); detached(source);
    });
    await check('detached transfer is checked after message getters', () => {
        const source = make(); clone(source,[source]); let reads = 0;
        expectError('DataCloneError', () => clone({get value(){reads++;}},[source])); assert(reads === 1);
    });
    for (const reverse of [false,true]) {
        await check('partial ArrayBuffer transfer respects list order ' + reverse, () => {
            const source = make(), buffer = new owner.ArrayBuffer(1); getContext.call(source,'2d');
            expectError('InvalidStateError', () => clone(1, reverse ? [source,buffer] : [buffer,source]));
            assert(buffer.byteLength === (reverse ? 1 : 0)); dimensions(source,3,2);
        });
        await check('partial OffscreenCanvas transfer respects list order ' + reverse, () => {
            const source = make(), busy = make(); getContext.call(busy,'2d');
            expectError('InvalidStateError', () => clone(1, reverse ? [busy,source] : [source,busy]));
            dimensions(source,reverse?3:0,reverse?2:0); if (!reverse) detached(source);
        });
    }
    await check('transfer iterable converts once before message getters', () => {
        const source = make(), order = [];
        const transfer = {[Symbol.iterator](){order.push('iterator');let done=false;return {next(){order.push('next');if(done)return {done:true};done=true;return {value:source,done:false};}};}};
        const result = callee.structuredClone({source,get value(){order.push('getter');return 1;}},{transfer});
        assert(order.join() === 'iterator,next,next,getter'); dimensions(result.source,3,2);
    });
    await check('resizing a detached canvas does not reattach it', () => {
        const source = make(); clone(source,[source]); width.set.call(source,4);height.set.call(source,5);
        dimensions(source,4,5); detached(source);
        expectError('DataCloneError', () => clone(source,[source]));
    });
    for (const kind of ['2d','bitmaprenderer','webgl','webgl2','webgpu']) {
        await check('detached getContext ' + kind, () => {
            const source = make(); clone(source,[source]);
            expectError('InvalidStateError', () => getContext.call(source,kind));
        });
    }
    await check('getContext conversion can detach the receiver', () => {
        const source = make();
        expectError('InvalidStateError', () => getContext.call(source,{toString(){clone(source,[source]);return '2d';}}));
    });
    await check('detached transferToImageBitmap rejects', () => {
        const source = make(); clone(source,[source]);
        expectError('InvalidStateError', () => bitmap.call(source));
    });
    await check('detached convertToBlob rejects its Promise', async () => {
        const source = make(); clone(source,[source]); let error;
        const promise = convertToBlob.call(source); assert(promise instanceof callee.Promise);
        try { await promise; } catch (caught) { error = caught; }
        assert(error instanceof callee.DOMException && error.name === 'InvalidStateError', String(error));
    });
    await check('convertToBlob converts options before detached validation', async () => {
        const source = make(), sentinel = {}; clone(source,[source]); let error;
        const promise = convertToBlob.call(source,{get type(){throw sentinel;}});
        assert(promise instanceof callee.Promise);
        try { await promise; } catch (caught) { error = caught; } assert(error === sentinel);
    });
    for (const resized of [false,true]) {
        await check('detached native image sources reject, resized=' + resized, async () => {
            const source = make(); clone(source,[source]);
            if (resized) { width.set.call(source,3);height.set.call(source,2); }
            const target = make().getContext('2d');
            const prototype = callee.OffscreenCanvasRenderingContext2D.prototype;
            expectError('InvalidStateError', () => prototype.drawImage.call(target,source,0,0));
            expectError('InvalidStateError', () => prototype.createPattern.call(target,source,'repeat'));
            let error; try { await callee.createImageBitmap(source); } catch (caught) { error = caught; }
            assert(error instanceof callee.DOMException && error.name === 'InvalidStateError', String(error));
        });
    }
    for (const mode of ['proxy','revoked','forged']) {
        await check(mode + ' receivers cannot supply transferable brands', async () => {
            const source = make(); let reads = 0, traps = 0, candidate;
            if (mode === 'forged') candidate = Object.create(source);
            else if (mode === 'revoked') { const pair = Proxy.revocable(source,{}); pair.revoke(); candidate = pair.proxy; }
            else candidate = new Proxy(source,{get(){traps++;throw Error('trap');}});
            expectError('DataCloneError', () => clone({get value(){reads++;}},[candidate]));
            assert(reads === 0 && traps === 0); dimensions(source,3,2);
            let conversions = 0;
            const poison = {valueOf(){conversions++;return 1;},toString(){conversions++;return '2d';}};
            for (const invoke of [() => width.get.call(candidate), () => width.set.call(candidate,poison), () => getContext.call(candidate,poison)]) {
                let error; try { invoke(); } catch (caught) { error = caught; }
                assert(error instanceof callee.TypeError, String(error));
            }
            let error;
            const promise = convertToBlob.call(candidate,{get type(){conversions++;return 'image/png';}});
            assert(promise instanceof callee.Promise);
            try { await promise; } catch (caught) { error = caught; }
            assert(error instanceof callee.TypeError && conversions === 0 && traps === 0, String(error));
        });
    }
    const carriers = [['structuredClone', (value,transfer) => clone(value,transfer)], ['MessagePort', (value,transfer) => {
        const channel = new callee.MessageChannel();
        return new Promise((resolve,reject) => {
            channel.port2.onmessage = event => { channel.port1.close();channel.port2.close();resolve(event.data); };
            try { channel.port1.postMessage(value,transfer); }
            catch(error) { channel.port1.close();channel.port2.close();reject(error); }
        });
    }]];
    if (typeof callee.document !== 'undefined') carriers.push(['Window', (value,transfer) => {
        const tag = label + ':' + checks.length;
        return new Promise((resolve,reject) => {
            const listener = event => {
                if (event.data && event.data.tag === tag) { callee.removeEventListener('message',listener);resolve(event.data.value); }
            };
            callee.addEventListener('message',listener);
            try { callee.postMessage({tag,value},'*',transfer); }
            catch(error) { callee.removeEventListener('message',listener);reject(error); }
        });
    }]);
    for (const [name,send] of carriers) {
        await check(name + ' delivers typed aliases and context ownership', async () => {
            const source = make(), result = await send({a:source,b:source},[source]);
            assert(result.a === result.b && result.a instanceof callee.OffscreenCanvas);
            dimensions(result.a,3,2); dimensions(source,0,0);
            assert(getContext.call(result.a,'2d').canvas === result.a);
        });
        await check(name + ' transfers an unreferenced canvas', async () => {
            const source = make(); assert(await send(9,[source]) === 9); dimensions(source,0,0);detached(source);
        });
    }
    return checks;
})
