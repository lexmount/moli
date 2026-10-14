async (owner, callee, label) => {
    const checks = [];
    const check = async (name, run) => {
        try {
            await run();
            checks.push({name: label + ': ' + name, passed: true});
        } catch (error) {
            checks.push({name: label + ': ' + name, passed: false, error: String(error)});
        }
    };
    const assert = (value, message = 'assertion failed') => {
        if (!value) throw new Error(message);
    };
    const same = (actual, expected) => actual.length === expected.length &&
        actual.every((value, index) => Object.is(value, expected[index]) || value === expected[index]);
    const throws = (run, expected = 'DataCloneError') => {
        try { run(); } catch (error) {
            assert(error instanceof callee.DOMException && error.name === expected, String(error));
            return;
        }
        throw new Error('expected ' + expected);
    };
    const make = (colorSpace = 'srgb', colorType = 'unorm8') => {
        const canvas = new owner.OffscreenCanvas(2, 1);
        const context = canvas.getContext('2d', {colorSpace, colorType});
        const data = colorType === 'float16'
            ? new owner.Float16Array([1.25, -0.25, 0.125, 0.5, 0.25, 0.5, 0.75, 1])
            : new owner.Uint8ClampedArray([17, 34, 51, 255, 255, 0, 128, 128]);
        context.putImageData(new owner.ImageData(data, 2, 1, {
            colorSpace, pixelFormat: colorType === 'float16' ? 'rgba-float16' : 'rgba-unorm8'
        }), 0, 0);
        return canvas.transferToImageBitmap();
    };
    const pixels = (bitmap, colorSpace = 'srgb', colorType = 'unorm8') => {
        const context = new callee.OffscreenCanvas(2, 1).getContext('2d', {colorSpace, colorType});
        context.drawImage(bitmap, 0, 0);
        return Array.from(context.getImageData(0, 0, 2, 1, {
            colorSpace, pixelFormat: colorType === 'float16' ? 'rgba-float16' : 'rgba-unorm8'
        }).data);
    };
    for (const colorSpace of ['srgb', 'display-p3']) {
        for (const colorType of ['unorm8', 'float16']) {
            for (const transfer of [false, true]) {
                await check(`${transfer ? 'transfer' : 'clone'} ${colorSpace} ${colorType}`, () => {
                    const bitmap = make(colorSpace, colorType);
                    const expected = pixels(bitmap, colorSpace, colorType);
                    const copy = callee.structuredClone(bitmap, {transfer: transfer ? [bitmap] : []});
                    assert(Object.getPrototypeOf(copy) === callee.ImageBitmap.prototype, 'receiving realm');
                    assert(copy.width === 2 && copy.height === 1);
                    assert(bitmap.width === (transfer ? 0 : 2) && bitmap.height === (transfer ? 0 : 1));
                    bitmap.close();
                    assert(same(pixels(copy, colorSpace, colorType), expected), 'exact pixels');
                    if (colorType === 'float16') assert(expected[0] > 1 && expected[1] < 0, 'HDR gamut retained');
                    copy.close();
                });
            }
        }
    }
    await check('graph cycles and repeated identity', () => {
        const bitmap = make(), graph = {first: bitmap, second: bitmap, map: new owner.Map([[bitmap, bitmap]])};
        graph.self = graph;
        const copy = callee.structuredClone(graph);
        assert(copy.self === copy && copy.first === copy.second && copy.map.get(copy.first) === copy.first);
        assert(copy.first !== bitmap);
        bitmap.close(); copy.first.close();
    });
    await check('transfer graph preserves repeated identity', () => {
        const bitmap = make(), copy = callee.structuredClone([bitmap, bitmap], {transfer: [bitmap]});
        assert(copy[0] === copy[1] && copy[0].width === 2 && bitmap.width === 0);
        copy[0].close();
    });
    await check('unreferenced transfers detach', () => {
        const bitmap = make();
        assert(callee.structuredClone(3, {transfer: [bitmap]}) === 3 && bitmap.width === 0);
    });
    await check('native slots ignore author geometry and prototype mutations', () => {
        const bitmap = make(), expected = pixels(bitmap);
        Object.defineProperties(bitmap, {width: {get() {throw new Error('author width');}}, height: {value: 999}});
        Object.setPrototypeOf(bitmap, null);
        Object.freeze(bitmap);
        const copy = callee.structuredClone(bitmap);
        assert(copy.width === 2 && copy.height === 1 && same(pixels(copy), expected));
        owner.ImageBitmap.prototype.close.call(bitmap); copy.close();
    });
    await check('closed bitmap rejects ordinary cloning', () => {
        const bitmap = make(); bitmap.close();
        throws(() => callee.structuredClone(bitmap));
    });
    await check('closed transfer is rejected after message serialization', () => {
        const bitmap = make(); bitmap.close();
        const sentinel = new Error('getter wins');
        let caught;
        try { callee.structuredClone({bitmap, get value() {throw sentinel;}}, {transfer: [bitmap]}); }
        catch (error) { caught = error; }
        assert(caught === sentinel);
        throws(() => callee.structuredClone(bitmap, {transfer: [bitmap]}));
    });
    await check('getter closes referenced transfer before transfer steps', () => {
        const bitmap = make(), buffer = new owner.ArrayBuffer(4);
        throws(() => callee.structuredClone({bitmap, get value() {bitmap.close(); return 1;}}, {transfer: [bitmap, buffer]}));
        assert(buffer.byteLength === 4);
    });
    await check('getter closes unreferenced transfer before transfer steps', () => {
        const bitmap = make(), buffer = new owner.ArrayBuffer(4);
        throws(() => callee.structuredClone({get value() {bitmap.close(); return 1;}}, {transfer: [bitmap, buffer]}));
        assert(buffer.byteLength === 4);
    });
    await check('serialization failure leaves all senders usable', () => {
        const bitmap = make(), buffer = new owner.ArrayBuffer(4);
        throws(() => callee.structuredClone({bitmap, bad() {}}, {transfer: [bitmap, buffer]}));
        assert(bitmap.width === 2 && buffer.byteLength === 4);
        bitmap.close();
    });
    await check('duplicate transfer rejects before message getters', () => {
        const bitmap = make(); let reads = 0;
        throws(() => callee.structuredClone({get value() {reads++;}}, {transfer: [bitmap, bitmap]}));
        assert(reads === 0 && bitmap.width === 2); bitmap.close();
    });
    await check('author proxies and forged prototypes have no native brand', () => {
        const bitmap = make(); let traps = 0;
        const revoked = owner.Proxy.revocable(bitmap, {}); revoked.revoke();
        for (const value of [new owner.Proxy(bitmap, {get() {traps++;}}), revoked.proxy,
            Object.create(bitmap), Object.create(owner.ImageBitmap.prototype)]) {
            throws(() => callee.structuredClone(1, {transfer: [value]}));
        }
        throws(() => callee.structuredClone(new owner.Proxy(bitmap, {get() {traps++;}})));
        assert(traps === 0 && bitmap.width === 2); bitmap.close();
    });
    await check('ArrayBuffer bytes are captured after message getters', () => {
        const bitmap = make(), bytes = new owner.Uint8Array([1, 2]);
        const copy = callee.structuredClone({bitmap, bytes, get mutate() {bytes[0] = 42;}}, {transfer: [bitmap, bytes.buffer]});
        assert(copy.bytes[0] === 42 && copy.bitmap.width === 2 && bytes.byteLength === 0);
        copy.bitmap.close();
    });
    await check('detached ArrayBuffer is checked after message getters', () => {
        const bitmap = make(), buffer = new owner.ArrayBuffer(4), sentinel = new Error('getter wins');
        owner.structuredClone(buffer, {transfer: [buffer]});
        let caught;
        try { callee.structuredClone({bitmap, get value() {throw sentinel;}}, {transfer: [buffer, bitmap]}); }
        catch (error) { caught = error; }
        assert(caught === sentinel && bitmap.width === 2);
        bitmap.close();
    });
    await check('getter detaches ArrayBuffer before mixed transfer steps', () => {
        const bitmap = make(), buffer = new owner.ArrayBuffer(4);
        throws(() => callee.structuredClone({bitmap, get value() {
            owner.structuredClone(buffer, {transfer: [buffer]}); return 1;
        }}, {transfer: [buffer, bitmap]}));
        assert(bitmap.width === 2);
        bitmap.close();
    });
    for (const bufferFirst of [false, true]) {
        await check('transfer failure respects list order: buffer first ' + bufferFirst, () => {
            const bitmap = make(), buffer = new owner.ArrayBuffer(4); bitmap.close();
            throws(() => callee.structuredClone({bitmap}, {transfer: bufferFirst ? [buffer, bitmap] : [bitmap, buffer]}));
            assert(buffer.byteLength === (bufferFirst ? 0 : 4));
        });
    }
    await check('MessageChannel round trip clones and transfers', async () => {
        const {port1, port2} = new owner.MessageChannel();
        try {
            for (const transfer of [false, true]) {
                const bitmap = make('display-p3', 'float16'), expected = pixels(bitmap, 'display-p3', 'float16');
                const received = new Promise(resolve => {port2.onmessage = event => resolve(event.data);});
                port1.postMessage([bitmap, bitmap], transfer ? [bitmap] : []);
                const copy = await received;
                assert(copy[0] === copy[1] && same(pixels(copy[0], 'display-p3', 'float16'), expected));
                assert(bitmap.width === (transfer ? 0 : 2));
                bitmap.close(); copy[0].close();
            }
        } finally { port1.close(); port2.close(); }
    });
    await check('IndexedDB stores independent bitmap snapshots', async () => {
        const request = req => new Promise((resolve, reject) => {
            req.onsuccess = () => resolve(req.result);
            req.onerror = () => reject(req.error);
        });
        const name = 'bitmap-clone-' + label;
        const open = owner.indexedDB.open(name, 1);
        open.onupgradeneeded = () => open.result.createObjectStore('bitmaps');
        const database = await request(open);
        try {
            const bitmap = make('display-p3', 'float16'), expected = pixels(bitmap, 'display-p3', 'float16');
            const transaction = database.transaction('bitmaps', 'readwrite');
            const complete = new Promise((resolve, reject) => {
                transaction.oncomplete = resolve;
                transaction.onabort = () => reject(transaction.error);
            });
            transaction.objectStore('bitmaps').put([bitmap, bitmap], 'bitmap');
            bitmap.close();
            await complete;
            const store = database.transaction('bitmaps').objectStore('bitmaps');
            const first = request(store.get('bitmap')), second = request(store.get('bitmap'));
            const [a, b] = await Promise.all([first, second]);
            assert(a[0] === a[1] && b[0] === b[1] && a[0] !== b[0]);
            assert(Object.getPrototypeOf(a[0]) === owner.ImageBitmap.prototype);
            a[0].close();
            assert(same(pixels(b[0], 'display-p3', 'float16'), expected));
            b[0].close();
        } finally { database.close(); await request(owner.indexedDB.deleteDatabase(name)); }
    });
    if (typeof owner.document !== 'undefined') {
        await check('structuredClone uses the receiving Window realm', () => {
            const bitmap = make();
            const copy = callee.structuredClone.call(owner, bitmap);
            assert(Object.getPrototypeOf(copy) === owner.ImageBitmap.prototype);
            bitmap.close(); copy.close();
        });
        await check('history storage preserves bitmap data and graph identity', () => {
            const bitmap = make('display-p3', 'float16'), expected = pixels(bitmap, 'display-p3', 'float16');
            owner.history.replaceState([bitmap, bitmap], '');
            bitmap.close();
            const copy = owner.history.state;
            assert(copy[0] === copy[1] && Object.getPrototypeOf(copy[0]) === owner.ImageBitmap.prototype);
            assert(same(pixels(copy[0], 'display-p3', 'float16'), expected));
            copy[0].close(); owner.history.replaceState(null, '');
        });
    }
    return checks;
}
